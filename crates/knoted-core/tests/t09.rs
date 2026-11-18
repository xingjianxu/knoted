//! T09 acceptance tests: the internal execution protocol and staging objects.
//!
//! The Rust side verifies what the worker is allowed to believe: the contract
//! shapes, the echo of the input version, the freshness of a result and the
//! digest of the artifact. The Python service has its own tests that run real
//! child processes; the `wiremock` stubs here only stand in for transport so the
//! client's refusals can be exercised deterministically. Neither side replaces
//! the other.

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, TimeZone, Utc};
use knoted_core::files::staging::{ObjectId, Sha256Hex, StagingError, StagingStore};
use knoted_core::intelligence::{
    ExecutionKind, ExecutionRecord, ExecutionRequest, ExecutionResult, ExecutionState,
    IntelligenceClient, IntelligenceError,
};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EXECUTION_ID: &str = "6f1c7f9e-4a3b-4c2d-9e8f-0a1b2c3d4e5f";

fn base_instant() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 4, 5, 0, 0).unwrap()
}

fn deadline() -> DateTime<Utc> {
    base_instant() + Duration::minutes(10)
}

fn execution_id() -> Uuid {
    Uuid::parse_str(EXECUTION_ID).expect("the fixture uuid parses")
}

fn request() -> ExecutionRequest {
    ExecutionRequest {
        execution_id: execution_id(),
        kind: ExecutionKind::Parse,
        input_object_id: ObjectId::parse("note-t09").expect("valid object id"),
        input_sha256: Sha256Hex::parse("a".repeat(64)).expect("valid digest"),
        source_version: 7,
        deadline: deadline(),
        options_revision: "parse-options-t09".to_owned(),
    }
}

fn record_with(
    request: &ExecutionRequest,
    state: ExecutionState,
    finished_at: Option<DateTime<Utc>>,
) -> ExecutionRecord {
    ExecutionRecord {
        compute_exit_confirmed: state.is_terminal(),
        execution_id: request.execution_id,
        kind: request.kind,
        state,
        input_sha256: request.input_sha256.clone(),
        source_version: request.source_version,
        options_revision: request.options_revision.clone(),
        created_at: base_instant(),
        updated_at: base_instant(),
        started_at: Some(base_instant() + Duration::seconds(1)),
        finished_at,
        error: None,
    }
}

fn result_with(object_id: &ObjectId, sha256: Sha256Hex, size: u64) -> ExecutionResult {
    ExecutionResult {
        object_id: object_id.clone(),
        sha256,
        size,
        media_type: "application/json".to_owned(),
        metadata: BTreeMap::new(),
    }
}

