//! Database failures surfaced by [`crate::db`].

/// Errors raised while connecting, migrating or querying PostgreSQL.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("unknown source identity type `{0}`")]
    UnknownSourceType(String),
    #[error("an enqueue without an idempotency key reported a duplicate conflict")]
    UnkeyedIdempotentConflict,
    #[error("invalid job attempt budget {0}: max_attempts must be at least 1")]
    InvalidMaxAttempts(i32),
    #[error("database returned unknown job state `{0}`")]
    UnknownJobState(String),
    #[error("refusing to use `{0}` as a test database: name must end with `_test`")]
    UnsafeTestTarget(String),
}
