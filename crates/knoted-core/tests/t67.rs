//! Reclamation uses real note references and confirmed execution records.
use std::{fs, os::unix::fs::symlink, path::Path, sync::Arc};

use chrono::{DateTime, Duration, TimeZone, Utc};
use knoted_core::{
    assets::{gc::collect_unreferenced_assets, references},
    auth,
    config::{FilesConfig, IntelligenceConfig},
    context::{AppContext, FixedClock},
    db::testing::TestDb,
    files::{Sha256Hex, gc::collect_staging},
    notes::{
        history::{self, FlushSnapshot},
        service::{self as notes, CreateNote, UpdateNote},
    },
};
use serde_json::json;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

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

fn execution_path(execution: Uuid) -> String {
    format!("/internal/v1/executions/{execution}")
}

async fn execution(server: &MockServer, id: Uuid, state: &str, proof: bool, age: Duration) {
    let started = now() - age - Duration::minutes(1);
    let finished = now() - age;
    let digest = Sha256Hex::digest(b"manifest").to_string();
    Mock::given(method("GET"))
        .and(path(execution_path(id)))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "execution_id": id,
            "kind": "embed",
            "state": state,
            "compute_exit_confirmed": proof,
            "input_sha256": digest,
            "source_version": 1,
            "options_revision": "fixture",
            "created_at": started,
            "updated_at": finished,
            "started_at": started,
            "finished_at": if state == "running" { None } else { Some(finished) },
            "error": null,
        })))
        .mount(server)
        .await;
}

fn staging(ctx: AppContext, root: &Path, server: &MockServer) -> AppContext {
    ctx.with_clock(Arc::new(FixedClock::new(now())))
        .with_files(FilesConfig {
            staging_root: Some(root.into()),
            ..FilesConfig::default()
        })
        .with_intelligence(IntelligenceConfig {
            base_url: server.uri(),
        })
}

fn output(root: &Path, id: Uuid) -> std::path::PathBuf {
    let directory = root.join("output").join(id.to_string());
    fs::create_dir_all(&directory).unwrap();
    let artifact = directory.join(format!("{id}-embed.json"));
    fs::write(&artifact, b"output").unwrap();
    artifact
}

#[tokio::test]
async fn t67_2_running_unproven_and_recent_objects_are_retained() {
    let db = TestDb::create().await.unwrap();
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    for (state, proof, age) in [
        ("running", false, Duration::days(2)),
        ("succeeded", false, Duration::days(2)),
        ("succeeded", true, Duration::hours(23)),
    ] {
        let id = Uuid::new_v4();
        execution(&server, id, state, proof, age).await;
        paths.push(output(root.path(), id));
    }
    let ctx = staging(AppContext::new(db.pool.clone()), root.path(), &server);
    let count = collect_staging(&ctx).await;
    db.drop().await;

    assert_eq!(count.unwrap(), 0);
    assert!(paths.iter().all(|object| object.exists()));
}

#[tokio::test]
async fn t67_3_reclaims_named_objects_and_preserves_other_directories() {
    let db = TestDb::create().await.unwrap();
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    let unrelated = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    execution(&server, id, "succeeded", true, Duration::hours(24)).await;
    Mock::given(method("GET"))
        .and(path(format!("{}/result", execution_path(id))))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object_id": format!("{id}-embed.json"),
            "size": 6,
            "sha256": Sha256Hex::digest(b"output").to_string(),
            "media_type": "application/json",
            "metadata": {
              "kind": "embed", "source_version": "1", "options_revision": "fixture",
              "input_sha256": Sha256Hex::digest(b"manifest").to_string(),
            }
        })))
        .mount(&server)
        .await;
    let eligible = output(root.path(), id);
    fs::create_dir(root.path().join("input")).unwrap();
    let input = root.path().join("input").join(format!(
        "embed-{id}-{}.json",
        Sha256Hex::digest(b"manifest")
    ));
    fs::write(&input, b"manifest").unwrap();
    let unknown = eligible.with_file_name("other-task.json");
    fs::write(&unknown, b"unrecorded object").unwrap();
    let foreign_id = Uuid::new_v4();
    let foreign = unrelated.path().join(format!("{foreign_id}-embed.json"));
    fs::write(&foreign, b"separate task").unwrap();
    symlink(
        unrelated.path(),
        root.path().join("output").join(foreign_id.to_string()),
    )
    .unwrap();
    let ctx = staging(AppContext::new(db.pool.clone()), root.path(), &server);

    let collected = collect_staging(&ctx).await;
    let second = collect_staging(&ctx).await;
    db.drop().await;
    assert_eq!(collected.unwrap(), 2);
    assert_eq!(second.unwrap(), 0);
    assert!(!eligible.exists());
    assert!(!input.exists());
    assert!(unknown.exists());
    assert!(foreign.exists());
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

#[tokio::test]
async fn t67_7_terminal_failure_can_leave_a_named_artifact() {
    let db = TestDb::create().await.unwrap();
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    execution(&server, id, "cancelled", true, Duration::hours(25)).await;
    let artifact = output(root.path(), id);
    let partial =
        artifact.with_file_name(format!(".{id}-embed.json.{}.part", Uuid::new_v4().simple()));
    fs::write(&partial, b"partial write").unwrap();
    let unknown = artifact.with_file_name("unrecorded.json");
    fs::write(&unknown, b"preserve").unwrap();
    let ctx = staging(AppContext::new(db.pool.clone()), root.path(), &server);
    let collected = collect_staging(&ctx).await;
    db.drop().await;
    assert_eq!(collected.unwrap(), 2);
    assert!(!artifact.exists());
    assert!(!partial.exists());
    assert!(unknown.exists());
}
