//! Transactional retirement of registered images and retryable file removal.
use chrono::Duration;
use uuid::Uuid;

use crate::{context::AppContext, files::safe_fs};

use super::references::lock_reference_writes;

#[derive(Debug, thiserror::Error)]
pub enum CollectionError {
    #[error("asset collection storage unavailable")]
    Storage(#[from] std::io::Error),
    #[error("asset collection database unavailable")]
    Database(#[from] sqlx::Error),
}

pub async fn collect_unreferenced_assets(ctx: &AppContext) -> Result<usize, CollectionError> {
    let root = safe_fs::open_collection_root(ctx, ctx.asset_root())?;
    let mut tx = ctx.pool().begin().await?;
    lock_reference_writes(&mut tx).await?;
    // A retired row keeps its zero-reference timestamp until byte removal is
    // durable. If either process exits between the two operations, the same
    // UUID is still selected on restart.
    let retired: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE assets AS a SET state='deleted'
         WHERE a.state IN ('ready','deleted') AND a.zero_ref_at <= $1
           AND NOT EXISTS (SELECT 1 FROM asset_refs WHERE asset_id=a.id)
         RETURNING a.id",
    )
    .bind(ctx.now() - Duration::hours(24))
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let mut removed = 0;
    let mut completed = Vec::new();
    let mut failure = None;
    for id in retired {
        match safe_fs::unlink_regular(&root, &id.to_string()) {
            Ok(existed) => {
                removed += usize::from(existed);
                completed.push(id);
            }
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    if !completed.is_empty() {
        rustix::fs::fsync(&root).map_err(std::io::Error::from)?;
        sqlx::query("UPDATE assets SET zero_ref_at=NULL WHERE id=ANY($1) AND state='deleted'")
            .bind(completed)
            .execute(ctx.pool())
            .await?;
    }
    if let Some(error) = failure {
        return Err(error.into());
    }
    Ok(removed)
}
