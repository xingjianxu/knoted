//! HTTP transport for the Knoted API process.
//!
//! The crate is split into a library so tests can build the router with an
//! injected [`knoted_core::context::AppContext`] and drive it without a socket.
//! `main.rs` only loads configuration, connects the pool, installs logging and
//! serves [`build_router`] on `server.bind`.

pub mod error;
pub mod logging;
pub mod middleware;
pub mod routes;

pub use error::{ApiErrorBody, FieldError};
pub use middleware::RequestId;

use axum::Router;
use knoted_core::context::AppContext;
use tower_http::catch_panic::CatchPanicLayer;

/// Builds the complete router: health probes, the fallback and the shared
/// middleware.
///
/// Layer order matters. The request id layer is outermost so every response,
/// including a 404 or a 500 produced by the panic guard, carries
/// `X-Request-Id`. The panic guard is inside it so a handler panic becomes a
/// 500 response instead of dropping the connection.
pub fn build_router(ctx: AppContext) -> Router {
    routes::router(ctx)
        .layer(CatchPanicLayer::new())
        .layer(axum::middleware::from_fn(middleware::request_id))
}
