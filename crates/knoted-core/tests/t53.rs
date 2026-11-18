//! T53: full-text publication against a real Meilisearch and a stubbed engine.
//!
//! `t53_1`, `t53_3`, `t53_4`, `t53_5` and the cleanup case need a live
//! Meilisearch with the full charabia tokenizers (Chinese segmentation); the
//! address comes from `KNOTED_TEST_MEILI_URL` and defaults to the dev port
//! `http://127.0.0.1:15605`. An unreachable engine fails the test instead of
//! silently skipping it: the card's acceptance requires a real hit.
//!
//! `t53_2` stubs the engine so the "task accepted is not success" window is
//! deterministic.

use chrono::{TimeZone, Utc};
use knoted_core::{
    config::SearchConfig,
    context::AppContext,
    db::testing::TestDb,
    jobs::{self, ExecutionOutcome, Job},
    search::meili::{self, GetPublication, PublicationStatus, SourceDocument, SourceType},
};
use serde_json::{Value, json};
use uuid::Uuid;

const MEILI_URL_ENV: &str = "KNOTED_TEST_MEILI_URL";
const DEFAULT_MEILI_URL: &str = "http://127.0.0.1:15605";

fn meili_url() -> String {
    match std::env::var(MEILI_URL_ENV) {
        Ok(url) if !url.trim().is_empty() => url.trim_end_matches('/').to_owned(),
        _ => DEFAULT_MEILI_URL.to_owned(),
    }
}

struct Engine {
    base: String,
    index: String,
    http: reqwest::Client,
}

impl Engine {
    fn new() -> Engine {
        let index = format!("knoted_t53_{}", &Uuid::new_v4().simple().to_string()[..8]);
        Engine {
            base: meili_url(),
            index,
            http: reqwest::Client::new(),
        }
    }

    fn config(&self) -> SearchConfig {
        SearchConfig {
            enabled: true,
            base_url: self.base.clone(),
            index_uid: self.index.clone(),
            api_key_credential: None,
        }
    }

    async fn get(&self, path: &str) -> Value {
        let response = self
            .http
            .get(format!("{}/{}", self.base, path))
            .send()
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "Meilisearch is required for t53 (set {MEILI_URL_ENV}); \
                     {} failed: {error}",
                    self.base
                )
            });
        let status = response.status();
        let body: Value = response.json().await.expect("Meilisearch returns JSON");
        assert!(status.is_success(), "GET {path} failed: {body}");
        body
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        let response = self
            .http
            .post(format!("{}/{}", self.base, path))
            .json(&body)
            .send()
            .await
            .expect("Meilisearch is reachable");
        let status = response.status();
        let body: Value = response.json().await.expect("Meilisearch returns JSON");
        assert!(status.is_success(), "POST {path} failed: {body}");
        body
    }

    async fn search(&self, query: &str) -> Value {
        self.post(
            &format!("indexes/{}/search", self.index),
            json!({ "q": query, "attributesToRetrieve": ["id"] }),
        )
        .await
    }

    async fn document_count(&self) -> i64 {
        let stats = self.get(&format!("indexes/{}/stats", self.index)).await;
        stats["numberOfDocuments"].as_i64().unwrap()
    }

    /// Best-effort removal of the throw-away index; a failure here must not
    /// hide the test result.
    async fn drop_index(&self) {
        let _ = self
            .http
            .delete(format!("{}/indexes/{}", self.base, self.index))
            .send()
            .await;
    }
}

fn document(source_id: Uuid, version: i64, body: &str) -> SourceDocument {
    SourceDocument {
        source_type: SourceType::Note,
        source_id,
        source_version: version,
        title: "周会纪要".into(),
        body: body.into(),
        project_id: None,
        tags: vec!["会议".into()],
        archived: false,
        deleted: false,
        path: None,
        format: None,
        updated_at: Utc.with_ymd_and_hms(2026, 11, 18, 9, 30, 0).unwrap(),
    }
}

/// Claims one specific job. The queue has no kind filter, so the other due jobs
/// (for example the cleanup of an older version) are pushed out of the way first.
async fn claim_job(ctx: &AppContext, job_id: Uuid) -> (jobs::Lease, Job) {
    sqlx::query(
        "UPDATE jobs SET available_at=now()+interval '1 hour' \
         WHERE id<>$1 AND state IN ('queued','retry_wait')",
    )
    .bind(job_id)
    .execute(ctx.pool())
    .await
    .unwrap();
    let job: Job = sqlx::query_as("SELECT * FROM jobs WHERE id=$1")
        .bind(job_id)
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    let lease = jobs::claim(ctx.pool(), "t53")
        .await
        .unwrap()
        .expect("the requested job is due");
    assert_eq!(lease.job_id, job_id);
    (lease, job)
}

