//! Passkey registration and login routes.
//!
//! Four anonymous writes, all mounted behind
//! [`crate::middleware::auth::require_origin`] because no
//! session exists yet:
//!
//! - `bootstrap/options` + `bootstrap/finish` create the owner's first Passkey
//!   from a CLI-issued one-time token.
//! - `login/options` + `login/finish` authenticate the owner with an existing
//!   Passkey and mint the session cookie.
//!
//! The cryptography is entirely `webauthn-rs`; these handlers only translate
//! HTTP, own the transaction boundaries and map domain errors onto the shared
//! [`ApiErrorBody`]. A finish writes the credential, the consumed ceremony, the
//! consumed token and the session in one transaction, so a rejected credential
//! leaves no partial state behind.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use knoted_core::auth::sessions::{self, CreatedSession};
use knoted_core::auth::webauthn::{
    CeremonyPurpose, PublicKeyCredential, RegisterPublicKeyCredential, build_webauthn,
    consume_ceremony, consume_ceremony_token, create_ceremony, find_owner, finish_authentication,
    finish_registration, insert_passkey, load_ceremony, load_passkeys, owner_has_passkey,
    start_authentication, start_registration, update_passkey_after_authentication,
};
use knoted_core::auth::{DomainAuthError, TokenPurpose, ensure_owner, verify_token};
use knoted_core::config::AuthConfig;
use knoted_core::context::AppContext;
use knoted_core::db::DbError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{ApiErrorBody, FieldError};
use crate::middleware::RequestId;
use crate::middleware::auth::attach_session_cookies;

/// Stable user name stored by the authenticator for the owner.
const OWNER_NAME: &str = "owner";
/// Default display name for the owner account shown by authenticators.
const OWNER_DISPLAY_NAME: &str = "Knoted Owner";
/// Default Passkey name when the request does not provide one.
const DEFAULT_PASSKEY_NAME: &str = "Passkey";
/// Longest accepted Passkey name, matching the OpenAPI schema.
const MAX_PASSKEY_NAME: usize = 100;

/// Auth routes, mounted under `/api/v1` by the application router.
///
/// `require_origin` is a route layer rather than a global one: only these
/// anonymous writes need it, and the layer reads the same [`AuthConfig`] the
/// handlers use to build WebAuthn.
pub fn router(config: AuthConfig) -> Router<AppContext> {
    Router::new()
        .route("/auth/bootstrap/options", post(bootstrap_options))
        .route("/auth/bootstrap/finish", post(bootstrap_finish))
        .route("/auth/login/options", post(login_options))
        .route("/auth/login/finish", post(login_finish))
        .route_layer(from_fn_with_state(
            config,
            crate::middleware::auth::require_origin,
        ))
}

/// `POST /auth/bootstrap/options`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapOptionsRequest {
    bootstrap_token: String,
}

/// `POST /auth/bootstrap/finish`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapFinishRequest {
    ceremony_id: Uuid,
    credential: RegisterPublicKeyCredential,
    #[serde(default)]
    name: Option<String>,
}

/// `POST /auth/login/finish`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginFinishRequest {
    ceremony_id: Uuid,
    credential: PublicKeyCredential,
}

/// `CeremonyOptions` from the contract.
#[derive(Debug, Serialize)]
struct CeremonyOptionsBody {
    ceremony_id: Uuid,
    options: Value,
}

/// `AuthenticationResult` from the contract.
#[derive(Debug, Serialize)]
struct AuthenticationResultBody {
    session: SessionBody,
    csrf_token: String,
}

/// `AuthSession` from the contract; `current` is always true for this response.
#[derive(Debug, Serialize)]
struct SessionBody {
    id: Uuid,
    user_id: Uuid,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    idle_expires_at: DateTime<Utc>,
    absolute_expires_at: DateTime<Utc>,
    current: bool,
}

impl From<&CreatedSession> for AuthenticationResultBody {
    fn from(created: &CreatedSession) -> Self {
        let session = &created.session;
        AuthenticationResultBody {
            session: SessionBody {
                id: session.id,
                user_id: session.user_id,
                created_at: session.created_at,
                last_seen_at: session.last_seen_at,
                idle_expires_at: session.idle_expires_at,
                absolute_expires_at: session.absolute_expires_at,
                current: true,
            },
            csrf_token: created.csrf_token.clone(),
        }
    }
}

