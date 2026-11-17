//! Reference replacement joins its caller's transaction; it never commits.
use super::service::DomainError;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgConnection;
use std::collections::BTreeSet;
use uuid::Uuid;

pub enum ReferenceKind {
    Current,
    Snapshot,
    Trash,
}
impl ReferenceKind {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Snapshot => "snapshot",
            Self::Trash => "trash",
        }
    }
}
pub struct ReplaceAssetRefs<'a> {
    pub kind: ReferenceKind,
    pub ref_id: Uuid,
    pub note_id: Uuid,
    pub document: &'a Value,
    pub now: DateTime<Utc>,
}
fn collect(value: &Value, ids: &mut BTreeSet<Uuid>) -> Result<(), DomainError> {
    match value {
        Value::Object(fields) => {
            if fields.get("type").and_then(Value::as_str) == Some("image") {
                let id = fields
                    .get("asset_id")
                    .and_then(Value::as_str)
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or(DomainError::InvalidReference)?;
                ids.insert(id);
            }
            // Only document blocks and their nested children contain blocks.
            for name in ["blocks", "children"] {
                if let Some(v) = fields.get(name) {
                    collect(v, ids)?;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect(item, ids)?;
            }
        }
        _ => (),
    }
    Ok(())
}

/// Call before taking any note/tree row locks. Single-owner reference writes
/// serialize across the whole transaction, including multi-note history batches.
pub use super::repository::lock_reference_writes;

/// Replace one document's deduplicated references in its existing transaction.
/// Callers must acquire `lock_reference_writes` before tree or note row locks.
pub async fn replace_asset_refs(
    tx: &mut PgConnection,
    input: ReplaceAssetRefs<'_>,
) -> Result<(), DomainError> {
    let mut ids = BTreeSet::new();
    collect(input.document, &mut ids)?;
    super::repository::replace_refs(tx, input, ids).await
}
