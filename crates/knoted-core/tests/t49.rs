use knoted_core::{
    config::FilesConfig,
    context::AppContext,
    db::testing::TestDb,
    files::{
        Sha256Hex,
        browse::{self, UpdateFile},
        download::{self, ReadContent},
        safe_fs,
    },
};
use uuid::Uuid;
async fn fixture() -> (TestDb, AppContext, tempfile::TempDir, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(dir.path().into()),
        ..Default::default()
    });
    std::fs::write(dir.path().join("a.txt"), b"source bytes").unwrap();
    let root = Uuid::new_v4();
    let scan = Uuid::new_v4();
    let id = Uuid::new_v4();
    let identity = safe_fs::metadata(safe_fs::open_root(dir.path()).unwrap())
        .unwrap()
        .identity;
    let m = safe_fs::metadata(std::fs::File::open(dir.path().join("a.txt")).unwrap()).unwrap();
    sqlx::query("INSERT INTO file_roots(id,configured_path,identity,generation,last_complete_generation) VALUES($1,$2,$3,1,1)").bind(root).bind(dir.path().to_str().unwrap()).bind(sqlx::types::Json(identity)).execute(ctx.pool()).await.unwrap();
    let job: Uuid = sqlx::query_scalar("INSERT INTO jobs(kind) VALUES('t49.fixture') RETURNING id")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO file_scans(id,root_id,generation,token,job_id,lease_token,complete,finished_at) VALUES($1,$2,1,$3,$4,$3,true,now())").bind(scan).bind(root).bind(Uuid::new_v4()).bind(job).execute(ctx.pool()).await.unwrap();
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'file')")
        .bind(id)
        .execute(ctx.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO file_sources(id,root_id,relative_path,device,inode,size,mtime_ns,ctime_ns,nlink,sha256,content_version,version,status,observation,last_seen_at,content_fresh,captured_observation,captured_observation_version,observation_version,captured_scan_id,input_object_id) VALUES($1,$2,'a.txt',$3,$4,$5,$6,$7,$8,$9,7,11,'present',$10,now(),true,$10,1,1,$11,'fixture')")
 .bind(id).bind(root).bind(m.identity.device as i64).bind(m.identity.inode as i64).bind(m.size as i64).bind(m.mtime_ns).bind(m.ctime_ns).bind(m.nlink as i64).bind(Sha256Hex::digest(b"source bytes").as_str()).bind(sqlx::types::Json(m)).bind(scan).execute(ctx.pool()).await.unwrap();
    (db, ctx, dir, id)
}
#[tokio::test]
async fn t49_content_current_bytes_metadata_and_missing() {
    let (db, ctx, _dir, id) = fixture().await;
    sqlx::query("INSERT INTO file_extractions(source_id,content_version,input_sha256,parser_revision,options_hash,job_id,blocks) SELECT id,content_version,sha256,'test',repeat('b',64),(SELECT id FROM jobs LIMIT 1),'[{\"text\":\"original\"}]'::jsonb FROM file_sources WHERE id=$1").bind(id).execute(ctx.pool()).await.unwrap();
    let before: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(e) FROM file_extractions e WHERE source_id=$1")
            .bind(id)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    let content = download::read_content(&ctx, ReadContent { id }).await;
    let update = browse::update_file(
        &ctx,
        UpdateFile {
            id,
            expected_version: 11,
            note: Some(Some("note".into())),
            tag_ids: None,
        },
    )
    .await;
    let second = download::read_content(&ctx, ReadContent { id }).await;
    sqlx::query("UPDATE file_sources SET status='missing' WHERE id=$1")
        .bind(id)
        .execute(ctx.pool())
        .await
        .unwrap();
    let missing = download::read_content(&ctx, ReadContent { id }).await;
    let detail = browse::get_file(&ctx, browse::GetFile { id }).await;
    let readonly = browse::update_file(
        &ctx,
        UpdateFile {
            id,
            expected_version: 12,
            note: Some(None),
            tag_ids: None,
        },
    )
    .await;
    let after: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(e) FROM file_extractions e WHERE source_id=$1")
            .bind(id)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    db.drop().await;
    assert_eq!(before, after);
    assert_eq!(content.unwrap().bytes, b"source bytes");
    assert_eq!(update.unwrap().content_version, 7);
    assert_eq!(second.unwrap().version, 12);
    assert!(matches!(missing, Err(download::DomainError::NotFound)));
    assert_eq!(detail.unwrap().note.as_deref(), Some("note"));
    assert!(matches!(
        readonly,
        Err(browse::DomainError::Conflict {
            current_version: Some(12)
        })
    ));
}
#[tokio::test]
async fn t49_2_symlink_fifo_replacement_and_invalidated_observation() {
    let (db, ctx, dir, id) = fixture().await;
    let leaf = dir.path().join("a.txt");
    let good = download::read_content(&ctx, ReadContent { id }).await;
    std::fs::rename(&leaf, dir.path().join("saved")).unwrap();
    std::os::unix::fs::symlink("saved", &leaf).unwrap();
    let symlink = download::read_content(&ctx, ReadContent { id }).await;
    std::fs::remove_file(&leaf).unwrap();
    rustix::fs::mkfifoat(rustix::fs::CWD, &leaf, rustix::fs::Mode::RUSR).unwrap();
    let fifo = download::read_content(&ctx, ReadContent { id }).await;
    std::fs::remove_file(&leaf).unwrap();
    std::fs::write(&leaf, b"source bytes").unwrap();
    let replacement = download::read_content(&ctx, ReadContent { id }).await;
    std::fs::remove_file(&leaf).unwrap();
    std::fs::rename(dir.path().join("saved"), &leaf).unwrap();
    sqlx::query("UPDATE file_sources SET content_fresh=false,observation_version=2 WHERE id=$1")
        .bind(id)
        .execute(ctx.pool())
        .await
        .unwrap();
    let stale = download::read_content(&ctx, ReadContent { id }).await;
    db.drop().await;
    assert!(good.is_ok());
    for result in [symlink, fifo, replacement, stale] {
        assert!(matches!(result, Err(download::DomainError::Unavailable)));
    }
}
#[tokio::test]
async fn t49_final_root_lock_revalidates_concurrent_state() {
    for mode in ["note", "observation", "scan", "write", "rename"] {
        let (db, ctx, dir, id) = fixture().await;
        let mut blocker = ctx.pool().begin().await.unwrap();
        sqlx::query("SELECT id FROM file_roots FOR UPDATE")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        let c = ctx.clone();
        let work =
            tokio::spawn(async move { download::read_content(&c, ReadContent { id }).await });
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut blocked = false;
        while tokio::time::Instant::now() < deadline {
            sqlx::query("SELECT pg_stat_clear_snapshot()")
                .execute(&mut *blocker)
                .await
                .unwrap();
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'SELECT configured_path,identity,%FOR UPDATE%')").fetch_one(&mut *blocker).await.unwrap();
            if waiting {
                blocked = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        if blocked {
            match mode {
                "note" => {
                    sqlx::query(
                        "UPDATE file_sources SET note='changed',version=version+1 WHERE id=$1",
                    )
                    .bind(id)
                    .execute(&mut *blocker)
                    .await
                    .unwrap();
                }
                "observation" => {
                    sqlx::query("UPDATE file_sources SET content_fresh=false,observation_version=2 WHERE id=$1").bind(id).execute(&mut *blocker).await.unwrap();
                }
                "scan" => {
                    sqlx::query("UPDATE file_roots SET scan_token=$1")
                        .bind(Uuid::new_v4())
                        .execute(&mut *blocker)
                        .await
                        .unwrap();
                }
                "write" => std::fs::write(dir.path().join("a.txt"), b"different!!!").unwrap(),
                _ => {
                    std::fs::rename(dir.path().join("a.txt"), dir.path().join("old")).unwrap();
                    std::fs::write(dir.path().join("a.txt"), b"source bytes").unwrap();
                }
            }
        }
        blocker.commit().await.unwrap();
        let result = work.await.unwrap();
        db.drop().await;
        assert!(blocked, "did not reach final root lock: {mode}");
        if mode == "note" {
            assert_eq!(result.unwrap().version, 12)
        } else {
            assert!(
                matches!(result, Err(download::DomainError::Unavailable)),
                "{mode}: {result:?}"
            )
        }
    }
}
#[tokio::test]
async fn t49_tags_atomic_cas_noop_history_and_cursor() {
    let (db, ctx, _dir, id) = fixture().await;
    let a = knoted_core::links::service::create_tag(
        &ctx,
        knoted_core::links::service::CreateTag { name: "a".into() },
    )
    .await
    .unwrap();
    let b = knoted_core::links::service::create_tag(
        &ctx,
        knoted_core::links::service::CreateTag { name: "b".into() },
    )
    .await
    .unwrap();
    let first = browse::update_file(
        &ctx,
        UpdateFile {
            id,
            expected_version: 11,
            note: Some(Some("keep".into())),
            tag_ids: Some(vec![b.id, a.id, a.id]),
        },
    )
    .await;
    let bad = browse::update_file(
        &ctx,
        UpdateFile {
            id,
            expected_version: 12,
            note: Some(None),
            tag_ids: Some(vec![Uuid::new_v4()]),
        },
    )
    .await;
    let after_bad = browse::get_file(&ctx, browse::GetFile { id }).await;
    let noop = browse::update_file(
        &ctx,
        UpdateFile {
            id,
            expected_version: 12,
            note: None,
            tag_ids: Some(vec![a.id, b.id]),
        },
    )
    .await;
    knoted_core::links::service::delete_tag(
        &ctx,
        knoted_core::links::service::DeleteTag {
            id: b.id,
            expected_version: 1,
        },
    )
    .await
    .unwrap();
    let clear = browse::update_file(
        &ctx,
        UpdateFile {
            id,
            expected_version: 12,
            note: None,
            tag_ids: Some(vec![]),
        },
    )
    .await;
    let history: i64 =
        sqlx::query_scalar("SELECT count(*) FROM file_tags WHERE entity_id=$1 AND tag_id=$2")
            .bind(id)
            .bind(b.id)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    knoted_core::links::service::assign_tag(
        &ctx,
        knoted_core::links::service::ChangeEntityTag {
            entity: knoted_core::links::service::Entity {
                entity_type: knoted_core::links::service::EntityType::File,
                id,
            },
            tag_id: a.id,
        },
    )
    .await
    .unwrap();
    let stale = browse::update_file(
        &ctx,
        UpdateFile {
            id,
            expected_version: 13,
            note: Some(None),
            tag_ids: None,
        },
    )
    .await;
    let page = browse::list_files(
        &ctx,
        browse::ListFiles {
            limit: Some(1),
            cursor: None,
        },
    )
    .await;
    let invalid = browse::list_files(
        &ctx,
        browse::ListFiles {
            limit: Some(101),
            cursor: None,
        },
    )
    .await;
    let wrong=browse::list_files(&ctx,browse::ListFiles{limit:None,cursor:Some("eyJ2IjoxLCJzY29wZSI6InRhZ3MiLCJpZCI6IjAwMDAwMDAwLTAwMDAtMDAwMC0wMDAwLTAwMDAwMDAwMDAwMCJ9".into())}).await;
    db.drop().await;
    assert_eq!(first.unwrap().version, 12);
    assert!(matches!(bad, Err(browse::DomainError::InvalidField)));
    assert_eq!(after_bad.unwrap().note.as_deref(), Some("keep"));
    assert_eq!(noop.unwrap().version, 12);
    assert_eq!(clear.unwrap().version, 13);
    assert_eq!(history, 1);
    assert!(matches!(
        stale,
        Err(browse::DomainError::Conflict {
            current_version: Some(14)
        })
    ));
    assert_eq!(page.unwrap().items.len(), 1);
    assert!(invalid.is_err());
    assert!(wrong.is_err());
}
#[tokio::test]
async fn t49_2_corrupt_paths_parent_symlink_root_and_size_limit() {
    let (db, ctx, dir, id) = fixture().await;
    // Deliberately corrupt the isolated fixture to prove FD checks, independently of SQL's path constraint.
    sqlx::query("ALTER TABLE file_sources DROP CONSTRAINT file_sources_relative_path_check")
        .execute(ctx.pool())
        .await
        .unwrap();
    let mut rejected = vec![];
    std::os::unix::fs::symlink(dir.path(), dir.path().join("link")).unwrap();
    for path in [
        "../a.txt",
        "/etc/passwd",
        "link/a.txt",
        "./a.txt",
        "a.txt/",
        "",
    ] {
        sqlx::query("UPDATE file_sources SET relative_path=$2 WHERE id=$1")
            .bind(id)
            .bind(path)
            .execute(ctx.pool())
            .await
            .unwrap();
        rejected.push(
            download::read_content(&ctx, ReadContent { id })
                .await
                .is_err(),
        );
    }
    sqlx::query("UPDATE file_sources SET relative_path='a.txt' WHERE id=$1")
        .bind(id)
        .execute(ctx.pool())
        .await
        .unwrap();
    let leaf = dir.path().join("a.txt");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&leaf)
        .unwrap()
        .set_len(100 * 1024 * 1024 + 1)
        .unwrap();
    let m = safe_fs::metadata(std::fs::File::open(&leaf).unwrap()).unwrap();
    sqlx::query(
        "UPDATE file_sources SET size=$2,observation=$3,captured_observation=$3 WHERE id=$1",
    )
    .bind(id)
    .bind(m.size as i64)
    .bind(sqlx::types::Json(m))
    .execute(ctx.pool())
    .await
    .unwrap();
    let large = download::read_content(&ctx, ReadContent { id }).await;
    let metadata = browse::get_file(&ctx, browse::GetFile { id }).await;
    db.drop().await;
    assert!(rejected.iter().all(|v| *v));
    assert!(matches!(large, Err(download::DomainError::Unavailable)));
    assert!(metadata.is_ok());
}
#[tokio::test]
async fn t49_root_unavailable_keeps_metadata_and_directory_is_rejected() {
    let (db, ctx, dir, id) = fixture().await;
    let path = dir.path().join("a.txt");
    let original_identity: serde_json::Value =
        sqlx::query_scalar("SELECT identity FROM file_roots")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    sqlx::query("UPDATE file_roots SET identity=jsonb_set(identity,'{inode}','0')")
        .execute(ctx.pool())
        .await
        .unwrap();
    let root = download::read_content(&ctx, ReadContent { id }).await;
    sqlx::query("UPDATE file_roots SET identity=$1")
        .bind(original_identity)
        .execute(ctx.pool())
        .await
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    let absent = download::read_content(&ctx, ReadContent { id }).await;
    std::fs::create_dir(&path).unwrap();
    let directory = download::read_content(&ctx, ReadContent { id }).await;
    let list = browse::list_files(
        &ctx,
        browse::ListFiles {
            limit: None,
            cursor: None,
        },
    )
    .await;
    db.drop().await;
    for result in [absent, directory, root] {
        assert!(matches!(result, Err(download::DomainError::Unavailable)));
    }
    assert_eq!(list.unwrap().items.len(), 1);
}
#[tokio::test]
async fn t49_paging_preserves_missing_and_parallel_cas_has_one_winner() {
    let (db, ctx, _dir, id) = fixture().await;
    let other = Uuid::new_v4();
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'file')")
        .bind(other)
        .execute(ctx.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO file_sources SELECT (jsonb_populate_record(NULL::file_sources,to_jsonb(s)||jsonb_build_object('id',$2::uuid,'relative_path','b.txt','status','missing'))).* FROM file_sources s WHERE id=$1").bind(id).bind(other).execute(ctx.pool()).await.unwrap();
    let first = browse::list_files(
        &ctx,
        browse::ListFiles {
            limit: Some(1),
            cursor: None,
        },
    )
    .await
    .unwrap();
    let second = browse::list_files(
        &ctx,
        browse::ListFiles {
            limit: Some(1),
            cursor: first.next_cursor.clone(),
        },
    )
    .await
    .unwrap();
    let (one, two) = tokio::join!(
        browse::update_file(
            &ctx,
            UpdateFile {
                id,
                expected_version: 11,
                note: Some(Some("one".into())),
                tag_ids: None
            }
        ),
        browse::update_file(
            &ctx,
            UpdateFile {
                id,
                expected_version: 11,
                note: Some(Some("two".into())),
                tag_ids: None
            }
        )
    );
    db.drop().await;
    assert!(first.next_cursor.is_some());
    assert!(second.next_cursor.is_none());
    let ids = vec![first.items[0].id, second.items[0].id];
    let mut expected = vec![id, other];
    expected.sort();
    assert_eq!(ids, expected);
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    let loser = if one.is_err() { one } else { two };
    assert!(matches!(
        loser,
        Err(browse::DomainError::Conflict {
            current_version: Some(12)
        })
    ));
}
