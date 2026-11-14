//! T01 acceptance tests: workspace layout and configuration loading.

use std::path::{Path, PathBuf};
use std::process::Command;

use knoted_core::config::{
    AppConfig, ConfigError, DatabaseConfig, LogFormat, LogLevel, ProcessKind,
};

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

fn assert_invalid_field(path: &Path, expected: &'static str) {
    match AppConfig::load(path) {
        Err(ConfigError::Invalid { field, .. }) => assert_eq!(field, expected),
        other => panic!("expected invalid `{expected}`, got {other:?}"),
    }
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

    // A present but empty `[server]` table is valid TOML. It must be
    // diagnosed as a missing required field for the API, not a syntax error,
    // and it must not prevent the worker from loading.
    let empty_server_path = dir.path().join("empty-server.toml");
    write_file(
        &empty_server_path,
        "[server]\n\n[database]\nurl = \"postgres://knoted:secret@127.0.0.1:5432/knoted\"\n",
    );
    assert!(matches!(
        AppConfig::load_for_process(&empty_server_path, ProcessKind::Api),
        Err(ConfigError::Missing {
            field: "server.bind",
            process: "api",
        })
    ));
    assert!(AppConfig::load_for_process(&empty_server_path, ProcessKind::Worker).is_ok());
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

#[test]
fn t01_5_redacts_malformed_database_urls() {
    for url in [
        "postgres://u:SENTINEL/pw@h/db",
        "postgres://SENTINEL_TOKEN@h",
    ] {
        let config = DatabaseConfig {
            url: url.to_owned(),
            max_connections: 10,
            connect_timeout_secs: 10,
        };
        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains("SENTINEL"),
            "malformed DSN leaked in Debug: {rendered}"
        );
    }
}

#[test]
fn t01_6_rejects_invalid_values_unknown_keys_and_missing_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("knoted.toml");
    let url = "postgres://knoted:secret@127.0.0.1:5432/knoted";

    for value in [0, 65] {
        write_file(
            &path,
            &format!("[database]\nurl = \"{url}\"\nmax_connections = {value}\n"),
        );
        assert_invalid_field(&path, "database.max_connections");
    }

    for value in [0, 301] {
        write_file(
            &path,
            &format!("[database]\nurl = \"{url}\"\nconnect_timeout_secs = {value}\n"),
        );
        assert_invalid_field(&path, "database.connect_timeout_secs");
    }

    write_file(
        &path,
        &format!("[database]\nurl = \"{url}\"\nbogus = true\n"),
    );
    assert!(matches!(
        AppConfig::load(&path),
        Err(ConfigError::Parse { .. })
    ));

    write_file(
        &path,
        &format!("bogus = true\n\n[database]\nurl = \"{url}\"\n"),
    );
    assert!(matches!(
        AppConfig::load(&path),
        Err(ConfigError::Parse { .. })
    ));

    let missing_credential_path = dir.path().join("does-not-exist.toml");
    write_file(
        &path,
        &format!(
            "[credentials]\ncredential_file = \"{}\"\n",
            missing_credential_path.display()
        ),
    );
    assert!(matches!(
        AppConfig::load(&path),
        Err(ConfigError::CredentialRead { .. })
    ));
}

#[test]
fn t01_7_parse_errors_do_not_echo_source() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.toml");
    write_file(
        &path,
        "[database]\nurl = \"postgres://u:SENTINEL_DB_PW@h/db\n",
    );

    let error = AppConfig::load(&path).unwrap_err();
    let ConfigError::Parse { line, column, .. } = &error else {
        panic!("expected a parse error, got {error:?}");
    };
    assert_eq!(
        (*line, *column),
        (2, 40),
        "the error must point at the unterminated string on line 2"
    );
    assert!(
        error.to_string().contains("line 2 column 40"),
        "the location must appear in the message: {error}"
    );
    let display = error.to_string();
    let debug = format!("{error:?}");
    assert!(
        !display.contains("SENTINEL"),
        "error Display leaked: {display}"
    );
    assert!(!debug.contains("SENTINEL"), "error Debug leaked: {debug}");
    assert!(
        std::error::Error::source(&error).is_none(),
        "the raw TOML error must not be retained: its Display echoes the line"
    );
}