async fn mount(server: &MockServer, verb: &'static str, route: String, status: u16, body: Value) {
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

fn record_json(record: &ExecutionRecord) -> Value {
    serde_json::to_value(record).expect("records serialise")
}

#[test]
fn t09_contract_shapes_round_trip() {
    let request = request();
    let wire = serde_json::to_value(&request).expect("request serialises");

    // Exactly the contract fields, snake_case, no invented "execution token".
    assert_eq!(
        wire,
        json!({
            "execution_id": EXECUTION_ID,
            "kind": "parse",
            "input_object_id": "note-t09",
            "input_sha256": "a".repeat(64),
            "source_version": 7,
            "deadline": "2026-03-04T05:10:00Z",
            "options_revision": "parse-options-t09",
        })
    );
    assert_eq!(
        serde_json::from_value::<ExecutionRequest>(wire.clone()).expect("request round-trips"),
        request
    );
    assert_eq!(
        serde_json::from_value::<ExecutionRequest>(wire).expect("request round-trips twice"),
        request
    );

    // Every contract state deserialises, and the terminal set is exact.
    let expectations = [
        ("queued", ExecutionState::Queued, false),
        ("running", ExecutionState::Running, false),
        ("succeeded", ExecutionState::Succeeded, true),
        ("failed", ExecutionState::Failed, true),
        ("cancel_requested", ExecutionState::CancelRequested, false),
        ("cancelled", ExecutionState::Cancelled, true),
        ("interrupted", ExecutionState::Interrupted, true),
    ];
    for (wire_state, expected, terminal) in expectations {
        let mut body = record_json(&record_with(
            &request,
            ExecutionState::Queued,
            Some(base_instant() + Duration::minutes(2)),
        ));
        body["state"] = json!(wire_state);
        let record: ExecutionRecord =
            serde_json::from_value(body).unwrap_or_else(|error| panic!("{wire_state}: {error}"));
        assert_eq!(record.state, expected);
        assert_eq!(record.state.is_terminal(), terminal, "{wire_state}");
        assert_eq!(
            record.state.cancellation_confirmed(),
            expected == ExecutionState::Cancelled,
            "only `cancelled` confirms an exit, not `{wire_state}`"
        );
    }

    // A state outside the contract is refused instead of guessed.
    let mut unknown = record_json(&record_with(&request, ExecutionState::Queued, None));
    unknown["state"] = json!("paused");
    assert!(serde_json::from_value::<ExecutionRecord>(unknown).is_err());

    // An invented extra field is refused: the contract forbids it.
    let mut extra = record_json(&record_with(&request, ExecutionState::Queued, None));
    extra["execution_token"] = json!("not-part-of-the-contract");
    assert!(serde_json::from_value::<ExecutionRecord>(extra).is_err());

    // Optional fields may be absent; `error` is required but nullable.
    let mut minimal = record_json(&record_with(&request, ExecutionState::Running, None));
    minimal.as_object_mut().expect("object").remove("error");
    minimal["started_at"] = Value::Null;
    let record: ExecutionRecord = serde_json::from_value(minimal).expect("optional fields");
    assert_eq!(record.error, None);
    assert_eq!(record.started_at, None);

    // `metadata` is optional, but when present it is an object of strings; a
    // null or a nested value is a contract violation, not something to coerce.
    let object_id = ObjectId::parse("result-t09.json").expect("valid id");
    let digest = Sha256Hex::parse("a".repeat(64)).expect("valid digest");
    let result = result_with(&object_id, digest, 1);
    let wire = serde_json::to_value(&result).expect("result serialises");
    assert!(
        wire.get("metadata").is_none(),
        "an empty metadata map is omitted rather than sent as null"
    );
    let mut with_metadata = wire.clone();
    with_metadata["metadata"] = json!({ "source_version": "1" });
    let parsed: ExecutionResult =
        serde_json::from_value(with_metadata).expect("a string map is accepted");
    assert_eq!(
        parsed.metadata.get("source_version").map(String::as_str),
        Some("1")
    );
    let mut null_metadata = wire;
    null_metadata["metadata"] = Value::Null;
    assert!(
        serde_json::from_value::<ExecutionResult>(null_metadata).is_err(),
        "null metadata is not the contract's string map"
    );
}

#[tokio::test]
async fn t09_client_rejects_digest_mismatch() {
    let server = MockServer::start().await;
    let directory = tempfile::tempdir().expect("temporary staging root");
    let store = StagingStore::new(directory.path(), 1 << 20).expect("the staging store opens");
    let object_id = ObjectId::parse(format!("{EXECUTION_ID}-parse.json")).expect("valid id");
    let payload = br#"{"kind":"parse","blocks":[]}"#;
    let staged = store
        .write(&object_id, payload)
        .expect("the artifact is staged");

    let request = request();
    let record = record_with(
        &request,
        ExecutionState::Succeeded,
        Some(base_instant() + Duration::minutes(2)),
    );
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&record),
    )
    .await;

    // The service claims a digest that the staged bytes do not have.
    let tampered = result_with(
        &object_id,
        Sha256Hex::parse("b".repeat(64)).expect("valid digest"),
        payload.len() as u64,
    );
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}/result"),
        200,
        serde_json::to_value(&tampered).expect("result serialises"),
    )
    .await;

    let client = IntelligenceClient::new(server.uri()).expect("client builds");
    let error = client
        .fetch_verified_result(&request, &store, base_instant() + Duration::minutes(3))
        .await
        .expect_err("a mismatched digest must be refused");
    assert!(
        matches!(
            error,
            IntelligenceError::Staging(StagingError::DigestMismatch { .. })
        ),
        "{error}"
    );

    // A size that disagrees with the file is refused as well.
    server.reset().await;
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&record),
    )
    .await;
    let wrong_size = result_with(&object_id, staged.sha256.clone(), payload.len() as u64 + 1);
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}/result"),
        200,
        serde_json::to_value(&wrong_size).expect("result serialises"),
    )
    .await;
    let error = client
        .fetch_verified_result(&request, &store, base_instant() + Duration::minutes(3))
        .await
        .expect_err("a mismatched size must be refused");
    assert!(
        matches!(
            error,
            IntelligenceError::Staging(StagingError::SizeMismatch { .. })
        ),
        "{error}"
    );

    // The honest manifest passes.
    server.reset().await;
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&record),
    )
    .await;
    let honest = result_with(&object_id, staged.sha256.clone(), staged.size);
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}/result"),
        200,
        serde_json::to_value(&honest).expect("result serialises"),
    )
    .await;
    let verified = client
        .fetch_verified_result(&request, &store, base_instant() + Duration::minutes(3))
        .await
        .expect("the verified result is accepted");
    assert_eq!(verified, honest);
}

