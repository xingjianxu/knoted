use knoted_core::{
    files::text::{ExtractedBlock, ExtractedDocument},
    rag::chunking::{BgeTokenizer, ChunkConfig, ChunkSource, chunk_document},
    search::meili::SourceType,
};
use serde_json::json;
use std::sync::OnceLock;
use uuid::Uuid;

fn tokenizer() -> &'static BgeTokenizer {
    static TOKENIZER: OnceLock<BgeTokenizer> = OnceLock::new();
    TOKENIZER.get_or_init(|| BgeTokenizer::from_env().expect("provision the BGE-M3 tokenizer"))
}

fn source() -> ChunkSource {
    ChunkSource {
        source_type: SourceType::File,
        source_id: Uuid::nil(),
        source_version: 7,
        extraction_id: Uuid::from_u128(3),
    }
}

fn document(blocks: Vec<ExtractedBlock>) -> ExtractedDocument {
    ExtractedDocument {
        blocks,
        source_format: "fixture".into(),
        title: None,
        page_count: None,
        truncated: false,
    }
}

fn block(
    ordinal: usize,
    text: &str,
    page: Option<u32>,
    structure: serde_json::Value,
) -> ExtractedBlock {
    ExtractedBlock {
        ordinal,
        kind: "paragraph".into(),
        text: text.into(),
        page,
        bbox: None,
        structure,
    }
}

#[test]
fn t57_1_splits_long_chinese_on_original_token_boundaries() {
    let paragraph = "深入研究中文检索与知识管理，找出不同段落之间的关联。".repeat(160);
    let doc = document(vec![block(0, &paragraph, Some(1), json!(null))]);
    let set = chunk_document(&doc, source(), &ChunkConfig::default(), tokenizer()).unwrap();
    assert!(set.chunks.len() > 2);
    let mut previous_end = 0;
    for chunk in set.chunks {
        assert!(chunk.token_count <= 768);
        assert_eq!(chunk.token_count, tokenizer().count(&chunk.text).unwrap());
        let range = &chunk.locations[0];
        assert_eq!(chunk.text, paragraph[range.byte_start..range.byte_end]);
        if previous_end > 0 {
            assert!(range.byte_start < previous_end && previous_end < range.byte_end);
        }
        previous_end = range.byte_end;
    }
    assert_eq!(previous_end, paragraph.len());
}

#[test]
fn t57_2_ids_are_stable_and_revision_scoped() {
    let config = ChunkConfig::default();
    let doc = document(vec![block(0, "会议记录", None, json!(null))]);
    let first = chunk_document(&doc, source(), &config, tokenizer()).unwrap();
    let second = chunk_document(&doc, source(), &config, tokenizer()).unwrap();
    assert_eq!(first, second);
    let revised = ChunkConfig {
        revision: "t57-alternative".into(),
    };
    let rebuilt = chunk_document(&doc, source(), &revised, tokenizer()).unwrap();
    assert_ne!(first.id, rebuilt.id);
    assert_ne!(first.chunks[0].id, rebuilt.chunks[0].id);
}

