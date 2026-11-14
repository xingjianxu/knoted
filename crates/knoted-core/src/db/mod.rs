//! PostgreSQL access: connection pooling, embedded schema migrations, and the
//! shared entity registry.
//!
//! SQLx is the only database dependency here and no HTTP type appears in this
//! module, per the architecture rule that `knoted-core` owns persistence while
//! the API crate only translates transport types. Queries use the runtime
//! `sqlx::query` API, never the compile-time `query!` macros, so the workspace
//! builds without a live database.

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

/// Reports whether every embedded migration is applied successfully.
///
/// This is the readiness primitive used by process health checks: a missing
/// `_sqlx_migrations` table (fresh or unreachable database) and a failed
/// migration row both mean "not ready", and are reported as `Ok(false)` rather
/// than as an error so probes never panic.
pub async fn is_ready(pool: &PgPool) -> Result<bool, DbError> {
    let applied: Vec<(i64, bool)> =
        match sqlx::query_as("SELECT version, success FROM _sqlx_migrations")
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows,
            Err(_) => return Ok(false),
        };

    if applied.iter().any(|(_, success)| !success) {
        return Ok(false);
    }

    let all_applied = migrator().iter().all(|migration| {
        applied
            .iter()
            .any(|(version, _)| *version == migration.version)
    });
    Ok(all_applied)
}
