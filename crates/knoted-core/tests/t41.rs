//! Fixed-clock real PostgreSQL regression coverage for the read-only overview.
use knoted_core::{
    context::{AppContext, FixedClock},
    dashboard::{self, GetDashboard},
    db::testing::TestDb,
    notes::service::{self as notes, CreateNote},
    tasks::service::{self as tasks, CreateTask, DateValue, TaskStatus},
};
use std::sync::Arc;
use uuid::Uuid;

async fn task(
    ctx: &AppContext,
    title: &str,
    status: TaskStatus,
    date: &str,
    time: Option<&str>,
) -> Uuid {
    tasks::create(
        ctx,
        CreateTask {
            title: title.into(),
            status,
            due: Some(DateValue {
                date: date.into(),
                time: time.map(Into::into),
                timezone: "Asia/Shanghai".into(),
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .id
}
#[tokio::test]
async fn t41_1_calendar_today_and_overdue_exclude_terminal_archived_deleted() {
    let db = TestDb::create().await.unwrap();
    // UTC Nov 16, already Shanghai Nov 17: catches accidental UTC calendar use.
    let clock = FixedClock::new("2026-11-16T16:30:00Z".parse().unwrap());
    let ctx = AppContext::new(db.pool.clone()).with_clock(Arc::new(clock.clone()));
    let old = task(&ctx, "past", TaskStatus::Todo, "2026-11-16", None).await;
    let exact = task(
        &ctx,
        "timed today",
        TaskStatus::InProgress,
        "2026-11-17",
        Some("23:59"),
    )
    .await;
    let today = task(&ctx, "date today", TaskStatus::Todo, "2026-11-17", None).await;
    task(&ctx, "future", TaskStatus::Todo, "2026-11-18", None).await;
    task(&ctx, "done", TaskStatus::Done, "2020-01-01", None).await;
    task(&ctx, "cancelled", TaskStatus::Cancelled, "2020-01-01", None).await;
    let archived = task(&ctx, "archived", TaskStatus::Done, "2020-01-01", None).await;
    sqlx::query("UPDATE tasks SET archived_at=$2 WHERE id=$1")
        .bind(archived)
        .bind(ctx.now())
        .execute(&db.pool)
        .await
        .unwrap();
    let deleted = task(&ctx, "deleted", TaskStatus::Todo, "2020-01-01", None).await;
    sqlx::query("UPDATE tasks SET deleted_at=$2 WHERE id=$1")
        .bind(deleted)
        .bind(ctx.now())
        .execute(&db.pool)
        .await
        .unwrap();
    let s = dashboard::summary(&ctx, GetDashboard).await.unwrap();
    assert_eq!(
        s.due_tasks.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![old, exact, today]
    );
    assert_eq!(s.counts.tasks_due_today, 2);
    assert_eq!(s.counts.tasks_open, 4);
    assert_eq!(
        (
            s.tasks_by_status.todo,
            s.tasks_by_status.in_progress,
            s.tasks_by_status.done,
            s.tasks_by_status.cancelled
        ),
        (3, 1, 1, 1)
    );
    assert_eq!(s.recent_tasks.len(), 6);
    clock.set("2026-11-17T16:00:00Z".parse().unwrap());
    let s = dashboard::summary(&ctx, GetDashboard).await.unwrap();
    assert_eq!(s.counts.tasks_due_today, 1);
    assert_eq!(s.due_tasks.len(), 4);
    db.drop().await;
}
#[tokio::test]
async fn t41_recent_sort_limits_and_untruncated_counts_with_no_write_side_effects() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_clock(Arc::new(FixedClock::new(
        "2026-11-17T00:00:00Z".parse().unwrap(),
    )));
    let mut note_ids = Vec::new();
    let mut task_ids = Vec::new();
    for i in 0..14 {
        note_ids.push(
            notes::create(
                &ctx,
                CreateNote {
                    title: format!("note{i}"),
                    parent_id: None,
                    document_json: r#"{"blocks":[]}"#.into(),
                },
            )
            .await
            .unwrap()
            .id,
        );
        task_ids.push(
            task(
                &ctx,
                &format!("task{i}"),
                TaskStatus::Todo,
                "2026-11-17",
                None,
            )
            .await,
        );
    }
    // Newer timestamps outrank UUID ties; UUID still determines tied order.
    let newest_note = *note_ids.iter().min().unwrap();
    let newest_task = *task_ids.iter().min().unwrap();
    for (table, id) in [("notes", newest_note), ("tasks", newest_task)] {
        sqlx::query(&format!("UPDATE {table} SET updated_at=$2 WHERE id=$1"))
            .bind(id)
            .bind(ctx.now() + chrono::Duration::seconds(1))
            .execute(&db.pool)
            .await
            .unwrap();
    }
    let deleted_note = notes::create(
        &ctx,
        CreateNote {
            title: "deleted".into(),
            parent_id: None,
            document_json: r#"{"blocks":[]}"#.into(),
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE notes SET deleted_at=$2 WHERE id=$1")
        .bind(deleted_note.id)
        .bind(ctx.now())
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO jobs (kind,state,last_error,payload) VALUES ('parse','failed','PRIVATE ERROR','{\"body\":\"PRIVATE BODY\"}'),('parse','failed',NULL,'{}'),('parse','retry_wait',NULL,'{}'),('parse','cancelled',NULL,'{}')").execute(&db.pool).await.unwrap();
    for title in ["active", "archived", "deleted"] {
        knoted_core::projects::service::create(
            &ctx,
            knoted_core::projects::service::CreateProject {
                title: title.into(),
                description: None,
            },
        )
        .await
        .unwrap();
    }
    sqlx::query("UPDATE projects SET archived_at=$1 WHERE title='archived'")
        .bind(ctx.now())
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE projects SET deleted_at=$1 WHERE title='deleted'")
        .bind(ctx.now())
        .execute(&db.pool)
        .await
        .unwrap();
    let before: (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM jobs),(SELECT count(*) FROM outbox)")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let s = dashboard::summary(&ctx, GetDashboard).await.unwrap();
    note_ids.sort_unstable_by(|a, b| b.cmp(a));
    note_ids.retain(|id| *id != newest_note);
    note_ids.insert(0, newest_note);
    task_ids.sort_unstable();
    assert_eq!(
        s.recent_notes.iter().map(|n| n.id).collect::<Vec<_>>(),
        note_ids[..10]
    );
    assert_eq!(
        s.due_tasks.iter().map(|t| t.id).collect::<Vec<_>>(),
        task_ids[..10]
    );
    task_ids.reverse();
    task_ids.retain(|id| *id != newest_task);
    task_ids.insert(0, newest_task);
    assert_eq!(
        s.recent_tasks.iter().map(|t| t.id).collect::<Vec<_>>(),
        task_ids[..10]
    );
    assert_eq!(
        (
            s.counts.notes,
            s.counts.tasks_open,
            s.counts.tasks_due_today,
            s.counts.projects_active,
            s.counts.files
        ),
        (14, 14, 14, 1, 0)
    );
    assert_eq!(s.job_failures, 2);
    assert!(!s.indexing_enabled);
    let body = serde_json::to_string(&s).unwrap();
    assert!(!body.contains("PRIVATE"));
    let after: (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM jobs),(SELECT count(*) FROM outbox)")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    db.drop().await;
}

#[tokio::test]
async fn t41_valid_precise_deadline_on_skipped_midnight_day() {
    let db = TestDb::create().await.unwrap();
    let clock = FixedClock::new("1919-04-12T18:30:00Z".parse().unwrap());
    let ctx = AppContext::new(db.pool.clone()).with_clock(Arc::new(clock));
    // Shanghai skipped 00:00 on this day; 03:00 is a valid precise deadline.
    // Creation exercises T35 validation independently of dashboard aggregation.
    let id = task(
        &ctx,
        "valid historical deadline",
        TaskStatus::Todo,
        "1919-04-13",
        Some("03:00"),
    )
    .await;
    let result = dashboard::summary(&ctx, GetDashboard).await;
    db.drop().await;
    let s = result.unwrap();
    assert_eq!(s.counts.tasks_due_today, 1);
    assert_eq!(s.due_tasks.len(), 1);
    assert_eq!(s.due_tasks[0].id, id);
}
