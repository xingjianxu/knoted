use knoted_core::{
    config::FilesConfig,
    context::AppContext,
    db::testing::TestDb,
    files::{
        capture::{SourceObservation, capture_stable},
        safe_fs,
        scanner::{ObserveRoot, observe_root, reconcile_complete_scan},
    },
};
use std::{path::PathBuf, sync::Arc};
use tempfile::TempDir;
fn temp() -> TempDir {
    tempfile::tempdir_in("/data/knoted-dev/tasks/t45/tmp").unwrap()
}
async fn lease(ctx: &AppContext) -> knoted_core::jobs::Lease {
    let mut tx = ctx.pool().begin().await.unwrap();
    knoted_core::jobs::enqueue(&mut tx, knoted_core::jobs::JobSpec::new("test.capture"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    knoted_core::jobs::claim(ctx.pool(), "t45")
        .await
        .unwrap()
        .unwrap()
}
async fn fixture(size: usize) -> (TestDb, AppContext, TempDir, TempDir, SourceObservation) {
    let db = TestDb::create().await.unwrap();
    let root = temp();
    let staging = temp();
    std::fs::write(root.path().join("a.txt"), vec![b'a'; size]).unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(root.path().into()),
        staging_root: Some(staging.path().into()),
        ..Default::default()
    });
    let l = lease(&ctx).await;
    let report = observe_root(
        &ctx,
        ObserveRoot {
            path: root.path().into(),
            cancel: Arc::default(),
            lease: l.clone(),
            rules: Default::default(),
        },
    )
    .await
    .unwrap();
    let scan_id = report.scan_id;
    let root_id = report.root_id;
    let generation = report.generation;
    assert!(reconcile_complete_scan(&ctx, report).await.unwrap());
    let fd = safe_fs::open_root(root.path()).unwrap();
    let file = std::fs::File::open(root.path().join("a.txt")).unwrap();
    let input = SourceObservation {
        scan_id,
        root_id,
        generation,
        root_path: root.path().into(),
        root_identity: safe_fs::metadata(fd).unwrap().identity,
        relative_path: PathBuf::from("a.txt"),
        metadata: safe_fs::metadata(file).unwrap(),
        lease: l,
        cancel: Arc::default(),
    };
    (db, ctx, root, staging, input)
}

async fn source_id(ctx: &AppContext) -> uuid::Uuid {
    sqlx::query_scalar("SELECT id FROM file_sources WHERE status='present'")
        .fetch_one(ctx.pool())
        .await
        .unwrap()
}
async fn rename_capture(ctx: &AppContext, root: &TempDir, previous: &SourceObservation) {
    std::fs::rename(root.path().join("a.txt"), root.path().join("b.txt")).unwrap();
    let report = observe_root(
        ctx,
        ObserveRoot {
            path: root.path().into(),
            cancel: Arc::default(),
            lease: previous.lease.clone(),
            rules: Default::default(),
        },
    )
    .await
    .unwrap();
    let input = SourceObservation {
        scan_id: report.scan_id,
        root_id: report.root_id,
        generation: report.generation,
        root_path: root.path().into(),
        root_identity: previous.root_identity.clone(),
        relative_path: "b.txt".into(),
        metadata: safe_fs::metadata(std::fs::File::open(root.path().join("b.txt")).unwrap())
            .unwrap(),
        lease: previous.lease.clone(),
        cancel: Arc::default(),
    };
    reconcile_complete_scan(ctx, report).await.unwrap();
    capture_stable(ctx, input).await.unwrap();
}
#[tokio::test]
async fn t45_unique_strong_move_inherits_identity() {
    let (db, ctx, root, _staging, input) = fixture(3).await;
    capture_stable(&ctx, input.clone()).await.unwrap();
    let old = source_id(&ctx).await;
    let strong = input.metadata.identity.handle.is_some();
    rename_capture(&ctx, &root, &input).await;
    let new = source_id(&ctx).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM file_sources")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    assert!(
        strong,
        "fixture filesystem must provide incarnation handles"
    );
    assert_eq!(
        new, old,
        "a uniquely proven rename must retain stable source identity"
    );
    assert_eq!(count, 1);
}
#[tokio::test]
async fn t45_file_tag_actual_membership_advances_metadata_only() {
    use knoted_core::links::service::*;
    let (db, ctx, _root, _staging, input) = fixture(3).await;
    capture_stable(&ctx, input).await.unwrap();
    let id = source_id(&ctx).await;
    let tag = create_tag(&ctx, CreateTag { name: "tag".into() })
        .await
        .unwrap();
    let change = || ChangeEntityTag {
        entity: Entity {
            entity_type: EntityType::File,
            id,
        },
        tag_id: tag.id,
    };
    assign_tag(&ctx, change()).await.unwrap();
    assign_tag(&ctx, change()).await.unwrap();
    let assigned: (i64, i64) =
        sqlx::query_as("SELECT version,content_version FROM file_sources WHERE id=$1")
            .bind(id)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    remove_tag(&ctx, change()).await.unwrap();
    remove_tag(&ctx, change()).await.unwrap();
    let removed: (i64, i64) =
        sqlx::query_as("SELECT version,content_version FROM file_sources WHERE id=$1")
            .bind(id)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    db.drop().await;
    assert_eq!(assigned, (2, 1));
    assert_eq!(removed, (3, 1));
}