#[tokio::test]
async fn t09_client_rejects_input_version_mismatch() {
    let server = MockServer::start().await;
    let directory = tempfile::tempdir().expect("temporary staging root");
    let store = StagingStore::new(directory.path(), 1 << 20).expect("staging store");
    let request = request();
    let client = IntelligenceClient::new(server.uri()).expect("client builds");

    // The record must echo the requested source version.
    let mut stale = record_with(&request, ExecutionState::Running, None);
    stale.source_version = request.source_version + 1;
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&stale),
    )
    .await;
    let error = client
        .fetch_verified_result(&request, &store, base_instant())
        .await
        .expect_err("a different source version must be refused");
    match error {
        IntelligenceError::EchoMismatch { field, .. } => assert_eq!(field, "source_version"),
        other => panic!("expected an echo mismatch, got {other}"),
    }

    // The input digest the worker staged must be echoed too.
    server.reset().await;
    let mut other_input = record_with(&request, ExecutionState::Running, None);
    other_input.input_sha256 = Sha256Hex::parse("c".repeat(64)).expect("valid digest");
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&other_input),
    )
    .await;
    let error = client
        .fetch_verified_result(&request, &store, base_instant())
        .await
        .expect_err("a different input digest must be refused");
    match error {
        IntelligenceError::EchoMismatch { field, .. } => assert_eq!(field, "input_sha256"),
        other => panic!("expected an echo mismatch, got {other}"),
    }

    // And the options revision, so a result from older options is not reused.
    server.reset().await;
    let mut other_options = record_with(&request, ExecutionState::Running, None);
    other_options.options_revision = "parse-options-old".to_owned();
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&other_options),
    )
    .await;
    let error = client
        .fetch_verified_result(&request, &store, base_instant())
        .await
        .expect_err("a different options revision must be refused");
    match error {
        IntelligenceError::EchoMismatch { field, .. } => assert_eq!(field, "options_revision"),
        other => panic!("expected an echo mismatch, got {other}"),
    }
}

