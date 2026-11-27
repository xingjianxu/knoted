//! The shared source registry.
//!
//! Search and entity links need one stable identifier per source
//! object. Later cards create the per-module tables and register their rows in
//! `entity_registry`; this module owns the type tags those rows may carry.

use sqlx::PgPool;
use uuid::Uuid;

use super::DbError;

/// Kinds of entity that may appear in `entity_registry`.
///
/// The string values are the database check constraint's vocabulary and must
/// stay in sync with `migrations/0001_t05_entity_registry.sql`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceIdentity {
    Note,
    File,
    Project,
    Task,
}

impl SourceIdentity {
    /// Every accepted identity type, in database constraint order.
    pub const ALL: [SourceIdentity; 4] = [
        SourceIdentity::Note,
        SourceIdentity::File,
        SourceIdentity::Project,
        SourceIdentity::Task,
    ];

    /// The database value of this identity type.
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceIdentity::Note => "note",
            SourceIdentity::File => "file",
            SourceIdentity::Project => "project",
            SourceIdentity::Task => "task",
        }
    }

    /// Parses a database value, rejecting anything outside [`Self::ALL`].
    pub fn parse(value: &str) -> Result<Self, DbError> {
        match value {
            "note" => Ok(SourceIdentity::Note),
            "file" => Ok(SourceIdentity::File),
            "project" => Ok(SourceIdentity::Project),
            "task" => Ok(SourceIdentity::Task),
            other => Err(DbError::UnknownSourceType(other.to_owned())),
        }
    }
}

/// Registers a new source row and returns its identifier.
pub async fn register_source(pool: &PgPool, source: SourceIdentity) -> Result<Uuid, DbError> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO entity_registry (id, entity_type) VALUES ($1, $2)")
        .bind(id)
        .bind(source.as_str())
        .execute(pool)
        .await?;
    Ok(id)
}

/// Looks up the identity type of a registered source.
pub async fn lookup_source(pool: &PgPool, id: Uuid) -> Result<Option<SourceIdentity>, DbError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT entity_type FROM entity_registry WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    row.map(|(value,)| SourceIdentity::parse(&value))
        .transpose()
}