/// Creates the first registration challenge for the owner.
async fn bootstrap_options(
    State(ctx): State<AppContext>,
    Extension(config): Extension<AuthConfig>,
    request_id: RequestId,
    payload: Result<Json<BootstrapOptionsRequest>, JsonRejection>,
) -> Response {
    let Json(body) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return bad_request(
                "the request body is not a valid bootstrap options request",
                request_id,
            );
        }
    };
    let now = ctx.now();
    let webauthn = match build_webauthn(&config) {
        Ok(webauthn) => webauthn,
        Err(error) => return auth_error(error, request_id),
    };
    // Verify without consuming: `bootstrap/finish` burns the token inside the
    // same transaction that stores the Passkey.
    let token = match verify_token(
        ctx.pool(),
        TokenPurpose::Bootstrap,
        &body.bootstrap_token,
        now,
    )
    .await
    {
        Ok(token) => token,
        Err(error) => return auth_error(error, request_id),
    };

    let result: Result<(Uuid, Value), DomainAuthError> = async {
        let mut tx = ctx.pool().begin().await.map_err(DbError::from)?;
        let owner = ensure_owner(&mut tx).await?;
        let (challenge, registration) =
            start_registration(&webauthn, owner.user.id, OWNER_NAME, OWNER_DISPLAY_NAME)?;
        let state =
            serde_json::to_value(&registration).map_err(|_| DomainAuthError::InvalidState)?;
        let ceremony_id = create_ceremony(
            &mut tx,
            CeremonyPurpose::Bootstrap,
            owner.user.id,
            Some(token.id),
            &state,
            now,
        )
        .await?;
        tx.commit().await.map_err(DbError::from)?;
        let options =
            serde_json::to_value(&challenge).map_err(|_| DomainAuthError::InvalidState)?;
        Ok((ceremony_id, options))
    }
    .await;

    match result {
        Ok((ceremony_id, options)) => (
            StatusCode::OK,
            Json(CeremonyOptionsBody {
                ceremony_id,
                options,
            }),
        )
            .into_response(),
        Err(error) => auth_error(error, request_id),
    }
}

/// Completes the first registration, consumes the token and starts a session.
async fn bootstrap_finish(
    State(ctx): State<AppContext>,
    Extension(config): Extension<AuthConfig>,
    request_id: RequestId,
    headers: HeaderMap,
    payload: Result<Json<BootstrapFinishRequest>, JsonRejection>,
) -> Response {
    let Json(body) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return bad_request(
                "the request body is not a valid bootstrap finish request",
                request_id,
            );
        }
    };
    let name = match passkey_name(body.name) {
        Ok(name) => name,
        Err(()) => {
            let body = ApiErrorBody::new(
                "validation_failed",
                "the Passkey name is invalid",
                request_id.get(),
            )
            .with_field_errors(vec![FieldError {
                field: "name".to_owned(),
                message: format!("must be at most {MAX_PASSKEY_NAME} characters"),
            }]);
            return body.into_response(StatusCode::BAD_REQUEST);
        }
    };
    let webauthn = match build_webauthn(&config) {
        Ok(webauthn) => webauthn,
        Err(error) => return auth_error(error, request_id),
    };
    let now = ctx.now();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok());
    let transports = credential_transports(&body.credential);

    // Read and verify before opening the transaction; the conditional consume
    // inside it is still what makes a replay impossible.
    let mut connection = match ctx.pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => return database_error(error, request_id),
    };
    let ceremony = match load_ceremony(
        &mut connection,
        body.ceremony_id,
        CeremonyPurpose::Bootstrap,
        now,
    )
    .await
    {
        Ok(ceremony) => ceremony,
        Err(error) => return auth_error(error, request_id),
    };
    drop(connection);

    let passkey = match finish_registration(&webauthn, &body.credential, &ceremony.state) {
        Ok(passkey) => passkey,
        Err(error) => return auth_error(error, request_id),
    };

    let result: Result<CreatedSession, DomainAuthError> = async {
        let mut tx = ctx.pool().begin().await.map_err(DbError::from)?;
        // Account-takeover guard: a previously issued bootstrap token must not
        // add a Passkey to an account that already has one. The check runs
        // before any consume so the token stays usable for the operator, and
        // again after the token is consumed to close a concurrent race.
        if owner_has_passkey(&mut tx, ceremony.user_id).await? {
            return Err(DomainAuthError::BootstrapAlreadyCompleted);
        }
        let consumed =
            consume_ceremony(&mut tx, body.ceremony_id, CeremonyPurpose::Bootstrap, now).await?;
        let outcome = consume_ceremony_token(&mut tx, &consumed, now).await?;
        if owner_has_passkey(&mut tx, outcome.user.id).await? {
            return Err(DomainAuthError::BootstrapAlreadyCompleted);
        }
        insert_passkey(&mut tx, outcome.user.id, &passkey, &name, &transports, now).await?;
        let created =
            sessions::create_session(&mut tx, outcome.user.id, now, user_agent, true).await?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(created)
    }
    .await;

    match result {
        Ok(created) => finished(StatusCode::CREATED, created, now),
        Err(error) => auth_error(error, request_id),
    }
}

