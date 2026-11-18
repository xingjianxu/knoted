//! Authenticated conversation routes and durable event replay.

use crate::{
    error::ApiErrorBody,
    middleware::{
        RequestId,
        auth::{CurrentSession, require_csrf, require_session},
    },
};
use axum::{
    Extension, Json, Router,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header},
    middleware::from_fn_with_state,
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use knoted_core::{
    config::AuthConfig,
    context::AppContext,
    rag::{
        chat::{self, ChatEvent, ChatScope, DomainError},
        citations::{self, CitationError, GetCitationSnapshots},
        save_note::{self, SaveAnswer, SaveError},
    },
};
use serde::{Deserialize, Deserializer, Serialize};
use std::{collections::VecDeque, convert::Infallible, time::Duration};
use uuid::Uuid;

pub fn router(auth: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/chat/sessions", get(list).post(create))
        .route(
            "/chat/sessions/{session_id}",
            get(detail).patch(update).delete(delete),
        )
        .route("/chat/sessions/{session_id}/messages", post(submit))
        .route("/chat/runs/{run_id}", get(run_detail))
        .route("/chat/runs/{run_id}/events", post(stream))
        .route("/chat/runs/{run_id}/cancel", post(cancel))
        .route("/chat/runs/{run_id}/retry", post(retry))
        .route("/chat/messages/{message_id}/citations", get(list_citations))
        .route("/chat/messages/{message_id}/save-note", post(save_answer))
        .route_layer(from_fn_with_state(auth, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveNoteRequest {
    title: String,
    parent_id: Option<Uuid>,
}

#[derive(Serialize)]
struct SaveNoteResponse {
    note_id: Uuid,
}

async fn save_answer(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    headers: HeaderMap,
    body: Result<Json<SaveNoteRequest>, JsonRejection>,
) -> Response {
    let Ok(Path(message_id)) = path else {
        return bad(id);
    };
    let Json(body) = match body {
        Ok(body) => body,
        Err(error) => return json_error(id, error),
    };
    let Some(key) = idempotency_key(&headers) else {
        return bad(id);
    };
    let result = save_note::save_answer_as_note(
        &ctx,
        SaveAnswer {
            user_id: actor.session.user_id,
            message_id,
            title: body.title,
            parent_id: body.parent_id,
            idempotency_key: key.into(),
        },
    )
    .await;
    match result {
        Ok(note_id) => data(StatusCode::CREATED, SaveNoteResponse { note_id }, None),
        Err(SaveError::BadKey) => bad(id),
        Err(SaveError::NotFound) => problem(id, DomainError::NotFound),
        Err(SaveError::Conflict) => problem(
            id,
            DomainError::Conflict {
                current_version: None,
            },
        ),
        Err(SaveError::Note(error)) => super::notes::error(id, error),
        Err(SaveError::Database(error)) => problem(id, DomainError::Database(error)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CitationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

async fn list_citations(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    query: Result<Query<CitationQuery>, QueryRejection>,
) -> Response {
    let (Ok(Path(message_id)), Ok(Query(query))) = (path, query) else {
        return bad(id);
    };
    match citations::get_citation_snapshots(
        &ctx,
        GetCitationSnapshots {
            user_id: actor.session.user_id,
            message_id,
            cursor: query.cursor,
            limit: query.limit,
        },
    )
    .await
    {
        Ok(page) => data(StatusCode::OK, page, None),
        Err(CitationError::NotFound) => problem(id, DomainError::NotFound),
        Err(CitationError::BadQuery) => bad(id),
        Err(CitationError::Database(_) | CitationError::Invalid) => ApiErrorBody::new(
            "dependency_unavailable",
            "citation storage unavailable",
            id.get(),
        )
        .into_response(StatusCode::SERVICE_UNAVAILABLE),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateSession {
    #[serde(default, deserialize_with = "non_null")]
    title: Option<String>,
    #[serde(default, deserialize_with = "non_null")]
    provider: Option<String>,
}

fn non_null<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    cursor: Option<String>,
    limit: Option<u32>,
    provider: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditSession {
    expected_version: i64,
    title: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Version {
    expected_version: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ask {
    content: String,
    #[serde(default)]
    scope: ChatScope,
}

fn bad(id: RequestId) -> Response {
    problem(id, DomainError::BadQuery)
}

fn json_error(id: RequestId, error: JsonRejection) -> Response {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        problem(id, DomainError::TooLarge)
    } else {
        bad(id)
    }
}

fn problem(id: RequestId, error: DomainError) -> Response {
    let (status, code, message) = match &error {
        DomainError::NotFound => (
            StatusCode::NOT_FOUND,
            "not_found",
            "chat resource not found",
        ),
        DomainError::Conflict { .. } => (
            StatusCode::CONFLICT,
            "version_conflict",
            "chat version or state conflict",
        ),
        DomainError::Invalid => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "invalid chat field",
        ),
        DomainError::BadQuery => (
            StatusCode::BAD_REQUEST,
            "bad_request",
            "invalid chat parameters",
        ),
        DomainError::TooLarge => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "chat message too large",
        ),
        DomainError::ProviderUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "model_provider_unavailable",
            "configured model provider unavailable",
        ),
        DomainError::Database(_) | DomainError::Queue(_) | DomainError::Context(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "chat storage unavailable",
        ),
    };
    let mut body = ApiErrorBody::new(code, message, id.get());
    if let DomainError::Conflict {
        current_version: Some(value),
    } = error
    {
        body = body.with_current_version(value);
    }
    body.into_response(status)
}

fn data<T: Serialize>(status: StatusCode, body: T, version: Option<i64>) -> Response {
    if let Some(version) = version {
        return (
            status,
            [(header::ETAG, format!("\"{version}\""))],
            Json(body),
        )
            .into_response();
    }
    (status, Json(body)).into_response()
}

async fn list(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return bad(id);
    };
    match chat::list_sessions(
        &ctx,
        chat::ListSessions {
            user_id: actor.session.user_id,
            provider: query.provider,
            after: query.cursor,
            limit: query.limit,
        },
    )
    .await
    {
        Ok(page) => data(StatusCode::OK, page, None),
        Err(error) => problem(id, error),
    }
}

async fn create(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    body: Result<Json<CreateSession>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(body) => body,
        Err(error) => return json_error(id, error),
    };
    match chat::create_session(&ctx, actor.session.user_id, body.title, body.provider).await {
        Ok(session) => {
            let version = session.version;
            data(StatusCode::CREATED, session, Some(version))
        }
        Err(error) => problem(id, error),
    }
}

async fn detail(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Ok(Path(session)) = path else {
        return bad(id);
    };
    match chat::get_session(&ctx, actor.session.user_id, session).await {
        Ok(detail) => {
            let version = detail.session.version;
            data(StatusCode::OK, detail, Some(version))
        }
        Err(error) => problem(id, error),
    }
}

async fn update(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<EditSession>, JsonRejection>,
) -> Response {
    let Ok(Path(session)) = path else {
        return bad(id);
    };
    let Json(body) = match body {
        Ok(body) => body,
        Err(error) => return json_error(id, error),
    };
    match chat::update_session(
        &ctx,
        actor.session.user_id,
        session,
        body.expected_version,
        body.title,
    )
    .await
    {
        Ok(session) => {
            let version = session.version;
            data(StatusCode::OK, session, Some(version))
        }
        Err(error) => problem(id, error),
    }
}

async fn delete(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    query: Result<Query<Version>, QueryRejection>,
) -> Response {
    let (Ok(Path(session)), Ok(Query(version))) = (path, query) else {
        return bad(id);
    };
    match chat::delete_session(
        &ctx,
        actor.session.user_id,
        session,
        version.expected_version,
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => problem(id, error),
    }
}

fn idempotency_key(headers: &HeaderMap) -> Option<&str> {
    headers.get("idempotency-key")?.to_str().ok()
}

async fn submit(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    headers: HeaderMap,
    body: Result<Json<Ask>, JsonRejection>,
) -> Response {
    let Ok(Path(session_id)) = path else {
        return bad(id);
    };
    let Json(body) = match body {
        Ok(body) => body,
        Err(error) => return json_error(id, error),
    };
    let Some(key) = idempotency_key(&headers) else {
        return bad(id);
    };
    match chat::submit_message(
        &ctx,
        chat::SubmitMessage {
            user_id: actor.session.user_id,
            session_id,
            idempotency_key: key.into(),
            content: body.content,
            scope: body.scope,
        },
    )
    .await
    {
        Ok(run) => data(StatusCode::CREATED, run, None),
        Err(error) => problem(id, error),
    }
}

async fn run_detail(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Ok(Path(run)) = path else { return bad(id) };
    match chat::get_run(&ctx, actor.session.user_id, run).await {
        Ok(run) => data(StatusCode::OK, run, None),
        Err(error) => problem(id, error),
    }
}

async fn cancel(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Ok(Path(run)) = path else { return bad(id) };
    match chat::cancel_run(&ctx, actor.session.user_id, run).await {
        Ok(run) => data(StatusCode::OK, run, None),
        Err(error) => problem(id, error),
    }
}

async fn retry(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Ok(Path(run)) = path else { return bad(id) };
    match chat::retry_run(&ctx, actor.session.user_id, run).await {
        Ok(run) => data(StatusCode::CREATED, run, None),
        Err(error) => problem(id, error),
    }
}

struct EventStream {
    ctx: AppContext,
    user: Uuid,
    auth_session: Uuid,
    run: Uuid,
    after: i64,
    batch: VecDeque<ChatEvent>,
    finished: bool,
}

async fn stream(
    State(ctx): State<AppContext>,
    Extension(actor): Extension<CurrentSession>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    headers: HeaderMap,
) -> Response {
    let Ok(Path(run)) = path else { return bad(id) };
    let after = match headers.get("last-event-id") {
        None => 0,
        Some(value) => match value
            .to_str()
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
        {
            Some(seq) if seq >= 0 => seq,
            _ => return bad(id),
        },
    };
    let user = actor.session.user_id;
    let batch = match chat::events(&ctx, user, run, after).await {
        Ok(batch) => batch,
        Err(error) => return problem(id, error),
    };
    let state = EventStream {
        ctx,
        user,
        auth_session: actor.session.id,
        run,
        after,
        finished: batch.terminal && batch.items.len() < 100,
        batch: batch.items.into(),
    };
    let events = futures_util::stream::unfold(state, async |mut state| {
        while state.batch.is_empty() {
            if state.finished {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            if !chat::stream_session_alive(&state.ctx, state.auth_session, state.user)
                .await
                .ok()?
            {
                return None;
            }
            let batch = chat::events(&state.ctx, state.user, state.run, state.after)
                .await
                .ok()?;
            state.finished = batch.terminal && batch.items.len() < 100;
            state.batch = batch.items.into();
        }
        let event = state.batch.pop_front()?;
        state.after = event.seq;
        let data = Event::default()
            .event(&event.kind)
            .id(event.seq.to_string())
            .json_data(event)
            .ok()?;
        Some((Ok::<_, Infallible>(data), state))
    });
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Sse::new(events).keep_alive(KeepAlive::default()),
    )
        .into_response()
}
