use knoted_core::{context::AppContext, db::testing::TestDb, maintenance::verify_quiescence};

use knoted_core::config::SearchConfig;
use knoted_core::maintenance::QuiescenceError;
use knoted_core::search::meili::MeiliClient;
use serde_json::json;
use std::time::Duration;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn expired_lease_does_not_hide_unresolved_external_submission() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    verify_quiescence(&ctx).await.unwrap();
    sqlx::query("INSERT INTO jobs(kind,state,lease_token,locked_by,lease_expires_at) VALUES ('test','running',gen_random_uuid(),'old', now()-interval '1 second')")
        .execute(ctx.pool()).await.unwrap();
    verify_quiescence(&ctx).await.unwrap();
    sqlx::query("INSERT INTO maintenance_external_submissions(id,index_uid) VALUES (gen_random_uuid(),'old_generation')")
        .execute(ctx.pool()).await.unwrap();
    assert!(verify_quiescence(&ctx).await.is_err());
    db.drop().await;
}

fn task_page() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "results":[],"total":0,"limit":1,"next":null,"from":null
    }))
}

async fn visible_index(server: &MockServer, uid: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/indexes/{uid}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"uid":uid})))
        .mount(server)
        .await;
}

fn absent_index(uid: &str) -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_json(json!({
        "code":"index_not_found", "type":"invalid_request",
        "message":format!("Index `{uid}` not found."),
        "link":"https://docs.meilisearch.com/errors#index_not_found"
    }))
}

#[tokio::test]
async fn canceled_document_submission_blocks_even_after_empty_queue_response() {
    let db = TestDb::create().await.unwrap();
    let server = MockServer::start().await;
    let ctx = AppContext::new(db.pool.clone()).with_search(SearchConfig {
        enabled: true,
        base_url: server.uri(),
        ..SearchConfig::default()
    });
    visible_index(&server, &ctx.search().index_uid).await;
    Mock::given(method("GET"))
        .and(path("/tasks"))
        .respond_with(task_page())
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/indexes/previous/documents"))
        .respond_with(
            ResponseTemplate::new(202)
                .set_delay(Duration::from_secs(8))
                .set_body_json(json!({"taskUid":3})),
        )
        .mount(&server)
        .await;
    let client = MeiliClient::from_context(&ctx).unwrap();
    verify_quiescence(&ctx).await.unwrap();
    let submit =
        tokio::spawn(async move { client.submit_document("previous", json!({"id":1})).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.method.as_str() == "PUT")
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    submit.abort();
    assert!(submit.await.unwrap_err().is_cancelled());
    assert!(matches!(
        verify_quiescence(&ctx).await,
        Err(QuiescenceError::Submission)
    ));
    db.drop().await;
}

#[tokio::test]
async fn checks_each_historic_generation_after_definite_task_receipt() {
    let db = TestDb::create().await.unwrap();
    let server = MockServer::start().await;
    let ctx = AppContext::new(db.pool.clone()).with_search(SearchConfig {
        enabled: true,
        base_url: server.uri(),
        ..SearchConfig::default()
    });
    Mock::given(method("PUT"))
        .and(path("/indexes/previous/documents"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({"taskUid":0})))
        .mount(&server)
        .await;
    let client = MeiliClient::from_context(&ctx).unwrap();
    assert_eq!(
        client
            .submit_document("previous", json!({"id":1}))
            .await
            .unwrap(),
        0
    );
    visible_index(&server, "previous").await;
    {
        let uid = &ctx.search().index_uid;
        visible_index(&server, uid).await;
        Mock::given(method("GET"))
            .and(path("/tasks"))
            .and(query_param("indexUids", uid.as_str()))
            .and(query_param("statuses", "enqueued,processing"))
            .and(query_param("limit", "1"))
            .respond_with(task_page())
            .mount(&server)
            .await;
    }
    for (i, reply) in [
        ResponseTemplate::new(401),
        ResponseTemplate::new(200).set_body_json(json!({"results":[]})),
        ResponseTemplate::new(200)
            .set_body_json(json!({"results":[{"uid":0}],"total":1,"limit":1,"next":null,"from":0})),
        task_page(),
    ]
    .into_iter()
    .enumerate()
    {
        let _guard = Mock::given(method("GET"))
            .and(path("/tasks"))
            .and(query_param("indexUids", "previous"))
            .respond_with(reply)
            .mount_as_scoped(&server)
            .await;
        let result = verify_quiescence(&ctx).await;
        if i == 3 {
            result.unwrap();
        } else {
            assert!(matches!(result, Err(QuiescenceError::Search)));
        }
    }
    db.drop().await;
}

#[tokio::test]
async fn empty_task_page_requires_authorized_visibility_for_each_index() {
    let db = TestDb::create().await.unwrap();
    let server = MockServer::start().await;
    let ctx = AppContext::new(db.pool.clone()).with_search(SearchConfig {
        base_url: server.uri(),
        enabled: true,
        ..Default::default()
    });
    let uid = &ctx.search().index_uid;
    Mock::given(method("GET"))
        .and(path("/tasks"))
        .respond_with(task_page())
        .mount(&server)
        .await;
    for denied in [
        ResponseTemplate::new(403),
        ResponseTemplate::new(200).set_body_json(json!({"uid":"other"})),
        ResponseTemplate::new(404).set_body_json(json!({"code":"index_not_found"})),
        absent_index("other"),
    ] {
        let _guard = Mock::given(method("GET"))
            .and(path(format!("/indexes/{uid}")))
            .respond_with(denied)
            .mount_as_scoped(&server)
            .await;
        assert!(matches!(
            verify_quiescence(&ctx).await,
            Err(QuiescenceError::Search)
        ));
    }
    Mock::given(method("GET"))
        .and(path(format!("/indexes/{uid}")))
        .respond_with(absent_index(uid))
        .mount(&server)
        .await;
    verify_quiescence(&ctx).await.unwrap();
    db.drop().await;
}

#[tokio::test]
async fn disabled_search_probes_recorded_index_with_configured_credential() {
    use wiremock::matchers::header_exists;
    let db = TestDb::create().await.unwrap();
    let server = MockServer::start().await;
    let config = SearchConfig {
        base_url: server.uri(),
        enabled: false,
        ..Default::default()
    };
    let ctx = AppContext::new(db.pool.clone())
        .with_search(config)
        .with_search_api_key(Some(uuid::Uuid::new_v4().to_string()));
    sqlx::query(
        "INSERT INTO maintenance_external_targets (origin,index_uid) VALUES ($1,'previous')",
    )
    .bind(server.uri())
    .execute(ctx.pool())
    .await
    .unwrap();
    assert!(MeiliClient::from_context(&ctx).is_err());
    Mock::given(method("GET"))
        .and(path("/indexes/previous"))
        .and(header_exists("authorization"))
        .respond_with(absent_index("previous"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks"))
        .and(query_param("indexUids", "previous"))
        .and(header_exists("authorization"))
        .respond_with(task_page())
        .expect(1)
        .mount(&server)
        .await;
    verify_quiescence(&ctx).await.unwrap();
    db.drop().await;
}
