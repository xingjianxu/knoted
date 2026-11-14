//! Request identity: one UUID per HTTP request, returned to the caller and
//! attached to the request's tracing span and log lines.
//!
//! A well-formed `X-Request-Id` from the caller is reused, so a client can
//! correlate its own logs with the server's. Anything else is replaced by a
//! freshly generated UUID: the id is metadata, never an authorization token.

use std::time::Instant;

use axum::extract::{FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;
use uuid::Uuid;

/// Header carrying the request id, on requests and on every response.
pub const X_REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// Request id attached to the request extension and its response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestId(Uuid);

impl RequestId {
    /// A fresh random UUIDv4.
    pub fn generate() -> RequestId {
        RequestId(Uuid::new_v4())
    }

    /// Reuses `value` when it is a UUID, generating a new id otherwise.
    pub fn from_header(value: Option<&HeaderValue>) -> RequestId {
        value
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Uuid::parse_str(value.trim()).ok())
            .map(RequestId)
            .unwrap_or_else(RequestId::generate)
    }

    /// The underlying UUID.
    pub fn get(self) -> Uuid {
        self.0
    }

    /// Header representation used on the request and the response.
    fn header_value(self) -> HeaderValue {
        HeaderValue::from_str(&self.0.hyphenated().to_string())
            .expect("a hyphenated UUID is always a valid header value")
    }
}

impl Default for RequestId {
    fn default() -> RequestId {
        RequestId::generate()
    }
}

impl From<RequestId> for Uuid {
    fn from(request_id: RequestId) -> Uuid {
        request_id.0
    }
}

impl<S> FromRequestParts<S> for RequestId
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<RequestId, Self::Rejection> {
        // The middleware runs first and always inserts the id. The fallback
        // keeps handlers usable from tests and nested routers that forget the
        // layer, instead of rejecting an otherwise valid request.
        if let Some(request_id) = parts.extensions.get::<RequestId>() {
            return Ok(*request_id);
        }
        let request_id = RequestId::generate();
        parts.extensions.insert(request_id);
        Ok(request_id)
    }
}

/// Assigns the request id, returns it in the response header and logs exactly
/// one line per request.
///
/// The log line records only the method, the path, the status, the latency and
/// the request id. Query strings, headers (cookies, authorization) and bodies
/// are deliberately absent, so credentials and note contents cannot leak into
/// logs through this path.
pub async fn request_id(mut request: Request, next: Next) -> Response {
    let request_id = RequestId::from_header(request.headers().get(X_REQUEST_ID_HEADER));
    request.extensions_mut().insert(request_id);
    // Downstream layers and handlers see the validated id, not the raw header.
    request
        .headers_mut()
        .insert(X_REQUEST_ID_HEADER, request_id.header_value());

    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = Instant::now();
    let span = tracing::info_span!("http_request", request_id = %request_id.get());

    let mut response = next.run(request).instrument(span).await;
    response
        .headers_mut()
        .insert(X_REQUEST_ID_HEADER, request_id.header_value());

    tracing::info!(
        target: "knoted_api::http",
        request_id = %request_id.get(),
        method = %method,
        path = %path,
        status = response.status().as_u16(),
        latency_ms = started.elapsed().as_millis() as u64,
        "request completed"
    );

    response
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    use super::{X_REQUEST_ID_HEADER, request_id};

    async fn panicking_handler() -> StatusCode {
        panic!("synthetic handler panic")
    }

    #[tokio::test]
    async fn t06_4_panic_500_keeps_request_id() {
        // A panic escaping a handler becomes a 500, and the outer request id
        // layer still labels it, so operators can find the failing request.
        let app: Router = Router::new()
            .route("/panic", get(panicking_handler))
            .layer(tower_http::catch_panic::CatchPanicLayer::new())
            .layer(axum::middleware::from_fn(request_id));

        let response = app
            .oneshot(Request::get("/panic").body(Body::empty()).expect("request"))
            .await
            .expect("a panicking handler must still answer with a 500");

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            response.headers().contains_key(X_REQUEST_ID_HEADER),
            "a 500 response must carry X-Request-Id"
        );
    }
}
