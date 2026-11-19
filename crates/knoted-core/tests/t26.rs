//! Image storage and reference lifecycle against real isolated storage/PostgreSQL.
use chrono::{Duration, Utc};
use image::{DynamicImage, ImageFormat, RgbImage};
use knoted_core::{
    assets::service::{self, DomainError, ReadAsset, UploadAsset},
    auth,
    context::{AppContext, FixedClock},
    db::testing::TestDb,
};
use std::{io::Cursor, sync::Arc};
use uuid::Uuid;
struct Harness {
    db: TestDb,
    ctx: AppContext,
    root: tempfile::TempDir,
    owner: Uuid,
    clock: Arc<FixedClock>,
}
impl Harness {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(FixedClock::new(Utc::now()));
        let ctx = AppContext::new(db.pool.clone())
            .with_clock(clock.clone())
            .with_asset_root(root.path());
        let mut tx = db.pool.begin().await.unwrap();
        let owner = auth::ensure_owner(&mut tx).await.unwrap().user.id;
        tx.commit().await.unwrap();
        Self {
            db,
            ctx,
            root,
            owner,
            clock,
        }
    }
    async fn upload(&self, bytes: Vec<u8>, key: &str) -> Result<service::Asset, DomainError> {
        service::upload(
            &self.ctx,
            UploadAsset {
                owner_id: self.owner,
                idempotency_key: key.to_owned(),
                bytes,
                sha256: None,
            },
        )
        .await
    }
    async fn count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM assets")
            .fetch_one(&self.db.pool)
            .await
            .unwrap()
    }
    fn published(&self) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(self.root.path())
            .unwrap()
            .filter_map(|e| {
                let e = e.unwrap();
                e.file_type().unwrap().is_file().then(|| e.path())
            })
            .collect()
    }
}
fn fixture(format: ImageFormat) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, image::Rgb([200, 10, 50])))
        .write_to(&mut bytes, format)
        .unwrap();
    bytes.into_inner()
}

