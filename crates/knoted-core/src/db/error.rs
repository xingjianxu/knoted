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
    #[error("refusing to use `{0}` as a test database: name must contain `test`")]
    UnsafeTestTarget(String),
}