use knoted_core::files::identity::{RelinkSource, relink_source};
use uuid::Uuid;
async fn pair() -> (TestDb, AppContext, Uuid, Uuid, TempDir) {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let root = Uuid::new_v4();
    let directory = temp();
    let identity = safe_fs::metadata(safe_fs::open_root(directory.path()).unwrap())
        .unwrap()
        .identity;
    sqlx::query("INSERT INTO file_roots(id,configured_path,identity) VALUES($1,$2,$3)")
        .bind(root)
        .bind(directory.path().to_str().unwrap())
        .bind(sqlx::types::Json(identity))
        .execute(ctx.pool())
        .await
        .unwrap();
    let old = seed(&ctx, root, "old", Some("my note")).await;
    let new = seed(&ctx, root, "new", None).await;
    (db, ctx, old, new, directory)
}
async fn seed(ctx: &AppContext, root: Uuid, path: &str, note: Option<&str>) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'file')")
        .bind(id)
        .execute(ctx.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO file_sources(id,root_id,relative_path,device,inode,size,mtime_ns,ctime_ns,nlink,sha256,content_version,version,status,note,observation,last_seen_at) VALUES($1,$2,$3,1,2,3,4,5,1,repeat('a',64),7,11,'present',$4,'{}',now())").bind(id).bind(root).bind(path).bind(note).execute(ctx.pool()).await.unwrap();
    id
}
fn relink(old: Uuid, new: Uuid, version: i64) -> RelinkSource {
    RelinkSource {
        id: old,
        new_source_id: new,
        expected_version: version,
    }
}
#[tokio::test]
async fn t45_3_relink_transfers_metadata_preserves_old_content_and_snapshots() {
    use knoted_core::links::service::*;
    let (db, ctx, old, new, _directory) = pair().await;
    let tag = create_tag(
        &ctx,
        CreateTag {
            name: "old-tag".into(),
        },
    )
    .await
    .unwrap();
    assign_tag(
        &ctx,
        ChangeEntityTag {
            entity: Entity {
                entity_type: EntityType::File,
                id: old,
            },
            tag_id: tag.id,
        },
    )
    .await
    .unwrap();
    let edge = create_link(
        &ctx,
        CreateLink {
            source_type: EntityType::File,
            source_id: old,
            target_type: EntityType::File,
            target_id: new,
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE file_sources SET status='missing' WHERE id=$1")
        .bind(old)
        .execute(ctx.pool())
        .await
        .unwrap();
    let snapshot: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(s)-'note'-'version'-'updated_at' FROM file_sources s WHERE id=$1",
    )
    .bind(old)
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    let result = relink_source(&ctx, relink(old, new, 12)).await;
    let after: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(s)-'note'-'version'-'updated_at' FROM file_sources s WHERE id=$1",
    )
    .bind(old)
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    let old_meta: (Option<String>, i64) =
        sqlx::query_as("SELECT note,version FROM file_sources WHERE id=$1")
            .bind(old)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    let moved: (Uuid, Uuid, i64) =
        sqlx::query_as("SELECT source_id,target_id,version FROM entity_links WHERE id=$1")
            .bind(edge.id)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    db.drop().await;
    let result = result.expect("explicit pair must atomically transfer metadata");
    assert_eq!(result.id, new);
    assert_eq!(result.note.as_deref(), Some("my note"));
    assert_eq!(result.tag_ids, vec![tag.id]);
    assert_eq!(result.version, 12);
    assert_eq!(result.content_version, 7);
    assert_eq!(old_meta, (None, 13));
    assert_eq!(snapshot, after);
    assert_eq!(moved, (new, new, 2));
}