/// Claims the queued publication job and returns the lease with its envelope.
async fn claim_publication(ctx: &AppContext, publication_id: Uuid) -> (jobs::Lease, Job) {
    let job_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE kind='fulltext.publish' AND payload->>'publication_id'=$1 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(publication_id.to_string())
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    claim_job(ctx, job_id).await
}

async fn execute(ctx: &AppContext, lease: &jobs::Lease, job: &Job) -> ExecutionOutcome {
    meili::execute(ctx, lease, job).await
}

#[tokio::test]
async fn t53_1_chinese_word_inside_a_long_sentence_is_found() {
    let engine = Engine::new();
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_search(engine.config());
    let source = Uuid::new_v4();
    let body = "这是一段比较长的中文句子，用来验证统一全文检索是否可以在长句中命中中文词语，\
                例如知识管理、检索召回与项目会议纪要（meeting notes）。";

    let publication = meili::publish_fulltext(&ctx, document(source, 1, body))
        .await
        .unwrap();
    assert_eq!(publication.status, PublicationStatus::Pending);
    assert!(publication.document_key.ends_with("_v1"));

    let (lease, job) = claim_publication(&ctx, publication.id).await;
    assert_eq!(
        execute(&ctx, &lease, &job).await,
        ExecutionOutcome::Succeeded
    );

    let published = meili::get_publication(
        &ctx,
        GetPublication {
            source_type: SourceType::Note,
            source_id: source,
            source_version: None,
        },
    )
    .await
    .unwrap()
    .expect("publication exists");
    assert_eq!(published.status, PublicationStatus::Published);
    assert!(published.meili_task_uid.is_some());

    // A word from the middle of the sentence, not its prefix.
    let hits = engine.search("知识管理").await;
    assert_eq!(hits["hits"].as_array().unwrap().len(), 1, "{hits}");
    assert_eq!(hits["hits"][0]["id"], json!(published.document_key));

    // An English word is searchable in the same index.
    let english = engine.search("meeting").await;
    assert_eq!(english["hits"].as_array().unwrap().len(), 1, "{english}");

    // Filters and sorting are real engine behaviour, not only settings values.
    let filtered = engine
        .post(
            &format!("indexes/{}/search", engine.index),
            json!({
                "q": "知识管理",
                "filter": "source_type = note AND tags = 会议 AND archived = false \
                           AND deleted = false AND source_version = 1",
                "sort": ["updated_at:desc"],
                "attributesToRetrieve": ["id"],
            }),
        )
        .await;
    assert_eq!(filtered["hits"].as_array().unwrap().len(), 1, "{filtered}");

    let settings = engine
        .get(&format!("indexes/{}/settings", engine.index))
        .await;
    assert_eq!(settings["searchableAttributes"], json!(["title", "body"]));
    for field in [
        "source_type",
        "source_id",
        "source_version",
        "project_id",
        "tags",
        "archived",
        "deleted",
        "path",
        "format",
    ] {
        assert!(
            settings["filterableAttributes"]
                .as_array()
                .unwrap()
                .contains(&json!(field)),
            "missing filterable field {field}"
        );
    }
    for field in ["updated_at", "version"] {
        assert!(
            settings["sortableAttributes"]
                .as_array()
                .unwrap()
                .contains(&json!(field)),
            "missing sortable field {field}"
        );
    }

    engine.drop_index().await;
}

