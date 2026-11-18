use knoted_core::{
    rag::citations::{CitationDraft, validate_citations},
    search::meili::SourceType,
};
use uuid::Uuid;

fn evidence() -> CitationDraft {
    CitationDraft {
        id: Uuid::new_v4(),
        chunk_set_id: Uuid::new_v4(),
        source_type: SourceType::Note,
        source_id: Uuid::new_v4(),
        source_version: 3,
        title: "固定资料".into(),
        location: Some("段落 2".into()),
        snippet: "已保存的原版本片段".into(),
    }
}

#[test]
fn t64_1_only_server_issued_citation_ids_are_evidence() {
    let draft = evidence();
    let unknown = Uuid::new_v4();
    let fabricated = format!("https://forged.invalid/ {unknown} [cite:{unknown}]");
    let invalid = validate_citations(&fabricated, std::slice::from_ref(&draft));
    assert!(invalid.ids.is_empty());
    assert_eq!(invalid.answer, "依据不足。");

    let valid = validate_citations(
        &format!("回答 {} {}", draft.marker(), draft.marker()),
        std::slice::from_ref(&draft),
    );
    assert_eq!(valid.ids, [draft.id]);
    assert!(valid.answer.contains("回答"));
}

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use knoted_api::build_router;
use knoted_core::{
    auth::{self, sessions},
    context::AppContext,
    db::testing::TestDb,
    jobs::{self, ExecutionOutcome, FinishExecution, Lease},
    notes::service::{self as notes, CreateNote, DeleteNote, Note, UpdateNote},
    rag::{
        chat::{self, ChatScope, SubmitMessage, Submitted},
        chunking::CHUNKER_REVISION,
        citations::persist_citations,
        context::{self as rag_context, ContextInput, HistoryTurn},
    },
};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Harness {
    db: TestDb,
    ctx: AppContext,
    app: Router,
    user: Uuid,
    session: Uuid,
    cookie: String,
}

impl Harness {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let ctx = AppContext::new(db.pool.clone());
        let mut tx = ctx.pool().begin().await.unwrap();
        let user = auth::ensure_owner(&mut tx).await.unwrap().user.id;
        let login = sessions::create_session(&mut tx, user, ctx.now(), None, true)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let session = chat::create_session(&ctx, user, Some("T64 fixture".into()), None)
            .await
            .unwrap()
            .id;
        Self {
            db,
            app: build_router(ctx.clone()),
            ctx,
            user,
            session,
            cookie: format!("knoted_session={}", login.token),
        }
    }

    async fn begin(&self) -> (Submitted, Lease) {
        let submitted = chat::submit_message(
            &self.ctx,
            SubmitMessage {
                user_id: self.user,
                session_id: self.session,
                idempotency_key: Uuid::new_v4().to_string(),
                content: "用户问题".into(),
                scope: ChatScope::default(),
            },
        )
        .await
        .unwrap();
        let lease = jobs::claim(self.ctx.pool(), "t64").await.unwrap().unwrap();
        chat::begin_generation(&self.ctx, &lease, submitted.run.id)
            .await
            .unwrap();
        chat::mark_upstream_requested(&self.ctx, &lease, submitted.run.id, "fixture-model")
            .await
            .unwrap();
        (submitted, lease)
    }

    async fn evidence(&self) -> (Note, CitationDraft) {
        let note = notes::create(
            &self.ctx,
            CreateNote {
                title: "固定资料".into(),
                parent_id: None,
                document_json: r#"{"blocks":[]}"#.into(),
            },
        )
        .await
        .unwrap();
        let mut draft = evidence();
        draft.source_id = note.id;
        draft.source_version = note.content_version;
        sqlx::query(
            "INSERT INTO rag_chunk_sets(id,source_type,source_id,source_version,extraction_id,chunker_revision)
             SELECT $1,source_type,source_id,source_version,extraction_id,$2
             FROM current_rag_sources WHERE source_type='note' AND source_id=$3",
        )
        .bind(draft.chunk_set_id)
        .bind(CHUNKER_REVISION)
        .bind(note.id)
        .execute(self.ctx.pool())
        .await
        .unwrap();
        (note, draft)
    }

    async fn complete(
        &self,
        submitted: &Submitted,
        lease: &Lease,
        drafts: &[CitationDraft],
    ) -> Uuid {
        let run_id = submitted.run.id;
        let assistant: Uuid =
            sqlx::query_scalar("SELECT assistant_message_id FROM generation_runs WHERE id=$1")
                .bind(run_id)
                .fetch_one(self.ctx.pool())
                .await
                .unwrap();
        let mut tx = self.ctx.pool().begin().await.unwrap();
        let ids = drafts
            .iter()
            .map(|draft| draft.chunk_set_id)
            .collect::<Vec<_>>();
        let dependencies = rag_context::capture_sources(&mut tx, &ids).await.unwrap();
        rag_context::persist_dependencies(&mut tx, run_id, &dependencies)
            .await
            .unwrap();
        let answer = format!(
            "早先的回答 {}",
            drafts
                .iter()
                .map(CitationDraft::marker)
                .collect::<Vec<_>>()
                .join(" ")
        );
        let validated = validate_citations(&answer, drafts);
        persist_citations(&mut tx, run_id, assistant, &validated, drafts)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        chat::record_delta(&self.ctx, lease, run_id, &validated.answer)
            .await
            .unwrap();
        chat::complete_generation(&self.ctx, lease, run_id)
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
        assistant
    }

    async fn get(&self, message: Uuid, params: &str, authenticate: bool) -> (StatusCode, Value) {
        let mut request =
            Request::builder().uri(format!("/api/v1/chat/messages/{message}/citations{params}"));
        if authenticate {
            request = request.header("cookie", &self.cookie);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }
}

