//! HTTP middleware.
//!
//! Only transport concerns live here: request identity and the request log.
//! Business rules stay in route handlers and the core crate.

mod request_id;

pub use request_id::{RequestId, X_REQUEST_ID_HEADER, request_id};
