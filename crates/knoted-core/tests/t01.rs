//! T01 acceptance tests: workspace layout and configuration loading.

use std::path::{Path, PathBuf};
use std::process::Command;

use knoted_core::config::{AppConfig, ConfigError, LogFormat, LogLevel, ProcessKind};

fn write_file(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
}

fn config_with(path: &Path, database: Option<&str>, server: Option<&str>) -> PathBuf {
    let mut toml = String::new();
    if let Some(bind) = server {
        toml.push_str(&format!("[server]\nbind = \"{bind}\"\n\n"));
    }
    if let Some(url) = database {
        toml.push_str(&format!("[database]\nurl = \"{url}\"\n\n"));
    }
    let config_path = path.join("knoted.toml");
    write_file(&config_path, &toml);
    config_path
}

#[test]
fn t01_1_api_and_worker_require_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = config_with(dir.path(), None, Some("127.0.0.1:15602"));

    assert!(matches!(
        AppConfig::load_for_process(&path, ProcessKind::Api),
        Err(ConfigError::Missing {
            field: "database",
            process: "api",
        })
    ));
    assert!(matches!(
        AppConfig::load_for_process(&path, ProcessKind::Worker),
        Err(ConfigError::Missing {
            field: "database",
            process: "worker",
        })
    ));
}

#[test]
fn t01_2_api_worker_are_independent_binaries() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(&root)
        .output()
        .expect("cargo metadata must run");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata returns JSON");
    let packages = metadata["packages"].as_array().unwrap();
    let find = |name: &str| {
        packages
            .iter()
            .find(|package| package["name"] == name)
            .unwrap_or_else(|| panic!("package {name} must be a workspace member"))
    };

    let core = find("knoted-core");
    let api = find("knoted-api");
    let worker = find("knoted-worker");

    let members = metadata["workspace_members"].as_array().unwrap();
    for package in [core, api, worker] {
        assert!(
            members.contains(&package["id"]),
            "{} must be listed in workspace_members",
            package["name"]
        );
        assert_eq!(package["edition"], "2024");
    }

    let has_bin = |package: &serde_json::Value, expected: &str| {
        package["targets"].as_array().unwrap().iter().any(|target| {
            target["name"] == expected
                && target["kind"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|kind| kind == "bin")
        })
    };
    assert!(has_bin(api, "knoted-api"));
    assert!(has_bin(worker, "knoted-worker"));

    let depends_on = |package: &serde_json::Value, dependency: &str| {
        package["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|dep| dep["name"] == dependency)
    };
    assert!(depends_on(api, "knoted-core"));
    assert!(depends_on(worker, "knoted-core"));
    assert!(!depends_on(api, "knoted-worker"));
    assert!(!depends_on(worker, "knoted-api"));
}

#[test]
fn t01_3_credentials_never_reach_logs() {
    let dir = tempfile::tempdir().unwrap();
    let credential_path = dir.path().join("credentials.toml");
    write_file(
        &credential_path,
        "[secrets]\nsession_secret = \"SENTINEL_CREDENTIAL\"\n",
    );

    let config_path = dir.path().join("knoted.toml");
    write_file(
        &config_path,
        &format!(
            "[database]\nurl = \"postgres://knoted:SENTINEL_DB_PW@127.0.0.1:5432/knoted_test\"\n\n\
             [credentials]\ncredential_file = \"{}\"\n",
            credential_path.display()
        ),
    );

    let config = AppConfig::load(&config_path).expect("valid config loads");
    assert_eq!(
        config.credential("session_secret"),
        Some("SENTINEL_CREDENTIAL")
    );

    let rendered = format!("{config:?}");
    assert!(
        !rendered.contains("SENTINEL_"),
        "AppConfig Debug leaked: {rendered}"
    );
    let database = format!("{:?}", config.database.as_ref().unwrap());
    assert!(
        !database.contains("SENTINEL_"),
        "DatabaseConfig Debug leaked: {database}"
    );
    assert!(!database.contains("SENTINEL_DB_PW"));
    let credentials = format!("{:?}", config.credentials);
    assert!(!credentials.contains("SENTINEL_CREDENTIAL"));

    let bad_credential_path = dir.path().join("bad-credentials.toml");
    write_file(
        &bad_credential_path,
        "[secrets\nsession_secret = \"SENTINEL_CREDENTIAL\"\n",
    );
    let bad_config_path = dir.path().join("bad-knoted.toml");
    write_file(
        &bad_config_path,
        &format!(
            "[credentials]\ncredential_file = \"{}\"\n",
            bad_credential_path.display()
        ),
    );

    let error = AppConfig::load(&bad_config_path).unwrap_err();
    assert!(matches!(error, ConfigError::CredentialParse { .. }));
    let message = error.to_string();
    assert!(
        !message.contains("SENTINEL_"),
        "error Display leaked: {message}"
    );
}

#[test]
fn t01_4_valid_config_loads_for_each_process() {
    let dir = tempfile::tempdir().unwrap();
    let path = config_with(
        dir.path(),
        Some("postgresql://knoted:secret@127.0.0.1:5432/knoted"),
        Some("127.0.0.1:15602"),
    );

    let api = AppConfig::load_for_process(&path, ProcessKind::Api).unwrap();
    assert_eq!(api.database.as_ref().unwrap().max_connections, 10);
    assert_eq!(api.database.as_ref().unwrap().connect_timeout_secs, 10);
    assert_eq!(api.logging.level, LogLevel::Info);
    assert_eq!(api.logging.format, LogFormat::Text);

    let worker = AppConfig::load_for_process(&path, ProcessKind::Worker).unwrap();
    assert_eq!(worker.server, api.server);
    assert_eq!(worker.database, api.database);

    let bad_path = config_with(
        dir.path(),
        Some("postgres://knoted:secret@127.0.0.1:5432/knoted"),
        Some("localhost:15602"),
    );
    let error = AppConfig::load_for_process(&bad_path, ProcessKind::Api).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Invalid {
            field: "server.bind",
            ..
        }
    ));

    // The same bind value is irrelevant to the worker.
    assert!(AppConfig::load_for_process(&bad_path, ProcessKind::Worker).is_ok());
}
