//! Model stream contracts with separate loopback peers for local and external.

use knoted_core::rag::providers::{
    ChatCancellation, ChatMessage, ChatProvider, ChatProvidersConfig, ChatRequest, FinishReason,
    ProviderConfig, ProviderError, ProviderEvent, ProviderRegistry, Role,
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tempfile::TempDir;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, header, method, path},
};

const LOCAL_MODEL: &str = "local-fixture";
const REMOTE_MODEL: &str = "external-fixture";

fn temporary_files() -> TempDir {
    tempfile::tempdir().unwrap()
}

fn service(server: &MockServer, model: &str, files: &TempDir, key: &str) -> ProviderConfig {
    let credential_file = files.path().join(model);
    std::fs::write(&credential_file, key).unwrap();
    ProviderConfig {
        base_url: server.uri(),
        model: model.into(),
        credential_file,
    }
}

fn request() -> ChatRequest {
    ChatRequest {
        messages: vec![ChatMessage {
            role: Role::User,
            content: "test prompt".into(),
        }],
        input_tokens: 20,
        max_output_tokens: 100,
    }
}

fn status(in_flight: u32, context: u32) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "service":"strata",
        "model":LOCAL_MODEL,
        "loaded":true,
        "cache_max_tokens":context,
        "context":{"native":32768,"max_positions":context},
        "activity":{"in_flight":in_flight}
    }))
}

async fn local_models(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data":[{"id":LOCAL_MODEL,"meta":{"n_ctx":32768}}]
        })))
        .mount(server)
        .await;
}

async fn local_ready(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/status"))
        .respond_with(status(0, 32768))
        .mount(server)
        .await;
    local_models(server).await;
}

fn chunk(model: &str, delta: serde_json::Value, reason: Option<&str>) -> String {
    format!(
        "data: {}\r\n\r\n",
        json!({
            "model":model,
            "choices":[{"index":0,"delta":delta,"finish_reason":reason}]
        })
    )
}

fn stream(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

#[tokio::test]
async fn t61_1_local_failure_never_calls_external() {
    let files = temporary_files();
    let local = MockServer::start().await;
    let external = MockServer::start().await;
    local_ready(&local).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&local)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&external)
        .await;

    let specs = ChatProvidersConfig {
        local: Some(service(&local, LOCAL_MODEL, &files, "local-fixture-value")),
        external: [(
            "remote".into(),
            service(&external, REMOTE_MODEL, &files, "external-fixture-value"),
        )]
        .into(),
    };
    let registry = ProviderRegistry::new(specs).unwrap();
    let provider = registry.select(None).unwrap();
    let error = provider
        .stream(request(), ChatCancellation::new())
        .await
        .unwrap_err();
    assert_eq!(error, ProviderError::Unavailable);
    assert!(external.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn t61_2_provider_errors_and_debug_redact_credential_and_prompt() {
    let files = temporary_files();
    let external = MockServer::start().await;
    let credential = format!("fixture-{}", Uuid::new_v4());
    let prompt = format!("prompt-{}", Uuid::new_v4());
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", format!("Bearer {credential}")))
        .respond_with(stream(format!(
            "event: error\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"error":{"message":format!("{credential} {prompt}")}})
        )))
        .expect(1)
        .mount(&external)
        .await;
    let registry = ProviderRegistry::new(ChatProvidersConfig {
        local: None,
        external: [(
            "remote".into(),
            service(&external, REMOTE_MODEL, &files, &credential),
        )]
        .into(),
    })
    .unwrap();
    let provider = registry.select(Some("remote")).unwrap();
    let mut payload = request();
    payload.messages[0].content.clone_from(&prompt);
    let diagnostics = format!("{registry:?} {provider:?} {payload:?}");
    let mut events = provider
        .stream(payload, ChatCancellation::new())
        .await
        .unwrap();
    let error = events.next().await.unwrap().unwrap_err();
    assert_eq!(error, ProviderError::UpstreamStreamError);
    let diagnostics = format!("{diagnostics} {events:?} {error:?} {error}");
    assert!(!diagnostics.contains(&credential));
    assert!(!diagnostics.contains(&prompt));
    assert!(events.next().await.is_none());
}

#[tokio::test]
async fn t61_3_stop_requires_terminal_evidence_and_disconnect_is_distinct() {
    let files = temporary_files();
    let external = MockServer::start().await;
    let registry = ProviderRegistry::new(ChatProvidersConfig {
        local: None,
        external: [(
            "remote".into(),
            service(&external, REMOTE_MODEL, &files, "external-fixture-value"),
        )]
        .into(),
    })
    .unwrap();
    let provider = registry.select(Some("remote")).unwrap();

    for (ending, expected) in [
        ("", Err(ProviderError::Interrupted)),
        (
            "data: [DONE]\n\n",
            Ok(ProviderEvent::Finished(FinishReason::Stop)),
        ),
    ] {
        let scoped = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({
                "model": REMOTE_MODEL,
                "stream": true,
                "max_tokens": 100,
            })))
            .respond_with(stream(
                chunk(
                    REMOTE_MODEL,
                    json!({"reasoning_content":"private reasoning"}),
                    None,
                ) + &chunk(REMOTE_MODEL, json!({"content":"两"}), None)
                    + &chunk(REMOTE_MODEL, json!({}), Some("stop"))
                    + ending,
            ))
            .expect(1)
            .mount_as_scoped(&external)
            .await;
        let mut events = provider
            .stream(request(), ChatCancellation::new())
            .await
            .unwrap();
        assert_eq!(
            events.next().await,
            Some(Ok(ProviderEvent::Delta("两".into())))
        );
        assert_eq!(events.next().await, Some(expected));
        assert!(events.next().await.is_none());
        drop(scoped);
    }
}

