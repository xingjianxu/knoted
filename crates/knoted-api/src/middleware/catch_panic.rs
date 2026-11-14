//! Converts a panicking handler into the shared JSON 500 response.
//!
//! A handler panic must not drop the connection or leak the panic payload,
//! which can embed user content. This middleware catches the unwind around the
//! rest of the stack and answers with [`ApiErrorBody`] carrying the request's
//! own id, so a 500 looks like every other failure to the client.
//!
//! The unwind is caught inside the request's own future (`catch_unwind`), not
//! in a spawned task: the request id is already in the extension, no per-request
//! task is created, and the request's tracing span stays attached. The trade-off
//! is that a panic aborts only this poll; the process keeps running, exactly as
//! a tower panic guard would behave.

use std::panic::AssertUnwindSafe;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use futures_util::FutureExt;

use crate::error::ApiErrorBody;
use crate::middleware::RequestId;

/// Stable error code for an unexpected handler failure.
pub const INTERNAL_ERROR: &str = "internal_error";

/// Catches a panic from the downstream stack and renders the shared 500 body.
pub async fn catch_panic(request: Request, next: Next) -> Response {
    // The `request_id` layer runs first and always inserts the id; the default
    // keeps this middleware usable on its own without inventing a header.
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .copied()
        .unwrap_or_default();

    // `AssertUnwindSafe` is required because Axum's request future is not
    // `UnwindSafe`; a caught panic leaves the process in a consistent state,
    // as with any tower panic guard.
    match AssertUnwindSafe(next.run(request)).catch_unwind().await {
        Ok(response) => response,
        Err(payload) => {
            // Dropped without formatting: the payload may contain note text,
            // tokens or other user data and must never reach a log sink.
            drop(payload);
            tracing::error!(
                target: "knoted_api::http",
                request_id = %request_id.get(),
                "handler panicked"
            );
            ApiErrorBody::new(
                INTERNAL_ERROR,
                "the request failed unexpectedly",
                request_id.get(),
            )
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
