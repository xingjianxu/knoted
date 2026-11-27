//! Reclamation uses real note references.
use std::{fs, os::unix::fs::symlink, sync::Arc};

use chrono::{DateTime, Duration, TimeZone, Utc};
use knoted_core::{
    assets::{gc::collect_unreferenced_assets, references},
    auth,
    context::{AppContext, FixedClock},
    db::testing::TestDb,
    files::Sha256Hex,
    notes::{
        history::{self, FlushSnapshot},
        service::{self as notes, CreateNote, UpdateNote},
    },
};
use serde_json::json;
use uuid::Uuid;

const IMAGE: &[u8] = b"registered fixture object";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 19, 8, 0, 0).unwrap()
}

fn note_document(image: Uuid) -> String {
    json!({"blocks":[{"id":"picture","type":"image","asset_id":image}]}).to_string()
}

async fn asset(ctx: &AppContext, age: Duration) -> Uuid {
    let id = Uuid::new_v4();
    let mut tx = ctx.pool().begin().await.unwrap();
    let owner = auth::ensure_owner(&mut tx).await.unwrap().user.id;
    sqlx::query(
        "INSERT INTO assets(id,owner_id,storage_key,mime,size,sha256,state,created_at,zero_ref_at)
         VALUES($1,$2,$3,'image/png',$4,$5,'ready',$6,$6)",
    )
    .bind(id)
    .bind(owner)
    .bind(id.to_string())
    .bind(IMAGE.len() as i64)
    .bind(Sha256Hex::digest(IMAGE).to_string())
    .bind(ctx.now() - age)
    .execute(&mut *tx)
    .await
    .unwrap();
    fs::write(ctx.asset_root().join(id.to_string()), IMAGE).unwrap();
    tx.commit().await.unwrap();
    id
}

