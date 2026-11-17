//! Task-only lifecycle commands. Relation admission, CAS and outbox share a transaction.
pub use super::service::DeleteTask;
use super::{
    repository as repo,
    service::{self, DomainError, Task},
};
use crate::context::AppContext;
use uuid::Uuid;

pub struct ArchiveTask {
    pub id: Uuid,
    pub expected_version: i64,
}
pub struct UnarchiveTask {
    pub id: Uuid,
    pub expected_version: i64,
}

pub struct RestoreTask {
    pub id: Uuid,
    pub expected_version: i64,
}

#[derive(Default)]
pub struct PurgeTrash {}
#[derive(Debug, Default)]
pub struct PurgeTrashReport {
    pub purged: u64,
}

/// One atomic pass: prune eligible leaves before parents, never shorten a
/// child's retention period. Restore and purge share relation admission, so
/// either restore commits first or it subsequently observes a missing row.
pub async fn purge_trash(
    ctx: &AppContext,
    _input: PurgeTrash,
) -> Result<PurgeTrashReport, DomainError> {
    let cutoff = ctx.now() - chrono::Duration::days(30);
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        crate::projects::repository::lock_relations(&mut tx).await?;
        repo::lock_trash(&mut tx, cutoff).await?;
        let mut report = PurgeTrashReport::default();
        loop {
            let leaves = repo::expired_leaves(&mut tx, cutoff).await?;
            if leaves.is_empty() {
                break;
            }
            for id in leaves {
                if repo::purge_task(&mut tx, id, cutoff).await? != 1 {
                    // A trigger refusing deletion must roll back dependent
                    // removals too, rather than loop forever or lose history.
                    return Err(DomainError::Conflict {
                        current_version: None,
                    });
                }
                report.purged += 1;
            }
        }
        Ok(report)
    }
    .await;
    service::finish(tx, result).await
}

#[derive(Clone, Copy)]
enum Action {
    Archive,
    Unarchive,
    Delete,
    Restore,
}

pub async fn archive(ctx: &AppContext, input: ArchiveTask) -> Result<Task, DomainError> {
    change(ctx, input.id, input.expected_version, Action::Archive).await
}
pub async fn unarchive(ctx: &AppContext, input: UnarchiveTask) -> Result<Task, DomainError> {
    change(ctx, input.id, input.expected_version, Action::Unarchive).await
}
pub async fn delete(ctx: &AppContext, input: DeleteTask) -> Result<(), DomainError> {
    change(ctx, input.id, input.expected_version, Action::Delete)
        .await
        .map(|_| ())
}
pub async fn restore(ctx: &AppContext, input: RestoreTask) -> Result<Task, DomainError> {
    change(ctx, input.id, input.expected_version, Action::Restore).await
}
async fn change(
    ctx: &AppContext,
    id: Uuid,
    expected: i64,
    action: Action,
) -> Result<Task, DomainError> {
    service::version(expected)?;
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        crate::projects::repository::lock_relations(&mut tx).await?;
        repo::lock_set(&mut tx, Some(id), None, None).await?;
        let old = repo::find(&mut tx, id)
            .await?
            .ok_or(DomainError::NotFound)?;
        if !matches!(action, Action::Restore) && old.deleted_at.is_some() {
            return Err(DomainError::NotFound);
        }
        if old.version != expected {
            return Err(service::conflict(Some(&old)));
        }
        service::relations_writable(&mut tx, &old).await?;
        let mut t = old.clone();
        t.updated_at = ctx.now();
        let operation = match action {
            Action::Archive => {
                if old.archived_at.is_some() {
                    return Err(service::conflict(Some(&old)));
                }
                t.archived_at = Some(t.updated_at);
                // Manual archives never join a project's reversible archive batch.
                t.archive_batch_id = None;
                "archived"
            }
            Action::Unarchive => {
                if old.archived_at.is_none() {
                    return Err(service::conflict(Some(&old)));
                }
                t.archived_at = None;
                t.archive_batch_id = None;
                "unarchived"
            }
            Action::Delete => {
                if old.archived_at.is_some()
                    || repo::children(&mut tx, id)
                        .await?
                        .iter()
                        .any(|c| c.deleted_at.is_none())
                {
                    return Err(service::conflict(Some(&old)));
                }
                t.deleted_at = Some(t.updated_at);
                "deleted"
            }
            Action::Restore => {
                if old.deleted_at.is_none() {
                    return Err(service::conflict(Some(&old)));
                }
                t.deleted_at = None;
                "restored"
            }
        };
        super::transitions::validate_final_state(&mut tx, &t, Some(&old)).await?;
        let t = repo::save(&mut tx, &t, &old).await?;
        repo::event(&mut tx, &t, operation).await?;
        Ok(t)
    }
    .await;
    service::finish(tx, result).await
}