#[tokio::test]
async fn t45_relink_rechecks_root_before_scanner_reports_replacement() {
    let (db, ctx, old, new, directory) = pair().await;
    let before: serde_json::Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM file_sources s")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    let path = directory.path().to_owned();
    let moved = path.with_extension("moved");
    std::fs::rename(&path, &moved).unwrap();
    let missing = relink_source(&ctx, relink(old, new, 11)).await;
    std::fs::create_dir(&path).unwrap();
    let replaced = relink_source(&ctx, relink(old, new, 11)).await;
    std::fs::remove_dir(&moved).unwrap();
    let after: serde_json::Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM file_sources s")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    db.drop().await;
    assert!(missing.is_err());
    assert!(replaced.is_err());
    assert_eq!(before, after);
}

async fn rejected_move(mode: &str) {
    let (db, ctx, root, _staging, input) = fixture(3).await;
    capture_stable(&ctx, input.clone()).await.unwrap();
    let old = source_id(&ctx).await;
    match mode {
        "inode" => {
            sqlx::query("UPDATE file_sources SET inode=inode+1")
                .execute(ctx.pool())
                .await
                .unwrap();
        }
        "handle" => {
            sqlx::query("UPDATE file_sources SET opaque_handle='{\"kind\":77,\"bytes\":[1,2,3]}'")
                .execute(ctx.pool())
                .await
                .unwrap();
        }
        "unsupported" => {
            sqlx::query("UPDATE file_sources SET opaque_handle=NULL")
                .execute(ctx.pool())
                .await
                .unwrap();
        }
        "historical" => {
            sqlx::query("UPDATE file_sources SET status='missing'")
                .execute(ctx.pool())
                .await
                .unwrap();
        }
        "old_hardlink" => {
            sqlx::query("UPDATE file_sources SET nlink=2")
                .execute(ctx.pool())
                .await
                .unwrap();
        }
        "new_hardlink" => {
            std::fs::hard_link(root.path().join("a.txt"), root.path().join("other.txt")).unwrap();
        }
        "digest" => {
            std::fs::write(root.path().join("a.txt"), b"bbb").unwrap();
        }
        "ambiguous" => {
            let extra = seed(&ctx, input.root_id, "another-old", None).await;
            sqlx::query("UPDATE file_sources SET device=$2,inode=$3,opaque_handle=$4 WHERE id=$1")
                .bind(extra)
                .bind(input.metadata.identity.device as i64)
                .bind(input.metadata.identity.inode as i64)
                .bind(sqlx::types::Json(&input.metadata.identity.handle))
                .execute(ctx.pool())
                .await
                .unwrap();
        }
        _ => panic!("unknown fixture"),
    }
    rename_capture(&ctx, &root, &input).await;
    let new = source_id(&ctx).await;
    let old_status: String = sqlx::query_scalar("SELECT status FROM file_sources WHERE id=$1")
        .bind(old)
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    assert_ne!(
        old, new,
        "ambiguous or unproven move must create a source: {mode}"
    );
    assert_eq!(old_status, "missing");
}
#[tokio::test]
async fn t45_1_same_hash_different_inode_not_merged() {
    rejected_move("inode").await;
}
#[tokio::test]
async fn t45_2_inode_reuse_different_handle_not_merged() {
    rejected_move("handle").await;
}
#[tokio::test]
async fn t45_unsupported_identity_not_merged() {
    rejected_move("unsupported").await;
}
#[tokio::test]
async fn t45_historical_missing_not_resurrected() {
    rejected_move("historical").await;
}
#[tokio::test]
async fn t45_old_hardlink_ambiguous() {
    rejected_move("old_hardlink").await;
}
#[tokio::test]
async fn t45_new_hardlink_ambiguous() {
    rejected_move("new_hardlink").await;
}
#[tokio::test]
async fn t45_changed_digest_not_merged() {
    rejected_move("digest").await;
}
#[tokio::test]
async fn t45_multiple_old_candidates_not_merged() {
    rejected_move("ambiguous").await;
}

