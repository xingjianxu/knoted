use knoted_core::{
    context::AppContext,
    db::testing::TestDb,
    rag::{
        chunking::CHUNKER_REVISION,
        embedding::{DIMENSIONS, Embedding, EmbeddingSet, MODEL, MODEL_REVISION},
        vector::{SearchVectors, publish_vectors, search_vectors},
    },
    search::meili::SourceType,
};
use uuid::Uuid;

async fn fixture(ctx: &AppContext, revision: &str) -> (Uuid, EmbeddingSet) {
    let note = knoted_core::notes::service::create(
        ctx,
        knoted_core::notes::service::CreateNote {
            title: "T59 fixture".into(),
            parent_id: None,
            document_json: r#"{"blocks":[{"id":"t59","type":"paragraph","content":[{"type":"text","text":"fixture"}]}]}"#.into(),
        },
    )
    .await
    .unwrap()
    .id;
    let set_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO rag_chunk_sets (id,source_type,source_id,source_version,extraction_id,chunker_revision)
          SELECT $1,source_type,source_id,source_version,extraction_id,$2
          FROM current_rag_sources WHERE source_type='note' AND source_id=$3",
    )
    .bind(set_id)
    .bind(CHUNKER_REVISION)
    .bind(note)
    .execute(ctx.pool())
    .await
    .unwrap();
    let chunk = Uuid::new_v4();
    sqlx::query("INSERT INTO rag_chunks VALUES ($1,$2,0,'fixture',1,'[]')")
        .bind(chunk)
        .bind(set_id)
        .execute(ctx.pool())
        .await
        .unwrap();
    (
        note,
        EmbeddingSet {
            chunk_set_id: set_id,
            model: MODEL.into(),
            model_revision: revision.into(),
            rows: vec![Embedding {
                chunk_id: chunk,
                vector: unit_vector(),
            }],
        },
    )
}

fn unit_vector() -> Vec<f32> {
    let mut vector = vec![0.0; DIMENSIONS];
    vector[0] = 1.0;
    vector
}

fn query() -> SearchVectors {
    SearchVectors {
        model: MODEL.into(),
        model_revision: MODEL_REVISION.into(),
        query: unit_vector(),
        source_types: vec![SourceType::Note, SourceType::Task],
        project_ids: vec![],
        tag_ids: vec![],
        limit: 40,
    }
}

#[tokio::test]
async fn t59_1_source_change_immediately_revokes_vector_visibility() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let (note, set) = fixture(&ctx, MODEL_REVISION).await;
    publish_vectors(&ctx, &set).await.unwrap();
    assert_eq!(search_vectors(&ctx, query()).await.unwrap().len(), 1);
    sqlx::query("UPDATE notes SET content_version=content_version+1 WHERE id=$1")
        .bind(note)
        .execute(ctx.pool())
        .await
        .unwrap();
    assert!(search_vectors(&ctx, query()).await.unwrap().is_empty());
}

#[tokio::test]
async fn t59_2_filtering_returns_only_real_matching_current_vectors() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let (note, set) = fixture(&ctx, MODEL_REVISION).await;
    let (_, different_space) = fixture(&ctx, "another-revision").await;
    publish_vectors(&ctx, &set).await.unwrap();
    publish_vectors(&ctx, &different_space).await.unwrap();

    let results = search_vectors(&ctx, query()).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].source_id, note);
    assert_eq!(results[0].chunk_id, set.rows[0].chunk_id);
    assert_eq!(results[0].distance, 0.0);
    let results = search_vectors(
        &ctx,
        SearchVectors {
            source_types: vec![SourceType::Task],
            ..query()
        },
    )
    .await
    .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn t59_3_repeated_atomic_publication_reuses_the_embedding_set() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let (_, mut set) = fixture(&ctx, MODEL_REVISION).await;
    let first = publish_vectors(&ctx, &set).await.unwrap();
    assert_eq!(publish_vectors(&ctx, &set).await.unwrap(), first);
    set.rows[0].vector.pop();
    assert!(publish_vectors(&ctx, &set).await.is_err());
    let results = search_vectors(&ctx, query()).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].embedding_set_id, first);
}
