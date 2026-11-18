//! Real Rust client -> isolated Python service. No wiremock substitute.
use knoted_core::files::{ObjectId, Sha256Hex};
use knoted_core::intelligence::{ExecutionKind, ExecutionRequest, IntelligenceClient};
use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};
fn selected_python(repository: &std::path::Path) -> String {
    std::env::var("KNOTED_TEST_PYTHON").unwrap_or_else(|_| {
        std::env::var("UV_PROJECT_ENVIRONMENT")
            .map(|p| format!("{p}/bin/python"))
            .unwrap_or_else(|_| {
                repository
                    .join("services/intelligence/.venv/bin/python")
                    .to_string_lossy()
                    .into_owned()
            })
    })
}

fn python_environment_root() -> std::path::PathBuf {
    let repository = std::env::var_os("T48_TEST_REPOSITORY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let output = Command::new(selected_python(&repository))
        .args(["-I", "-B", "-c", "import sys; print(sys.prefix)"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "selected Python must report its environment"
    );
    std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}

struct Service {
    python: String,
    process: Child,
    _directory: tempfile::TempDir,
    client: IntelligenceClient,
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
        std::fs::create_dir_all(std::env::temp_dir()).unwrap();
        let directory = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let repository = std::env::var_os("T48_TEST_REPOSITORY")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
        let python = selected_python(&repository);
        let process = Command::new(&python)
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
            .env("KNOTED_INTELLIGENCE_CHILD_HOLD_MS", "0")
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
            python,
            process,
            _directory: directory,
            client,
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
async fn t48_real_http_office_artifact_identity_digest_and_metadata() {
    use knoted_core::{config::FilesConfig, context::AppContext, files::office};
    let service = Service::start().await;
    let root = service._directory.path();
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(
        Command::new(&service.python)
            .args(["-I", "-B"])
            .arg(repository.join("crates/knoted-core/tests/fixtures/t48_office.py"))
            .arg(root.join("fixtures"))
            .status()
            .unwrap()
            .success()
    );
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy(&knoted_core::db::testing::test_database_url())
        .unwrap();
    let ctx = AppContext::new(pool).with_files(FilesConfig {
        enabled: true,
        staging_root: Some(root.join("staging")),
        ..Default::default()
    });
    for format in ["docx", "xlsx", "pptx"] {
        let raw = std::fs::read(root.join(format!("fixtures/sample.{format}"))).unwrap();
        let object = format!("opaque-{format}");
        std::fs::write(root.join("staging/input").join(&object), &raw).unwrap();
        let request = ExecutionRequest {
            execution_id: uuid::Uuid::new_v4(),
            kind: ExecutionKind::Parse,
            input_object_id: ObjectId::parse(object).unwrap(),
            input_sha256: Sha256Hex::digest(&raw),
            source_version: 7,
            deadline: chrono::Utc::now() + chrono::Duration::seconds(60),
            options_revision: office::OPTIONS_REVISION.into(),
        };
        service.client.create_execution(&request).await.unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let record = service
                .client
                .get_execution(request.execution_id)
                .await
                .unwrap();
            if record.compute_exit_confirmed {
                record.verify_resource_exit(&request).unwrap();
                assert_eq!(
                    record.state,
                    knoted_core::intelligence::ExecutionState::Succeeded,
                    "{record:?}"
                );
                break;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let result = service
            .client
            .get_result(request.execution_id)
            .await
            .unwrap();
        let document = office::read_artifact(&ctx, &request, &result)
            .await
            .unwrap();
        assert_eq!(document.source_format, format);
        let mut wrong = result.clone();
        wrong.sha256 = Sha256Hex::digest(b"different");
        assert!(office::read_artifact(&ctx, &request, &wrong).await.is_err());
        let mut wrong = result.clone();
        wrong.metadata.insert("source_version".into(), "999".into());
        assert!(office::read_artifact(&ctx, &request, &wrong).await.is_err());
        let mut wrong = result.clone();
        wrong.size += 1;
        assert!(office::read_artifact(&ctx, &request, &wrong).await.is_err());
        let file = root
            .join("staging/output")
            .join(request.execution_id.to_string())
            .join(result.object_id.as_str());
        let mut bytes = std::fs::read(&file).unwrap();
        bytes[0] ^= 1;
        std::fs::write(file, bytes).unwrap();
        assert!(
            office::read_artifact(&ctx, &request, &result)
                .await
                .is_err()
        );
    }
}

#[test]
fn t48_default_venv_runs_service_and_office_fixture_without_optional_env() {
    let env_root = python_environment_root();
    let repository = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
    let intelligence = repository.path().join("services/intelligence");
    std::fs::create_dir_all(&intelligence).unwrap();
    std::os::unix::fs::symlink(
        std::fs::canonicalize(env_root).unwrap(),
        intelligence.join(".venv"),
    )
    .unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "t48_real_http_office_artifact_identity_digest_and_metadata",
            "--nocapture",
        ])
        .env_remove("KNOTED_TEST_PYTHON")
        .env_remove("UV_PROJECT_ENVIRONMENT")
        .env("T48_TEST_REPOSITORY", repository.path())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "default venv must serve and generate fixtures without optional env"
    );
}

#[test]
fn t48_python_overrides_support_path_uv_and_knoted_precedence() {
    let env_root = python_environment_root();
    let mut paths = vec![env_root.join("bin")];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let path = std::env::join_paths(paths).unwrap();
    for mode in ["path", "uv", "both"] {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "t48_default_venv_runs_service_and_office_fixture_without_optional_env",
                "--nocapture",
            ])
            .env_remove("KNOTED_TEST_PYTHON")
            .env_remove("UV_PROJECT_ENVIRONMENT")
            .env("PATH", &path);
        if mode != "uv" {
            child.env("KNOTED_TEST_PYTHON", "python");
        }
        if mode == "uv" {
            child.env("UV_PROJECT_ENVIRONMENT", &env_root);
        } else if mode == "both" {
            // A conflicting unusable UV root must never outrank KNOTED.
            child.env(
                "UV_PROJECT_ENVIRONMENT",
                env_root.join("not-an-environment"),
            );
        }
        assert!(child.status().unwrap().success(), "interpreter mode {mode}");
    }
}
