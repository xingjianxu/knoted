//! Answer conversion through authenticated HTTP and the real note store.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use knoted_api::build_router_with_auth;
use knoted_core::{
    auth::{self, sessions},
    config::AuthConfig,
    context::AppContext,
    db::testing::TestDb,
    jobs::{self, ExecutionOutcome, FinishExecution, Lease},
    notes::service::{self as notes, CreateNote, Note},
    rag::{
        chat::{self, ChatScope, SubmitMessage, Submitted},
        chunking::CHUNKER_REVISION,
        citations::{self, CitationDraft},
        context as rag_context,
        providers::ProviderInfo,
        retrieval::EvidenceSet,
    },
    search::meili::SourceType,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const ANSWER: &str =
    "第一段\n\n<script>保留原文</script> ![外链图片](https://fixture.invalid/image.png)";

struct Fixture {
    db: TestDb,
    ctx: AppContext,
    app: Router,
    user: Uuid,
    session: Uuid,
    cookie: String,
    csrf: String,
}

impl Fixture {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let ctx = AppContext::new(db.pool.clone());
        let mut tx = ctx.pool().begin().await.unwrap();
        let user = auth::ensure_owner(&mut tx).await.unwrap().user.id;
        let login = sessions::create_session(&mut tx, user, ctx.now(), None, true)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let session = chat::create_session(&ctx, user, Some("T66 夹具".into()), None)
            .await
            .unwrap()
            .id;
        Self {
            db,
            app: build_router_with_auth(ctx.clone(), AuthConfig::default()),
            ctx,
            user,
            session,
            cookie: format!(
                "knoted_session={}; knoted_csrf={}",
                login.token, login.csrf_token
            ),
            csrf: login.csrf_token,
        }
    }

    async fn send(
        &self,
        method: &str,
        path: &str,
        body: Value,
        key: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(format!("/api/v1{path}"))
            .header("cookie", &self.cookie)
            .header("origin", AuthConfig::default().origin)
            .header("content-type", "application/json")
            .header("x-csrf-token", &self.csrf);
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn save(&self, message: Uuid, body: Value, key: Option<&str>) -> (StatusCode, Value) {
        self.send(
            "POST",
            &format!("/chat/messages/{message}/save-note"),
            body,
            key,
        )
        .await
    }

    async fn note(&self, title: &str) -> Note {
        notes::create(
            &self.ctx,
            CreateNote {
                title: title.into(),
                parent_id: None,
                document_json: r#"{"blocks":[]}"#.into(),
            },
        )
        .await
        .unwrap()
    }

    async fn begin(&self) -> (Submitted, Uuid, Lease) {
        let submitted = chat::submit_message(
            &self.ctx,
            SubmitMessage {
                user_id: self.user,
                session_id: self.session,
                idempotency_key: Uuid::new_v4().to_string(),
                content: "合成问题".into(),
                scope: ChatScope::default(),
            },
        )
        .await
        .unwrap();
        let assistant: Uuid =
            sqlx::query_scalar("SELECT assistant_message_id FROM generation_runs WHERE id=$1")
                .bind(submitted.run.id)
                .fetch_one(self.ctx.pool())
                .await
                .unwrap();
        let lease = jobs::claim(self.ctx.pool(), "t66").await.unwrap().unwrap();
        chat::begin_generation(&self.ctx, &lease, submitted.run.id)
            .await
            .unwrap();
        (submitted, assistant, lease)
    }

    async fn complete(&self, run: Uuid, assistant: Uuid, lease: &Lease, source: Option<&Note>) {
        let input = chat::load_generation_input(&self.ctx, run).await.unwrap();
        chat::prepare_generation_context(
            &self.ctx,
            lease,
            &input,
            EvidenceSet { items: vec![] },
            &ProviderInfo {
                name: "local".into(),
                model: "fixture-model".into(),
                context_window: 32 * 1024,
                max_input_tokens: 28 * 1024,
            },
            None,
        )
        .await
        .unwrap();
        if let Some(note) = source {
            let draft = CitationDraft {
                id: Uuid::new_v4(),
                chunk_set_id: Uuid::new_v4(),
                source_type: SourceType::Note,
                source_id: note.id,
                source_version: note.content_version,
                title: note.title.clone(),
                location: Some("段落 2".into()),
                snippet: "固定的原始引用".into(),
            };
            let mut tx = self.ctx.pool().begin().await.unwrap();
            sqlx::query("INSERT INTO rag_chunk_sets(id,source_type,source_id,source_version,extraction_id,chunker_revision)
                SELECT $1,source_type,source_id,source_version,extraction_id,$2
                FROM current_rag_sources WHERE source_type='note' AND source_id=$3")
                .bind(draft.chunk_set_id)
                .bind(CHUNKER_REVISION)
                .bind(note.id)
                .execute(&mut *tx)
                .await
                .unwrap();
            let dependencies = rag_context::capture_sources(&mut tx, &[draft.chunk_set_id])
                .await
                .unwrap();
            rag_context::persist_dependencies(&mut tx, run, &dependencies)
                .await
                .unwrap();
            let validated = citations::validate_citations(
                &format!("{ANSWER} {}", draft.marker()),
                std::slice::from_ref(&draft),
            );
            citations::persist_citations(&mut tx, run, assistant, &validated, &[draft])
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }
        chat::record_delta(&self.ctx, lease, run, ANSWER)
            .await
            .unwrap();
        chat::complete_generation(&self.ctx, lease, run)
            .await
            .unwrap();
        jobs::service::finish_execution(
            &self.ctx,
            FinishExecution {
                job_id: lease.job_id,
                lease_token: lease.token,
                outcome: ExecutionOutcome::Succeeded,
                cancel_confirmed: false,
            },
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn t66_1_replays_concurrent_saves_and_rejects_a_changed_request() {
    let fixture = Fixture::new().await;
    let (submitted, assistant, lease) = fixture.begin().await;
    fixture
        .complete(submitted.run.id, assistant, &lease, None)
        .await;
    let body = json!({"title":"转存"});
    let (first, second) = tokio::join!(
        fixture.save(assistant, body.clone(), Some("same-key")),
        fixture.save(assistant, body, Some("same-key")),
    );
    let changed = fixture
        .save(assistant, json!({"title":"另一标题"}), Some("same-key"))
        .await;
    let changed_invalid = fixture
        .save(assistant, json!({"title":" "}), Some("same-key"))
        .await;
    let fresh_invalid = fixture
        .save(assistant, json!({"title":" "}), Some("fresh-key"))
        .await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM notes")
        .fetch_one(fixture.ctx.pool())
        .await
        .unwrap();
    fixture.db.drop().await;

    assert_eq!(first.0, StatusCode::CREATED, "{}", first.1);
    assert_eq!(second, first);
    assert!(
        first.1["note_id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_some()
    );
    assert_eq!(count, 1);
    assert_eq!(changed.0, StatusCode::CONFLICT);
    assert_eq!(changed_invalid.0, StatusCode::CONFLICT);
    assert_eq!(fresh_invalid.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn t66_2_note_and_citation_survive_deletion_of_sources() {
    let fixture = Fixture::new().await;
    let source = fixture.note("资料标题").await;
    let (submitted, assistant, lease) = fixture.begin().await;
    fixture
        .complete(submitted.run.id, assistant, &lease, Some(&source))
        .await;
    let saved = fixture
        .save(assistant, json!({"title":"已转存"}), Some("save"))
        .await;
    notes::delete(
        &fixture.ctx,
        notes::DeleteNote {
            id: source.id,
            expected_version: 1,
        },
    )
    .await
    .unwrap();
    let state = chat::get_session(&fixture.ctx, fixture.user, fixture.session)
        .await
        .unwrap();
    chat::delete_session(
        &fixture.ctx,
        fixture.user,
        fixture.session,
        state.session.version,
    )
    .await
    .unwrap();
    let revoked = fixture
        .save(assistant, json!({"title":"新笔记"}), Some("new"))
        .await;
    let path = format!(
        "/notes/{}",
        saved.1["note_id"].as_str().unwrap_or("missing")
    );
    let (status, note) = fixture.send("GET", &path, Value::Null, None).await;
    fixture.db.drop().await;

    assert_eq!(saved.0, StatusCode::CREATED, "{}", saved.1);
    assert_eq!(revoked.0, StatusCode::NOT_FOUND);
    assert_eq!(status, StatusCode::OK, "{note}");
    assert_eq!(note["title"], "已转存");
    let blocks = note["document"]["blocks"].as_array().unwrap();
    let contents = blocks
        .iter()
        .flat_map(|block| block["content"].as_array().unwrap())
        .map(|inline| {
            assert_eq!(inline["type"], "text");
            inline["text"].as_str().unwrap()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(contents.contains("第一段"));
    assert!(contents.contains("<script>保留原文</script>"));
    for snapshot_value in [
        source.id.to_string(),
        assistant.to_string(),
        fixture.session.to_string(),
        "资料标题".into(),
        "段落 2".into(),
        "固定的原始引用".into(),
    ] {
        assert!(contents.contains(&snapshot_value), "{snapshot_value}");
    }
}

#[tokio::test]
async fn t66_3_saved_note_uses_the_normal_edit_and_index_paths() {
    let fixture = Fixture::new().await;
    let parent = fixture.note("父页").await;
    let (submitted, assistant, lease) = fixture.begin().await;
    fixture
        .complete(submitted.run.id, assistant, &lease, None)
        .await;
    let saved = fixture
        .save(
            assistant,
            json!({"title":"初始标题","parent_id":parent.id}),
            Some("save"),
        )
        .await;
    let id = saved.1["note_id"].as_str().unwrap_or("missing");
    let path = format!("/notes/{id}");
    let (_, original) = fixture.send("GET", &path, Value::Null, None).await;
    let (status, modified) = fixture.send("PATCH", &path, json!({
        "title":"编辑后的标题","expected_version":1,
        "document":{"blocks":[{"id":"edit","type":"paragraph","content":[{"type":"text","text":"属于笔记的正文"}]}]}
    }), None).await;
    let indexed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM outbox WHERE topic='note.changed' AND payload->>'source_id'=$1 AND payload->>'operation'='created')",
    ).bind(id).fetch_one(fixture.ctx.pool()).await.unwrap();
    let session = chat::get_session(&fixture.ctx, fixture.user, fixture.session)
        .await
        .unwrap();
    fixture.db.drop().await;

    assert_eq!(saved.0, StatusCode::CREATED, "{}", saved.1);
    assert_eq!(original["parent_id"], parent.id.to_string());
    assert_eq!(status, StatusCode::OK, "{modified}");
    assert_eq!(modified["content_version"], 2);
    assert_eq!(modified["title"], "编辑后的标题");
    assert_eq!(
        modified["document"]["blocks"][0]["content"][0]["text"],
        "属于笔记的正文"
    );
    assert!(indexed);
    assert_eq!(session.messages.last().unwrap().body, ANSWER);
}

#[tokio::test]
async fn t66_4_rejects_unfinished_and_user_messages_without_consuming_the_key() {
    let fixture = Fixture::new().await;
    let (submitted, assistant, lease) = fixture.begin().await;
    let body = json!({"title":"转存"});
    let missing_key = fixture.save(assistant, body.clone(), None).await;
    let unfinished = fixture.save(assistant, body.clone(), Some("retry")).await;
    let question = fixture
        .save(submitted.message.id, body.clone(), Some("question"))
        .await;
    let absent = fixture
        .save(Uuid::new_v4(), body.clone(), Some("absent"))
        .await;
    fixture
        .complete(submitted.run.id, assistant, &lease, None)
        .await;
    let completed = fixture.save(assistant, body, Some("retry")).await;
    fixture.db.drop().await;

    assert_eq!(missing_key.0, StatusCode::BAD_REQUEST);
    assert_eq!(unfinished.0, StatusCode::CONFLICT);
    assert_eq!(question.0, StatusCode::CONFLICT);
    assert_eq!(absent.0, StatusCode::NOT_FOUND);
    assert_eq!(completed.0, StatusCode::CREATED, "{}", completed.1);
}

#[tokio::test]
async fn t66_7_requires_session_and_csrf() {
    let fixture = Fixture::new().await;
    let (submitted, assistant, lease) = fixture.begin().await;
    fixture
        .complete(submitted.run.id, assistant, &lease, None)
        .await;
    let mut statuses = Vec::new();
    for cookie in [None, Some(fixture.cookie.as_str())] {
        let mut request = Request::post(format!("/api/v1/chat/messages/{assistant}/save-note"))
            .header("content-type", "application/json")
            .header("origin", AuthConfig::default().origin)
            .header("idempotency-key", "authorization");
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        let response = fixture
            .app
            .clone()
            .oneshot(
                request
                    .body(Body::from(r#"{"title":"有授权才转存"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        statuses.push(response.status());
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM notes")
        .fetch_one(fixture.ctx.pool())
        .await
        .unwrap();
    fixture.db.drop().await;

    assert_eq!(statuses, [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN]);
    assert_eq!(count, 0);
}