/// Creates an authentication challenge for the owner's Passkeys.
async fn login_options(
    State(ctx): State<AppContext>,
    Extension(config): Extension<AuthConfig>,
    request_id: RequestId,
) -> Response {
    let webauthn = match build_webauthn(&config) {
        Ok(webauthn) => webauthn,
        Err(error) => return auth_error(error, request_id),
    };
    let now = ctx.now();

    let mut connection = match ctx.pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => return database_error(error, request_id),
    };
    let Some(user_id) = (match find_owner(&mut connection).await {
        Ok(owner) => owner,
        Err(error) => return auth_error(error, request_id),
    }) else {
        return api_error(
            StatusCode::CONFLICT,
            "not_registered",
            "the account has not been registered yet",
            request_id,
        );
    };
    let passkeys = match load_passkeys(&mut connection, user_id).await {
        Ok(passkeys) => passkeys,
        Err(error) => return auth_error(error, request_id),
    };
    if passkeys.is_empty() {
        return api_error(
            StatusCode::CONFLICT,
            "no_passkey",
            "the account has no Passkey to authenticate with",
            request_id,
        );
    }

    let (challenge, authentication) = match start_authentication(&webauthn, &passkeys) {
        Ok(challenge) => challenge,
        Err(error) => return auth_error(error, request_id),
    };
    let state = match serde_json::to_value(&authentication) {
        Ok(state) => state,
        Err(_) => return auth_error(DomainAuthError::InvalidState, request_id),
    };
    let ceremony_id = match create_ceremony(
        &mut connection,
        CeremonyPurpose::Login,
        user_id,
        None,
        &state,
        now,
    )
    .await
    {
        Ok(id) => id,
        Err(error) => return auth_error(error, request_id),
    };
    let options = match serde_json::to_value(&challenge) {
        Ok(options) => options,
        Err(_) => return auth_error(DomainAuthError::InvalidState, request_id),
    };

    (
        StatusCode::OK,
        Json(CeremonyOptionsBody {
            ceremony_id,
            options,
        }),
    )
        .into_response()
}

/// Completes a Passkey login, updates its metadata and starts a session.
async fn login_finish(
    State(ctx): State<AppContext>,
    Extension(config): Extension<AuthConfig>,
    request_id: RequestId,
    headers: HeaderMap,
    payload: Result<Json<LoginFinishRequest>, JsonRejection>,
) -> Response {
    let Json(body) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return bad_request(
                "the request body is not a valid login finish request",
                request_id,
            );
        }
    };
    let webauthn = match build_webauthn(&config) {
        Ok(webauthn) => webauthn,
        Err(error) => return auth_error(error, request_id),
    };
    let now = ctx.now();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok());

    let mut connection = match ctx.pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => return database_error(error, request_id),
    };
    let ceremony = match load_ceremony(
        &mut connection,
        body.ceremony_id,
        CeremonyPurpose::Login,
        now,
    )
    .await
    {
        Ok(ceremony) => ceremony,
        Err(error) => return auth_error(error, request_id),
    };
    drop(connection);

    let authentication = match finish_authentication(&webauthn, &body.credential, &ceremony.state) {
        Ok(authentication) => authentication,
        Err(error) => return auth_error(error, request_id),
    };

    let result: Result<CreatedSession, DomainAuthError> = async {
        let mut tx = ctx.pool().begin().await.map_err(DbError::from)?;
        let consumed =
            consume_ceremony(&mut tx, body.ceremony_id, CeremonyPurpose::Login, now).await?;
        let updated = update_passkey_after_authentication(&mut tx, &authentication, now).await?;
        if !updated {
            return Err(DomainAuthError::CredentialRejected);
        }
        let created =
            sessions::create_session(&mut tx, consumed.user_id, now, user_agent, true).await?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(created)
    }
    .await;

    match result {
        Ok(created) => finished(StatusCode::OK, created, now),
        Err(error) => auth_error(error, request_id),
    }
}

