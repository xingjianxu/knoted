//! HTTP middleware.
//!
//! Only transport concerns live here: request identity, the request log and the
//! panic guard. Business rules stay in route handlers and the core crate.

mod catch_panic;
mod request_id;

pub use catch_panic::{INTERNAL_ERROR, catch_panic};
pub use request_id::{RequestId, X_REQUEST_ID_HEADER, request_id};
