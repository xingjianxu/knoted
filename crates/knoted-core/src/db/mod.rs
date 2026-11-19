//! PostgreSQL access: connection pooling, embedded schema migrations, and the
//! shared entity registry.
//!
//! SQLx is the only database dependency here and no HTTP type appears in this
//! module, per the architecture rule that `knoted-core` owns persistence while
//! the API crate only translates transport types. Queries use the runtime
//! `sqlx::query` API, never the compile-time `query!` macros, so the workspace
//! builds without a live database.

pub mod backup;
mod entity_registry;
mod error;
pub mod testing;

use std::time::Duration;

use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;

pub use entity_registry::{SourceIdentity, lookup_source, register_source};
pub use error::DbError;

use crate::config::DatabaseConfig;

/// Schema migrations embedded in the binary.
///
/// The path is relative to `crates/knoted-core`, and migration versions are
/// assigned serially by task card (`migrations/NNNN_tXX_<purpose>.sql`).
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

fn migrator() -> &'static Migrator {
    &MIGRATOR
}

/// Number of migrations embedded in this binary.
///
/// Readiness checks and tests use this instead of a hard-coded version count, so
/// adding `0002_...` cannot leave a stale expectation behind.
pub fn embedded_migration_count() -> usize {
    migrator().iter().count()
}

/// Opens a connection pool and verifies that the database answers.
///
/// `config.max_connections` sizes the pool and `config.connect_timeout_secs`
/// bounds how long a connection attempt may take, so a wrong URL fails here
/// instead of on the first request.
pub async fn connect(config: &DatabaseConfig) -> Result<PgPool, DbError> {
    let pool = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
        .connect(&config.url)
        .await?;
    sqlx::query("SELECT 1").execute(&pool).await?;
    Ok(pool)
}

/// Applies every embedded migration that is not recorded yet.
///
/// Idempotent: running it again on an up-to-date database is a no-op.
/// Migration failures are propagated unchanged so callers can distinguish them
/// from connection errors.
pub async fn migrate(pool: &PgPool) -> Result<(), DbError> {
    migrator().run(pool).await?;
    Ok(())
}

/// Reports whether the database schema matches the migrations embedded in this
/// binary.
///
/// This is the readiness primitive used by process health checks. It reports
/// `Ok(false)` (never an error or a panic) when the answer is not trustworthy:
///
/// - the database or `_sqlx_migrations` table is unreachable/missing;
/// - an embedded version is not applied, so the schema is too old;
/// - an applied row failed, so the schema is inconsistent;
/// - an applied version is unknown to this binary, so the schema was written by
///   a newer build (an old binary must not report ready against it);
/// - an applied checksum differs from the embedded migration, so the migration
///   file changed after being applied.
pub async fn is_ready(pool: &PgPool) -> Result<bool, DbError> {
    let applied: Vec<(i64, bool, Vec<u8>)> =
        match sqlx::query_as("SELECT version, success, checksum FROM _sqlx_migrations")
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows,
            Err(_) => return Ok(false),
        };

    Ok(migrations_match(&applied))
}

fn migrations_match(applied: &[(i64, bool, Vec<u8>)]) -> bool {
    let is_known = |version: i64, checksum: &[u8]| {
        migrator().iter().any(|migration| {
            migration.version == version && migration.checksum.as_ref() == checksum
        })
    };

    // Every applied row must be a successful, unmodified embedded migration.
    let applied_rows_are_known = applied
        .iter()
        .all(|(version, success, checksum)| *success && is_known(*version, checksum));

    // And every embedded migration must be applied.
    let embedded_versions_are_applied = migrator().iter().all(|migration| {
        applied
            .iter()
            .any(|(version, _, _)| *version == migration.version)
    });

    applied_rows_are_known && embedded_versions_are_applied
}
