//! The error envelope shared by every API response.
//!
//! The shape mirrors `ApiError` in `api/openapi.yaml`, so the browser and
//! internal clients can branch on `code` and quote `request_id` in bug reports.
//! Routes construct [`ApiErrorBody`] with the id taken from the request so the
//! body, the `X-Request-Id` header and the server logs always agree.

use axum::Json;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One invalid field inside an [`ApiErrorBody`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldError {
    /// Request field that failed validation.
    pub field: String,
    /// Human-readable explanation; clients must branch on codes, not this text.
    pub message: String,
}

/// Stable error response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
    /// Request id, also returned in the `X-Request-Id` header.
    pub request_id: Uuid,
    /// Field-level validation failures, when the code is `validation_failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_errors: Option<Vec<FieldError>>,
    /// Current version, returned with a `version_conflict`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_version: Option<i64>,
}

impl ApiErrorBody {
    /// Builds an error body with the required fields.
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        request_id: Uuid,
    ) -> ApiErrorBody {
        ApiErrorBody {
            code: code.into(),
            message: message.into(),
            request_id,
            field_errors: None,
            current_version: None,
        }
    }

    /// Attaches field-level validation failures.
    pub fn with_field_errors(mut self, field_errors: Vec<FieldError>) -> ApiErrorBody {
        self.field_errors = Some(field_errors);
        self
    }

    /// Attaches the current version for a conflict response.
    pub fn with_current_version(mut self, current_version: i64) -> ApiErrorBody {
        self.current_version = Some(current_version);
        self
    }

    /// Renders the body as a JSON response with `status`.
    pub fn into_response(self, status: StatusCode) -> Response {
        (status, Json(self)).into_response()
    }
}
