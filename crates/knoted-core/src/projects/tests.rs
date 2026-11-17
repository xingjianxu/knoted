//! Deterministic real PostgreSQL overlap tests using the actual Task services.
use super::{
    repository as repo,
    service::{self, ArchiveProject, CreateProject, DomainError},
};
use crate::{
    context::AppContext,
    db::testing::{TestDb, test_database_url},
};
use sqlx::{Connection, PgConnection};
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone, Copy)]
enum TaskWrite {
    Create,
    Move,
    State,
}
async fn connection(db: &TestDb) -> PgConnection {
    let mut c = PgConnection::connect(&test_database_url()).await.unwrap();
    sqlx::query(&format!("SET search_path TO {}, public", db.schema()))
        .execute(&mut c)
        .await
        .unwrap();
    c
}
async fn task_write(
    ctx: &AppContext,
    project: Uuid,
    task: Uuid,
    kind: TaskWrite,
) -> Result<(), crate::tasks::service::DomainError> {
    use crate::tasks::service::{
        self as tasks, CreateTask, MoveTaskProject, TaskStatus, UpdateTask,
    };
    match kind {
        TaskWrite::Create => {
            tasks::create(
                ctx,
                CreateTask {
                    title: "race child".into(),
                    project_id: Some(Some(project)),
                    ..Default::default()
                },
            )
            .await
        }
        TaskWrite::Move => {
            tasks::move_project(
                ctx,
                MoveTaskProject {
                    id: task,
                    expected_version: 1,
                    project_id: Some(project),
                },
            )
            .await
        }
        TaskWrite::State => {
            tasks::update(
                ctx,
                UpdateTask {
                    id: task,
                    expected_version: 1,
                    status: Some(TaskStatus::Todo),
                    ..Default::default()
                },
            )
            .await
        }
    }
    .map(|_| ())
}
async fn blocked(c: &mut PgConnection, by: i32, exclude: Option<i32>) -> i32 {
    tokio::time::timeout(Duration::from_secs(5),async{
        loop{
            sqlx::query("SELECT pg_stat_clear_snapshot()").execute(&mut *c).await.unwrap();
            let pid:Option<i32>=sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE pid<>pg_backend_pid() AND pid IS DISTINCT FROM $2 AND $1=ANY(pg_blocking_pids(pid)) ORDER BY pid LIMIT 1").bind(by).bind(exclude).fetch_optional(&mut *c).await.unwrap();
            if let Some(pid)=pid{return pid;}
            tokio::task::yield_now().await;
        }
    }).await.expect("operation must reach the controlled PostgreSQL wait")
}
async fn race(kind: TaskWrite, writer_first: bool) {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let p = service::create(
        &ctx,
        CreateProject {
            title: "race".into(),
            description: None,
        },
    )
    .await
    .unwrap();
    use crate::tasks::service::{self as tasks, CreateTask, TaskStatus};
    let task = if matches!(kind, TaskWrite::Create) {
        Uuid::nil()
    } else {
        tasks::create(
            &ctx,
            CreateTask {
                title: "race task".into(),
                project_id: Some(if matches!(kind, TaskWrite::Move) {
                    None
                } else {
                    Some(p.id)
                }),
                status: if matches!(kind, TaskWrite::State) {
                    TaskStatus::Done
                } else {
                    TaskStatus::Todo
                },
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
    };
    let mut gate = connection(&db).await;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut gate)
        .await
        .unwrap();
    let mut tx = gate.begin().await.unwrap();
    let (row, table) = if !writer_first || matches!(kind, TaskWrite::Create) || p.id < task {
        (p.id, "projects")
    } else {
        (task, "tasks")
    };
    sqlx::query(&format!("SELECT id FROM {table} WHERE id=$1 FOR UPDATE"))
        .bind(row)
        .execute(&mut *tx)
        .await
        .unwrap();
    let id = p.id;
    let spawn_writer = || {
        let clone = ctx.clone();
        tokio::spawn(async move { task_write(&clone, id, task, kind).await })
    };
    let spawn_archive = || {
        let clone = ctx.clone();
        tokio::spawn(async move {
            service::archive(
                &clone,
                ArchiveProject {
                    id,
                    expected_version: 1,
                },
            )
            .await
        })
    };
    let (writer, archive) = if writer_first {
        let writer = spawn_writer();
        let writer_pid = blocked(&mut tx, pid, None).await;
        let archive = spawn_archive();
        blocked(&mut tx, writer_pid, Some(writer_pid)).await;
        (writer, archive)
    } else {
        let archive = spawn_archive();
        let archive_pid = blocked(&mut tx, pid, None).await;
        let writer = spawn_writer();
        blocked(&mut tx, archive_pid, Some(archive_pid)).await;
        (writer, archive)
    };
    tx.commit().await.unwrap();
    let (writer, archive) = tokio::time::timeout(Duration::from_secs(10), async {
        (writer.await.unwrap(), archive.await.unwrap())
    })
    .await
    .unwrap();
    if writer_first {
        writer.unwrap();
        assert!(matches!(archive, Err(DomainError::Conflict { .. })));
    } else {
        archive.unwrap();
        assert!(matches!(writer, Err(tasks::DomainError::Conflict { .. })));
    }
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let baseline = if matches!(kind, TaskWrite::Create) {
        1
    } else {
        2
    };
    let cascade = if !writer_first && matches!(kind, TaskWrite::State) {
        1
    } else {
        0
    };
    assert_eq!(
        events,
        baseline + 1 + cascade,
        "only the admitted writer and exact archive cascade emit"
    );
    let invalid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects p JOIN tasks t ON t.project_id=p.id WHERE p.archived_at IS NOT NULL AND t.deleted_at IS NULL AND t.status NOT IN ('done','cancelled'))").fetch_one(&db.pool).await.unwrap();
    assert!(!invalid);
}
#[tokio::test]
async fn t33_archive_vs_task_admission() {
    race(TaskWrite::Create, true).await;
    race(TaskWrite::Create, false).await;
}
#[tokio::test]
async fn t33_archive_vs_task_migration() {
    race(TaskWrite::Move, true).await;
    race(TaskWrite::Move, false).await;
}
#[tokio::test]
async fn t33_archive_vs_task_state_change() {
    race(TaskWrite::State, true).await;
    race(TaskWrite::State, false).await;
}

#[tokio::test]
async fn t33_relation_lock_release_on_error() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let p = service::create(
        &ctx,
        CreateProject {
            title: "rollback".into(),
            description: None,
        },
    )
    .await
    .unwrap();
    let mut c = connection(&db).await;
    crate::tasks::service::create(
        &ctx,
        crate::tasks::service::CreateTask {
            title: "block archive".into(),
            project_id: Some(Some(p.id)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    for expected in [99, 1] {
        assert!(matches!(
            service::archive(
                &ctx,
                ArchiveProject {
                    id: p.id,
                    expected_version: expected
                }
            )
            .await,
            Err(DomainError::Conflict { .. })
        ));
        let mut tx = c.begin().await.unwrap();
        let available: bool = sqlx::query_scalar(
            "SELECT pg_try_advisory_xact_lock(33001,'projects'::regclass::oid::integer)",
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        assert!(available);
        // The actual helper succeeds on the next connection, not just try-lock.
        repo::lock_relations(&mut tx).await.unwrap();
        tx.rollback().await.unwrap();
    }
}