#[test]
fn t57_3_page_and_worksheet_locations_remain_distinct() {
    let doc = document(vec![
        block(0, "第一页", Some(1), json!(null)),
        block(1, "第二页", Some(2), json!(null)),
        block(2, "工作表甲", None, json!(r#"{"sheet":"甲","cell":"A4"}"#)),
        block(3, "工作表乙", None, json!(r#"{"sheet":"乙","cell":"B7"}"#)),
    ]);
    let set = chunk_document(&doc, source(), &ChunkConfig::default(), tokenizer()).unwrap();
    assert_eq!(set.chunks.len(), 4);
    assert_eq!(set.chunks[0].locations[0].page, Some(1));
    assert_eq!(set.chunks[1].locations[0].page, Some(2));
    assert_eq!(set.chunks[2].locations[0].structure["sheet"], "甲");
    assert_eq!(set.chunks[2].locations[0].structure["cell"], "A4");
    assert_eq!(set.chunks[3].locations[0].structure["sheet"], "乙");
}

#[tokio::test]
async fn t57_4_published_lexical_hits_keep_the_persisted_chunk_identity() {
    use knoted_core::{
        config::SearchConfig,
        context::AppContext,
        db::testing::TestDb,
        jobs::{self, ExecutionOutcome, JobOutcome, JobState},
        notes::service::{CreateNote, DeleteNote, create, delete},
        search::chunks,
    };

    let db = TestDb::create().await.unwrap();
    let base_url = std::env::var("KNOTED_TEST_MEILI_URL")
        .expect("set KNOTED_TEST_MEILI_URL or run tests via tools/test/rust.sh");
    let ctx = AppContext::new(db.pool.clone()).with_search(SearchConfig {
        rag_chunk_index_uid: format!("knoted_t57_{}", Uuid::new_v4().simple()),
        base_url: base_url.clone(),
        enabled: true,
        ..SearchConfig::default()
    });
    let note = create(
        &ctx,
        CreateNote {
            title: "研究记录".into(),
            parent_id: None,
            document_json: json!({"blocks":[{"id":"note-block","type":"paragraph",
            "content":[{"type":"text","text":"这篇文章详细讨论了知识管理与中文检索。"}]}]})
            .to_string(),
        },
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO source_documents(source_type, source_id, source_version, title, body, updated_at) VALUES ('chat_message',$1,1,'聊天','知识管理',now())")
        .bind(Uuid::new_v4()).execute(ctx.pool()).await.unwrap();
    assert_eq!(chunks::enqueue_missing(&ctx).await.unwrap(), 1);
    assert_eq!(chunks::enqueue_missing(&ctx).await.unwrap(), 0);
    for kind in [chunks::BUILD_JOB_KIND, chunks::PUBLISH_JOB_KIND] {
        let lease = jobs::claim(ctx.pool(), "t57").await.unwrap().unwrap();
        let job = jobs::load_claimed(ctx.pool(), &lease)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.kind, kind);
        assert_eq!(
            chunks::execute(&ctx, &lease, &job).await,
            ExecutionOutcome::Succeeded
        );
        assert!(
            jobs::finish_with_token(
                ctx.pool(),
                &lease,
                JobOutcome::Succeeded,
                &[JobState::Running]
            )
            .await
            .unwrap()
        );
    }

    let (chunk_set_id,): (Uuid,) = sqlx::query_as(
        "SELECT chunk_set_id FROM rag_chunk_publications WHERE status='published' AND published_at IS NOT NULL AND last_confirmed_meili_task_uid IS NOT NULL",
    ).fetch_one(ctx.pool()).await.unwrap();
    let set = chunks::load_chunk_set(&ctx, chunk_set_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(set.source.source_id, note.id);
    assert_eq!(set.source.source_version, note.content_version);
    assert_eq!(
        set.chunks[0].locations[0].structure["block_id"],
        "note-block"
    );
    let index = chunks::chunk_index_uid(&ctx);
    let http = reqwest::Client::new();
    let matches: serde_json::Value = http
        .post(format!("{base_url}/indexes/{index}/search"))
        .json(&json!({"q":"知识管理"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let hits = matches["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["chunk_id"], set.chunks[0].id.to_string());
    assert_eq!(hits[0]["chunk_set_id"], set.id.to_string());
    assert_eq!(hits[0]["source_version"], set.source.source_version);
    delete(
        &ctx,
        DeleteNote {
            id: note.id,
            expected_version: note.content_version,
        },
    )
    .await
    .unwrap();
    assert!(!chunks::chunk_set_is_current(&ctx, &set).await.unwrap());
    let _ = http
        .delete(format!("{base_url}/indexes/{index}"))
        .send()
        .await;
}

#[test]
fn t57_5_chunks_share_up_to_64_tokens_without_crossing_pages() {
    let paragraph = "研究资料和实施记录。".repeat(100);
    let doc = document(vec![
        block(0, &paragraph, Some(1), json!(null)),
        block(1, "实施测试结束。", Some(1), json!(null)),
        block(2, "第二页附件。", Some(2), json!(null)),
    ]);
    let set = chunk_document(&doc, source(), &ChunkConfig::default(), tokenizer()).unwrap();
    assert_eq!(set.chunks.len(), 3);
    let overlap = &set.chunks[1].locations[0];
    assert_eq!(overlap.block_ordinal, 0);
    assert_eq!(overlap.byte_end, paragraph.len());
    assert!((1..=64).contains(&tokenizer().count(&paragraph[overlap.byte_start..]).unwrap()));
    assert!(set.chunks[1].text.ends_with("\n实施测试结束。"));
    assert_eq!(set.chunks[2].locations.len(), 1);
    assert_eq!(set.chunks[2].locations[0].page, Some(2));
}

#[tokio::test]
async fn t57_6_reobservation_only_revokes_visibility_until_digest_is_confirmed() {
    use knoted_core::{
        config::SearchConfig,
        context::AppContext,
        db::testing::TestDb,
        jobs::{self, ExecutionOutcome},
        search::chunks,
    };

    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_search(SearchConfig {
        enabled: true,
        ..SearchConfig::default()
    });
    let file_id = Uuid::new_v4();
    let root_id = Uuid::new_v4();
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES ($1,'file')")
        .bind(file_id)
        .execute(ctx.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO file_roots(id,configured_path) VALUES ($1,'/fixture')")
        .bind(root_id)
        .execute(ctx.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO file_sources(id,root_id,relative_path,device,inode,size,mtime_ns,ctime_ns,nlink,
             sha256,content_version,version,status,observation,content_fresh,last_seen_at)
         VALUES ($1,$2,'source.pdf',1,2,3,4,5,1,repeat('a',64),7,1,'present','{}',true,now())",
    ).bind(file_id).bind(root_id).execute(ctx.pool()).await.unwrap();
    let extraction_job: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs(kind,payload,available_at) VALUES ('file.extract','{}',now()+interval '1 day') RETURNING id",
    ).fetch_one(ctx.pool()).await.unwrap();
    sqlx::query(
        "INSERT INTO file_extractions(source_id,content_version,input_sha256,parser_revision,options_hash,job_id,blocks)
         VALUES ($1,7,repeat('a',64),'fixture',repeat('b',64),$2,$3)",
    ).bind(file_id).bind(extraction_job)
     .bind(sqlx::types::Json(vec![block(0, "用于确认摘要的中文材料。", Some(5), json!(null))]))
     .execute(ctx.pool()).await.unwrap();

    assert_eq!(chunks::enqueue_missing(&ctx).await.unwrap(), 1);
    sqlx::query("UPDATE file_sources SET content_fresh=false,version=version+1 WHERE id=$1")
        .bind(file_id)
        .execute(ctx.pool())
        .await
        .unwrap();
    let lease = jobs::claim(ctx.pool(), "t57").await.unwrap().unwrap();
    let job = jobs::load_claimed(ctx.pool(), &lease)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.kind, chunks::BUILD_JOB_KIND);
    assert_eq!(
        chunks::execute(&ctx, &lease, &job).await,
        ExecutionOutcome::Succeeded
    );
    let set_id: Uuid = sqlx::query_scalar("SELECT id FROM rag_chunk_sets")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    let set = chunks::load_chunk_set(&ctx, set_id).await.unwrap().unwrap();
    assert_eq!(set.chunks[0].locations[0].page, Some(5));
    assert!(!chunks::chunk_set_is_current(&ctx, &set).await.unwrap());

    sqlx::query("UPDATE file_sources SET content_fresh=true WHERE id=$1")
        .bind(file_id)
        .execute(ctx.pool())
        .await
        .unwrap();
    assert!(chunks::chunk_set_is_current(&ctx, &set).await.unwrap());
}

#[test]
fn t57_7_keeps_a_worksheet_row_together_beyond_the_target() {
    let text = "研究资料和实施记录。".repeat(100);
    let doc = document(vec![
        block(0, &text, None, json!({"sheet":"甲","cell":"A1"})),
        block(1, "测试结论", None, json!({"sheet":"甲","cell":"B1"})),
        block(2, "下一条记录", None, json!({"sheet":"甲","cell":"A2"})),
    ]);
    let set = chunk_document(&doc, source(), &ChunkConfig::default(), tokenizer()).unwrap();
    let first = &set.chunks[0];
    assert_eq!(first.locations[0].structure["cell"], "A1");
    assert_eq!(first.locations[1].structure["cell"], "B1");
    assert!(first.text.ends_with("\n测试结论"));
    assert!(!first.text.contains("下一条记录"));
}

#[tokio::test]
async fn t57_8_document_index_changes_do_not_rebuild_the_lexical_generation() {
    use knoted_core::{config::SearchConfig, context::AppContext, search::chunks::chunk_index_uid};
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused_test")
        .unwrap();
    let first = AppContext::new(pool.clone()).with_search(SearchConfig {
        index_uid: "documents_a".into(),
        ..SearchConfig::default()
    });
    let next = AppContext::new(pool).with_search(SearchConfig {
        index_uid: "documents_b".into(),
        ..SearchConfig::default()
    });
    assert_eq!(chunk_index_uid(&first), "knoted_rag_chunks");
    assert_eq!(chunk_index_uid(&first), chunk_index_uid(&next));
}