async fn snapshot(ctx: &AppContext) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('sources',(SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM file_sources s),'tags',(SELECT jsonb_agg(to_jsonb(t) ORDER BY entity_id,tag_id) FROM file_tags t),'links',(SELECT jsonb_agg(to_jsonb(l) ORDER BY id) FROM entity_links l))").fetch_one(ctx.pool()).await.unwrap()
}
#[tokio::test]
async fn t45_conflicts_are_atomic_and_release_locks() {
    let (db, ctx, old, new, _directory) = pair().await;
    sqlx::query("UPDATE file_sources SET note='target note' WHERE id=$1")
        .bind(new)
        .execute(ctx.pool())
        .await
        .unwrap();
    let before = snapshot(&ctx).await;
    let different = relink_source(&ctx, relink(old, new, 11)).await;
    let stale = relink_source(&ctx, relink(old, new, 10)).await;
    let self_pair = relink_source(&ctx, relink(old, old, 11)).await;
    let absent = relink_source(&ctx, relink(old, Uuid::new_v4(), 11)).await;
    let after = snapshot(&ctx).await;
    // A subsequent transaction can lock and update both rows after error replies.
    sqlx::query("UPDATE file_sources SET note=NULL WHERE id=$1")
        .bind(new)
        .execute(ctx.pool())
        .await
        .unwrap();
    let success = relink_source(&ctx, relink(old, new, 11)).await;
    db.drop().await;
    assert!(matches!(
        different,
        Err(knoted_core::files::identity::DomainError::Conflict { .. })
    ));
    assert!(matches!(
        stale,
        Err(knoted_core::files::identity::DomainError::Conflict {
            current_version: Some(11)
        })
    ));
    assert!(self_pair.is_err());
    assert!(absent.is_err());
    assert_eq!(before, after);
    assert!(success.is_ok());
}
#[tokio::test]
async fn t45_union_dedup_preserves_target_edges_and_deleted_endpoint_rows() {
    use knoted_core::links::service::*;
    let (db, ctx, old, new, _directory) = pair().await;
    let root: Uuid = sqlx::query_scalar("SELECT root_id FROM file_sources WHERE id=$1")
        .bind(old)
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    let third = seed(&ctx, root, "third", None).await;
    let mut tag_ids = vec![];
    for (name, ids) in [
        ("shared", vec![old, new]),
        ("old-only", vec![old]),
        ("new-only", vec![new]),
        ("deleted", vec![old]),
    ] {
        let tag = create_tag(&ctx, CreateTag { name: name.into() })
            .await
            .unwrap();
        for id in ids {
            assign_tag(
                &ctx,
                ChangeEntityTag {
                    entity: Entity {
                        entity_type: EntityType::File,
                        id,
                    },
                    tag_id: tag.id,
                },
            )
            .await
            .unwrap();
        }
        if name == "deleted" {
            delete_tag(
                &ctx,
                DeleteTag {
                    id: tag.id,
                    expected_version: 1,
                },
            )
            .await
            .unwrap();
        } else {
            tag_ids.push(tag.id);
        }
    }
    let mut retained = vec![];
    for (source_id, target_id) in [
        (new, third),
        (third, new),
        (new, new),
        (old, third),
        (third, old),
        (old, new),
        (new, old),
        (old, old),
    ] {
        let e = create_link(
            &ctx,
            CreateLink {
                source_type: EntityType::File,
                source_id,
                target_type: EntityType::File,
                target_id,
            },
        )
        .await
        .unwrap();
        if source_id != old && target_id != old {
            retained.push(e.id);
        }
    }
    sqlx::query("UPDATE file_sources SET status='missing' WHERE id=$1")
        .bind(third)
        .execute(ctx.pool())
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("SELECT version FROM file_sources WHERE id=$1")
        .bind(old)
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    let result = relink_source(&ctx, relink(old, new, version)).await;
    let edges: Vec<(Uuid, i64)> = sqlx::query_as("SELECT id,version FROM entity_links ORDER BY id")
        .fetch_all(ctx.pool())
        .await
        .unwrap();
    let memberships: Vec<(Uuid, i64)> =
        sqlx::query_as("SELECT entity_id,count(*) FROM file_tags GROUP BY entity_id")
            .fetch_all(ctx.pool())
            .await
            .unwrap();
    db.drop().await;
    let result = result.unwrap();
    tag_ids.sort();
    retained.sort();
    assert_eq!(result.tag_ids, tag_ids);
    assert_eq!(
        edges,
        retained.into_iter().map(|id| (id, 1)).collect::<Vec<_>>()
    );
    assert_eq!(memberships, vec![(new, 4)]);
    assert_eq!(result.note.as_deref(), Some("my note"));
}

