//! T05 acceptance tests: connection pool, embedded migrations, isolation and
//! the shared entity registry.
//!
//! These tests run against the real local PostgreSQL test database
//! (`KNOTED_TEST_DATABASE_URL`, default
//! `postgres:///knoted_test?host=/run/postgresql`). Each test gets its own
//! schema, which is dropped afterwards, so the default parallel test harness is
//! safe. A missing database is a test failure, never a skip.

use std::time::Duration;

use knoted_core::config::DatabaseConfig;
use knoted_core::db::testing::{TestDb, test_database_url};
use knoted_core::db::{self, DbError, SourceIdentity};
use sqlx::Connection;
use sqlx::postgres::PgConnection;

const UNREACHABLE_HINT: &str = "the local PostgreSQL test database must be running \
     (KNOTED_TEST_DATABASE_URL, default postgres:///knoted_test?host=/run/postgresql)";

async fn applied_migrations(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(pool)
        .await
        .expect("the migration bookkeeping table must be readable")
}

#[tokio::test]
async fn t05_1_empty_database_migrates() {
    let db = TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("empty schema must migrate: {error}; {UNREACHABLE_HINT}"));

    assert!(
        db::is_ready(&db.pool)
            .await
            .expect("is_ready must not fail"),
        "a freshly migrated schema must be ready"
    );

    // The registry table accepts one row per identity type.
    for source in SourceIdentity::ALL {
        let id = db::register_source(&db.pool, source)
            .await
            .expect("register_source must insert");
        assert_eq!(
            db::lookup_source(&db.pool, id)
                .await
                .expect("lookup_source"),
            Some(source)
        );
    }

    db::migrate(&db.pool)
        .await
        .expect("migrating an up-to-date schema is a no-op");
    assert!(
        db::is_ready(&db.pool)
            .await
            .expect("is_ready must not fail")
    );
    assert_eq!(
        applied_migrations(&db.pool).await,
        db::embedded_migration_count() as i64,
        "every embedded migration must be applied exactly once"
    );
    assert!(
        db::embedded_migration_count() >= 1,
        "the entity registry migration must stay embedded"
    );

    db.drop().await;
}

#[tokio::test]
async fn t05_1_connect_uses_config() {
    let config = DatabaseConfig {
        url: test_database_url(),
        max_connections: 2,
        connect_timeout_secs: 5,
    };
    let pool = db::connect(&config).await.unwrap_or_else(|error| {
        panic!("connect must reach the test database: {error}; {UNREACHABLE_HINT}")
    });

    assert_eq!(pool.options().get_max_connections(), 2);
    assert_eq!(
        pool.options().get_acquire_timeout(),
        Duration::from_secs(5),
        "connect_timeout_secs must bound connection acquisition"
    );
    let one: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&pool)
        .await
        .expect("the pool must answer a query");
    assert_eq!(one, 1);
    pool.close().await;

    // A bad URL fails at connect time with a database error, not a panic.
    let bogus = DatabaseConfig {
        url: "postgres://knoted_test@127.0.0.1:1/knoted_test".to_owned(),
        max_connections: 2,
        connect_timeout_secs: 5,
    };
    let error = db::connect(&bogus)
        .await
        .expect_err("an unreachable database must fail");
    assert!(
        matches!(error, DbError::Sqlx(_)),
        "unexpected error: {error:?}"
    );
    assert!(
        error.to_string().starts_with("database error: "),
        "unexpected message: {error}"
    );
}

#[tokio::test]
async fn t05_2_failed_migration_blocks_ready() {
    let db = TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"));

    // Simulate a schema that already holds an incompatible `entity_registry`
    // and has no record of migration 0001.
    sqlx::query("DROP TABLE entity_registry CASCADE")
        .execute(&db.pool)
        .await
        .expect("dropping the migrated table must work");
    sqlx::query("CREATE TABLE entity_registry (legacy_id text PRIMARY KEY)")
        .execute(&db.pool)
        .await
        .expect("the legacy table must be creable");
    sqlx::query("DROP TABLE _sqlx_migrations CASCADE")
        .execute(&db.pool)
        .await
        .expect("dropping the bookkeeping table must work");

    // No migrations applied and no bookkeeping table: not ready, no error.
    assert!(
        !db::is_ready(&db.pool)
            .await
            .expect("is_ready must not fail"),
        "a schema without migrations must not be ready"
    );

    let error = db::migrate(&db.pool)
        .await
        .expect_err("0001 collides with the legacy table");
    assert!(
        matches!(error, DbError::Migrate(_)),
        "unexpected error: {error:?}"
    );
    assert!(
        error.to_string().starts_with("migration failed: "),
        "unexpected message: {error}"
    );
    assert!(
        !db::is_ready(&db.pool)
            .await
            .expect("is_ready must not fail"),
        "a failed migration must leave the database unready"
    );

    db.drop().await;
}

