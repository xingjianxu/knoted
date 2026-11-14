//! Test-only PostgreSQL helpers.
//!
//! Every test gets its own throw-away schema in the shared test database, so
//! tests can run in parallel without sharing tables and without creating or
//! dropping databases. This module is compiled into the library (integration
//! tests are separate crates and cannot use `cfg(test)` items) but it is meant
//! for test code only: production paths must never call it.
//!
//! The helper refuses any database whose name does not end with `_test`, so a
//! misconfigured `KNOTED_TEST_DATABASE_URL` cannot point the test suite at
//! `knoted_dev`, `knoted_prod` or a look-alike such as `knoted_latest`.

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
        // Require the `_test` suffix: `contains("test")` would also accept a
        // real-looking name such as `knoted_latest`.
        if !database.ends_with("_test") {
            return Err(DbError::UnsafeTestTarget(database));
        }

        let schema = sql_identifier(&format!(
            "knoted_test_{}",
            &Uuid::new_v4().simple().to_string()[..12]
        ));

        create_schema(url, &schema).await?;
        // The schema now exists: from here on `guard` removes it again if any
        // later setup step fails, so a failed pool or migration leaves nothing
        // behind. On success `TestDb` owns the cleanup instead.
        let guard = SchemaGuard::new(url, schema.clone());

        // `public` stays on the path so extension types installed there
        // (pgvector `vector`, pg_trgm) remain visible to schema-scoped tests.
        let search_path = format!("SET search_path TO {schema}, public");
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

        migrate(&pool).await?;

        guard.disarm();
        Ok(TestDb {
            pool,
            schema,
            base_url: url.to_owned(),
            cleaned: Cell::new(false),
        })
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
        self.pool.close().await;
        let _ = drop_schema(&self.base_url, &self.schema).await;
        // Marked only after the cleanup attempt: if this future is cancelled
        // mid-await, `Drop` still runs and retries (DROP ... IF EXISTS is safe).
        self.cleaned.set(true);
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
        // `TestDb::drop` was never awaited (the test panicked or called
        // `std::mem::drop`, for example). Clean up from a dedicated runtime so
        // a finished test cannot leak its schema.
        cleanup_schema_blocking(&self.base_url, &self.schema);
    }
}

/// Removes a schema if setup fails after the schema was created.
///
/// The guard is declared before the pool, so on an early return the pool is
/// dropped first and the schema last.
struct SchemaGuard {
    base_url: String,
    schema: String,
    armed: bool,
}

impl SchemaGuard {
    fn new(base_url: &str, schema: String) -> Self {
        SchemaGuard {
            base_url: base_url.to_owned(),
            schema,
            armed: true,
        }
    }

    /// Hands the cleanup over to [`TestDb`] once setup succeeded.
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for SchemaGuard {
    fn drop(&mut self) {
        if self.armed {
            cleanup_schema_blocking(&self.base_url, &self.schema);
        }
    }
}

async fn create_schema(base_url: &str, schema: &str) -> Result<(), sqlx::Error> {
    let mut connection = PgConnection::connect(base_url).await?;
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&mut connection)
        .await?;
    // The schema exists and is committed; a failing close must not hide that
    // fact, otherwise the caller would skip its cleanup guard.
    let _ = connection.close().await;
    Ok(())
}

async fn drop_schema(base_url: &str, schema: &str) -> Result<(), sqlx::Error> {
    let mut connection = PgConnection::connect(base_url).await?;
    sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .execute(&mut connection)
        .await?;
    connection.close().await?;
    Ok(())
}

/// Best-effort schema removal that works from a synchronous `Drop`.
///
/// A dedicated current-thread runtime is used because the caller may already be
/// inside a Tokio runtime (where `block_on` would panic) or may be unwinding.
fn cleanup_schema_blocking(base_url: &str, schema: &str) {
    let base_url = base_url.to_owned();
    let schema = schema.to_owned();
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
