//! Deterministic real PostgreSQL overlap tests. The simulated T34 writers use
//! the very same relation helper; replace them with T34 services when available.
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
async fn insert_task(c: &mut PgConnection, id: Uuid, project: Option<Uuid>, status: &str) {
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'task')")
        .bind(id)
        .execute(&mut *c)
        .await
        .unwrap();
    sqlx::query("INSERT INTO tasks(id,project_id,status,created_at,updated_at) VALUES($1,$2,$3,now(),now())").bind(id).bind(project).bind(status).execute(c).await.unwrap();
}
// Mirrors the future admission contract, using production's exact lock helper.
async fn simulate_t34(
    c: &mut PgConnection,
    project: Uuid,
    task: Uuid,
    kind: TaskWrite,
) -> Result<(), DomainError> {
    repo::lock_relations(c).await?;
    let mut rows = vec![(project, false)];
    if !matches!(kind, TaskWrite::Create) {
        rows.push((task, true));
    }
    rows.sort_by_key(|r| r.0);
    for (id, is_task) in rows {
        sqlx::query(if is_task {
            "SELECT id FROM tasks WHERE id=$1 FOR UPDATE"
        } else {
            "SELECT id FROM projects WHERE id=$1 FOR UPDATE"
        })
        .bind(id)
        .execute(&mut *c)
        .await?;
    }
    let p = repo::find(c, project).await?.ok_or(DomainError::NotFound)?;
    if p.archived_at.is_some() || p.deleted_at.is_some() {
        return Err(DomainError::Conflict {
            current_version: Some(p.version),
        });
    }
    match kind {
        TaskWrite::Create => insert_task(c, task, Some(project), "todo").await,
        TaskWrite::Move | TaskWrite::State => {
            let archived: bool = sqlx::query_scalar(
                "SELECT archived_at IS NOT NULL OR deleted_at IS NOT NULL FROM tasks WHERE id=$1",
            )
            .bind(task)
            .fetch_one(&mut *c)
            .await?;
            if archived {
                return Err(DomainError::Conflict {
                    current_version: None,
                });
            }
            sqlx::query("UPDATE tasks SET project_id=$2,status='todo',version=version+1,updated_at=now() WHERE id=$1").bind(task).bind(project).execute(c).await?;
        }
    }
    Ok(())
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
    let task = Uuid::from_u128(1);
    let mut gate = connection(&db).await;
    if !matches!(kind, TaskWrite::Create) {
        insert_task(
            &mut gate,
            task,
            if matches!(kind, TaskWrite::Move) {
                None
            } else {
                Some(p.id)
            },
            "done",
        )
        .await;
    }
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut gate)
        .await
        .unwrap();
    let mut tx = gate.begin().await.unwrap();
    if writer_first {
        simulate_t34(&mut tx, p.id, task, kind).await.unwrap();
        let clone = ctx.clone();
        let id = p.id;
        let archive = tokio::spawn(async move {
            service::archive(
                &clone,
                ArchiveProject {
                    id,
                    expected_version: 1,
                },
            )
            .await
        });
        blocked(&mut tx, pid, None).await;
        tx.commit().await.unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), archive)
                .await
                .unwrap()
                .unwrap(),
            Err(DomainError::Conflict { .. })
        ));
    } else {
        sqlx::query("SELECT id FROM projects WHERE id=$1 FOR UPDATE")
            .bind(p.id)
            .execute(&mut *tx)
            .await
            .unwrap();
        let clone = ctx.clone();
        let id = p.id;
        let archive = tokio::spawn(async move {
            service::archive(
                &clone,
                ArchiveProject {
                    id,
                    expected_version: 1,
                },
            )
            .await
        });
        let arch_pid = blocked(&mut tx, pid, None).await;
        let pool = db.pool.clone();
        let writer = tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            let result = simulate_t34(&mut tx, id, task, kind).await;
            match result {
                Ok(()) => {
                    tx.commit().await.unwrap();
                    Ok(())
                }
                Err(e) => {
                    tx.rollback().await.unwrap();
                    Err(e)
                }
            }
        });
        blocked(&mut tx, arch_pid, Some(arch_pid)).await;
        tx.commit().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), archive)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), writer)
                .await
                .unwrap()
                .unwrap(),
            Err(DomainError::Conflict { .. })
        ));
    }
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
    insert_task(&mut c, Uuid::new_v4(), Some(p.id), "todo").await;
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