#[tokio::test]
async fn t45_concurrent_tag_writers_and_relink_obey_old_cas() {
    use knoted_core::links::service::*;
    for assign in [true, false] {
        let (db, ctx, old, new, _directory) = pair().await;
        let tag = create_tag(
            &ctx,
            CreateTag {
                name: "racing tag".into(),
            },
        )
        .await
        .unwrap();
        let change = || ChangeEntityTag {
            entity: Entity {
                entity_type: EntityType::File,
                id: old,
            },
            tag_id: tag.id,
        };
        if !assign {
            assign_tag(&ctx, change()).await.unwrap();
        }
        let race = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(
                relink_source(&ctx, relink(old, new, if assign { 11 } else { 12 })),
                async {
                    if assign {
                        assign_tag(&ctx, change()).await
                    } else {
                        remove_tag(&ctx, change()).await
                    }
                }
            )
        })
        .await;
        let members: Vec<Uuid> = sqlx::query_scalar(
            "SELECT entity_id FROM file_tags WHERE tag_id=$1 ORDER BY entity_id",
        )
        .bind(tag.id)
        .fetch_all(ctx.pool())
        .await
        .unwrap();
        let notes: Vec<(Uuid, Option<String>)> =
            sqlx::query_as("SELECT id,note FROM file_sources ORDER BY id")
                .fetch_all(ctx.pool())
                .await
                .unwrap();
        db.drop().await;
        let (moved, written) = race.expect("no lock inversion");
        written.unwrap();
        if assign {
            assert_eq!(members.len(), 1);
            // Assignment first invalidates old CAS. Assignment after transfer
            // remains on the old present source, not silently redirected.
            assert_eq!(members, vec![old]);
        } else if moved.is_ok() {
            assert_eq!(members, vec![new]);
        } else {
            assert!(members.is_empty());
        }
        let note_owner = notes
            .into_iter()
            .find(|(_, note)| note.is_some())
            .unwrap()
            .0;
        assert_eq!(note_owner, if moved.is_ok() { new } else { old });
    }
}
#[tokio::test]
async fn t45_concurrent_create_remove_links_serialize_with_transfer() {
    use knoted_core::links::service::*;
    for create in [true, false] {
        let (db, ctx, old, new, _directory) = pair().await;
        let root: Uuid = sqlx::query_scalar("SELECT root_id FROM file_sources WHERE id=$1")
            .bind(old)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
        let third = seed(&ctx, root, "third", None).await;
        let input = || CreateLink {
            source_type: EntityType::File,
            source_id: old,
            target_type: EntityType::File,
            target_id: third,
        };
        let edge = if !create {
            Some(create_link(&ctx, input()).await.unwrap())
        } else {
            None
        };
        let race = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(relink_source(&ctx, relink(old, new, 11)), async {
                if create {
                    create_link(&ctx, input()).await.map(|_| ())
                } else {
                    let edge = edge.as_ref().unwrap();
                    remove_link(
                        &ctx,
                        RemoveLink {
                            id: edge.id,
                            expected_version: edge.version,
                        },
                    )
                    .await
                }
            })
        })
        .await;
        let edges: Vec<(Uuid, Uuid, i64)> =
            sqlx::query_as("SELECT source_id,target_id,version FROM entity_links")
                .fetch_all(ctx.pool())
                .await
                .unwrap();
        db.drop().await;
        let (moved, written) = race.expect("no lock inversion");
        moved.unwrap();
        if create {
            written.unwrap();
            assert_eq!(edges.len(), 1);
            assert!([old, new].contains(&edges[0].0));
            assert_eq!(edges[0].1, third);
        } else if written.is_ok() {
            assert!(edges.is_empty());
        } else {
            assert_eq!(edges, vec![(new, third, 2)]);
            assert!(matches!(
                written,
                Err(DomainError::Conflict {
                    current_version: Some(2)
                })
            ));
        }
    }
}
#[tokio::test]
async fn t45_concurrent_complete_scan_target_missing_cannot_transfer() {
    let (db, ctx, root, _staging, input) = fixture(3).await;
    capture_stable(&ctx, input.clone()).await.unwrap();
    let old = source_id(&ctx).await;
    sqlx::query("UPDATE file_sources SET note='keep',version=version+1 WHERE id=$1")
        .bind(old)
        .execute(ctx.pool())
        .await
        .unwrap();
    let new = seed(&ctx, input.root_id, "disappeared", None).await;
    let report = observe_root(
        &ctx,
        ObserveRoot {
            path: root.path().into(),
            cancel: Arc::default(),
            lease: input.lease.clone(),
            rules: Default::default(),
        },
    )
    .await
    .unwrap();
    let race = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(
            reconcile_complete_scan(&ctx, report),
            relink_source(&ctx, relink(old, new, 2))
        )
    })
    .await;
    let note: Option<String> = sqlx::query_scalar("SELECT note FROM file_sources WHERE id=$1")
        .bind(old)
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM file_sources WHERE id=$1")
        .bind(new)
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    let (scan, moved) = race.expect("scan and relink must not deadlock");
    assert!(scan.unwrap());
    assert!(moved.is_err());
    assert_eq!(note.as_deref(), Some("keep"));
    assert_eq!(status, "missing");
}