// Sniffing alone would accept all three truncated payloads; full decode must run.
#[tokio::test]
async fn t26_all_three_formats_decode_and_corrupt_contents_are_rejected() {
    let h = Harness::new().await;
    for (format, mime) in [
        (ImageFormat::Png, "image/png"),
        (ImageFormat::Jpeg, "image/jpeg"),
        (ImageFormat::WebP, "image/webp"),
    ] {
        let bytes = fixture(format);
        let result = h.upload(bytes.clone(), mime).await.unwrap();
        assert_eq!(result.mime, mime);
        let read = service::read(
            &h.ctx,
            ReadAsset {
                id: result.id,
                owner_id: h.owner,
            },
        )
        .await
        .unwrap();
        assert_eq!(read.bytes, bytes);
        let broken = bytes[..bytes.len() / 2].to_vec();
        assert!(
            matches!(
                h.upload(broken, &Uuid::new_v4().to_string()).await,
                Err(DomainError::InvalidImage)
            ),
            "{mime}"
        );
    }
    assert_eq!(h.count().await, 3);
    assert_eq!(h.published().len(), 3);
    let staging = h.root.path().join(".staging");
    assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(staging).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for p in h.published() {
            assert_eq!(
                std::fs::metadata(p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[tokio::test]
async fn t26_idempotency_seven_day_expiry_and_owner_scope() {
    let h = Harness::new().await;
    let png = fixture(ImageFormat::Png);
    let first = h.upload(png.clone(), "expiry").await.unwrap();
    h.clock
        .advance(Duration::days(7) - Duration::milliseconds(1));
    assert_eq!(h.upload(png.clone(), "expiry").await.unwrap(), first);
    h.clock.advance(Duration::milliseconds(1));
    let next = h.upload(png.clone(), "expiry").await.unwrap();
    assert_ne!(next.id, first.id);
    assert_eq!(h.count().await, 2);
    assert!(matches!(
        service::read(
            &h.ctx,
            ReadAsset {
                id: first.id,
                owner_id: Uuid::new_v4()
            }
        )
        .await,
        Err(DomainError::NotFound)
    ));
    let changed = h.upload(fixture(ImageFormat::Jpeg), "expiry").await;
    assert!(matches!(changed, Err(DomainError::Conflict)));
    let expiry: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT expires_at FROM asset_upload_requests WHERE key='expiry'")
            .fetch_one(&h.db.pool)
            .await
            .unwrap();
    assert_eq!(
        expiry.timestamp_micros(),
        (h.ctx.now() + Duration::days(7)).timestamp_micros()
    );
}

// Files may be published before a late constraint fails; none may remain readable.
#[tokio::test]
async fn t26_database_failure_after_publication_removes_file_and_request() {
    let h = Harness::new().await;
    sqlx::query("ALTER TABLE asset_upload_requests ADD CONSTRAINT t26_fail_request CHECK (key <> 'forced') NOT VALID").execute(&h.db.pool).await.unwrap();
    assert!(matches!(
        h.upload(fixture(ImageFormat::Png), "forced").await,
        Err(DomainError::Database(_))
    ));
    assert_eq!(h.count().await, 0);
    assert_eq!(h.published().len(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM asset_upload_requests")
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        h.upload(fixture(ImageFormat::Png), "okay")
            .await
            .unwrap()
            .state,
        "ready"
    );
}

#[tokio::test]
async fn t26_file_failure_leaves_no_registration_or_idempotency_state() {
    let h = Harness::new().await;
    let file = h.root.path().join("blocked");
    std::fs::write(&file, b"occupied").unwrap();
    let ctx = h.ctx.clone().with_asset_root(file);
    let result = service::upload(
        &ctx,
        UploadAsset {
            owner_id: h.owner,
            idempotency_key: "file".into(),
            bytes: fixture(ImageFormat::Png),
            sha256: None,
        },
    )
    .await;
    assert!(matches!(result, Err(DomainError::Storage)));
    assert_eq!(h.count().await, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM asset_upload_requests")
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        0
    );
    let asset = h.upload(fixture(ImageFormat::Png), "file").await.unwrap();
    // Corrupted disk content is never returned under trusted image headers.
    std::fs::write(h.root.path().join(asset.id.to_string()), b"corrupt").unwrap();
    assert!(matches!(
        service::read(
            &h.ctx,
            ReadAsset {
                id: asset.id,
                owner_id: h.owner
            }
        )
        .await,
        Err(DomainError::Storage)
    ));
}

#[tokio::test]
async fn t26_2_compressed_pixel_bomb_rejected_before_decode_allocation() {
    use base64::Engine;
    let h = Harness::new().await;
    let bomb=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAJxAAACcQCAIAAAA1LPVwAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC").unwrap();
    assert!(bomb.len() < 100);
    assert!(matches!(
        h.upload(bomb, "bomb").await,
        Err(DomainError::Limit)
    ));
    assert_eq!(h.count().await, 0);
    assert_eq!(h.published().len(), 0);
}

// Without acquiring the transaction-wide ref lock before note locks, batches
// can invert asset lock ordering or deadlock with structural sibling repacking.
#[tokio::test]
async fn t26_note_writers_wait_for_reference_lock_before_locking_note_rows() {
    use knoted_core::notes::service::{self as notes, CreateNote, UpdateNote};
    let h = Harness::new().await;
    let note = notes::create(
        &h.ctx,
        CreateNote {
            title: "before".into(),
            parent_id: None,
            document_json: r#"{"blocks":[]}"#.into(),
        },
    )
    .await
    .unwrap();
    let mut lock = h.db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(26001, 'asset_refs'::regclass::oid::integer)")
        .execute(&mut *lock)
        .await
        .unwrap();
    let ctx = h.ctx.clone();
    let id = note.id;
    let writer = tokio::spawn(async move {
        notes::update(
            &ctx,
            UpdateNote {
                id,
                expected_version: 1,
                title: Some("after".into()),
                document_json: None,
            },
        )
        .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), async {
            while !writer.is_finished() {
                tokio::task::yield_now().await
            }
        })
        .await
        .is_err(),
        "writer must wait for the transaction reference lock"
    );
    sqlx::query("SELECT id FROM notes WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(id)
        .execute(&mut *lock)
        .await
        .unwrap();
    lock.rollback().await.unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), writer)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .title,
        "after"
    );
}

#[tokio::test]
async fn t26_truncated_container_tails_are_rejected() {
    let h = Harness::new().await;
    for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
        let bytes = fixture(format);
        for removed in [1, 8] {
            let r = h
                .upload(
                    bytes[..bytes.len() - removed].to_vec(),
                    &Uuid::new_v4().to_string(),
                )
                .await;
            assert!(
                matches!(r, Err(DomainError::InvalidImage)),
                "{format:?} missing {removed} tail bytes accepted: {r:?}"
            );
        }
    }
}

// Disconnecting an HTTP caller while a published registration is blocked must
// not cancel COMMIT or delete a file whose metadata might already be committed.
#[tokio::test]
async fn t26_cancelled_caller_does_not_interrupt_asset_registration() {
    let h = Harness::new().await;
    sqlx::query("CREATE FUNCTION t26_pause_asset() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(26002, 'assets'::regclass::oid::integer); RETURN NEW; END $$").execute(&h.db.pool).await.unwrap();
    sqlx::query("CREATE TRIGGER t26_pause BEFORE INSERT ON assets FOR EACH ROW EXECUTE FUNCTION t26_pause_asset()").execute(&h.db.pool).await.unwrap();
    let mut blocker = h.db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(26002, 'assets'::regclass::oid::integer)")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let ctx = h.ctx.clone();
    let owner_id = h.owner;
    let caller = tokio::spawn(async move {
        service::upload(
            &ctx,
            UploadAsset {
                owner_id,
                idempotency_key: "cancel".into(),
                bytes: fixture(ImageFormat::Png),
                sha256: None,
            },
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while h.published().is_empty() {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    caller.abort();
    let _ = caller.await;
    let refused = knoted_core::maintenance::exclusive(
        h.ctx.admission_pool(),
        std::time::Duration::from_millis(200),
    )
    .await
    .is_err();
    blocker.rollback().await.unwrap();
    let completed = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if h.count().await == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        completed.is_ok(),
        "registration must finish independently of a disconnected caller"
    );
    assert!(
        refused,
        "the detached upload must retain write admission until registration commits"
    );
    let replay = h.upload(fixture(ImageFormat::Png), "cancel").await.unwrap();
    assert_eq!(
        h.published(),
        vec![h.root.path().join(replay.id.to_string())]
    );
}

#[tokio::test]
async fn t26_prune_failure_rolls_back_snapshot_refs_and_zero_ref_clock() {
    use knoted_core::notes::{
        history::{self, FlushSnapshot, MaintainHistory},
        service::{self as notes, CreateNote, UpdateNote},
    };
    let h = Harness::new().await;
    let asset = h.upload(fixture(ImageFormat::Png), "prune").await.unwrap();
    let document =
        serde_json::json!({"blocks":[{"id":"img","type":"image","asset_id":asset.id}]}).to_string();
    let note = notes::create(
        &h.ctx,
        CreateNote {
            title: "prune".into(),
            parent_id: None,
            document_json: document,
        },
    )
    .await
    .unwrap();
    let first = history::flush_snapshot(&h.ctx, FlushSnapshot { note_id: note.id })
        .await
        .unwrap();
    notes::update(
        &h.ctx,
        UpdateNote {
            id: note.id,
            expected_version: 1,
            title: None,
            document_json: Some(r#"{"blocks":[]}"#.into()),
        },
    )
    .await
    .unwrap();
    history::flush_snapshot(&h.ctx, FlushSnapshot { note_id: note.id })
        .await
        .unwrap();
    h.clock.advance(Duration::days(91));
    sqlx::query("CREATE FUNCTION t26_reject_prune() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test rejects prune'; END $$").execute(&h.db.pool).await.unwrap();
    sqlx::query("CREATE TRIGGER t26_reject_prune BEFORE DELETE ON asset_refs FOR EACH ROW EXECUTE FUNCTION t26_reject_prune()").execute(&h.db.pool).await.unwrap();
    assert!(
        history::maintain_history(&h.ctx, MaintainHistory { recover: true })
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM note_snapshots WHERE snapshot_id=$1")
            .bind(first.snapshot_id)
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM asset_refs WHERE ref_id=$1")
            .bind(first.snapshot_id)
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        1
    );
    assert!(
        sqlx::query_scalar::<_, Option<chrono::DateTime<Utc>>>(
            "SELECT zero_ref_at FROM assets WHERE id=$1"
        )
        .bind(asset.id)
        .fetch_one(&h.db.pool)
        .await
        .unwrap()
        .is_none()
    );
    sqlx::query("DROP TRIGGER t26_reject_prune ON asset_refs")
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        history::maintain_history(&h.ctx, MaintainHistory { recover: true })
            .await
            .unwrap()
            .pruned,
        1
    );
    let zero: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT zero_ref_at FROM assets WHERE id=$1")
            .bind(asset.id)
            .fetch_one(&h.db.pool)
            .await
            .unwrap();
    assert_eq!(zero.timestamp_micros(), h.ctx.now().timestamp_micros());
    h.clock.advance(Duration::days(1));
    history::maintain_history(&h.ctx, MaintainHistory { recover: true })
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, chrono::DateTime<Utc>>(
            "SELECT zero_ref_at FROM assets WHERE id=$1"
        )
        .bind(asset.id)
        .fetch_one(&h.db.pool)
        .await
        .unwrap(),
        zero
    );
    assert_eq!(
        h.count().await,
        1,
        "no garbage collection is performed by T26"
    );
}

#[test]
fn t26_config_rejects_relative_storage_and_defaults_to_fixed_instance_root() {
    use knoted_core::config::AppConfig;
    let config = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(config.path(), "").unwrap();
    assert_eq!(
        AppConfig::load(config.path()).unwrap().assets.root,
        std::path::PathBuf::from("/data/knoted-dev/service/assets")
    );
    std::fs::write(config.path(), "[assets]\nroot = 'relative/assets'\n").unwrap();
    assert!(AppConfig::load(config.path()).is_err());
    std::fs::write(
        config.path(),
        "[assets]\nroot = '/tmp/private-test-assets'\n",
    )
    .unwrap();
    assert_eq!(
        AppConfig::load(config.path()).unwrap().assets.root,
        std::path::PathBuf::from("/tmp/private-test-assets")
    );
}

#[tokio::test]
async fn t26_valid_extended_and_animated_webp_remain_supported() {
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = kind.to_vec();
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(data);
        if data.len() % 2 == 1 {
            bytes.push(0);
        }
        bytes
    }
    fn container(chunks: Vec<u8>) -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(chunks.len() as u32 + 4).to_le_bytes());
        bytes.extend_from_slice(b"WEBP");
        bytes.extend(chunks);
        bytes
    }
    let h = Harness::new().await;
    let encoded = fixture(ImageFormat::WebP);
    let mut canvas = [0; 10];
    canvas[4] = 1;
    canvas[7] = 1;
    let mut extended = chunk(b"VP8X", &canvas);
    extended.extend_from_slice(&encoded[12..]);
    let static_asset = h.upload(container(extended), "extended").await.unwrap();
    assert_eq!(static_asset.mime, "image/webp");
    canvas[0] = 2;
    let mut animated = chunk(b"VP8X", &canvas);
    animated.extend(chunk(b"ANIM", &[0; 6]));
    let mut frame = vec![0; 16];
    frame[6] = 1;
    frame[9] = 1;
    frame[12] = 10;
    frame.extend_from_slice(&encoded[12..]);
    animated.extend(chunk(b"ANMF", &frame));
    let bytes = container(animated);
    let asset = h.upload(bytes.clone(), "animation").await.unwrap();
    let content = service::read(
        &h.ctx,
        ReadAsset {
            id: asset.id,
            owner_id: h.owner,
        },
    )
    .await
    .unwrap();
    assert_eq!(content.bytes, bytes);
}