/// Builds the authentication result and attaches the session and CSRF cookies.
fn finished(status: StatusCode, created: CreatedSession, now: DateTime<Utc>) -> Response {
    let body = AuthenticationResultBody::from(&created);
    let mut response = (status, Json(body)).into_response();
    attach_session_cookies(response.headers_mut(), &created, now);
    response
}

/// The browser's transport hints, as stored in `passkey_credentials.transports`.
fn credential_transports(credential: &RegisterPublicKeyCredential) -> Vec<String> {
    credential
        .response
        .transports
        .iter()
        .flatten()
        .filter_map(|transport| {
            serde_json::to_value(transport)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
        })
        .collect()
}

/// Trims and bounds the optional Passkey name.
fn passkey_name(name: Option<String>) -> Result<String, ()> {
    let name = name.map(|name| name.trim().to_owned()).unwrap_or_default();
    if name.is_empty() {
        return Ok(DEFAULT_PASSKEY_NAME.to_owned());
    }
    if name.chars().count() > MAX_PASSKEY_NAME {
        return Err(());
    }
    Ok(name)
}

/// Maps a domain error to the shared error body and a stable status code.
fn auth_error(error: DomainAuthError, request_id: RequestId) -> Response {
    use DomainAuthError::*;
    match error {
        InvalidToken => api_error(
            StatusCode::CONFLICT,
            "invalid_token",
            "the bootstrap token is not valid",
            request_id,
        ),
        ExpiredToken => api_error(
            StatusCode::CONFLICT,
            "expired_token",
            "the bootstrap token has expired",
            request_id,
        ),
        ConsumedToken => api_error(
            StatusCode::CONFLICT,
            "consumed_token",
            "the bootstrap token has already been used",
            request_id,
        ),
        Conflict => api_error(
            StatusCode::CONFLICT,
            "conflict",
            "a recovery token is required",
            request_id,
        ),
        BootstrapAlreadyCompleted => api_error(
            StatusCode::CONFLICT,
            "bootstrap_already_completed",
            "the account already has a Passkey; a recovery token is required",
            request_id,
        ),
        CredentialRejected => api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "credential_rejected",
            "the WebAuthn credential could not be verified",
            request_id,
        ),
        CeremonyNotFound => api_error(
            StatusCode::NOT_FOUND,
            "ceremony_not_found",
            "the ceremony does not exist",
            request_id,
        ),
        CeremonyReplayed => api_error(
            StatusCode::CONFLICT,
            "ceremony_replayed",
            "the ceremony has already been used",
            request_id,
        ),
        CeremonyExpired => api_error(
            StatusCode::CONFLICT,
            "ceremony_expired",
            "the ceremony has expired",
            request_id,
        ),
        Configuration | InvalidState => {
            tracing::error!(
                target: "knoted_api::auth",
                request_id = %request_id.get(),
                error = %error,
                "authentication configuration or state is invalid"
            );
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "the authentication configuration is invalid",
                request_id,
            )
        }
        Database(error) => database_error(error, request_id),
    }
}

/// A database failure while handling an auth request.
fn database_error(error: impl std::fmt::Display, request_id: RequestId) -> Response {
    tracing::warn!(
        target: "knoted_api::auth",
        request_id = %request_id.get(),
        error = %error,
        "auth request failed on the database"
    );
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "dependency_unavailable",
        "the authentication request could not be completed",
        request_id,
    )
}

/// A 400 for a malformed JSON body.
fn bad_request(message: &'static str, request_id: RequestId) -> Response {
    api_error(
        StatusCode::BAD_REQUEST,
        "invalid_request",
        message,
        request_id,
    )
}

/// Any error in the shared shape.
fn api_error(
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: RequestId,
) -> Response {
    ApiErrorBody::new(code, message, request_id.get()).into_response(status)
}
