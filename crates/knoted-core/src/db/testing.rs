//! Test-only PostgreSQL helpers.
//!
//! Every test gets its own throw-away schema in the shared test database, so
//! tests can run in parallel without sharing tables and without creating or
//! dropping databases. This module is compiled into the library (integration
//! tests are separate crates and cannot use `cfg(test)` items) but it is meant
//! for test code only: production paths must never call it.
//!
//! The helper refuses any database whose name does not contain `test`, so a
//! misconfigured `KNOTED_TEST_DATABASE_URL` cannot point the test suite at
//! `knoted_dev` or `knoted_prod`.

use std::cell::Cell;
use std::fmt;
use std::str::FromStr;

use sqlx::postgres::{PgConnectOptions, PgConnection, PgPoolOptions};
use sqlx::{Connection, PgPool};
use uuid::Uuid;

use super::{DbError, migrate};

/// Environment variable holding the base URL of the test database.
pub const TEST_DATABASE_URL_ENV: &str = "KNOTED_TEST_DATABASE_URL";

/// Default base URL: the local PostgreSQL over its unix socket.
pub const DEFAULT_TEST_DATABASE_URL: &str = "postgres:///knoted_test?host=/run/postgresql";

/// Base URL of the test database, from [`TEST_DATABASE_URL_ENV`] or the default.
pub fn test_database_url() -> String {
    match std::env::var(TEST_DATABASE_URL_ENV) {
        Ok(url) if !url.trim().is_empty() => url,
        _ => DEFAULT_TEST_DATABASE_URL.to_owned(),
    }
}

/// An isolated schema with migrations applied, dropped when the test ends.
pub struct TestDb {
    /// Pool whose connections all use the isolated schema as `search_path`.
    pub pool: PgPool,
    schema: String,
    base_url: String,
    cleaned: Cell<bool>,
}

impl TestDb {
    /// Creates an isolated schema in the test database and migrates it.
    pub async fn create() -> Result<TestDb, DbError> {
        TestDb::create_from_url(&test_database_url()).await
    }

    /// Same as [`TestDb::create`] for an explicit base URL.
    ///
    /// Exposed so tests can prove that a non-test target is refused: the
    /// database name is validated before any connection is opened.
    pub async fn create_from_url(url: &str) -> Result<TestDb, DbError> {
        let options = PgConnectOptions::from_str(url)?;
        let database = options.get_database().unwrap_or_default().to_owned();
        if !database.contains("test") {
            return Err(DbError::UnsafeTestTarget(database));
        }

        let schema = format!("knoted_t05_{}", &Uuid::new_v4().simple().to_string()[..12]);
        let schema = sql_identifier(&schema);

        let mut admin = PgConnection::connect(url).await?;
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&mut admin)
            .await?;
        admin.close().await?;

        let search_path = format!("SET search_path TO {schema}");
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .after_connect(move |connection, _metadata| {
                let search_path = search_path.clone();
                Box::pin(async move {
                    sqlx::query(&search_path).execute(connection).await?;
                    Ok(())
                })
            })
            .connect(url)
            .await?;

        let db = TestDb {
            pool,
            schema,
            base_url: url.to_owned(),
            cleaned: Cell::new(false),
        };
        migrate(&db.pool).await?;
        Ok(db)
    }

    /// Name of the isolated schema backing this test.
    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// Closes the pool and drops the schema with `CASCADE`.
    ///
    /// The statement uses `IF EXISTS`, so a schema that is already gone is not
    /// an error; dropping a second time is a no-op.
    pub async fn drop(self) {
        self.cleaned.set(true);
        self.pool.close().await;
        let _ = drop_schema(&self.base_url, &self.schema).await;
    }
}

impl fmt::Debug for TestDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestDb")
            .field("schema", &self.schema)
            .finish_non_exhaustive()
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        if self.cleaned.replace(true) {
            return;
        }
        // `TestDb::drop` was never awaited (the test panicked, for example).
        // Clean up from a dedicated runtime so a failed run cannot leak its
        // schema. Best effort: the test already failed, so errors are ignored.
        let base_url = self.base_url.clone();
        let schema = self.schema.clone();
        let _ = std::thread::spawn(move || {
            if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                let _ = runtime.block_on(drop_schema(&base_url, &schema));
            }
        })
        .join();
    }
}

async fn drop_schema(base_url: &str, schema: &str) -> Result<(), sqlx::Error> {
    let mut connection = PgConnection::connect(base_url).await?;
    sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .execute(&mut connection)
        .await?;
    connection.close().await?;
    Ok(())
}

/// Guards the SQL we interpolate: only lowercase letters, digits and `_`.
///
/// Schema names are generated above, but an identifier is never interpolated
/// into a statement without this check.
fn sql_identifier(value: &str) -> String {
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
        "refusing to interpolate unsafe SQL identifier `{value}`"
    );
    value.to_owned()
}