#[tokio::test]
async fn t53_2_accepted_engine_task_is_not_published() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let index = "knoted_t53_stub";
    Mock::given(method("GET"))
        .and(path(format!("/indexes/{index}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": index, "primaryKey": "id"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/indexes/{index}/settings")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "searchableAttributes": ["title", "body"],
            "filterableAttributes": ["source_type", "source_id", "source_version",
                "project_id", "tags", "archived", "deleted", "path", "format"],
            "sortableAttributes": ["updated_at", "version"]
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/indexes/{index}/documents")))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "taskUid": 7, "indexUid": index, "status": "enqueued",
            "type": "documentAdditionOrUpdate"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": 7, "indexUid": index, "status": "enqueued",
            "type": "documentAdditionOrUpdate"
        })))
        .mount(&server)
        .await;

    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_search(SearchConfig {
        enabled: true,
        base_url: server.uri(),
        index_uid: index.into(),
        api_key_credential: None,
    });
    let source = Uuid::new_v4();
    let publication = meili::publish_fulltext(&ctx, document(source, 1, "会议纪要正文"))
        .await
        .unwrap();

    let (lease, job) = claim_publication(&ctx, publication.id).await;
    assert!(matches!(
        execute(&ctx, &lease, &job).await,
        ExecutionOutcome::TemporaryFailure { .. }
    ));
    let pending = meili::get_publication(
        &ctx,
        GetPublication {
            source_type: SourceType::Note,
            source_id: source,
            source_version: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(pending.status, PublicationStatus::Submitted);
    assert_eq!(pending.meili_task_uid, Some(7));
    assert!(
        pending.published_at.is_none(),
        "an accepted task must not be recorded as published"
    );

    // The same task now reports success: the retry only polls, it does not
    // submit a second document.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(format!("/indexes/{index}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": index, "primaryKey": "id"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/indexes/{index}/settings")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "searchableAttributes": ["title", "body"],
            "filterableAttributes": ["source_type", "source_id", "source_version",
                "project_id", "tags", "archived", "deleted", "path", "format"],
            "sortableAttributes": ["updated_at", "version"]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": 7, "indexUid": index, "status": "succeeded",
            "type": "documentAdditionOrUpdate"
        })))
        .mount(&server)
        .await;
    knoted_core::jobs::service::finish_execution(
        &ctx,
        knoted_core::jobs::FinishExecution {
            job_id: lease.job_id,
            lease_token: lease.token,
            outcome: ExecutionOutcome::TemporaryFailure {
                message: "task not confirmed".into(),
            },
            cancel_confirmed: false,
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE jobs SET available_at=now() WHERE id=$1")
        .bind(lease.job_id)
        .execute(ctx.pool())
        .await
        .unwrap();
    let (lease, job) = claim_publication(&ctx, publication.id).await;
    assert_eq!(
        execute(&ctx, &lease, &job).await,
        ExecutionOutcome::Succeeded
    );
    let published = meili::get_publication(
        &ctx,
        GetPublication {
            source_type: SourceType::Note,
            source_id: source,
            source_version: Some(1),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(published.status, PublicationStatus::Published);
    assert_eq!(published.meili_task_uid, Some(7));
    assert_eq!(
        published.document_key,
        meili::document_key(SourceType::Note, source, 1)
    );
}

#[tokio::test]
async fn t53_3_retry_does_not_create_a_duplicate_visible_item() {
    let engine = Engine::new();
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_search(engine.config());
    let source = Uuid::new_v4();

    let publication =
        meili::publish_fulltext(&ctx, document(source, 1, "重试不应产生重复的会议纪要"))
            .await
            .unwrap();
    // Publishing the same source/version again is idempotent: the same row and
    // the same job, not a second one.
    let same = meili::publish_fulltext(&ctx, document(source, 1, "重试不应产生重复的会议纪要"))
        .await
        .unwrap();
    assert_eq!(same.id, publication.id);
    let job_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind='fulltext.publish'")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    assert_eq!(job_count, 1);

    let (lease, job) = claim_publication(&ctx, publication.id).await;
    assert_eq!(
        execute(&ctx, &lease, &job).await,
        ExecutionOutcome::Succeeded
    );

    // The engine accepted the document but the acknowledgement was lost: the
    // ledger is back to "unknown task" and the same job is retried with the same
    // document key.
    sqlx::query(
        "UPDATE fulltext_publications SET status='pending', meili_task_uid=NULL, \
         published_at=NULL WHERE id=$1",
    )
    .bind(publication.id)
    .execute(ctx.pool())
    .await
    .unwrap();
    knoted_core::jobs::service::finish_execution(
        &ctx,
        knoted_core::jobs::FinishExecution {
            job_id: lease.job_id,
            lease_token: lease.token,
            outcome: ExecutionOutcome::TemporaryFailure {
                message: "submit response lost".into(),
            },
            cancel_confirmed: false,
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE jobs SET available_at=now() WHERE id=$1")
        .bind(lease.job_id)
        .execute(ctx.pool())
        .await
        .unwrap();

    let (lease, job) = claim_publication(&ctx, publication.id).await;
    assert_eq!(
        execute(&ctx, &lease, &job).await,
        ExecutionOutcome::Succeeded
    );

    assert_eq!(engine.document_count().await, 1);
    let hits = engine.search("重复").await;
    assert_eq!(hits["hits"].as_array().unwrap().len(), 1, "{hits}");
    assert_eq!(hits["hits"][0]["id"], json!(publication.document_key));

    engine.drop_index().await;
}

#[tokio::test]
async fn t53_4_lost_lease_cannot_mark_a_document_published() {
    let engine = Engine::new();
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_search(engine.config());
    let source = Uuid::new_v4();
    let publication = meili::publish_fulltext(&ctx, document(source, 1, "租约丢失的会议纪要"))
        .await
        .unwrap();
    let (lease, job) = claim_publication(&ctx, publication.id).await;

    // A takeover: another worker now owns the job, so this attempt's writes
    // must be rejected even though the engine already accepted the document.
    sqlx::query("UPDATE jobs SET lease_token=gen_random_uuid() WHERE id=$1")
        .bind(lease.job_id)
        .execute(ctx.pool())
        .await
        .unwrap();
    assert_eq!(
        execute(&ctx, &lease, &job).await,
        ExecutionOutcome::Cancelled
    );

    let state: (String, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as("SELECT status, published_at FROM fulltext_publications WHERE id=$1")
            .bind(publication.id)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    assert_ne!(state.0, "published");
    assert!(state.1.is_none());

    engine.drop_index().await;
}

#[tokio::test]
async fn t53_5_invalid_document_writes_nothing() {
    let engine = Engine::new();
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_search(engine.config());
    let mut invalid = document(Uuid::new_v4(), 1, "正文");
    invalid.title = "   ".into();

    let error = meili::publish_fulltext(&ctx, invalid).await.unwrap_err();
    assert!(matches!(error, meili::DomainError::Invalid(_)), "{error}");

    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM source_documents), \
         (SELECT count(*) FROM fulltext_publications), \
         (SELECT count(*) FROM jobs), (SELECT count(*) FROM outbox)",
    )
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    assert_eq!(counts, (0, 0, 0, 0));

    engine.drop_index().await;
}

#[tokio::test]
async fn t53_6_cleanup_removes_the_superseded_version() {
    let engine = Engine::new();
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_search(engine.config());
    let source = Uuid::new_v4();

    let first = meili::publish_fulltext(&ctx, document(source, 1, "第一版会议纪要"))
        .await
        .unwrap();
    let (lease, job) = claim_publication(&ctx, first.id).await;
    assert_eq!(
        execute(&ctx, &lease, &job).await,
        ExecutionOutcome::Succeeded
    );

    let second = meili::publish_fulltext(&ctx, document(source, 2, "第二版会议纪要"))
        .await
        .unwrap();
    assert_ne!(second.document_key, first.document_key);
    let (lease, job) = claim_publication(&ctx, second.id).await;
    assert_eq!(
        execute(&ctx, &lease, &job).await,
        ExecutionOutcome::Succeeded
    );

    // Both versions are visible until the background cleanup runs; visibility
    // never depends on the deletion being fast, it depends on the read-time
    // version check that T54 adds.
    assert_eq!(engine.document_count().await, 2);
    let superseded: String =
        sqlx::query_scalar("SELECT status FROM fulltext_publications WHERE id=$1")
            .bind(first.id)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    assert_eq!(superseded, "superseded");

    let (lease, job) = claim_publication_cleanup(&ctx).await;
    assert_eq!(
        execute(&ctx, &lease, &job).await,
        ExecutionOutcome::Succeeded
    );
    assert_eq!(engine.document_count().await, 1);
    let hits = engine.search("第二版").await;
    assert_eq!(hits["hits"][0]["id"], json!(second.document_key));

    let old_documents: i64 =
        sqlx::query_scalar("SELECT count(*) FROM source_documents WHERE source_id=$1")
            .bind(source)
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    assert_eq!(old_documents, 1, "the superseded projection is cleaned up");

    engine.drop_index().await;
}

async fn claim_publication_cleanup(ctx: &AppContext) -> (jobs::Lease, Job) {
    let job_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE kind='fulltext.cleanup' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    claim_job(ctx, job_id).await
}