#[tokio::test]
async fn t67_1_history_retains_its_image_and_only_old_unreferenced_images_are_deleted() {
    let db = TestDb::create().await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let ctx = AppContext::new(db.pool.clone())
        .with_asset_root(root.path())
        .with_clock(Arc::new(FixedClock::new(now())));
    let historical = asset(&ctx, Duration::days(3)).await;
    let old = asset(&ctx, Duration::hours(24)).await;
    let fresh = asset(&ctx, Duration::hours(23)).await;
    let note = notes::create(
        &ctx,
        CreateNote {
            title: "历史图".into(),
            parent_id: None,
            document_json: note_document(historical),
        },
    )
    .await
    .unwrap();
    history::flush_snapshot(&ctx, FlushSnapshot { note_id: note.id })
        .await
        .unwrap();
    notes::update(
        &ctx,
        UpdateNote {
            id: note.id,
            expected_version: 1,
            title: None,
            document_json: Some(r#"{"blocks":[]}"#.into()),
        },
    )
    .await
    .unwrap();
    let collected = collect_unreferenced_assets(&ctx).await;
    let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT id,state FROM assets")
        .fetch_all(ctx.pool())
        .await
        .unwrap();
    db.drop().await;

    assert_eq!(collected.unwrap(), 1);
    assert!(rows.contains(&(historical, "ready".into())));
    assert!(rows.contains(&(fresh, "ready".into())));
    assert!(rows.contains(&(old, "deleted".into())));
    assert_eq!(
        fs::read(root.path().join(historical.to_string())).unwrap(),
        IMAGE
    );
    assert!(root.path().join(fresh.to_string()).exists());
    assert!(!root.path().join(old.to_string()).exists());
}

#[tokio::test]
async fn t67_4_reference_committed_while_collection_waits_protects_the_file() {
    let db = TestDb::create().await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let ctx = AppContext::new(db.pool.clone())
        .with_asset_root(root.path())
        .with_clock(Arc::new(FixedClock::new(now())));
    let image = asset(&ctx, Duration::days(2)).await;
    let note = notes::create(
        &ctx,
        CreateNote {
            title: "同步引用".into(),
            parent_id: None,
            document_json: r#"{"blocks":[]}"#.into(),
        },
    )
    .await
    .unwrap();
    let mut tx = ctx.pool().begin().await.unwrap();
    references::lock_reference_writes(&mut tx).await.unwrap();
    let mut collect = Box::pin(collect_unreferenced_assets(&ctx));
    let blocked = tokio::time::timeout(std::time::Duration::from_millis(150), &mut collect)
        .await
        .is_err();
    let doc = serde_json::from_str(&note_document(image)).unwrap();
    references::replace_asset_refs(
        &mut tx,
        references::ReplaceAssetRefs {
            kind: references::ReferenceKind::Current,
            ref_id: note.id,
            note_id: note.id,
            document: &doc,
            now: ctx.now(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let collected = collect.await;
    db.drop().await;

    assert!(blocked, "reference writer must serialize with collection");
    assert_eq!(collected.unwrap(), 0);
    assert!(root.path().join(image.to_string()).exists());
}

#[tokio::test]
async fn t67_5_replay_preserves_the_registered_identity_after_reclamation() {
    use knoted_core::assets::service::{self as assets, DomainError, ReadAsset, UploadAsset};
    use std::io::Cursor;

    let db = TestDb::create().await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let clock = FixedClock::new(now());
    let ctx = AppContext::new(db.pool.clone())
        .with_asset_root(root.path())
        .with_clock(Arc::new(clock.clone()));

    let mut tx = ctx.pool().begin().await.unwrap();
    let owner = auth::ensure_owner(&mut tx).await.unwrap().user.id;
    tx.commit().await.unwrap();
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(1, 1)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    let request = || UploadAsset {
        owner_id: owner,
        idempotency_key: "fixture-replay".into(),
        bytes: out.get_ref().clone(),
        sha256: None,
    };
    let original = assets::upload(&ctx, request()).await.unwrap();
    clock.advance(Duration::hours(24));
    let collected = collect_unreferenced_assets(&ctx).await;
    let replay = assets::upload(&ctx, request()).await;
    let read = assets::read(
        &ctx,
        ReadAsset {
            id: original.id,
            owner_id: owner,
        },
    )
    .await;
    db.drop().await;

    assert_eq!(collected.unwrap(), 1);
    let replayed = replay.unwrap();
    assert_eq!(replayed.id, original.id);
    assert_eq!(replayed.state, "deleted");
    assert!(matches!(read, Err(DomainError::NotFound)));
}

#[tokio::test]
async fn t67_6_retries_removal_without_following_a_substituted_image_path() {
    let db = TestDb::create().await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let unrelated = tempfile::NamedTempFile::new().unwrap();
    let ctx = AppContext::new(db.pool.clone())
        .with_asset_root(root.path())
        .with_clock(Arc::new(FixedClock::new(now())));
    let old = asset(&ctx, Duration::days(2)).await;
    let path = root.path().join(old.to_string());
    fs::remove_file(&path).unwrap();
    symlink(unrelated.path(), &path).unwrap();
    let first = collect_unreferenced_assets(&ctx).await;
    let preserved = path.is_symlink() && unrelated.path().is_file();
    let retired: (String, bool) =
        sqlx::query_as("SELECT state,zero_ref_at IS NOT NULL FROM assets WHERE id=$1")
            .bind(old)
            .fetch_one(ctx.pool())
            .await
            .unwrap();

    fs::remove_file(&path).unwrap();
    fs::write(&path, IMAGE).unwrap();
    let recovered = collect_unreferenced_assets(&ctx).await;
    let completed: (String, bool) =
        sqlx::query_as("SELECT state,zero_ref_at IS NULL FROM assets WHERE id=$1")
            .bind(old)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    db.drop().await;

    assert!(first.is_err());
    assert!(preserved);
    assert_eq!(retired, ("deleted".into(), true));
    assert_eq!(recovered.unwrap(), 1);
    assert_eq!(completed, ("deleted".into(), true));
    assert!(!path.exists());
}
