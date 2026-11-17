//! Asset SQL. Services/reference operations own transaction boundaries.
use super::{
    references::ReplaceAssetRefs,
    service::{Asset, DomainError},
};
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use std::collections::BTreeSet;
use uuid::Uuid;

pub(super) async fn lock_upload(
    c: &mut PgConnection,
    owner: Uuid,
    key: &str,
) -> Result<(), sqlx::Error> {
    let identity = serde_json::to_string(&(owner, "asset.upload", key)).expect("strings serialize");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(identity)
        .execute(c)
        .await?;
    Ok(())
}
pub(super) async fn replay(
    c: &mut PgConnection,
    owner: Uuid,
    key: &str,
    now: DateTime<Utc>,
) -> Result<Option<(String, Uuid)>, sqlx::Error> {
    sqlx::query_as("SELECT request_digest,asset_id FROM asset_upload_requests WHERE owner_id=$1 AND operation='asset.upload' AND key=$2 AND expires_at>$3")
        .bind(owner).bind(key).bind(now).fetch_optional(c).await
}
pub(super) async fn find(
    c: &mut PgConnection,
    id: Uuid,
    owner: Uuid,
) -> Result<Option<Asset>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM assets WHERE id=$1 AND owner_id=$2 AND state='ready'")
        .bind(id)
        .bind(owner)
        .fetch_optional(c)
        .await
}
pub(super) async fn insert(
    c: &mut PgConnection,
    owner: Uuid,
    asset: &Asset,
) -> Result<Asset, sqlx::Error> {
    sqlx::query_as("INSERT INTO assets(id,owner_id,storage_key,mime,size,sha256,state,created_at,zero_ref_at) VALUES($1,$2,$3,$4,$5,$6,'ready',$7,$7) RETURNING *")
        .bind(asset.id).bind(owner).bind(asset.id.to_string()).bind(&asset.mime).bind(asset.size).bind(&asset.sha256).bind(asset.created_at).fetch_one(c).await
}
pub(super) async fn remember(
    c: &mut PgConnection,
    owner: Uuid,
    key: &str,
    digest: &str,
    id: Uuid,
    expires: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM asset_upload_requests WHERE owner_id=$1 AND operation='asset.upload' AND key=$2").bind(owner).bind(key).execute(&mut *c).await?;
    sqlx::query("INSERT INTO asset_upload_requests(owner_id,operation,key,request_digest,asset_id,expires_at) VALUES($1,'asset.upload',$2,$3,$4,$5)")
        .bind(owner).bind(key).bind(digest).bind(id).bind(expires).execute(c).await?;
    Ok(())
}

/// Acquire before tree/note locks; the scope includes all notes in a batch.
pub async fn lock_reference_writes(c: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(26001, 'asset_refs'::regclass::oid::integer)")
        .execute(c)
        .await?;
    Ok(())
}
pub(super) async fn replace_refs(
    c: &mut PgConnection,
    input: ReplaceAssetRefs<'_>,
    ids: BTreeSet<Uuid>,
) -> Result<(), DomainError> {
    lock_reference_writes(c).await?;
    let prior: Vec<Uuid> =
        sqlx::query_scalar("SELECT asset_id FROM asset_refs WHERE ref_kind=$1 AND ref_id=$2")
            .bind(input.kind.as_str())
            .bind(input.ref_id)
            .fetch_all(&mut *c)
            .await?;
    let mut touched = ids.clone();
    touched.extend(prior);
    // Future T67 must acquire the transaction-wide reference lock before asset
    // rows too; refs cannot race a ready->deleted transition.
    for id in &touched {
        let ready: Option<String> =
            sqlx::query_scalar("SELECT state FROM assets WHERE id=$1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *c)
                .await?;
        if ids.contains(id) && ready.as_deref() != Some("ready") {
            return Err(DomainError::InvalidReference);
        }
    }
    sqlx::query("DELETE FROM asset_refs WHERE ref_kind=$1 AND ref_id=$2")
        .bind(input.kind.as_str())
        .bind(input.ref_id)
        .execute(&mut *c)
        .await?;
    for id in ids {
        sqlx::query("INSERT INTO asset_refs(asset_id,ref_kind,ref_id,note_id) VALUES($1,$2,$3,$4)")
            .bind(id)
            .bind(input.kind.as_str())
            .bind(input.ref_id)
            .bind(input.note_id)
            .execute(&mut *c)
            .await?;
    }
    for id in touched {
        sqlx::query("UPDATE assets SET zero_ref_at=CASE WHEN EXISTS(SELECT 1 FROM asset_refs WHERE asset_id=$1) THEN NULL ELSE COALESCE(zero_ref_at,$2) END WHERE id=$1")
            .bind(id).bind(input.now).execute(&mut *c).await?;
    }
    Ok(())
}
