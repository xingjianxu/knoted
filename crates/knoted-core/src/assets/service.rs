use super::{repository as repo, storage};
use crate::{context::AppContext, files::staging::Sha256Hex};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use uuid::Uuid;

pub const MAX_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 40_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct Asset {
    pub id: Uuid,
    pub mime: String,
    pub size: i64,
    pub sha256: String,
    pub state: String,
    pub created_at: DateTime<Utc>,
}
pub struct UploadAsset {
    pub owner_id: Uuid,
    pub idempotency_key: String,
    pub bytes: Vec<u8>,
    pub sha256: Option<String>,
}
pub struct ReadAsset {
    pub id: Uuid,
    pub owner_id: Uuid,
}
pub struct AssetContent {
    pub asset: Asset,
    pub bytes: Vec<u8>,
}
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("image limit exceeded")]
    Limit,
    #[error("invalid image contents or digest")]
    InvalidImage,
    #[error("invalid idempotency key")]
    InvalidKey,
    #[error("idempotency request conflict")]
    Conflict,
    #[error("asset not found")]
    NotFound,
    #[error("unknown image asset reference")]
    InvalidReference,
    #[error("asset storage unavailable")]
    Storage,
    #[error("asset persistence unavailable")]
    Database(#[from] sqlx::Error),
}

/// Registration finishes independently of a disconnected caller. In particular,
/// cancellation must not drop the publication guard while COMMIT is in flight.
pub async fn upload(ctx: &AppContext, input: UploadAsset) -> Result<Asset, DomainError> {
    let ctx = ctx.clone();
    tokio::spawn(async move { upload_inner(&ctx, input).await })
        .await
        .map_err(|_| DomainError::Storage)?
}
async fn upload_inner(ctx: &AppContext, input: UploadAsset) -> Result<Asset, DomainError> {
    if input.idempotency_key.is_empty()
        || input.idempotency_key.len() > 255
        || input.idempotency_key.contains('\0')
    {
        return Err(DomainError::InvalidKey);
    }
    if input.bytes.len() > MAX_BYTES {
        return Err(DomainError::Limit);
    }
    let sha = Sha256Hex::digest(&input.bytes).to_string();
    // Digest semantic multipart fields; browser-generated boundaries and names
    // do not alter the request. Optional hash presence does alter it.
    let digest =
        Sha256Hex::digest(&serde_json::to_vec(&(&sha, &input.sha256)).expect("strings serialize"))
            .to_string();
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        repo::lock_upload(&mut tx, input.owner_id, &input.idempotency_key).await?;
        let previous =
            repo::replay(&mut tx, input.owner_id, &input.idempotency_key, ctx.now()).await?;
        if let Some((prior, id)) = previous {
            if prior != digest {
                return Err(DomainError::Conflict);
            }
            let asset = repo::find(&mut tx, id, input.owner_id)
                .await?
                .ok_or(DomainError::NotFound)?;
            return Ok((asset, None));
        }
        if input.sha256.as_ref().is_some_and(|s| s != &sha) {
            return Err(DomainError::InvalidImage);
        }
        let root = ctx.asset_root().to_path_buf();
        let staged = tokio::task::spawn_blocking(move || storage::stage(&root, &input.bytes))
            .await
            .map_err(|_| DomainError::Storage)??;
        let asset = Asset {
            id: Uuid::new_v4(),
            mime: staged.mime.to_owned(),
            size: staged.size()?,
            sha256: sha,
            state: "ready".into(),
            created_at: ctx.now(),
        };
        let root = ctx.asset_root().to_path_buf();
        let id = asset.id;
        let published = tokio::task::spawn_blocking(move || staged.publish(&root, id))
            .await
            .map_err(|_| DomainError::Storage)??;
        let asset = repo::insert(&mut tx, input.owner_id, &asset).await?;
        repo::remember(
            &mut tx,
            input.owner_id,
            &input.idempotency_key,
            &digest,
            asset.id,
            ctx.now() + Duration::days(7),
        )
        .await?;
        Ok((asset, Some(published)))
    }
    .await;
    match result {
        Ok((asset, published)) => {
            // Once COMMIT is sent, a network error is ambiguous. Keep the full
            // file: deletion here could break a committed ready asset. Without
            // a committed registration an orphan remains private/inaccessible.
            if let Some(p) = published {
                p.retain();
            }
            tx.commit().await?;
            Ok(asset)
        }
        Err(e) => {
            tx.rollback().await?;
            Err(e)
        }
    }
}

pub async fn read(ctx: &AppContext, input: ReadAsset) -> Result<AssetContent, DomainError> {
    let asset = repo::find(&mut *ctx.pool().acquire().await?, input.id, input.owner_id)
        .await?
        .ok_or(DomainError::NotFound)?;
    let root = ctx.asset_root().to_path_buf();
    let bytes = tokio::task::spawn_blocking(move || storage::read(&root, input.id))
        .await
        .map_err(|_| DomainError::Storage)??;
    if bytes.len() as i64 != asset.size || Sha256Hex::digest(&bytes).as_str() != asset.sha256 {
        return Err(DomainError::Storage);
    }
    Ok(AssetContent { asset, bytes })
}
