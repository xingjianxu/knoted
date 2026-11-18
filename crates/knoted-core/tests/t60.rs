use knoted_core::{
    config::SearchConfig,
    context::AppContext,
    db::testing::TestDb,
    notes::service::{CreateNote, create},
    rag::{
        chunking::CHUNKER_REVISION,
        embedding::{DIMENSIONS, Embedding, EmbeddingSet, MODEL, MODEL_REVISION},
        retrieval::{RetrievalQuery, Scope, retrieve},
        vector::publish_vectors,
    },
    search::{chunks::chunk_index_uid, meili::SourceType},
};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn unit_vector() -> Vec<f32> {
    let mut vector = vec![0.0; DIMENSIONS];
    vector[0] = 1.0;
    vector
}

fn request(embedding: bool) -> RetrievalQuery {
    RetrievalQuery {
        text: "knowledge".into(),
        embedding: embedding.then(unit_vector),
        scope: Scope::default(),
    }
}

async fn context(server: &MockServer) -> (TestDb, AppContext) {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_search(SearchConfig {
        enabled: true,
        base_url: server.uri(),
        ..Default::default()
    });
    (db, ctx)
}

async fn fixture(ctx: &AppContext) -> (Uuid, Value) {
    let note=create(ctx,CreateNote {
        title:"fixture".into(), parent_id:None,
        document_json:r#"{"blocks":[{"id":"block","type":"paragraph","content":[{"type":"text","text":"knowledge fixture"}]}]}"#.into(),
    }).await.unwrap();
    let set_id = Uuid::new_v4();
    let chunk_id = Uuid::new_v4();
    sqlx::query("INSERT INTO rag_chunk_sets (id,source_type,source_id,source_version,extraction_id,chunker_revision)
        SELECT $1,source_type,source_id,source_version,extraction_id,$2
        FROM current_rag_sources WHERE source_id=$3")
        .bind(set_id).bind(CHUNKER_REVISION).bind(note.id).execute(ctx.pool()).await.unwrap();
    sqlx::query("INSERT INTO rag_chunks VALUES ($1,$2,0,'knowledge fixture',2,'[]')")
        .bind(chunk_id)
        .bind(set_id)
        .execute(ctx.pool())
        .await
        .unwrap();
    let mut tx = ctx.pool().begin().await.unwrap();
    let job = knoted_core::jobs::enqueue(&mut tx, knoted_core::jobs::JobSpec::new("test"))
        .await
        .unwrap()
        .job;
    sqlx::query(
        "INSERT INTO rag_chunk_publications (chunk_set_id,index_uid,job_id,status,published_at)
        VALUES ($1,$2,$3,'published',now())",
    )
    .bind(set_id)
    .bind(chunk_index_uid(ctx))
    .bind(job.id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    publish_vectors(
        ctx,
        &EmbeddingSet {
            chunk_set_id: set_id,
            model: MODEL.into(),
            model_revision: MODEL_REVISION.into(),
            rows: vec![Embedding {
                chunk_id,
                vector: unit_vector(),
            }],
        },
    )
    .await
    .unwrap();
    (
        note.id,
        json!({"chunk_id":chunk_id,"chunk_set_id":set_id,"source_version":note.content_version}),
    )
}

async fn lexical_matches(server: &MockServer, ctx: &AppContext, hits: Vec<Value>) {
    Mock::given(method("POST"))
        .and(path(format!("/indexes/{}/search", chunk_index_uid(ctx))))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"hits":hits})))
        .mount(server)
        .await;
}

#[tokio::test]
async fn t60_1_document_index_and_chat_cannot_supply_evidence() {
    let server = MockServer::start().await;
    let (_db, ctx) = context(&server).await;
    let (_id, hit) = fixture(&ctx).await;
    Mock::given(method("POST"))
        .and(path(format!("/indexes/{}/search", ctx.search().index_uid)))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"hits":[hit]})))
        .expect(0)
        .mount(&server)
        .await;
    lexical_matches(&server, &ctx, vec![]).await;
    assert!(
        retrieve(&ctx, request(false))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let mut chat = request(true);
    chat.scope.source_types = vec![SourceType::ChatMessage, SourceType::Project];
    assert!(retrieve(&ctx, chat).await.unwrap().items.is_empty());
}

#[tokio::test]
async fn t60_2_current_visibility_filters_both_paths() {
    let server = MockServer::start().await;
    let (_db, ctx) = context(&server).await;
    let (note, hit) = fixture(&ctx).await;
    lexical_matches(&server, &ctx, vec![hit]).await;
    let items = retrieve(&ctx, request(true)).await.unwrap().items;
    assert_eq!(items.len(), 1);
    assert!(items[0].score > 1.0 / 61.0);
    sqlx::query("UPDATE notes SET deleted_at=now() WHERE id=$1")
        .bind(note)
        .execute(ctx.pool())
        .await
        .unwrap();
    assert!(
        retrieve(&ctx, request(true))
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn t60_3_stable_ranking_and_scope_apply_to_both_paths() {
    let server = MockServer::start().await;
    let (_db, ctx) = context(&server).await;
    let (one, first) = fixture(&ctx).await;
    let (_, second) = fixture(&ctx).await;
    lexical_matches(&server, &ctx, vec![first, second]).await;
    let order = || async {
        retrieve(&ctx, request(true))
            .await
            .unwrap()
            .items
            .iter()
            .map(|c| c.chunk_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(order().await, order().await);
    assert_eq!(order().await.len(), 2);

    let mut scoped = request(true);
    scoped.scope.tag_ids = vec![Uuid::new_v4()];
    assert!(retrieve(&ctx, scoped).await.unwrap().items.is_empty());
    assert_eq!(
        retrieve(&ctx, request(false)).await.unwrap().items[0].source_id,
        one
    );
}
