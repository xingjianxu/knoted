//! Structural note operations. Every structural writer takes the shared tree lock before
//! row locks, so ancestor validation and sibling shifts see one serialized tree.
use super::{
    repository as repo,
    service::{DomainError, Note, check_version, finish, version},
};
use crate::context::AppContext;
use std::collections::HashSet;
use uuid::Uuid;

pub struct MoveNote {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub position: i64,
    pub expected_version: i64,
}

pub struct RestoreParent {
    pub parent_id: Option<Uuid>,
}

pub struct RestoreNote {
    pub id: Uuid,
    pub expected_version: i64,
    /// None retains the stored parent; Some with parent_id=None moves to root.
    pub restore_parent: Option<RestoreParent>,
}

/// Writers retain row locks; snapshot readers reuse the exact parent predicate
/// with `lock=false` and must never use this hint as command admission.
pub(crate) async fn validate_parent(
    c: &mut sqlx::PgConnection,
    note: &Note,
    parent: Option<Uuid>,
    lock: bool,
) -> Result<(), DomainError> {
    let conflict = || DomainError::Conflict {
        current_version: Some(note.content_version),
    };
    let mut ancestor = parent;
    let mut seen = HashSet::from([note.id]);
    while let Some(id) = ancestor {
        if !seen.insert(id) {
            return Err(conflict());
        }
        // Traverse stored ancestry, including deleted rows: deletion preserves
        // parent links, and those links still matter when preventing cycles.
        let row = if lock {
            repo::find_any(c, id).await?
        } else {
            repo::find_any_readonly(c, id).await?
        }
        .ok_or_else(conflict)?;
        if Some(id) == parent && row.deleted_at.is_some() {
            return Err(conflict());
        }
        ancestor = row.parent_id;
    }
    Ok(())
}

pub async fn move_note(ctx: &AppContext, input: MoveNote) -> Result<Note, DomainError> {
    version(input.expected_version)?;
    if input.position < 0 {
        return Err(DomainError::InvalidField("position"));
    }
    let mut tx = repo::begin_write(ctx).await?;
    let result = async {
        repo::lock_tree(&mut tx).await?;
        let mut note = repo::find(&mut tx, input.id, true)
            .await?
            .ok_or(DomainError::NotFound)?;
        check_version(&note, input.expected_version)?;
        validate_parent(&mut tx, &note, input.parent_id, true).await?;
        // Requested indices beyond the end append. Excluding the moving node
        // makes same-parent indices refer to the final sibling order.
        let position = input
            .position
            .min(repo::sibling_count(&mut tx, input.parent_id, note.id).await?);
        if note.parent_id == input.parent_id && note.position == position {
            return Ok(note);
        }
        repo::remove_position(&mut tx, &note).await?;
        repo::insert_position(&mut tx, input.parent_id, position, note.id).await?;
        note.parent_id = input.parent_id;
        note.position = position;
        note.updated_at = ctx.now();
        let note = repo::save_structure(&mut tx, &note, false).await?;
        repo::event(
            &mut tx,
            note.id,
            note.content_version,
            "moved",
            note.updated_at,
        )
        .await?;
        Ok(note)
    }
    .await;
    finish(tx, result).await
}

pub async fn restore(ctx: &AppContext, input: RestoreNote) -> Result<Note, DomainError> {
    version(input.expected_version)?;
    let mut tx = repo::begin_write(ctx).await?;
    let result = async {
        repo::lock_tree(&mut tx).await?;
        let mut note = repo::find_any(&mut tx, input.id)
            .await?
            .ok_or(DomainError::NotFound)?;
        check_version(&note, input.expected_version)?;
        if note.deleted_at.is_none() {
            return Err(DomainError::Conflict {
                current_version: Some(note.content_version),
            });
        }
        let parent = input.restore_parent.map_or(note.parent_id, |p| p.parent_id);
        validate_parent(&mut tx, &note, parent, true).await?;
        // Retain the former slot when possible without displacing the relative
        // order of existing live siblings. A shorter sibling list clamps it.
        let position = note
            .position
            .min(repo::sibling_count(&mut tx, parent, note.id).await?);
        repo::insert_position(&mut tx, parent, position, note.id).await?;
        note.parent_id = parent;
        note.position = position;
        note.updated_at = ctx.now();
        let note = repo::save_structure(&mut tx, &note, true).await?;
        repo::event(
            &mut tx,
            note.id,
            note.content_version,
            "restored",
            note.updated_at,
        )
        .await?;
        Ok(note)
    }
    .await;
    finish(tx, result).await
}
