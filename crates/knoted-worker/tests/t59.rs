//! The persisted vector batch receipts fence rollout, retries and cancellation.

use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use knoted_core::{
    config::{FilesConfig, IntelligenceConfig},
    context::AppContext,
    db::testing::TestDb,
    files::{ObjectId, Sha256Hex},
    intelligence::{ExecutionKind, ExecutionRequest},
    jobs::{self, Job, JobSpec, Lease},
    notes::service::{self as notes, CreateNote},
    rag::{chunking::CHUNKER_REVISION, embedding::CHUNKS_PER_EXECUTION},
};
use knoted_worker::handlers::vector;
use serde_json::json;
use uuid::Uuid;

async fn enqueue(ctx: &AppContext, set_id: Uuid) -> Job {
    let mut spec = JobSpec::new(vector::KIND);
    spec.payload = json!({"chunk_set_id": set_id});
    let mut tx = ctx.pool().begin().await.unwrap();
    let result = jobs::enqueue(&mut tx, spec).await.unwrap();
    tx.commit().await.unwrap();
    result.job
}

#[tokio::test]
async fn t59_cancel_can_only_replay_batches_that_have_reached_dispatch() {
    let db = TestDb::create().await.unwrap();
    let staging = std::path::Path::new("/data/knoted-dev/tasks/t59/tmp")
        .join(Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&staging).unwrap();
    let ctx = AppContext::new(db.pool.clone())
        .with_files(FilesConfig {
            staging_root: Some(staging.clone()),
            ..FilesConfig::default()
        })
        .with_intelligence(IntelligenceConfig {
            // A failed first POST represents a transport ambiguity before any
            // later batch has been selected for dispatch.
            base_url: "http://127.0.0.1:0".into(),
        })
        .with_vectors_enabled(true);

    let note = notes::create(
        &ctx,
        CreateNote {
            title: "Multiple batches".into(),
            parent_id: None,
            document_json: r#"{"blocks":[]}"#.into(),
        },
    )
    .await
    .unwrap();
    let set_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO rag_chunk_sets (id,source_type,source_id,source_version,extraction_id,chunker_revision)
         SELECT $1,source_type,source_id,source_version,extraction_id,$2
         FROM current_rag_sources WHERE source_type='note' AND source_id=$3",
    )
    .bind(set_id)
    .bind(CHUNKER_REVISION)
    .bind(note.id)
    .execute(ctx.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO rag_chunks
         SELECT gen_random_uuid(),$1,ordinal,'batch text',2,'[]'
         FROM generate_series(0,$2) ordinal",
    )
    .bind(set_id)
    .bind(CHUNKS_PER_EXECUTION as i32)
    .execute(ctx.pool())
    .await
    .unwrap();

    let job = enqueue(&ctx, set_id).await;
    let lease = jobs::claim(ctx.pool(), "t59").await.unwrap().unwrap();
    assert_eq!(lease.job_id, job.id);
    let claimed: Job = sqlx::query_as("SELECT * FROM jobs WHERE id=$1")
        .bind(lease.job_id)
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    let outcome = vector::execute(&ctx, &lease, &claimed).await;
    let staged: Vec<i32> = sqlx::query_scalar(
        "SELECT ordinal FROM rag_embedding_executions WHERE job_id=$1 ORDER BY ordinal",
    )
    .bind(job.id)
    .fetch_all(ctx.pool())
    .await
    .unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    db.drop().await;
    assert!(matches!(
        outcome,
        jobs::ExecutionOutcome::TemporaryFailure { .. }
    ));
    assert_eq!(staged, vec![0]);
}

fn receipt(lease: &Lease) -> ExecutionRequest {
    ExecutionRequest {
        execution_id: Uuid::new_v4(),
        kind: ExecutionKind::Embed,
        input_object_id: ObjectId::parse("test-input.json").unwrap(),
        input_sha256: Sha256Hex::digest(b"test input"),
        source_version: 1,
        deadline: lease.expires_at,
        options_revision: "test-embed-revision".into(),
    }
}

struct ExitPeer {
    origin: String,
    observed: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ExitPeer {
    fn new(receipts: Vec<ExecutionRequest>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let observed = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let observed_by_peer = observed.clone();
        let stop_peer = stop.clone();

        let thread = thread::spawn(move || {
            while !stop_peer.load(Ordering::Acquire) {
                let mut connection = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("could not accept test connection: {error}"),
                };
                connection
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut line = String::new();
                {
                    let mut reader = BufReader::new(&mut connection);
                    reader.read_line(&mut line).unwrap();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                            break;
                        }
                    }
                }
                let route = line.split_whitespace().nth(1).unwrap();
                observed_by_peer.lock().unwrap().push(route.into());
                let receipt = receipts
                    .iter()
                    .find(|receipt| route.ends_with(&receipt.execution_id.to_string()))
                    .unwrap();
                let now = chrono::Utc::now();
                let body = json!({
                    "execution_id":receipt.execution_id,
                    "kind":"embed",
                    "state":"cancelled",
                    "compute_exit_confirmed":true,
                    "input_sha256":receipt.input_sha256,
                    "source_version":receipt.source_version,
                    "options_revision":receipt.options_revision,
                    "created_at":now,
                    "updated_at":now,
                    "finished_at":now,
                })
                .to_string();
                write!(
                    connection,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });

        Self {
            origin,
            observed,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for ExitPeer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[tokio::test]
async fn t59_stale_owner_only_settles_its_own_attempt() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let job = enqueue(&ctx, Uuid::new_v4()).await;
    let stale = jobs::claim(ctx.pool(), "old").await.unwrap().unwrap();
    sqlx::query(
        "UPDATE jobs SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(job.id)
    .execute(ctx.pool())
    .await
    .unwrap();
    let current = jobs::claim(ctx.pool(), "new").await.unwrap().unwrap();
    assert_ne!(stale.token, current.token);
    let earlier = receipt(&stale);
    let later = receipt(&current);
    for (lease, receipt) in [(&stale, &earlier), (&current, &later)] {
        sqlx::query(
            "INSERT INTO rag_embedding_executions (job_id,attempt,ordinal,request) VALUES ($1,$2,0,$3)",
        )
        .bind(job.id)
        .bind(lease.attempt)
        .bind(sqlx::types::Json(receipt))
        .execute(ctx.pool())
        .await
        .unwrap();
    }
    let endpoint = ExitPeer::new(vec![earlier.clone(), later]);
    let ctx = ctx.with_intelligence(IntelligenceConfig {
        base_url: endpoint.origin.clone(),
    });
    let confirmed = vector::cancel_and_wait(&ctx, &stale).await;

    assert!(confirmed);
    assert_eq!(
        *endpoint.observed.lock().unwrap(),
        [format!("/internal/v1/executions/{}", earlier.execution_id)],
    );
    db.drop().await;
}