#[tokio::test]
async fn t09_client_rejects_expired_results() {
    let server = MockServer::start().await;
    let directory = tempfile::tempdir().expect("temporary staging root");
    let store = StagingStore::new(directory.path(), 1 << 20).expect("staging store");
    let request = request();
    let client = IntelligenceClient::new(server.uri()).expect("client builds");

    // Finished after the deadline: stale even though the state is `succeeded`.
    let late = record_with(
        &request,
        ExecutionState::Succeeded,
        Some(request.deadline + Duration::seconds(1)),
    );
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&late),
    )
    .await;
    let error = client
        .fetch_verified_result(&request, &store, request.deadline + Duration::minutes(1))
        .await
        .expect_err("a result finished after the deadline must be refused");
    assert!(
        matches!(error, IntelligenceError::Expired { .. }),
        "{error}"
    );

    // Still running past the deadline: nothing publishable can come out of it.
    server.reset().await;
    let running = record_with(&request, ExecutionState::Running, None);
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&running),
    )
    .await;
    let error = client
        .fetch_verified_result(&request, &store, request.deadline + Duration::seconds(30))
        .await
        .expect_err("an execution past its deadline must be refused");
    assert!(
        matches!(error, IntelligenceError::Expired { .. }),
        "{error}"
    );

    // `succeeded` without `finished_at` cannot be ordered against the deadline.
    server.reset().await;
    let no_timestamp = record_with(&request, ExecutionState::Succeeded, None);
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&no_timestamp),
    )
    .await;
    let error = client
        .fetch_verified_result(&request, &store, base_instant())
        .await
        .expect_err("a succeeded record must carry finished_at");
    assert!(
        matches!(error, IntelligenceError::MalformedResponse { .. }),
        "{error}"
    );

    // A running execution inside its deadline is simply not ready.
    server.reset().await;
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&record_with(&request, ExecutionState::Running, None)),
    )
    .await;
    let error = client
        .fetch_verified_result(&request, &store, base_instant())
        .await
        .expect_err("a running execution has no result yet");
    match error {
        IntelligenceError::UnexpectedState { state, expected } => {
            assert_eq!(state, ExecutionState::Running);
            assert_eq!(expected, "succeeded");
        }
        other => panic!("expected an unexpected state, got {other}"),
    }
}

#[tokio::test]
async fn t09_cancel_requires_confirmation() {
    let server = MockServer::start().await;
    let request = request();
    let client = IntelligenceClient::new(server.uri()).expect("client builds");

    // The service accepted the cancellation but the child has not exited.
    mount(
        &server,
        "POST",
        format!("/internal/v1/executions/{EXECUTION_ID}/cancel"),
        200,
        record_json(&record_with(
            &request,
            ExecutionState::CancelRequested,
            None,
        )),
    )
    .await;
    let pending = client
        .cancel_execution(execution_id())
        .await
        .expect("a cancel request is accepted");
    assert_eq!(pending.state, ExecutionState::CancelRequested);
    assert!(
        !pending.state.is_terminal(),
        "cancel_requested is not final"
    );
    assert!(
        !pending.state.cancellation_confirmed(),
        "a signalled group is not a confirmed exit"
    );
    assert_eq!(pending.finished_at, None);

    // Later the child is gone and the record confirms the cancellation.
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&record_with(
            &request,
            ExecutionState::Cancelled,
            Some(base_instant() + Duration::minutes(1)),
        )),
    )
    .await;
    let confirmed = client
        .get_execution(execution_id())
        .await
        .expect("the record is readable");
    assert_eq!(confirmed.state, ExecutionState::Cancelled);
    assert!(confirmed.state.cancellation_confirmed());
    assert!(confirmed.finished_at.is_some());

    // A service restart leaves a retryable terminal state, not a confirmation.
    assert!(ExecutionState::Interrupted.is_terminal());
    assert!(!ExecutionState::Interrupted.cancellation_confirmed());
}