#[tokio::test]
async fn t05_2_migration_state_from_another_binary_blocks_ready() {
    let db = TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"));
    assert!(db::is_ready(&db.pool).await.expect("is_ready"));

    // A newer binary applied a migration this build does not know about: an old
    // process must not claim readiness against that schema.
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES ($1, $2, true, $3, 0)",
    )
    .bind(9_999_i64)
    .bind("synthetic migration from a newer binary")
    .bind(vec![0_u8; 48])
    .execute(&db.pool)
    .await
    .expect("a synthetic migration row must be insertable");
    assert!(
        !db::is_ready(&db.pool)
            .await
            .expect("is_ready must not fail"),
        "an applied migration unknown to this binary must block readiness"
    );

    // The same holds when an applied migration was edited after it ran.
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = $1")
        .bind(9_999_i64)
        .execute(&db.pool)
        .await
        .expect("the synthetic row must be removable");
    assert!(db::is_ready(&db.pool).await.expect("is_ready"));
    sqlx::query("UPDATE _sqlx_migrations SET checksum = $1")
        .bind(vec![1_u8; 48])
        .execute(&db.pool)
        .await
        .expect("the checksum must be updatable");
    assert!(
        !db::is_ready(&db.pool)
            .await
            .expect("is_ready must not fail"),
        "a modified migration must block readiness"
    );

    db.drop().await;
}

#[tokio::test]
async fn t05_3_registries_are_isolated() {
    let first = TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("first schema failed: {error}; {UNREACHABLE_HINT}"));
    let second = TestDb::create()
        .await
        .expect("second schema in the same database must exist");

    assert_ne!(first.schema(), second.schema());
    assert_eq!(
        applied_migrations(&first.pool).await,
        db::embedded_migration_count() as i64
    );
    assert_eq!(
        applied_migrations(&second.pool).await,
        db::embedded_migration_count() as i64
    );

    let first_id = db::register_source(&first.pool, SourceIdentity::Note)
        .await
        .expect("register in the first schema");
    assert_eq!(
        db::lookup_source(&first.pool, first_id)
            .await
            .expect("lookup in the first schema"),
        Some(SourceIdentity::Note)
    );
    assert_eq!(
        db::lookup_source(&second.pool, first_id)
            .await
            .expect("lookup in the second schema"),
        None,
        "a row in one schema must be invisible in the other"
    );

    let first_schema = first.schema().to_owned();
    first.drop().await;

    let leftover: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_namespace WHERE nspname = $1")
        .bind(&first_schema)
        .fetch_one(&second.pool)
        .await
        .expect("pg_namespace must be readable");
    assert_eq!(leftover, 0, "dropping a TestDb must remove its schema");

    let second_id = db::register_source(&second.pool, SourceIdentity::File)
        .await
        .expect("the second schema stays usable");
    assert_eq!(
        db::lookup_source(&second.pool, second_id)
            .await
            .expect("lookup after dropping the other schema"),
        Some(SourceIdentity::File)
    );
    assert!(db::is_ready(&second.pool).await.expect("is_ready"));

    second.drop().await;
}

#[tokio::test]
async fn t05_3_drop_without_await_removes_schema() {
    let db = TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"));
    let schema = db.schema().to_owned();

    // `TestDb::drop` is async; a test that returns early or panics never awaits
    // it. Dropping the value must still remove the schema.
    std::mem::drop(db);

    let mut connection = PgConnection::connect(&test_database_url())
        .await
        .unwrap_or_else(|error| panic!("observer connection failed: {error}; {UNREACHABLE_HINT}"));
    let leftover: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_namespace WHERE nspname = $1")
        .bind(&schema)
        .fetch_one(&mut connection)
        .await
        .expect("pg_namespace must be readable");
    connection.close().await.expect("the observer must close");

    assert_eq!(
        leftover, 0,
        "dropping a TestDb without awaiting `drop` must not leak schema `{schema}`"
    );
}

#[tokio::test]
async fn t05_1_source_identity_round_trips() {
    for (source, name) in [
        (SourceIdentity::Note, "note"),
        (SourceIdentity::File, "file"),
        (SourceIdentity::Project, "project"),
        (SourceIdentity::Task, "task"),
        (SourceIdentity::ChatMessage, "chat_message"),
    ] {
        assert_eq!(source.as_str(), name);
        assert_eq!(
            SourceIdentity::parse(name).expect("known types parse"),
            source
        );
    }

    let error = SourceIdentity::parse("unknown").expect_err("unknown types are rejected");
    assert!(
        matches!(&error, DbError::UnknownSourceType(value) if value == "unknown"),
        "unexpected error: {error:?}"
    );
    assert_eq!(error.to_string(), "unknown source identity type `unknown`");

    let db = TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"));
    let rejected =
        sqlx::query("INSERT INTO entity_registry (id, entity_type) VALUES ($1, 'bogus')")
            .bind(uuid::Uuid::new_v4())
            .execute(&db.pool)
            .await;
    assert!(
        rejected.is_err(),
        "the check constraint must reject unknown entity types"
    );

    db.drop().await;
}

#[tokio::test]
async fn t05_4_refuses_non_test_databases() {
    // Validation happens before any connection, so these URLs cannot touch the
    // real development or production databases.
    let error = TestDb::create_from_url("postgres:///knoted_dev?host=/run/postgresql")
        .await
        .expect_err("knoted_dev must be refused");
    assert!(
        matches!(&error, DbError::UnsafeTestTarget(name) if name == "knoted_dev"),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        error.to_string(),
        "refusing to use `knoted_dev` as a test database: name must contain `test`"
    );

    for url in [
        "postgres:///knoted_prod?host=/run/postgresql",
        "postgres://knoted@127.0.0.1:5432/knoted",
    ] {
        assert!(
            matches!(
                TestDb::create_from_url(url).await.expect_err(url),
                DbError::UnsafeTestTarget(_)
            ),
            "{url} must be refused"
        );
    }
}