// image-webp decodes the internal lossy frame before comparing it with a small
// VP8X canvas. Its frame header allocates buffers from internal dimensions.
#[tokio::test]
async fn t26_webp_nested_frame_pixel_limits_precede_official_decoder_allocation() {
    fn chunk(name: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut b = name.to_vec();
        b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        b.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            b.push(0);
        }
        b
    }
    fn container(chunks: Vec<u8>) -> Vec<u8> {
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&(chunks.len() as u32 + 4).to_le_bytes());
        b.extend_from_slice(b"WEBP");
        b.extend(chunks);
        b
    }
    let h = Harness::new().await;
    let mut vp8 = vec![0x10, 0, 0, 0x9d, 1, 0x2a];
    vp8.extend_from_slice(&6401u16.to_le_bytes());
    vp8.extend_from_slice(&6400u16.to_le_bytes());
    let mut lossless = vec![0x2f];
    lossless.extend_from_slice(&(6400u32 | (6399u32 << 14)).to_le_bytes());
    let static_canvas = chunk(b"VP8X", &[0; 10]);
    for frame in [chunk(b"VP8 ", &vp8), chunk(b"VP8L", &lossless)] {
        let mut parts = static_canvas.clone();
        parts.extend(frame.clone());
        assert!(
            matches!(
                h.upload(container(parts), &Uuid::new_v4().to_string())
                    .await,
                Err(DomainError::Limit)
            ),
            "static internal frame must be bounded before decoding"
        );
        let mut parts = chunk(b"VP8X", &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        parts.extend(chunk(b"ANIM", &[0; 6]));
        let mut anmf = vec![0; 16];
        anmf.extend(frame);
        parts.extend(chunk(b"ANMF", &anmf));
        assert!(
            matches!(
                h.upload(container(parts), &Uuid::new_v4().to_string())
                    .await,
                Err(DomainError::Limit)
            ),
            "animated internal frame must be bounded before decoding"
        );
    }
    assert_eq!(h.count().await, 0);
}