#[tokio::test]
async fn t45_3_relink_keeps_closed_input_and_outbox_history() {
    let (db, ctx, _root, staging, input) = fixture(3).await;
    let object = capture_stable(&ctx, input.clone()).await.unwrap();
    let old = source_id(&ctx).await;
    let new = seed(&ctx, input.root_id, "target.txt", None).await;
    sqlx::query(
        "UPDATE file_sources SET note='transfer',version=version+1,status='missing' WHERE id=$1",
    )
    .bind(old)
    .execute(ctx.pool())
    .await
    .unwrap();
    let payload =
        serde_json::json!({"source_id":old,"content_version":1,"title":"a.txt","snippet":"aaa"});
    let event = Uuid::new_v4();
    sqlx::query("INSERT INTO outbox(id,topic,payload) VALUES($1,'test.historical-source',$2)")
        .bind(event)
        .bind(&payload)
        .execute(ctx.pool())
        .await
        .unwrap();
    let before: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(s)-'note'-'version'-'updated_at' FROM file_sources s WHERE id=$1",
    )
    .bind(old)
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    let moved = relink_source(&ctx, relink(old, new, 2)).await;
    let after: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(s)-'note'-'version'-'updated_at' FROM file_sources s WHERE id=$1",
    )
    .bind(old)
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    let historical: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM outbox WHERE id=$1")
            .bind(event)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    let bytes =
        std::fs::read(staging.path().join("input").join(object.object_id.as_str())).unwrap();
    db.drop().await;
    assert!(moved.is_ok());
    assert_eq!(before, after);
    assert_eq!(payload, historical);
    assert_eq!(bytes, b"aaa");
}
#[tokio::test]
async fn t45_previous_complete_missing_not_eligible_in_later_scan() {
    let (db, ctx, root, _staging, input) = fixture(3).await;
    capture_stable(&ctx, input.clone()).await.unwrap();
    let old = source_id(&ctx).await;
    // Keep the original inode outside observation, then rediscover it in a
    // later complete scan. Even a strong handle cannot revive history.
    std::fs::rename(root.path().join("a.txt"), root.path().join(".hidden")).unwrap();
    let report = observe_root(
        &ctx,
        ObserveRoot {
            path: root.path().into(),
            cancel: Arc::default(),
            lease: input.lease.clone(),
            rules: Default::default(),
        },
    )
    .await
    .unwrap();
    reconcile_complete_scan(&ctx, report).await.unwrap();
    std::fs::rename(root.path().join(".hidden"), root.path().join("a.txt")).unwrap();
    rename_capture(&ctx, &root, &input).await;
    let new = source_id(&ctx).await;
    db.drop().await;
    assert_ne!(old, new);
}
#[tokio::test]
async fn t45_same_bytes_real_copy_is_separate_source() {
    let (db, ctx, root, _staging, input) = fixture(3).await;
    capture_stable(&ctx, input.clone()).await.unwrap();
    let old = source_id(&ctx).await;
    std::fs::copy(root.path().join("a.txt"), root.path().join("copy.txt")).unwrap();
    std::fs::remove_file(root.path().join("a.txt")).unwrap();
    std::fs::rename(root.path().join("copy.txt"), root.path().join("a.txt")).unwrap();
    let copied =
        safe_fs::metadata(std::fs::File::open(root.path().join("a.txt")).unwrap()).unwrap();
    rename_capture(&ctx, &root, &input).await;
    let new = source_id(&ctx).await;
    db.drop().await;
    assert_ne!(copied.identity.inode, input.metadata.identity.inode);
    assert_ne!(old, new);
}
#[tokio::test]
async fn t45_equal_notes_clear_old_but_empty_different_note_conflicts() {
    for target in ["my note", ""] {
        let (db, ctx, old, new, _directory) = pair().await;
        sqlx::query("UPDATE file_sources SET note=$2 WHERE id=$1")
            .bind(new)
            .bind(target)
            .execute(ctx.pool())
            .await
            .unwrap();
        let before = snapshot(&ctx).await;
        let result = relink_source(&ctx, relink(old, new, 11)).await;
        let after = snapshot(&ctx).await;
        db.drop().await;
        if target.is_empty() {
            assert!(result.is_err());
            assert_eq!(before, after);
        } else {
            let result = result.unwrap();
            assert_eq!(result.note.as_deref(), Some("my note"));
            assert_eq!(result.version, 11);
        }
    }
}
