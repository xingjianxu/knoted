//! Final-state admission for every task command, inside the caller's transaction.
use super::{
    repository as repo,
    service::{self, DomainError, Task, TaskStatus, UpdateTask},
};
use crate::context::AppContext;
use sqlx::PgConnection;

/// Apply the existing PATCH command, including mixed status/relationship/date
/// changes, with one transaction, CAS and outbox commit.
pub async fn transition_task(ctx: &AppContext, input: UpdateTask) -> Result<Task, DomainError> {
    service::update_command(ctx, input).await
}

fn terminal(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::Done | TaskStatus::Cancelled)
}

/// Commands must hold relation admission and the globally ordered row lock set.
/// Snapshot readers may reuse this read-only predicate as an eligibility hint.
/// Re-read each affected parent and its children, overlay the complete
/// candidate (including detachment/deletion), and validate only that final set.
/// The guard never mutates versions or implicitly reopens a parent.
pub(crate) async fn validate_final_state(
    c: &mut PgConnection,
    candidate: &Task,
    old: Option<&Task>,
) -> Result<(), DomainError> {
    let mut parents = vec![candidate.id];
    parents.extend(candidate.parent_id);
    parents.extend(old.and_then(|t| t.parent_id));
    parents.sort_unstable();
    parents.dedup();
    for id in parents {
        let parent = if id == candidate.id {
            Some(candidate.clone())
        } else {
            repo::find(c, id).await?
        };
        let Some(parent) = parent else { continue };
        if parent.deleted_at.is_some() || parent.status != TaskStatus::Done {
            continue;
        }
        let existing = repo::children(c, id).await?;
        let invalid_existing = existing.iter().any(|child| {
            child.id != candidate.id && child.deleted_at.is_none() && !terminal(child.status)
        });
        let invalid_candidate = candidate.parent_id == Some(id)
            && candidate.deleted_at.is_none()
            && !terminal(candidate.status);
        if invalid_existing || invalid_candidate {
            return Err(DomainError::Conflict {
                current_version: old.map(|t| t.version),
            });
        }
    }
    Ok(())
}
