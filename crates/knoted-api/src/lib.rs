//! HTTP transport for the Knoted API process.
//!
//! The crate is split into a library so tests can build the router with an
//! injected [`knoted_core::context::AppContext`] and drive it without a socket.
//! `main.rs` only loads configuration, connects the pool, installs logging and
//! serves [`build_router`] on `server.bind`.

pub mod admin;
pub mod error;
pub mod logging;
pub mod middleware;
pub mod routes;

pub use error::{ApiErrorBody, FieldError};
pub use middleware::RequestId;

use axum::Router;
use knoted_core::config::AuthConfig;
use knoted_core::context::AppContext;

/// Wraps a router with the transport layer stack shared by the binary and the
/// tests.
///
/// Layer order matters. The request id layer is outermost, so every response,
/// including a 404, a 405 or a 500 produced by the panic guard, carries
/// `X-Request-Id`. The panic guard sits inside it and converts an unwind into
/// the shared JSON 500 body.
pub fn with_transport_layers<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .layer(axum::middleware::from_fn(middleware::catch_panic))
        .layer(axum::middleware::from_fn(middleware::request_id))
}

/// Builds the complete router: health probes, the fallbacks and the shared
/// middleware, with the default authentication settings.
///
/// Processes that loaded a configuration use [`build_router_with_auth`], so the
/// configured RP ID and origin reach the handlers instead of a default.
pub fn build_router(ctx: AppContext) -> Router {
    build_router_with_auth(ctx, AuthConfig::default())
}

/// Builds the complete router with explicit authentication settings.
///
/// `auth` travels in a request extension, so the auth routes (T14) and the CSRF
/// guard read one copy of the settings. Nothing else about the router changes:
/// health probes, the request id layer and the fallbacks behave exactly as with
/// [`build_router`].
pub fn build_router_with_auth(ctx: AppContext, auth: AuthConfig) -> Router {
    with_transport_layers(routes::router(ctx).layer(axum::Extension(auth)))
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use knoted_core::config::{LogFormat, LogLevel};
    use serde_json::Value;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::logging::{self, LogBuffer};
    use crate::with_transport_layers;

    async fn panicking_handler() -> StatusCode {
        panic!("SUPERSECRETBODY")
    }

    static LOG_BUFFER: OnceLock<LogBuffer> = OnceLock::new();

    fn captured_logs() -> &'static LogBuffer {
        LOG_BUFFER.get_or_init(|| {
            let buffer = LogBuffer::new();
            let sink = buffer.clone();
            logging::init_with_writer(move || sink.clone(), LogLevel::Info, LogFormat::Text)
                .expect("the test logging subscriber must install cleanly");
            buffer
        })
    }

    #[tokio::test]
    async fn t06_4_panic_500_uses_shared_error_body() {
        let logs = captured_logs();
        // Production installs this hook from `logging::init`; the test calls it
        // directly so the hook's own output is captured instead of reaching the
        // real stderr.
        crate::logging::install_panic_hook();
        // The same stack `build_router` uses, so the test cannot drift from
        // production layer ordering.
        let app = with_transport_layers(Router::new().route("/panic", get(panicking_handler)));

        let supplied = Uuid::new_v4();
        let response = app
            .oneshot(
                Request::get("/panic")
                    .header("x-request-id", supplied.to_string())
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("a panicking handler must still answer");

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response
                .headers()
                .get("x-request-id")
                .expect("a 500 response must carry X-Request-Id")
                .to_str()
                .expect("X-Request-Id must be ASCII"),
            supplied.to_string(),
            "the 500 must keep the request's own id"
        );

        let request_id = supplied;
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("the 500 body must be readable");
        let body: Value = serde_json::from_slice(&bytes).expect("the 500 body must be JSON");
        assert_eq!(body["code"], "internal_error");
        assert!(
            body["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty()),
            "the error message must explain the failure: {body}"
        );
        assert_eq!(
            body["request_id"],
            request_id.to_string(),
            "the 500 body must carry the same request id as the header"
        );

        let logs = logs.contents();
        // Positive assertions: removing these log lines would otherwise let the
        // redaction assertions below pass vacuously.
        assert!(
            logs.contains("handler panicked"),
            "the panic guard must log a fixed message: {logs}"
        );
        assert!(
            logs.contains("process panicked"),
            "the production panic hook must log a fixed message: {logs}"
        );
        assert!(
            logs.contains("lib.rs:"),
            "the panic hook must record the source location: {logs}"
        );
        assert!(
            logs.contains(&request_id.to_string()),
            "the handler-panic log must carry the request id: {logs}"
        );
        assert!(
            !logs.contains("SUPERSECRETBODY"),
            "the panic payload may contain user content and must never be logged: {logs}"
        );
    }
}
