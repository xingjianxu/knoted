use knoted_core::intelligence::{ExecutionRecord, ExecutionRequest, ExecutionState};
use serde_json::{Value, json};

fn request() -> ExecutionRequest {
    serde_json::from_str(include_str!(
        "../../../api/examples/valid/execution-request-parse.json"
    ))
    .unwrap()
}
fn record() -> Value {
    serde_json::from_str(include_str!(
        "../../../api/examples/valid/execution-record-exit-confirmed.json"
    ))
    .unwrap()
}
#[test]
fn t47_wire_exit_proof_is_strict_and_legacy_fails_closed() {
    let mut value = record();
    value
        .as_object_mut()
        .unwrap()
        .remove("compute_exit_confirmed");
    let legacy: ExecutionRecord = serde_json::from_value(value.clone()).unwrap();
    assert!(!legacy.compute_exit_confirmed);
    assert!(legacy.verify_resource_exit(&request()).is_err());
    for invalid in [json!("true"), json!(1), Value::Null] {
        value["compute_exit_confirmed"] = invalid;
        assert!(serde_json::from_value::<ExecutionRecord>(value.clone()).is_err());
    }
    for state in ["queued", "running", "cancel_requested"] {
        value["state"] = json!(state);
        value["compute_exit_confirmed"] = json!(true);
        assert!(serde_json::from_value::<ExecutionRecord>(value.clone()).is_err());
    }
}
#[test]
fn t47_resource_proof_requires_terminal_true_and_every_echo() {
    let request = request();
    let mut value = record();
    value["execution_id"] = json!(request.execution_id);
    value["kind"] = json!(request.kind);
    value["input_sha256"] = json!(request.input_sha256);
    value["source_version"] = json!(request.source_version);
    value["options_revision"] = json!(request.options_revision);
    for state in ["succeeded", "failed", "cancelled", "interrupted"] {
        value["state"] = json!(state);
        value["compute_exit_confirmed"] = json!(true);
        let accepted: ExecutionRecord = serde_json::from_value(value.clone()).unwrap();
        accepted.verify_resource_exit(&request).unwrap();
        for (field, wrong) in [
            ("execution_id", json!(uuid::Uuid::new_v4())),
            ("kind", json!("embed")),
            ("input_sha256", json!("0".repeat(64))),
            ("source_version", json!(999)),
            ("options_revision", json!("wrong")),
            ("compute_exit_confirmed", json!(false)),
        ] {
            let mut changed = value.clone();
            changed[field] = wrong;
            let rejected: ExecutionRecord = serde_json::from_value(changed).unwrap();
            assert!(rejected.verify_resource_exit(&request).is_err(), "{field}");
        }
    }
    assert!(ExecutionState::Cancelled.cancellation_confirmed());
    assert!(!ExecutionState::Succeeded.cancellation_confirmed());
    assert!(!ExecutionState::Interrupted.cancellation_confirmed());
}

#[tokio::test]
async fn t47_attempt_is_durable_lease_fenced_and_replay_keeps_deadline() {
    use knoted_core::{db::testing::TestDb, files::extraction, jobs};
    let db = TestDb::create().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    jobs::enqueue(&mut tx, jobs::JobSpec::new("file.extract"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let lease = jobs::claim(&db.pool, "t47").await.unwrap().unwrap();
    let req = request();
    let first = extraction::register_attempt(&db.pool, &lease, req.clone())
        .await
        .unwrap();
    let mut changed = req.clone();
    changed.execution_id = uuid::Uuid::new_v4();
    changed.deadline += chrono::Duration::seconds(30);
    let replay = extraction::register_attempt(&db.pool, &lease, changed)
        .await
        .unwrap();
    assert_eq!(first.request, replay.request);
    assert_eq!(first.request.deadline, req.deadline);
    let mut stale = lease.clone();
    stale.token = uuid::Uuid::new_v4();
    assert!(
        extraction::register_attempt(&db.pool, &stale, req)
            .await
            .is_err()
    );
    assert_eq!(
        extraction::load_attempts(&db.pool, lease.job_id)
            .await
            .unwrap()
            .len(),
        1
    );
    db.drop().await;
}

#[test]
fn t47_pdf_artifact_requires_complete_echo_and_locations() {
    use knoted_core::files::pdf;
    let req = request();
    let mut artifact = json!({"kind":"parse", "source_version":req.source_version,
      "input_sha256":req.input_sha256,"options_revision":pdf::OPTIONS_REVISION,
      "child_pid":123,"text_length":2,"title":null,"source_format":"pdf","page_count":1,
      "truncated":false,"parser":{"options_revision":pdf::OPTIONS_REVISION, "docling":"2.150.1","docling_ibm_models":"4.0.4","rapidocr":"3.10.0","onnxruntime":"1.32.0","torch":"2.15.0","transformers":"5.23.0","ocr":"onnxruntime/small/ch"},
      "blocks":[{"ordinal":0,"type":"paragraph","text":"中文","page":1,"bbox":[0,2,2,0],"structure":null}]});
    let mut req = req;
    req.options_revision = pdf::OPTIONS_REVISION.into();
    assert!(pdf::decode_artifact(&serde_json::to_vec(&artifact).unwrap(), &req).is_ok());
    artifact["truncated"] = json!(true);
    assert!(pdf::decode_artifact(&serde_json::to_vec(&artifact).unwrap(), &req).is_err());
    artifact["truncated"] = json!(false);
    artifact["blocks"][0]["ordinal"] = json!(1);
    assert!(pdf::decode_artifact(&serde_json::to_vec(&artifact).unwrap(), &req).is_err());
    artifact["blocks"][0]["ordinal"] = json!(0);
    artifact["blocks"][0]["page"] = json!(2);
    assert!(pdf::decode_artifact(&serde_json::to_vec(&artifact).unwrap(), &req).is_err());
    for path in ["a.pdf", "a.PNG", "a.jpeg", "a.jpg", "a.webp"] {
        assert!(pdf::supports(path));
    }
    for path in ["a.docx", "a.gif", "a.txt"] {
        assert!(!pdf::supports(path));
    }
}

#[test]
fn t47_intelligence_configuration_is_explicit_loopback_only() {
    use knoted_core::config::IntelligenceConfig;
    assert!(IntelligenceConfig::default().validate().is_ok());
    for url in [
        "https://127.0.0.1:15603",
        "http://example.com",
        "http://127.0.0.1:15603/path",
        "http://user@127.0.0.1:15603",
    ] {
        assert!(
            IntelligenceConfig {
                base_url: url.into()
            }
            .validate()
            .is_err()
        );
    }
}
