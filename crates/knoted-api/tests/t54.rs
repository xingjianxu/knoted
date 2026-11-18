//! T54 contract tests against PostgreSQL and a real Meilisearch index.

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use knoted_api::build_router;
use knoted_core::{
    auth::{self, sessions},
    config::SearchConfig,
    context::AppContext,
    db::testing::TestDb,
    jobs::{self, ExecutionOutcome, Job},
    notes::service::{self as notes, CreateNote, DeleteNote, Note, UpdateNote},
    search::meili::{self, MeiliClient, SourceDocument, SourceType},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    db: TestDb,
    ctx: AppContext,
    app: Router,
    cookie: String,
}

impl Fixture {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let ctx = AppContext::new(db.pool.clone()).with_search(SearchConfig {
            enabled: true,
            base_url: std::env::var("KNOTED_TEST_MEILI_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:15605".into()),
            index_uid: format!("knoted_t54_{}", Uuid::new_v4().simple()),
            ..SearchConfig::default()
        });
        let mut tx = ctx.pool().begin().await.unwrap();
        let owner = auth::ensure_owner(&mut tx).await.unwrap();
        let session = sessions::create_session(&mut tx, owner.user.id, ctx.now(), None, true)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        Self {
            db,
            app: build_router(ctx.clone()),
            ctx,
            cookie: format!("knoted_session={}", session.token),
        }
    }

    async fn create_note(&self, title: &str) -> Note {
        notes::create(
            &self.ctx,
            CreateNote {
                title: title.into(),
                parent_id: None,
                document_json: json!({"blocks":[]}).to_string(),
            },
        )
        .await
        .unwrap()
    }

    async fn publish(&self, note: &Note, body: &str) {
        meili::publish_fulltext(
            &self.ctx,
            SourceDocument {
                source_type: SourceType::Note,
                source_id: note.id,
                source_version: note.content_version,
                title: note.title.clone(),
                body: body.into(),
                project_id: None,
                tags: Vec::new(),
                archived: false,
                deleted: false,
                path: None,
                format: None,
                updated_at: note.updated_at,
            },
        )
        .await
        .unwrap();
        self.run_publication().await;
    }

    async fn run_publication(&self) {
        let lease = jobs::claim(self.ctx.pool(), "t54").await.unwrap().unwrap();
        let job: Job = sqlx::query_as("SELECT * FROM jobs WHERE id=$1")
            .bind(lease.job_id)
            .fetch_one(self.ctx.pool())
            .await
            .unwrap();
        assert_eq!(
            meili::execute(&self.ctx, &lease, &job).await,
            ExecutionOutcome::Succeeded
        );
    }

    async fn search(&self, query: &str) -> (StatusCode, Value) {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/search?{query}"))
                    .header("cookie", &self.cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn finish(self) {
        let indexed_keys: Vec<String> = sqlx::query_scalar(
            "SELECT document_key FROM fulltext_publications WHERE status='published' OR status='superseded'",
        )
        .fetch_all(self.ctx.pool())
        .await
        .unwrap();
        if !indexed_keys.is_empty() {
            let client = MeiliClient::from_context(&self.ctx).unwrap();
            let task = client
                .delete_documents(&self.ctx.search().index_uid, &indexed_keys)
                .await
                .unwrap();
            assert_eq!(
                client.wait_task(task).await.unwrap(),
                meili::EngineTask::Succeeded
            );
        }
        self.db.drop().await;
    }
}

#[tokio::test]
async fn t54_1_soft_delete_hides_the_existing_engine_hit() {
    let fixture = Fixture::new().await;
    let note = fixture.create_note("Private draft").await;
    fixture.publish(&note, "searchable sentence").await;
    let before = fixture.search("q=searchable").await;
    notes::delete(
        &fixture.ctx,
        DeleteNote {
            id: note.id,
            expected_version: note.content_version,
        },
    )
    .await
    .unwrap();
    let after = fixture.search("q=searchable").await;
    knoted_core::search::service::project_pending(&fixture.ctx)
        .await
        .unwrap();
    let tombstone: Option<(bool, String)> = sqlx::query_as(
        "SELECT deleted, body FROM source_documents WHERE source_id=$1 AND superseded_at IS NULL",
    )
    .bind(note.id)
    .fetch_optional(fixture.ctx.pool())
    .await
    .unwrap();
    fixture.finish().await;
    assert_eq!(tombstone, Some((true, String::new())));

    assert_eq!(before.0, StatusCode::OK);
    assert_eq!(before.1["items"][0]["id"], note.id.to_string());
    assert_eq!(
        after,
        (StatusCode::OK, json!({"items":[],"next_cursor":null}))
    );
}

#[tokio::test]
async fn t54_2_cursor_is_bound_to_current_filters() {
    let fixture = Fixture::new().await;
    for title in ["First draft", "Second draft"] {
        let note = fixture.create_note(title).await;
        fixture.publish(&note, "searchable sentence").await;
    }
    let page = fixture.search("q=searchable&limit=1&type=note").await;
    let cursor = page.1["next_cursor"].as_str().unwrap_or("missing");
    let next = fixture
        .search(&format!("q=searchable&limit=1&type=note&cursor={cursor}"))
        .await;
    let mismatched = fixture
        .search(&format!("q=searchable&limit=1&type=file&cursor={cursor}"))
        .await;
    fixture.finish().await;

    assert_eq!(page.0, StatusCode::OK);
    assert_ne!(cursor, "missing");
    assert_eq!(next.0, StatusCode::OK);
    assert_ne!(page.1["items"][0]["id"], next.1["items"][0]["id"]);
    assert!(next.1["next_cursor"].is_null());
    assert_eq!(mismatched.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn t54_3_only_latest_fulltext_is_visible_without_vectors() {
    let fixture = Fixture::new().await;
    let note = fixture.create_note("First title").await;
    fixture.publish(&note, "obsolete").await;
    let current = notes::update(
        &fixture.ctx,
        UpdateNote {
            id: note.id,
            expected_version: note.content_version,
            title: Some("Second title".into()),
            document_json: None,
        },
    )
    .await
    .unwrap();
    let before = fixture.search("q=obsolete").await;
    fixture.publish(&current, "current search text").await;
    let after = fixture.search("q=current").await;
    fixture.finish().await;

    assert_eq!(
        before,
        (StatusCode::OK, json!({"items":[],"next_cursor":null}))
    );
    assert_eq!(after.0, StatusCode::OK);
    assert_eq!(after.1["items"][0]["title"], "Second title");
    assert_eq!(after.1["items"][0]["snippet"], "current search text");
}

#[tokio::test]
async fn t54_4_note_save_persists_latest_plain_text_projection_once() {
    use knoted_core::search::service::project_pending;

    let fixture = Fixture::new().await;
    let note = fixture.create_note("Draft").await;
    let current = notes::update(
        &fixture.ctx,
        UpdateNote {
            id: note.id,
            expected_version: note.content_version,
            title: Some("Reviewed".into()),
            document_json: Some(
                json!({"blocks":[{
                    "id":"b", "type":"paragraph",
                    "content":[{"type":"text", "text":"Current <b>raw text</b>"}]
                }]})
                .to_string(),
            ),
        },
    )
    .await
    .unwrap();
    let processed = project_pending(&fixture.ctx).await.unwrap();
    let duplicate = project_pending(&fixture.ctx).await.unwrap();
    let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT sd.source_version, sd.title, sd.body, j.state
         FROM source_documents sd JOIN fulltext_publications p ON p.document_id=sd.id
         JOIN jobs j ON p.job_id=j.id WHERE sd.source_id=$1",
    )
    .bind(current.id)
    .fetch_all(fixture.ctx.pool())
    .await
    .unwrap();
    fixture.finish().await;

    assert_eq!(processed, 2);
    assert_eq!(duplicate, 0);
    assert_eq!(
        rows,
        vec![(
            current.content_version,
            "Reviewed".into(),
            "Current <b>raw text</b>".into(),
            "queued".into()
        )]
    );
}

#[tokio::test]
async fn t54_5_repeated_tag_filters_follow_current_associations() {
    use knoted_core::links::service::{
        ChangeEntityTag, CreateTag, Entity, EntityType, assign_tag, create_tag, remove_tag,
    };

    let fixture = Fixture::new().await;
    let note = fixture.create_note("Tagged note").await;
    fixture.publish(&note, "independent source").await;
    let entity = Entity {
        entity_type: EntityType::Note,
        id: note.id,
    };
    let first = create_tag(&fixture.ctx, CreateTag { name: "one".into() })
        .await
        .unwrap();
    let second = create_tag(&fixture.ctx, CreateTag { name: "two".into() })
        .await
        .unwrap();
    for tag_id in [first.id, second.id] {
        assign_tag(&fixture.ctx, ChangeEntityTag { entity, tag_id })
            .await
            .unwrap();
    }
    let query = format!("q=independent&tags={}&tags={}", first.id, second.id);
    let before = fixture.search(&query).await;
    remove_tag(
        &fixture.ctx,
        ChangeEntityTag {
            entity,
            tag_id: first.id,
        },
    )
    .await
    .unwrap();
    let after = fixture.search(&query).await;
    fixture.finish().await;

    assert_eq!(before.0, StatusCode::OK);
    assert_eq!(before.1["items"][0]["id"], note.id.to_string());
    assert_eq!(
        after,
        (StatusCode::OK, json!({"items":[],"next_cursor":null}))
    );
}

#[tokio::test]
async fn t54_6_updated_sort_takes_precedence_over_relevance_within_one_second() {
    use chrono::{TimeZone, Utc};

    let fixture = Fixture::new().await;
    let mut old = fixture.create_note("needle").await;
    let mut new = fixture.create_note("Latest").await;
    old.updated_at = Utc.timestamp_opt(1_700_000_000, 100_000_000).unwrap();
    new.updated_at = Utc.timestamp_opt(1_700_000_000, 200_000_000).unwrap();
    for (note, body) in [(&old, "needle"), (&new, "needle amongst other words")] {
        sqlx::query("UPDATE notes SET updated_at=$2 WHERE id=$1")
            .bind(note.id)
            .bind(note.updated_at)
            .execute(fixture.ctx.pool())
            .await
            .unwrap();
        fixture.publish(note, body).await;
    }
    let descending = fixture.search("q=needle&sort=updated_desc").await;
    let ascending = fixture.search("q=needle&sort=updated_asc").await;
    fixture.finish().await;

    assert_eq!(descending.0, StatusCode::OK);
    assert_eq!(descending.1["items"][0]["id"], new.id.to_string());
    assert_eq!(ascending.0, StatusCode::OK);
    assert_eq!(ascending.1["items"][0]["id"], old.id.to_string());
}

#[tokio::test]
async fn t54_7_only_fresh_file_content_with_current_path_is_returned() {
    let fixture = Fixture::new().await;
    let pool = fixture.ctx.pool();
    let root: Uuid = sqlx::query_scalar(
        "INSERT INTO file_roots(id,configured_path) VALUES(gen_random_uuid(),'/unavailable') RETURNING id",
    ).fetch_one(pool).await.unwrap();
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO entity_registry(id,entity_type) VALUES(gen_random_uuid(),'file') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO file_sources(id,root_id,relative_path,device,inode,size,mtime_ns,ctime_ns,nlink,
                                  sha256,content_version,version,status,observation,last_seen_at,content_fresh)
         VALUES($1,$2,'archive/Quarter.pdf',1,2,3,4,5,1,repeat('a',64),1,1,'present','{}',now(),true)",
    ).bind(id).bind(root).execute(pool).await.unwrap();
    let job: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs(kind,state) VALUES('fixture.extract','succeeded') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let extraction: Uuid = sqlx::query_scalar(
        "INSERT INTO file_extractions(source_id,content_version,input_sha256,parser_revision,options_hash,job_id,blocks)
         VALUES($1,1,repeat('a',64),'fixture',repeat('b',64),$2,$3) RETURNING id",
    ).bind(id).bind(job).bind(json!([{"text":"quarterly projections"}])).fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO outbox(job_id,topic,payload) VALUES($1,'file.extracted',$2)")
        .bind(job)
        .bind(json!({"source_id":id, "source_version":1, "extraction_id":extraction}))
        .execute(pool)
        .await
        .unwrap();
    knoted_core::search::service::project_pending(&fixture.ctx)
        .await
        .unwrap();
    fixture.run_publication().await;

    let original = fixture
        .search("q=quarterly&path=archive&format=pdf&type=file")
        .await;
    sqlx::query("UPDATE file_sources SET relative_path='current/Quarter.pdf' WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    let old_path = fixture
        .search("q=quarterly&path=archive&format=pdf&type=file")
        .await;
    let new_path = fixture
        .search("q=quarterly&path=current&format=pdf&type=file")
        .await;
    sqlx::query("UPDATE file_sources SET content_fresh=false WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    let stale = fixture.search("q=quarterly&type=file").await;
    fixture.finish().await;

    let empty = (StatusCode::OK, json!({"items":[],"next_cursor":null}));
    assert_eq!(original.0, StatusCode::OK);
    assert_eq!(original.1["items"][0]["path"], "archive/Quarter.pdf");
    assert_eq!(old_path, empty);
    assert_eq!(new_path.0, StatusCode::OK);
    assert_eq!(new_path.1["items"][0]["path"], "current/Quarter.pdf");
    assert_eq!(stale, empty);
}