#[tokio::test]
async fn t64_2_citations_keep_fixed_snapshot_after_version_change_and_deletion() {
    let fixture = Harness::new().await;
    let (note, draft) = fixture.evidence().await;
    let (submitted, lease) = fixture.begin().await;
    let assistant = fixture
        .complete(&submitted, &lease, std::slice::from_ref(&draft))
        .await;

    let before = fixture.get(assistant, "", true).await;
    let note = notes::update(
        &fixture.ctx,
        UpdateNote {
            id: note.id,
            expected_version: note.content_version,
            title: Some("更新标题".into()),
            document_json: None,
        },
    )
    .await
    .unwrap();
    let superseded = fixture.get(assistant, "", true).await;
    notes::delete(
        &fixture.ctx,
        DeleteNote {
            id: note.id,
            expected_version: note.content_version,
        },
    )
    .await
    .unwrap();
    let deleted = fixture.get(assistant, "", true).await;
    fixture.db.drop().await;

    for (response, expected) in [
        (before, "current"),
        (superseded, "missing"),
        (deleted, "deleted"),
    ] {
        assert_eq!(response.0, StatusCode::OK);
        assert_eq!(
            response.1,
            json!({"next_cursor":null,"items":[{
                "id":draft.id, "message_id":assistant, "source_type":"note", "source_id":note.id,
                "title":draft.title, "location":draft.location, "snippet":draft.snippet,
                "source_state":expected,
            }]})
        );
    }
}

#[tokio::test]
async fn t64_3_persisted_snapshots_are_not_materialized_into_new_prompt_history() {
    let fixture = Harness::new().await;
    let (_, draft) = fixture.evidence().await;
    let (old, lease) = fixture.begin().await;
    fixture
        .complete(&old, &lease, std::slice::from_ref(&draft))
        .await;
    let (new, _) = fixture.begin().await;
    let new = chat::load_generation_input(&fixture.ctx, new.run.id)
        .await
        .unwrap();
    let mut conn = fixture.ctx.pool().acquire().await.unwrap();
    let dependencies = rag_context::load_dependencies(&mut conn, old.run.id)
        .await
        .unwrap();
    drop(conn);
    let rebuilt = rag_context::build_context(
        ContextInput {
            system: "新运行".into(),
            question: new.question.body,
            history: vec![HistoryTurn {
                question: new.history[0].body.clone(),
                answer: new.history[1].body.clone(),
                dependencies,
            }],
            evidence: vec![],
            unavailable: Default::default(),
            input_limit: 28000,
        },
        str::len,
    )
    .unwrap();
    fixture.db.drop().await;

    assert_eq!(rebuilt.history[0].answer, "早先的回答");
    assert!(!rebuilt.history[0].answer.contains(&draft.snippet));
    assert!(rebuilt.evidence.is_empty());
    assert_eq!(rebuilt.dependencies[0].chunk_set_id, draft.chunk_set_id);
}

#[tokio::test]
async fn t64_4_protects_history_and_binds_cursor_to_the_answer() {
    let fixture = Harness::new().await;
    let (_, first) = fixture.evidence().await;
    let (_, second) = fixture.evidence().await;
    let (submitted, lease) = fixture.begin().await;
    let assistant = fixture
        .complete(&submitted, &lease, &[first.clone(), second.clone()])
        .await;
    let unauthenticated = fixture.get(assistant, "", false).await;
    let page = fixture.get(assistant, "?limit=1", true).await;
    let cursor = page.1["next_cursor"].as_str().unwrap_or("missing");
    let next = fixture
        .get(assistant, &format!("?limit=1&cursor={cursor}"), true)
        .await;
    let changed = fixture
        .get(
            submitted.message.id,
            &format!("?limit=1&cursor={cursor}"),
            true,
        )
        .await;
    let version = chat::get_session(&fixture.ctx, fixture.user, fixture.session)
        .await
        .unwrap()
        .session
        .version;
    chat::delete_session(&fixture.ctx, fixture.user, fixture.session, version)
        .await
        .unwrap();
    let deleted_session = fixture.get(assistant, "", true).await;
    fixture.db.drop().await;

    assert_eq!(unauthenticated.0, StatusCode::UNAUTHORIZED);
    assert_eq!(page.0, StatusCode::OK);
    assert_eq!(page.1["items"][0]["id"], first.id.to_string());
    assert_ne!(cursor, "missing");
    assert_eq!(next.0, StatusCode::OK);
    assert_eq!(next.1["items"][0]["id"], second.id.to_string());
    assert_eq!(next.1["next_cursor"], Value::Null);
    assert_eq!(changed.0, StatusCode::BAD_REQUEST);
    assert_eq!(deleted_session.0, StatusCode::NOT_FOUND);
}
