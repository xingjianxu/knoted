//! Real PostgreSQL eligibility parity, keyset pagination and read-only checks.
use knoted_core::{
    context::AppContext,
    db::testing::TestDb,
    notes::service::{self as notes, CreateNote},
    projects::service::{self as projects, CreateProject},
    tasks::service::{self as tasks, CreateTask, TaskStatus},
    trash::{self, EntityType, ListTrash},
};
use uuid::Uuid;
async fn note(ctx: &AppContext, title: &str, parent_id: Option<Uuid>) -> Uuid {
    notes::create(
        ctx,
        CreateNote {
            title: title.into(),
            parent_id,
            document_json: r#"{"blocks":[]}"#.into(),
        },
    )
    .await
    .unwrap()
    .id
}
async fn task(
    ctx: &AppContext,
    title: &str,
    parent_id: Option<Uuid>,
    project_id: Option<Uuid>,
) -> Uuid {
    tasks::create(
        ctx,
        CreateTask {
            title: title.into(),
            parent_id,
            project_id: Some(project_id),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .id
}
#[tokio::test]
async fn t42_tuple_seek_filter_versions_and_registry_archive_exclusion() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let n = note(&ctx, "note", None).await;
    let n2 = note(&ctx, "second note", None).await;
    let t = task(&ctx, "task", None, None).await;
    let p = projects::create(
        &ctx,
        CreateProject {
            title: "project".into(),
            description: None,
        },
    )
    .await
    .unwrap()
    .id;
    let archived = task(&ctx, "archive only", None, None).await;
    sqlx::query("UPDATE tasks SET archived_at=now() WHERE id=$1")
        .bind(archived)
        .execute(&db.pool)
        .await
        .unwrap();
    for (table, id, version_col) in [
        ("notes", n, "content_version"),
        ("notes", n2, "content_version"),
        ("tasks", t, "version"),
        ("projects", p, "version"),
    ] {
        sqlx::query(&format!(
            "UPDATE {table} SET deleted_at='2026-10-01T00:00:00Z', {version_col}=7 WHERE id=$1"
        ))
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    // A stable deleted registry identity without an entity row is not a trash row.
    sqlx::query("INSERT INTO entity_registry(entity_type,id) VALUES ('note',$1)")
        .bind(Uuid::new_v4())
        .execute(&db.pool)
        .await
        .unwrap();
    let mut ids = vec![];
    let mut cursor = None;
    let mut first = None;
    loop {
        let page = trash::list(
            &ctx,
            ListTrash {
                limit: Some(1),
                cursor: cursor.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page.items.len(), 1);
        let i = &page.items[0];
        assert_eq!(i.version, 7);
        assert!(i.can_restore);
        assert!(i.reason.is_none());
        ids.push(i.id);
        cursor = page.next_cursor;
        if first.is_none() {
            first = cursor.clone();
        }
        if cursor.is_none() {
            break;
        }
    }
    let mut expected = vec![n, n2];
    expected.sort();
    expected.extend([p, t]);
    assert_eq!(ids, expected);
    assert!(matches!(
        trash::list(
            &ctx,
            ListTrash {
                cursor: first,
                entity_type: Some(EntityType::Task),
                ..Default::default()
            }
        )
        .await,
        Err(trash::DomainError::InvalidQuery)
    ));
    let page = trash::list(
        &ctx,
        ListTrash {
            entity_type: Some(EntityType::Task),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, t);
    db.drop().await;
}
#[tokio::test]
async fn t42_note_ancestor_parity_and_reads_never_lock_or_mutate() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let grand = note(&ctx, "grand", None).await;
    let parent = note(&ctx, "parent", Some(grand)).await;
    let child = note(&ctx, "child", Some(parent)).await;
    sqlx::query("UPDATE notes SET deleted_at=now()-interval '40 days' WHERE id=$1 OR id=$2")
        .bind(grand)
        .bind(child)
        .execute(&db.pool)
        .await
        .unwrap();
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let mut lock = db.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM notes FOR UPDATE")
        .fetch_all(&mut *lock)
        .await
        .unwrap();
    let page = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        trash::list(&ctx, Default::default()),
    )
    .await;
    lock.rollback().await.unwrap();
    let page = page.unwrap().unwrap();
    assert!(
        page.items
            .iter()
            .find(|i| i.id == child)
            .unwrap()
            .can_restore,
        "deleted distant ancestor is permitted"
    );
    assert_eq!(
        before,
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM outbox")
            .fetch_one(&db.pool)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE notes SET deleted_at=now() WHERE id=$1")
        .bind(parent)
        .execute(&db.pool)
        .await
        .unwrap();
    let page = trash::list(&ctx, Default::default()).await.unwrap();
    assert!(
        !page
            .items
            .iter()
            .find(|i| i.id == child)
            .unwrap()
            .can_restore
    );
    // Cyclic stored ancestry is blocked even if the direct parent is live.
    sqlx::query("UPDATE notes SET deleted_at=NULL,parent_id=$1 WHERE id=$2")
        .bind(child)
        .bind(parent)
        .execute(&db.pool)
        .await
        .unwrap();
    let page = trash::list(&ctx, Default::default()).await.unwrap();
    assert!(
        !page
            .items
            .iter()
            .find(|i| i.id == child)
            .unwrap()
            .can_restore
    );
    db.drop().await;
}
#[tokio::test]
async fn t42_task_parent_project_and_final_state_match_restore_rules() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let p = projects::create(
        &ctx,
        CreateProject {
            title: "project".into(),
            description: None,
        },
    )
    .await
    .unwrap()
    .id;
    let parent = task(&ctx, "parent", None, Some(p)).await;
    let child = task(&ctx, "child", Some(parent), Some(p)).await;
    sqlx::query("UPDATE tasks SET deleted_at=now() WHERE id=$1")
        .bind(child)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE tasks SET status='done' WHERE id=$1")
        .bind(parent)
        .execute(&db.pool)
        .await
        .unwrap();
    let page = trash::list(&ctx, Default::default()).await.unwrap();
    assert!(!page.items[0].can_restore);
    assert!(page.items[0].reason.as_ref().unwrap().contains("状态"));
    sqlx::query("UPDATE tasks SET status='cancelled' WHERE id=$1")
        .bind(parent)
        .execute(&db.pool)
        .await
        .unwrap();
    let page = trash::list(&ctx, Default::default()).await.unwrap();
    assert!(
        page.items[0].can_restore,
        "cancelled parent does not impose done's restriction"
    );
    sqlx::query("UPDATE projects SET archived_at=now() WHERE id=$1")
        .bind(p)
        .execute(&db.pool)
        .await
        .unwrap();
    let page = trash::list(&ctx, Default::default()).await.unwrap();
    assert!(!page.items[0].can_restore);
    sqlx::query("UPDATE projects SET deleted_at=now() WHERE id=$1")
        .bind(p)
        .execute(&db.pool)
        .await
        .unwrap();
    let page = trash::list(&ctx, Default::default()).await.unwrap();
    assert!(
        page.items.iter().find(|i| i.id == p).unwrap().can_restore,
        "deleted project restores even if archived"
    );
    // Restoring a done object with a surviving nonterminal child is also blocked.
    sqlx::query("UPDATE projects SET archived_at=NULL,deleted_at=NULL WHERE id=$1")
        .bind(p)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE tasks SET deleted_at=NULL,status='todo' WHERE id=$1")
        .bind(child)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE tasks SET deleted_at=now(),status='done' WHERE id=$1")
        .bind(parent)
        .execute(&db.pool)
        .await
        .unwrap();
    let page = trash::list(&ctx, Default::default()).await.unwrap();
    assert!(!page.items[0].can_restore);
    // The hint did not mutate the deleted object's stored version/state.
    let stored: (i64, TaskStatus) = sqlx::query_as("SELECT version,status FROM tasks WHERE id=$1")
        .bind(parent)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(stored, (1, TaskStatus::Done));
    db.drop().await;
}