#[tokio::test]
async fn t61_4_cancel_reports_provider_confirmation() {
    let files = temporary_files();
    let local = MockServer::start().await;
    let external = MockServer::start().await;
    local_models(&local).await;
    let probes = Arc::new(AtomicUsize::new(0));
    let observed = probes.clone();
    Mock::given(method("GET"))
        .and(path("/v1/status"))
        .respond_with(move |_: &wiremock::Request| {
            let in_flight = u32::from(observed.fetch_add(1, Ordering::SeqCst) < 2);
            status(in_flight, 32768)
        })
        .mount(&local)
        .await;
    for (server, model) in [(&local, LOCAL_MODEL), (&external, REMOTE_MODEL)] {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(stream(chunk(model, json!({"content":"partial"}), None)))
            .expect(1)
            .mount(server)
            .await;
    }
    let registry = ProviderRegistry::new(ChatProvidersConfig {
        local: Some(service(&local, LOCAL_MODEL, &files, "local-fixture-value")),
        external: [(
            "remote".into(),
            service(&external, REMOTE_MODEL, &files, "external-fixture-value"),
        )]
        .into(),
    })
    .unwrap();
    for (selection, confirmed) in [(None, true), (Some("remote"), false)] {
        let provider = registry.select(selection).unwrap();
        let cancel = ChatCancellation::new();
        let mut events = provider.stream(request(), cancel.clone()).await.unwrap();
        assert_eq!(
            events.next().await,
            Some(Ok(ProviderEvent::Delta("partial".into())))
        );
        cancel.cancel();
        assert_eq!(
            events.next().await,
            Some(Err(ProviderError::Cancelled { confirmed })),
        );
        assert!(events.next().await.is_none());
    }
    assert!(probes.load(Ordering::SeqCst) >= 3);
}

#[tokio::test]
async fn t61_5_local_input_budget_respects_strata_runtime_limit() {
    let files = temporary_files();
    let local = MockServer::start().await;
    local_models(&local).await;
    Mock::given(method("GET"))
        .and(path("/v1/status"))
        .respond_with(status(0, 8192))
        .mount(&local)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&local)
        .await;
    let registry = ProviderRegistry::new(ChatProvidersConfig {
        local: Some(service(&local, LOCAL_MODEL, &files, "local-fixture-value")),
        external: Default::default(),
    })
    .unwrap();
    let provider = registry.select(None).unwrap();
    let info = provider.info().await.unwrap();
    assert_eq!(info.name, "local");
    assert_eq!(info.model, LOCAL_MODEL);
    assert_eq!(info.context_window, 8192);
    assert_eq!(info.max_input_tokens, 4096);
    let mut payload = request();
    payload.input_tokens = 4097;
    assert_eq!(
        provider
            .stream(payload, ChatCancellation::new())
            .await
            .unwrap_err(),
        ProviderError::ContextTooLarge
    );
}

#[test]
fn t61_6_process_configuration_loads_named_providers_without_embedded_secrets() {
    use knoted_core::config::AppConfig;

    let files = temporary_files();
    let path = files.path().join("knoted.toml");
    let credential_file = files.path().join("credential");
    std::fs::write(
        &path,
        format!(
            r#"
[chat.local]
base_url = "http://127.0.0.1:15604"
model = "local-fixture"
credential_file = "{}"

[chat.external.team]
base_url = "https://example.invalid/provider/v1"
model = "external-fixture"
credential_file = "{}"
"#,
            credential_file.display(),
            credential_file.display(),
        ),
    )
    .unwrap();
    let config = AppConfig::load(&path).unwrap();
    assert_eq!(config.chat.local.unwrap().model, LOCAL_MODEL);
    assert_eq!(config.chat.external["team"].model, REMOTE_MODEL);

    let marker = format!("fixture-{}", Uuid::new_v4());
    std::fs::write(
        &path,
        format!(
            r#"
[chat.external.team]
base_url = "https://user:{marker}@example.invalid/v1"
model = "external-fixture"
credential_file = "{}"
"#,
            credential_file.display(),
        ),
    )
    .unwrap();
    let err = AppConfig::load(&path).unwrap_err();
    assert!(!format!("{err:?} {err}").contains(&marker));
}

#[test]
fn t61_external_registry_accepts_only_api_provider_names() {
    use knoted_core::rag::providers::{ChatProvidersConfig, ProviderConfig, ProviderRegistry};
    use std::path::PathBuf;
    for (name, accepted) in [
        ("-outside", false),
        ("_outside", false),
        ("outside-1", true),
    ] {
        let config = ChatProvidersConfig {
            local: None,
            external: [(
                name.into(),
                ProviderConfig {
                    base_url: "https://example.invalid".into(),
                    model: "fixture-model".into(),
                    credential_file: PathBuf::from("/run/knoted/fixture.key"),
                },
            )]
            .into(),
        };
        assert_eq!(ProviderRegistry::new(config).is_ok(), accepted);
    }
}