#[tokio::test]
async fn t09_cancel_timeout_is_not_a_confirmation() {
    let server = MockServer::start().await;
    let route = format!("/internal/v1/executions/{EXECUTION_ID}/cancel");
    Mock::given(method("POST"))
        .and(path(route.clone()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(StdDuration::from_millis(600))
                .set_body_json(record_json(&record_with(
                    &request(),
                    ExecutionState::Cancelled,
                    Some(base_instant() + Duration::minutes(1)),
                ))),
        )
        .mount(&server)
        .await;

    let client = IntelligenceClient::new(server.uri())
        .expect("client builds")
        .with_cancel_timeout(StdDuration::from_millis(150));
    let error = client
        .cancel_execution(execution_id())
        .await
        .expect_err("a timed-out cancel is not an answer");
    match error {
        IntelligenceError::CancelUnconfirmed { execution_id } => {
            assert_eq!(execution_id, self::execution_id());
        }
        other => panic!("expected an unconfirmed cancellation, got {other}"),
    }

    // A timed-out read is a plain transport failure: nothing was confirmed and
    // nothing may be assumed about the record either.
    server.reset().await;
    mount(
        &server,
        "GET",
        format!("/internal/v1/executions/{EXECUTION_ID}"),
        200,
        record_json(&record_with(&request(), ExecutionState::Running, None)),
    )
    .await;
    let http = reqwest::Client::builder()
        .timeout(StdDuration::from_millis(50))
        .build()
        .expect("the test client builds");
    let impatient = IntelligenceClient::with_client(server.uri(), http)
        .expect("client builds")
        .with_cancel_timeout(StdDuration::from_millis(50));
    // The route above answers immediately, so make the delay explicit.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(format!("/internal/v1/executions/{EXECUTION_ID}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(StdDuration::from_millis(600))
                .set_body_json(record_json(&record_with(
                    &request(),
                    ExecutionState::Running,
                    None,
                ))),
        )
        .mount(&server)
        .await;
    let error = impatient
        .get_execution(execution_id())
        .await
        .expect_err("the read times out");
    assert!(matches!(error, IntelligenceError::Transport(_)), "{error}");
}

#[tokio::test]
async fn t09_client_create_outcome_and_error_envelope() {
    let server = MockServer::start().await;
    let request = request();
    let client = IntelligenceClient::new(server.uri()).expect("client builds");
    let route = "/internal/v1/executions".to_owned();

    mount(
        &server,
        "POST",
        route.clone(),
        201,
        record_json(&record_with(&request, ExecutionState::Queued, None)),
    )
    .await;
    let created = client
        .create_execution(&request)
        .await
        .expect("a new execution is accepted");
    assert!(created.created);
    assert_eq!(created.record.state, ExecutionState::Queued);

    // `200` is a replay: identical input, no new computation.
    server.reset().await;
    mount(
        &server,
        "POST",
        route.clone(),
        200,
        record_json(&record_with(&request, ExecutionState::Succeeded, None)),
    )
    .await;
    let replayed = client
        .create_execution(&request)
        .await
        .expect("a duplicate request is replayed");
    assert!(!replayed.created, "a replay must not look like new work");

    // A replay about a different input version is not usable.
    server.reset().await;
    let mut drifted = record_with(&request, ExecutionState::Succeeded, None);
    drifted.options_revision = "parse-options-other".to_owned();
    mount(&server, "POST", route.clone(), 200, record_json(&drifted)).await;
    let error = client
        .create_execution(&request)
        .await
        .expect_err("a replay with a different echo must be refused");
    assert!(
        matches!(error, IntelligenceError::EchoMismatch { .. }),
        "{error}"
    );

    // A conflict carries the service's stable code, not a parsed message.
    server.reset().await;
    mount(
        &server,
        "POST",
        route,
        409,
        json!({
            "code": "execution_conflict",
            "message": "execution already exists with different parameters",
            "request_id": "00000000-0000-4000-8000-000000000001",
        }),
    )
    .await;
    let error = client
        .create_execution(&request)
        .await
        .expect_err("a conflict is an error");
    match error {
        IntelligenceError::Status { status, code, .. } => {
            assert_eq!(status, 409);
            assert_eq!(code.as_deref(), Some("execution_conflict"));
        }
        other => panic!("expected a status error, got {other}"),
    }

    // Health is readable and typed.
    server.reset().await;
    mount(
        &server,
        "GET",
        "/internal/v1/health".to_owned(),
        200,
        json!({ "status": "ok", "active_executions": 0 }),
    )
    .await;
    let health = client.health().await.expect("health is readable");
    assert_eq!(health.active_executions, 0);

    // A base url that cannot address the service is refused up front.
    assert!(matches!(
        IntelligenceClient::new("127.0.0.1:15603"),
        Err(IntelligenceError::InvalidBaseUrl { .. })
    ));
}

