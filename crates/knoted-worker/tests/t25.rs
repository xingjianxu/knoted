//! The actual Worker binary must stay alive and recover durable history.
use knoted_core::{
    context::AppContext,
    db::testing::{TestDb, test_database_url},
    notes::service::{self, CreateNote, UpdateNote},
};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use uuid::Uuid;

struct Worker {
    child: Child,
    config: PathBuf,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.config);
    }
}
fn start(db: &TestDb) -> Worker {
    let base = test_database_url();
    let url = format!(
        "{base}{}options=-csearch_path%3D{}%2Cpublic",
        if base.contains('?') { "&" } else { "?" },
        db.schema()
    );
    let config = std::env::temp_dir().join(format!("knoted-t25-{}.toml", Uuid::new_v4()));
    fs::write(&config, format!("[database]\nurl = {url:?}\n")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_knoted-worker"))
        .arg("--config")
        .arg(&config)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    Worker { child, config }
}
async fn wait_for_snapshot(db: &TestDb, note: Uuid, version: i64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM note_snapshots WHERE note_id=$1 AND content_version=$2",
        )
        .bind(note)
        .bind(version)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        if count == 1 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not recover latest version"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// The previous binary exited immediately after loading config and saved nothing.
#[tokio::test]
async fn t25_2_real_worker_stays_running_backfills_and_restarts_idempotently() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let n = service::create(
        &ctx,
        CreateNote {
            title: "crash latest".into(),
            parent_id: None,
            document_json: r#"{"blocks":[]}"#.into(),
        },
    )
    .await
    .unwrap();
    let mut worker = start(&db);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        worker.child.try_wait().unwrap().is_none(),
        "worker must keep running after loading configuration"
    );
    wait_for_snapshot(&db, n.id, 1).await;
    drop(worker);
    let n = service::update(
        &ctx,
        UpdateNote {
            id: n.id,
            expected_version: 1,
            title: Some("edited while stopped".into()),
            document_json: None,
        },
    )
    .await
    .unwrap();
    let mut worker = start(&db);
    wait_for_snapshot(&db, n.id, 2).await;
    assert!(worker.child.try_wait().unwrap().is_none());
    drop(worker);
    let mut worker = start(&db);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(worker.child.try_wait().unwrap().is_none());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM note_snapshots WHERE note_id=$1")
        .bind(n.id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

// An unknown CLI value may contain pasted private data; diagnostics must omit it.
#[test]
fn t25_worker_cli_rejects_unknown_argument_without_echoing_value() {
    let output = Command::new(env!("CARGO_BIN_EXE_knoted-worker"))
        .arg("--unknown=private-marker")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unknown argument"));
    assert!(!stderr.contains("private-marker"));
}

// Regression check of the running process: startup-only maintenance would miss
// an edit made after the recovery pass. Aging only the durable anchor avoids a
// five-minute wall-clock wait; core tests independently verify that interval.
#[tokio::test]
async fn t25_real_worker_periodic_loop_saves_edits_without_restart() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let n = service::create(
        &ctx,
        CreateNote {
            title: "initial".into(),
            parent_id: None,
            document_json: r#"{"blocks":[]}"#.into(),
        },
    )
    .await
    .unwrap();
    let mut worker = start(&db);
    wait_for_snapshot(&db, n.id, 1).await;
    service::update(
        &ctx,
        UpdateNote {
            id: n.id,
            expected_version: 1,
            title: Some("after startup".into()),
            document_json: None,
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE notes SET history_dirty_at=now()-interval '6 minutes' WHERE id=$1")
        .bind(n.id)
        .execute(&db.pool)
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        assert!(worker.child.try_wait().unwrap().is_none());
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT title,reason FROM note_snapshots WHERE note_id=$1 AND content_version=2",
        )
        .bind(n.id)
        .fetch_optional(&db.pool)
        .await
        .unwrap();
        if let Some((title, reason)) = row {
            assert_eq!(title, "after startup");
            assert_eq!(reason, "periodic");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "running worker never performed its periodic pass"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
