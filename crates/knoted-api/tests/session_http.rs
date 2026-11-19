//! Production-router regression for the session HTTP endpoints missing at T21.
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use knoted_api::build_router;
use knoted_core::auth::sessions::CreatedSession;
use knoted_core::auth::{self, sessions};
use knoted_core::config::AuthConfig;
use knoted_core::context::{AppContext, FixedClock};
use knoted_core::db::testing::TestDb;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

async fn setup() -> (TestDb, Router, CreatedSession, CreatedSession) {
    let db = TestDb::create().await.expect("real PostgreSQL test schema");
    let now = Utc::now();
    let mut tx = db.pool.begin().await.unwrap();
    let user = auth::ensure_owner(&mut tx).await.unwrap().user.id;
    tx.commit().await.unwrap();
    let mut connection = db.pool.acquire().await.unwrap();
    let first = sessions::create_session(&mut connection, user, now, None, true)
        .await
        .unwrap();
    let other = sessions::create_session(
        &mut connection,
        user,
        now - Duration::seconds(1),
        None,
        true,
    )
    .await
    .unwrap();
    drop(connection);
    let ctx = AppContext::new(db.pool.clone()).with_clock(Arc::new(FixedClock::new(now)));
    (db, build_router(ctx), first, other)
}
fn request(
    method: Method,
    path: &str,
    session: Option<&CreatedSession>,
    csrf_cookie: Option<&str>,
    write_guard: bool,
) -> Request<Body> {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(session) = session {
        let cookie = match csrf_cookie {
            Some(csrf) => format!("knoted_session={}; knoted_csrf={csrf}", session.token),
            None => format!("knoted_session={}", session.token),
        };
        req = req.header(header::COOKIE, cookie);
        if write_guard {
            req = req
                .header(header::ORIGIN, AuthConfig::default().origin)
                .header("X-CSRF-Token", csrf_cookie.unwrap_or(&session.csrf_token));
        }
    }
    req.body(Body::empty()).unwrap()
}
async fn body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}
#[tokio::test]
async fn session_http_anonymous_endpoints_are_protected_not_missing() {
    let (db, app, _, _) = setup().await;
    for (method, path) in [
        (Method::GET, "/api/v1/auth/session".to_owned()),
        (Method::POST, "/api/v1/auth/logout".to_owned()),
        (Method::GET, "/api/v1/auth/sessions".to_owned()),
        (
            Method::DELETE,
            format!("/api/v1/auth/sessions/{}", Uuid::new_v4()),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request(method, &path, None, None, false))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(body(response).await["code"], "unauthenticated");
    }
    db.drop().await;
}
#[tokio::test]
async fn session_http_read_returns_current_dto_and_reuses_valid_csrf() {
    let (db, app, current, _) = setup().await;
    let response = app
        .oneshot(request(
            Method::GET,
            "/api/v1/auth/session",
            Some(&current),
            Some(&current.csrf_token),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .next()
            .is_none()
    );
    let data = body(response).await;
    assert_eq!(data["session"]["id"], current.session.id.to_string());
    assert_eq!(data["session"]["current"], true);
    assert_eq!(data["csrf_token"], current.csrf_token);
    assert!(data["session"].get("csrf_token_hash").is_none());
    assert!(data["session"].get("token").is_none());
    db.drop().await;
}
#[tokio::test]
async fn session_http_missing_or_invalid_csrf_cookie_recovers_and_allows_logout() {
    for invalid in [None, Some("invalid-csrf-cookie")] {
        let (db, app, current, _) = setup().await;
        let response = app
            .clone()
            .oneshot(request(
                Method::GET,
                "/api/v1/auth/session",
                Some(&current),
                invalid,
                false,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let data = body(response).await;
        let csrf = data["csrf_token"].as_str().unwrap();
        assert!(csrf.len() >= 16);
        assert_ne!(csrf, current.csrf_token);
        assert!(cookie.starts_with(&format!("knoted_csrf={csrf};")));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(!cookie.contains("HttpOnly"));
        let response = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/v1/auth/logout",
                Some(&current),
                Some(csrf),
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        db.drop().await;
    }
}
#[tokio::test]
async fn session_http_logout_checks_origin_csrf_and_revokes_only_current() {
    let (db, app, current, other) = setup().await;
    for guarded in [false, true] {
        let mut req = request(
            Method::POST,
            "/api/v1/auth/logout",
            Some(&current),
            Some(&current.csrf_token),
            guarded,
        );
        if guarded {
            req.headers_mut()
                .insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        }
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    let response = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/auth/logout",
            Some(&current),
            Some(&current.csrf_token),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let cookies: Vec<_> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|h| h.to_str().unwrap())
        .collect();
    assert_eq!(cookies.len(), 2);
    assert!(
        cookies
            .iter()
            .all(|v| v.contains("Max-Age=0") && v.contains("Secure"))
    );
    assert_eq!(
        app.clone()
            .oneshot(request(
                Method::GET,
                "/api/v1/auth/session",
                Some(&current),
                None,
                false
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.oneshot(request(
            Method::GET,
            "/api/v1/auth/session",
            Some(&other),
            Some(&other.csrf_token),
            false
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::OK
    );
    db.drop().await;
}
#[tokio::test]
async fn session_http_list_uses_keyset_cursor_and_validates_queries() {
    let (db, app, current, other) = setup().await;
    let response = app
        .clone()
        .oneshot(request(
            Method::GET,
            "/api/v1/auth/sessions?limit=1",
            Some(&current),
            None,
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let first = body(response).await;
    assert_eq!(first["items"][0]["id"], current.session.id.to_string());
    assert_eq!(first["items"][0]["current"], true);
    let cursor = first["next_cursor"].as_str().expect("another page exists");
    let response = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!("/api/v1/auth/sessions?limit=1&cursor={cursor}"),
            Some(&current),
            None,
            false,
        ))
        .await
        .unwrap();
    let second = body(response).await;
    assert_eq!(second["items"][0]["id"], other.session.id.to_string());
    assert_eq!(second["items"][0]["current"], false);
    assert!(second["next_cursor"].is_null());
    for query in ["limit=0", "limit=101", "limit=abc", "cursor=garbage"] {
        let response = app
            .clone()
            .oneshot(request(
                Method::GET,
                &format!("/api/v1/auth/sessions?{query}"),
                Some(&current),
                None,
                false,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        assert!(body(response).await.get("request_id").is_some());
    }
    db.drop().await;
}
#[tokio::test]
async fn session_http_delete_checks_guards_missing_other_and_current() {
    let (db, app, current, other) = setup().await;
    let path = format!("/api/v1/auth/sessions/{}", other.session.id);
    assert_eq!(
        app.clone()
            .oneshot(request(Method::DELETE, &path, Some(&current), None, false))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.clone()
            .oneshot(request(
                Method::DELETE,
                &path,
                Some(&current),
                Some(&current.csrf_token),
                true
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.clone()
            .oneshot(request(
                Method::GET,
                "/api/v1/auth/session",
                Some(&other),
                None,
                false
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for id in [other.session.id, Uuid::new_v4()] {
        assert_eq!(
            app.clone()
                .oneshot(request(
                    Method::DELETE,
                    &format!("/api/v1/auth/sessions/{id}"),
                    Some(&current),
                    Some(&current.csrf_token),
                    true
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    let response = app
        .clone()
        .oneshot(request(
            Method::DELETE,
            &format!("/api/v1/auth/sessions/{}", current.session.id),
            Some(&current),
            Some(&current.csrf_token),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .count(),
        2
    );
    assert_eq!(
        app.oneshot(request(
            Method::GET,
            "/api/v1/auth/session",
            Some(&current),
            None,
            false
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::UNAUTHORIZED
    );
    db.drop().await;
}

#[tokio::test]
async fn session_http_maximum_page_keeps_the_101st_session_reachable() {
    let (db, app, current, _) = setup().await;
    let mut connection = db.pool.acquire().await.unwrap();
    for seconds in 2..101 {
        sessions::create_session(
            &mut connection,
            current.session.user_id,
            current.session.created_at - Duration::seconds(seconds),
            None,
            false,
        )
        .await
        .unwrap();
    }
    drop(connection);
    let response = app
        .clone()
        .oneshot(request(
            Method::GET,
            "/api/v1/auth/sessions?limit=100",
            Some(&current),
            None,
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let first = body(response).await;
    assert_eq!(first["items"].as_array().unwrap().len(), 100);
    let cursor = first["next_cursor"]
        .as_str()
        .expect("101st session must be reachable");
    let response = app
        .oneshot(request(
            Method::GET,
            &format!("/api/v1/auth/sessions?limit=100&cursor={cursor}"),
            Some(&current),
            None,
            false,
        ))
        .await
        .unwrap();
    let last = body(response).await;
    assert_eq!(last["items"].as_array().unwrap().len(), 1);
    assert!(last["next_cursor"].is_null());
    assert!(
        !first["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == last["items"][0]["id"])
    );
    db.drop().await;
}

#[tokio::test]
async fn t68_maintenance_allows_session_read_without_renewal_and_refuses_rotation() {
    let (db, app, _, current) = setup().await;
    let ctx = AppContext::new(db.pool.clone());
    let exclusive = knoted_core::maintenance::exclusive(
        ctx.admission_pool(),
        std::time::Duration::from_secs(2),
    )
    .await
    .unwrap();
    let before: (chrono::DateTime<Utc>, Vec<u8>) =
        sqlx::query_as("SELECT last_seen_at, csrf_token_hash FROM sessions WHERE id=$1")
            .bind(current.session.id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    for (method, path, cookie, expected) in [
        (
            Method::GET,
            "/api/v1/auth/session",
            Some(current.csrf_token.as_str()),
            StatusCode::OK,
        ),
        (
            Method::GET,
            "/api/v1/auth/session",
            None,
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (
            Method::POST,
            "/api/v1/auth/logout",
            Some(current.csrf_token.as_str()),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request(method, path, Some(&current), cookie, true))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::SERVICE_UNAVAILABLE {
            let data = body(response).await;
            assert_eq!(data["code"], "dependency_unavailable");
            assert_eq!(data["message"], "writes are paused for maintenance");
        }
    }
    // A valid CSRF-protected SSE replay still reaches its read-only handler.
    let replay = format!("/api/v1/chat/runs/{}/events", Uuid::new_v4());
    let response = app
        .clone()
        .oneshot(request(
            Method::POST,
            &replay,
            Some(&current),
            Some(&current.csrf_token),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let after = sqlx::query_as("SELECT last_seen_at, csrf_token_hash FROM sessions WHERE id=$1")
        .bind(current.session.id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    drop(exclusive);
    db.drop().await;
}
