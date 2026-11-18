//! Real Rust client -> isolated Python service. No wiremock substitute.
use knoted_core::files::{ObjectId, Sha256Hex};
use knoted_core::intelligence::{ExecutionKind, ExecutionRequest, IntelligenceClient};
use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};
struct Service {
    process: Child,
    _directory: tempfile::TempDir,
    client: IntelligenceClient,
    port: u16,
}
impl Drop for Service {
    fn drop(&mut self) {
        if self.process.try_wait().ok().flatten().is_none() {
            let _ = Command::new("kill")
                .args(["-TERM", &self.process.id().to_string()])
                .status();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while self.process.try_wait().ok().flatten().is_none()
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
impl Service {
    async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let python = std::env::var("KNOTED_TEST_PYTHON").unwrap_or_else(|_| {
            std::env::var("UV_PROJECT_ENVIRONMENT")
                .map(|p| format!("{p}/bin/python"))
                .unwrap_or_else(|_| {
                    repository
                        .join("services/intelligence/.venv/bin/python")
                        .to_string_lossy()
                        .into_owned()
                })
        });
        let process = Command::new(python)
            .args([
                "-I",
                "-B",
                "-c",
                "from knoted_intelligence.app import main;main()",
            ])
            .current_dir(repository)
            .env("KNOTED_INTELLIGENCE_HOST", "127.0.0.1")
            .env("KNOTED_INTELLIGENCE_PORT", port.to_string())
            .env(
                "KNOTED_INTELLIGENCE_STATE_DIR",
                directory.path().join("state"),
            )
            .env(
                "KNOTED_INTELLIGENCE_STAGING_DIR",
                directory.path().join("staging"),
            )
            .env("KNOTED_INTELLIGENCE_CHILD_HOLD_MS", "60000")
            .env("KNOTED_INTELLIGENCE_CHILD_IGNORE_SIGTERM", "1")
            .env("KNOTED_INTELLIGENCE_CANCEL_GRACE_SECONDS", "0.1")
            .env("CUDA_VISIBLE_DEVICES", "")
            .env("KNOTED_EMBED_GPU_UUID", "")
            .env("KNOTED_INTELLIGENCE_DOCLING_DEVICE", "cpu")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let client = IntelligenceClient::new(format!("http://127.0.0.1:{port}")).unwrap();
        let mut service = Self {
            process,
            _directory: directory,
            client,
            port,
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            assert!(
                service.process.try_wait().unwrap().is_none(),
                "Python test interpreter/package unavailable"
            );
            if service.client.health().await.is_ok() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "owned service did not start"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        service
    }
}
#[tokio::test]
async fn t47_real_late_post_materialization_and_terminal_409_resource_proof() {
    let service = Service::start().await;
    let request = ExecutionRequest {
        execution_id: uuid::Uuid::new_v4(),
        kind: ExecutionKind::Parse,
        input_object_id: ObjectId::parse("missing").unwrap(),
        input_sha256: Sha256Hex::digest(b"missing"),
        source_version: 7,
        deadline: chrono::Utc::now() - chrono::Duration::seconds(1),
        options_revision: "test".into(),
    };
    // GET404 cannot prove a delayed original POST won't arrive. Settle persists
    // the exact expired request first; the later original cannot revive it.
    service.client.settle_execution(&request).await.unwrap();
    let replay = service.client.create_execution(&request).await.unwrap();
    assert!(!replay.created);
    replay.record.verify_resource_exit(&request).unwrap();
    assert_eq!(replay.record.error.unwrap().code, "deadline_exceeded");
    assert!(
        service
            .client
            .cancel_execution(request.execution_id)
            .await
            .is_err()
    );
    service.client.settle_execution(&request).await.unwrap();
    let mut wrong = request.clone();
    wrong.deadline += chrono::Duration::minutes(1);
    assert!(service.client.create_execution(&wrong).await.is_err());
}
#[tokio::test]
async fn t47_real_running_cancel_proves_exit_before_replay() {
    let service = Service::start().await;
    std::fs::write(
        service._directory.path().join("staging/input/source"),
        b"source",
    )
    .unwrap();
    let request = ExecutionRequest {
        execution_id: uuid::Uuid::new_v4(),
        kind: ExecutionKind::Parse,
        input_object_id: ObjectId::parse("source").unwrap(),
        input_sha256: Sha256Hex::digest(b"source"),
        source_version: 2,
        deadline: chrono::Utc::now() + chrono::Duration::seconds(60),
        options_revision: "test".into(),
    };
    service.client.create_execution(&request).await.unwrap();
    service.client.settle_execution(&request).await.unwrap();
    let record = service
        .client
        .get_execution(request.execution_id)
        .await
        .unwrap();
    record.verify_resource_exit(&request).unwrap();
    let replay = service.client.create_execution(&request).await.unwrap();
    assert!(!replay.created);
    replay.record.verify_resource_exit(&request).unwrap();
}

#[tokio::test]
async fn t47_real_lost_post_response_replays_original_request() {
    use std::io::{Read, Write};
    let service = Service::start().await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_url = format!("http://{}", listener.local_addr().unwrap());
    let port = service.port;
    let proxy = std::thread::spawn(move || {
        let (mut inbound, _) = listener.accept().unwrap();
        inbound
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = inbound.read(&mut buffer).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
            if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&request[..end]);
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let mut upstream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        upstream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        upstream.write_all(&request).unwrap();
        // Observe a real HTTP response, then intentionally discard it. No fake
        // successful response is ever returned to the Rust client.
        let count = upstream.read(&mut buffer).unwrap();
        assert!(String::from_utf8_lossy(&buffer[..count]).starts_with("HTTP/1.1 201"));
    });
    let request = ExecutionRequest {
        execution_id: uuid::Uuid::new_v4(),
        kind: ExecutionKind::Parse,
        input_object_id: ObjectId::parse("missing").unwrap(),
        input_sha256: Sha256Hex::digest(b"missing"),
        source_version: 1,
        deadline: chrono::Utc::now() - chrono::Duration::seconds(1),
        options_revision: "test".into(),
    };
    assert!(
        IntelligenceClient::new(proxy_url)
            .unwrap()
            .create_execution(&request)
            .await
            .is_err()
    );
    proxy.join().unwrap();
    service.client.settle_execution(&request).await.unwrap();
    let replay = service.client.create_execution(&request).await.unwrap();
    assert!(!replay.created);
    replay.record.verify_resource_exit(&request).unwrap();
}