#[test]
fn t09_staging_object_ids_and_atomic_write() {
    let directory = tempfile::tempdir().expect("temporary staging root");
    let store = StagingStore::new(directory.path(), 1024).expect("staging store opens");

    for rejected in [
        "../x",
        "a/b",
        "a\\b",
        "http://evil",
        "https://evil.example/object",
        ".",
        "..",
        "a..b",
        "",
        "a:b",
        "with space",
    ] {
        assert!(
            ObjectId::parse(rejected).is_err(),
            "{rejected:?} must be rejected as an object id"
        );
    }
    let id = ObjectId::parse("object-t09.json").expect("a controlled id is accepted");
    assert_eq!(
        serde_json::to_value(&id).expect("ids serialise"),
        json!("object-t09.json")
    );
    assert!(
        serde_json::from_value::<ObjectId>(json!("../x")).is_err(),
        "an id that arrives over the wire is validated too"
    );

    let staged = store.write(&id, b"payload").expect("the object is written");
    assert_eq!(staged.object_id, id);
    assert_eq!(staged.sha256, Sha256Hex::digest(b"payload"));
    assert_eq!(staged.size, 7);

    // The write is atomic: the object is complete and no temporary file remains.
    assert_eq!(store.read(&id).expect("readable"), b"payload");
    assert_eq!(store.stat(&id).expect("stat works"), staged);
    store.verify(&staged).expect("the object verifies");
    let leftovers: Vec<String> = fs::read_dir(directory.path())
        .expect("the staging root is listable")
        .map(|entry| entry.expect("readable entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".part"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "leftover temporary files: {leftovers:?}"
    );

    // Rewriting with a different size is caught, and so is a tampered payload.
    let error = store
        .verify(&staged)
        .and_then(|_| {
            store.write(&id, b"payload-plus")?;
            store.verify(&staged)
        })
        .expect_err("a rewritten object is no longer the verified one");
    assert!(
        matches!(error, StagingError::SizeMismatch { .. }),
        "{error}"
    );

    store.write(&id, b"PAYLOAD").expect("same-length rewrite");
    let error = store.verify(&staged).expect_err("tampering is detected");
    assert!(
        matches!(error, StagingError::DigestMismatch { .. }),
        "{error}"
    );

    // The size limit applies to writes and refuses a partial object.
    let oversized = store
        .write(&id, &[0_u8; 2048])
        .expect_err("objects above the limit are refused");
    assert!(
        matches!(oversized, StagingError::TooLarge { .. }),
        "{oversized}"
    );

    // Digests are validated as well, so a malformed manifest is refused early.
    assert!(Sha256Hex::parse("XYZ").is_err());
    assert!(Sha256Hex::parse("A".repeat(64)).is_err());
    assert!(
        serde_json::from_value::<Sha256Hex>(json!("A".repeat(64))).is_err(),
        "uppercase digests are not the contract"
    );
    assert_eq!(
        Sha256Hex::parse(staged.sha256.as_str()).expect("the computed digest parses"),
        staged.sha256
    );

    // A symlink planted in the staging root cannot be used to reach outside it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let link = ObjectId::parse("escape-link").expect("valid id");
        symlink("/etc/passwd", directory.path().join("escape-link"))
            .expect("the symlink is created");
        let read_error = store
            .read(&link)
            .expect_err("a symlink outside the root is refused");
        assert!(
            matches!(read_error, StagingError::EscapesRoot { .. }),
            "{read_error}"
        );
        let write_error = store
            .write(&link, b"must-not-reach-the-target")
            .expect_err("writing through the symlink is refused");
        assert!(
            matches!(write_error, StagingError::EscapesRoot { .. }),
            "{write_error}"
        );
    }

    // A missing object is reported, not guessed.
    let missing = ObjectId::parse("not-staged.json").expect("valid id");
    assert!(matches!(
        store.read(&missing).expect_err("missing objects fail"),
        StagingError::Missing { .. }
    ));
}
