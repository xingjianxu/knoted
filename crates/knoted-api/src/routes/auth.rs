//! Passkey registration, login, credential management and account recovery.
//!
//! The bootstrap and login ceremonies, plus `recovery/options` and
//! `recovery/finish`, are anonymous writes mounted behind
//! [`crate::middleware::auth::require_origin`] because no session exists yet:
//!
//! - `bootstrap/options` + `bootstrap/finish` create the owner's first Passkey
//!   from a CLI-issued one-time token.
//! - `login/options` + `login/finish` authenticate the owner with an existing
//!   Passkey and mint the session cookie.
//! - `recovery/options` + `recovery/finish` replace every credential and
//!   session from a CLI-issued recovery token when the old Passkeys are lost.
//!
//! The Passkey credential routes need a live session and additionally a Passkey
//! verification within five minutes for every write; route layers apply
//! [`crate::middleware::auth::require_session`] and
//! [`crate::middleware::auth::require_csrf`] in that order.
//!
//! The cryptography is entirely `webauthn-rs`; these handlers only translate
//! HTTP, own the transaction boundaries and map domain errors onto the shared
//! [`ApiErrorBody`]. A finish writes the credential, the consumed ceremony, the
//! consumed token and the session in one transaction, so a rejected credential
//! leaves no partial state behind.

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use knoted_core::auth::sessions::{self, CreatedSession};
use knoted_core::auth::webauthn::{
    CeremonyPurpose, PublicKeyCredential, RegisterPublicKeyCredential, build_webauthn,
    consume_ceremony, consume_ceremony_token, create_ceremony, find_owner, finish_authentication,
    finish_registration, insert_passkey, load_ceremony, load_passkeys, owner_has_passkey,
    start_authentication, start_registration, update_passkey_after_authentication,
};
use knoted_core::auth::{DomainAuthError, TokenPurpose, credentials, ensure_owner, verify_token};
use knoted_core::config::AuthConfig;
use knoted_core::context::AppContext;
use knoted_core::db::DbError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{ApiErrorBody, FieldError};
use crate::middleware::RequestId;
use crate::middleware::auth::{
    CurrentSession, attach_session_cookies, require_csrf, require_session,
};

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
/// `require_origin` is a route layer rather than a global one: only the
/// anonymous writes need it, and the layer reads the same [`AuthConfig`] the
/// handlers use to build WebAuthn. The credential routes need a session, so
/// they get [`require_session`] and [`require_csrf`] instead; the session layer
/// is added last, which makes it outermost and therefore runs first, so the
/// CSRF guard can read [`CurrentSession`].
pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    let anonymous = Router::new()
        .route("/auth/bootstrap/options", post(bootstrap_options))
        .route("/auth/bootstrap/finish", post(bootstrap_finish))
        .route("/auth/login/options", post(login_options))
        .route("/auth/login/finish", post(login_finish))
        .route("/auth/recovery/options", post(recovery_options))
        .route("/auth/recovery/finish", post(recovery_finish))
        .route_layer(from_fn_with_state(
            config.clone(),
            crate::middleware::auth::require_origin,
        ));

    let authenticated = Router::new()
        .route("/auth/passkeys", get(passkey_list))
        .route("/auth/passkeys/options", post(passkey_options))
        .route("/auth/passkeys/finish", post(passkey_finish))
        .route("/auth/passkeys/{passkey_id}", delete(delete_passkey))
        .route_layer(from_fn_with_state(config, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session));

    anonymous.merge(authenticated)
}

/// `POST /auth/bootstrap/options`
///
/// `Debug` is deliberately not derived: the struct carries a secret token and a
/// future log line must not be able to print it.
#[derive(Deserialize)]
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

/// `POST /auth/passkeys/finish`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeyFinishRequest {
    ceremony_id: Uuid,
    credential: RegisterPublicKeyCredential,
    #[serde(default)]
    name: Option<String>,
}

/// `POST /auth/recovery/options`
///
/// `Debug` is deliberately not derived: the struct carries a secret token and a
/// future log line must not be able to print it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryOptionsRequest {
    recovery_token: String,
}

/// `POST /auth/recovery/finish`
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryFinishRequest {
    recovery_token: String,
    ceremony_id: Uuid,
    credential: RegisterPublicKeyCredential,
    #[serde(default)]
    name: Option<String>,
}

/// `Passkey` from the contract.
///
/// `credential_id` is base64url rather than the raw bytes, and the stored
/// public key is never part of the response.
#[derive(Debug, Serialize)]
struct PasskeyBody {
    id: Uuid,
    name: String,
    credential_id: String,
    transports: Vec<String>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

impl From<&credentials::PasskeyRecord> for PasskeyBody {
    fn from(record: &credentials::PasskeyRecord) -> Self {
        PasskeyBody {
            id: record.id,
            name: record.name.clone(),
            credential_id: record.credential_id_base64(),
            transports: record.transports.clone(),
            created_at: record.created_at,
            last_used_at: record.last_used_at,
        }
    }
}

/// `PasskeyPage` from the contract.
///
/// `next_cursor` is the opaque keyset cursor of the next page, or `null` on the
/// last page.
#[derive(Debug, Serialize)]
struct PasskeyPageBody {
    items: Vec<PasskeyBody>,
    next_cursor: Option<String>,
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
        Err(()) => return invalid_passkey_name(request_id),
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
        // add a Passkey to an account that already has one. The first check
        // runs before any consume so the token stays usable for the operator.
        // The second runs after consumption inside the same transaction as
        // defence in depth; what actually serialises the flow is T13's
        // single-live-bootstrap-token invariant, not this check.
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

/// `GET /auth/passkeys`
///
/// Lists one page of the owner's credentials, newest first. The stored public
/// key is not part of the response; `credential_id` is base64url so a browser
/// can compare it with `PublicKeyCredential.rawId`. `cursor` is the opaque,
/// versioned encoding of the keyset position `(created_at, id)`, and `limit`
/// follows the contract's default (50) and maximum (100).
async fn passkey_list(
    State(ctx): State<AppContext>,
    Extension(current): Extension<CurrentSession>,
    request_id: RequestId,
    query: Result<Query<PasskeyListQuery>, QueryRejection>,
) -> Response {
    let Query(query) = match query {
        Ok(query) => query,
        Err(_) => {
            return bad_request(
                "the cursor or limit query parameter is not valid",
                request_id,
            );
        }
    };
    if let Some(limit) = query.limit
        && !(1..=credentials::PASSKEY_PAGE_MAX_LIMIT).contains(&limit)
    {
        return validation_error(
            "limit",
            &format!(
                "must be between 1 and {}",
                credentials::PASSKEY_PAGE_MAX_LIMIT
            ),
            request_id,
        );
    }
    let cursor = match query.cursor.as_deref().map(decode_passkey_cursor) {
        None => None,
        Some(Some(cursor)) => Some(cursor),
        Some(None) => {
            return validation_error("cursor", "the cursor is not valid", request_id);
        }
    };
    let mut connection = match ctx.pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => return database_error(error, request_id),
    };
    match credentials::list_passkeys(
        &mut connection,
        current.session.user_id,
        cursor,
        query.limit,
    )
    .await
    {
        Ok(page) => (
            StatusCode::OK,
            Json(PasskeyPageBody {
                items: page.items.iter().map(PasskeyBody::from).collect(),
                next_cursor: page
                    .next_cursor
                    .map(|(created_at, id)| encode_passkey_cursor(created_at, id)),
            }),
        )
            .into_response(),
        Err(error) => auth_error(error, request_id),
    }
}

/// The `cursor` and `limit` query parameters of `GET /auth/passkeys`.
#[derive(Debug, Deserialize)]
struct PasskeyListQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

/// Version tag of the encoded Passkey cursor; a differing tag is rejected.
const PASSKEY_CURSOR_VERSION: u8 = 1;

/// The payload behind the opaque `next_cursor` string.
#[derive(Debug, Serialize, Deserialize)]
struct PasskeyCursor {
    v: u8,
    created_at: DateTime<Utc>,
    id: Uuid,
}

/// Encodes the keyset position as base64url versioned JSON, per the contract.
fn encode_passkey_cursor(created_at: DateTime<Utc>, id: Uuid) -> String {
    let cursor = PasskeyCursor {
        v: PASSKEY_CURSOR_VERSION,
        created_at,
        id,
    };
    let bytes = serde_json::to_vec(&cursor).expect("a cursor is always JSON");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Decodes a client-supplied cursor, rejecting anything that is not ours.
///
/// The cursor is not an authorisation token: the listing is still scoped to the
/// signed-in owner. It only carries the sort key, and a malformed or
/// future-version value is a client error rather than a silent reset.
fn decode_passkey_cursor(raw: &str) -> Option<(DateTime<Utc>, Uuid)> {
    let bytes = URL_SAFE_NO_PAD.decode(raw).ok()?;
    let cursor: PasskeyCursor = serde_json::from_slice(&bytes).ok()?;
    (cursor.v == PASSKEY_CURSOR_VERSION).then_some((cursor.created_at, cursor.id))
}

/// `POST /auth/passkeys/options`
///
/// Starts an add-Passkey registration ceremony for the signed-in owner. The
/// five-minute verification window is checked here as well as on finish, so a
/// stale session cannot even obtain a challenge.
async fn passkey_options(
    State(ctx): State<AppContext>,
    Extension(config): Extension<AuthConfig>,
    Extension(current): Extension<CurrentSession>,
    request_id: RequestId,
) -> Response {
    let now = ctx.now();
    if let Err(error) = credentials::require_recent_verification(&current.session, now) {
        return auth_error(error, request_id);
    }
    let webauthn = match build_webauthn(&config) {
        Ok(webauthn) => webauthn,
        Err(error) => return auth_error(error, request_id),
    };
    let (challenge, registration) = match start_registration(
        &webauthn,
        current.session.user_id,
        OWNER_NAME,
        OWNER_DISPLAY_NAME,
    ) {
        Ok(pair) => pair,
        Err(error) => return auth_error(error, request_id),
    };
    let state = match serde_json::to_value(&registration) {
        Ok(state) => state,
        Err(_) => return auth_error(DomainAuthError::InvalidState, request_id),
    };
    let mut connection = match ctx.pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => return database_error(error, request_id),
    };
    let ceremony_id = match create_ceremony(
        &mut connection,
        CeremonyPurpose::AddPasskey,
        current.session.user_id,
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

/// `POST /auth/passkeys/finish`
///
/// Persists a verified add-Passkey registration. The ceremony must belong to the
/// signed-in account, so a ceremony id from someone else behaves like an unknown
/// one. Finishing also records the verification on the session, which is what
/// keeps a long credential-management session inside the five-minute window.
async fn passkey_finish(
    State(ctx): State<AppContext>,
    Extension(config): Extension<AuthConfig>,
    Extension(current): Extension<CurrentSession>,
    request_id: RequestId,
    payload: Result<Json<PasskeyFinishRequest>, JsonRejection>,
) -> Response {
    let Json(body) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return bad_request(
                "the request body is not a valid Passkey finish request",
                request_id,
            );
        }
    };
    let name = match passkey_name(body.name) {
        Ok(name) => name,
        Err(()) => return invalid_passkey_name(request_id),
    };
    let now = ctx.now();
    if let Err(error) = credentials::require_recent_verification(&current.session, now) {
        return auth_error(error, request_id);
    }
    let webauthn = match build_webauthn(&config) {
        Ok(webauthn) => webauthn,
        Err(error) => return auth_error(error, request_id),
    };

    let mut connection = match ctx.pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => return database_error(error, request_id),
    };
    let ceremony = match load_ceremony(
        &mut connection,
        body.ceremony_id,
        CeremonyPurpose::AddPasskey,
        now,
    )
    .await
    {
        Ok(ceremony) => ceremony,
        Err(error) => return auth_error(error, request_id),
    };
    drop(connection);
    if ceremony.user_id != current.session.user_id {
        return auth_error(DomainAuthError::CeremonyNotFound, request_id);
    }

    let passkey = match finish_registration(&webauthn, &body.credential, &ceremony.state) {
        Ok(passkey) => passkey,
        Err(error) => return auth_error(error, request_id),
    };
    let transports = credential_transports(&body.credential);

    let result: Result<Option<credentials::PasskeyRecord>, DomainAuthError> = async {
        let mut tx = ctx.pool().begin().await.map_err(DbError::from)?;
        consume_ceremony(&mut tx, body.ceremony_id, CeremonyPurpose::AddPasskey, now).await?;
        let record = credentials::add_passkey(
            &mut tx,
            current.session.user_id,
            &passkey,
            &name,
            &transports,
            now,
        )
        .await?;
        // The session row can disappear or expire between the guard and this
        // write. Refusing the whole transaction keeps the answer honest: a
        // revoked session must not receive a 200 and a fresh credential.
        if !sessions::mark_verified(&mut tx, current.session.id, now).await? {
            tx.rollback().await.map_err(DbError::from)?;
            return Ok(None);
        }
        tx.commit().await.map_err(DbError::from)?;
        Ok(Some(record))
    }
    .await;

    match result {
        Ok(Some(record)) => (StatusCode::CREATED, Json(PasskeyBody::from(&record))).into_response(),
        Ok(None) => api_error(
            StatusCode::UNAUTHORIZED,
            crate::middleware::auth::UNAUTHENTICATED,
            "a live session cookie is required",
            request_id,
        ),
        Err(error) => auth_error(error, request_id),
    }
}

/// `DELETE /auth/passkeys/{passkeyId}`
///
/// Removes one credential. The last one cannot go: an account with no Passkey
/// could only be recovered, so the normal path refuses instead of locking the
/// owner out.
async fn delete_passkey(
    State(ctx): State<AppContext>,
    Extension(current): Extension<CurrentSession>,
    Path(passkey_id): Path<String>,
    request_id: RequestId,
) -> Response {
    let Ok(passkey_id) = Uuid::parse_str(&passkey_id) else {
        return bad_request("the Passkey id is not a UUID", request_id);
    };
    let now = ctx.now();
    if let Err(error) = credentials::require_recent_verification(&current.session, now) {
        return auth_error(error, request_id);
    }

    match credentials::remove_passkey(ctx.pool(), current.session.user_id, passkey_id, now).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => auth_error(error, request_id),
    }
}

/// `POST /auth/recovery/options`
///
/// Starts a recovery registration ceremony from a CLI-issued recovery token. The
/// token is verified but not consumed, so a failed finish leaves it usable; the
/// ceremony carries the token's row id so finish can re-check the binding.
async fn recovery_options(
    State(ctx): State<AppContext>,
    Extension(config): Extension<AuthConfig>,
    request_id: RequestId,
    payload: Result<Json<RecoveryOptionsRequest>, JsonRejection>,
) -> Response {
    let Json(body) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return bad_request(
                "the request body is not a valid recovery options request",
                request_id,
            );
        }
    };
    let now = ctx.now();
    let webauthn = match build_webauthn(&config) {
        Ok(webauthn) => webauthn,
        Err(error) => return auth_error(error, request_id),
    };
    let token = match verify_token(
        ctx.pool(),
        TokenPurpose::Recovery,
        &body.recovery_token,
        now,
    )
    .await
    {
        Ok(token) => token,
        Err(error) => return auth_error(error, request_id),
    };

    let mut connection = match ctx.pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => return database_error(error, request_id),
    };
    // Recovery replaces the Passkeys of an existing account; it never creates
    // the owner. Without an account the operator must run the bootstrap flow.
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
    let (challenge, registration) =
        match start_registration(&webauthn, user_id, OWNER_NAME, OWNER_DISPLAY_NAME) {
            Ok(pair) => pair,
            Err(error) => return auth_error(error, request_id),
        };
    let state = match serde_json::to_value(&registration) {
        Ok(state) => state,
        Err(_) => return auth_error(DomainAuthError::InvalidState, request_id),
    };
    let ceremony_id = match create_ceremony(
        &mut connection,
        CeremonyPurpose::Recovery,
        user_id,
        Some(token.id),
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

/// `POST /auth/recovery/finish`
///
/// Completes recovery: one transaction consumes the ceremony and the token,
/// installs the new Passkey, deletes the old ones, revokes every old session and
/// creates a verified session for the caller. The token is verified outside the
/// transaction (to reject a bad request cheaply) and again inside it, bound to
/// the ceremony, so only the token that started the ceremony can finish it.
async fn recovery_finish(
    State(ctx): State<AppContext>,
    Extension(config): Extension<AuthConfig>,
    request_id: RequestId,
    headers: HeaderMap,
    payload: Result<Json<RecoveryFinishRequest>, JsonRejection>,
) -> Response {
    let Json(body) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return bad_request(
                "the request body is not a valid recovery finish request",
                request_id,
            );
        }
    };
    let name = match passkey_name(body.name) {
        Ok(name) => name,
        Err(()) => return invalid_passkey_name(request_id),
    };
    let webauthn = match build_webauthn(&config) {
        Ok(webauthn) => webauthn,
        Err(error) => return auth_error(error, request_id),
    };
    let now = ctx.now();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok());
    let token = match verify_token(
        ctx.pool(),
        TokenPurpose::Recovery,
        &body.recovery_token,
        now,
    )
    .await
    {
        Ok(token) => token,
        Err(error) => return auth_error(error, request_id),
    };

    let mut connection = match ctx.pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => return database_error(error, request_id),
    };
    let ceremony = match load_ceremony(
        &mut connection,
        body.ceremony_id,
        CeremonyPurpose::Recovery,
        now,
    )
    .await
    {
        Ok(ceremony) => ceremony,
        Err(error) => return auth_error(error, request_id),
    };
    drop(connection);
    if ceremony.credential_token_id != Some(token.id) {
        return auth_error(DomainAuthError::InvalidToken, request_id);
    }

    let passkey = match finish_registration(&webauthn, &body.credential, &ceremony.state) {
        Ok(passkey) => passkey,
        Err(error) => return auth_error(error, request_id),
    };
    let transports = credential_transports(&body.credential);

    let result: Result<credentials::RecoveryOutcome, DomainAuthError> = async {
        let mut tx = ctx.pool().begin().await.map_err(DbError::from)?;
        let outcome = credentials::complete_recovery(
            &mut tx,
            &token,
            body.ceremony_id,
            &credentials::NewPasskey {
                passkey: &passkey,
                name: &name,
                transports: &transports,
            },
            user_agent,
            now,
        )
        .await?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(outcome)
    }
    .await;

    match result {
        Ok(outcome) => finished(StatusCode::CREATED, outcome.session, now),
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

/// The browser's transport hints, filtered to the contract's enum.
///
/// `webauthn-rs` also models the non-contract `test` and `unknown` values. They
/// are hints, not a security property, so they are dropped instead of failing
/// the whole registration: the browser's list may legitimately contain a value
/// this API version does not expose.
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
        .filter(|transport| {
            matches!(
                transport.as_str(),
                "usb" | "nfc" | "ble" | "hybrid" | "internal"
            )
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

/// The `400 validation_failed` used by both finish handlers for a bad name.
fn invalid_passkey_name(request_id: RequestId) -> Response {
    ApiErrorBody::new(
        "validation_failed",
        "the Passkey name is invalid",
        request_id.get(),
    )
    .with_field_errors(vec![FieldError {
        field: "name".to_owned(),
        message: format!("must be at most {MAX_PASSKEY_NAME} characters"),
    }])
    .into_response(StatusCode::BAD_REQUEST)
}

/// A `400 validation_failed` naming a query parameter that failed validation.
fn validation_error(field: &str, message: &str, request_id: RequestId) -> Response {
    ApiErrorBody::new(
        "validation_failed",
        "the request query is invalid",
        request_id.get(),
    )
    .with_field_errors(vec![FieldError {
        field: field.to_owned(),
        message: message.to_owned(),
    }])
    .into_response(StatusCode::BAD_REQUEST)
}

/// Maps a domain error to the shared error body and a stable status code.
fn auth_error(error: DomainAuthError, request_id: RequestId) -> Response {
    use DomainAuthError::*;
    match error {
        InvalidToken => api_error(
            StatusCode::CONFLICT,
            "invalid_token",
            "the credential token is not valid",
            request_id,
        ),
        ExpiredToken => api_error(
            StatusCode::CONFLICT,
            "expired_token",
            "the credential token has expired",
            request_id,
        ),
        ConsumedToken => api_error(
            StatusCode::CONFLICT,
            "consumed_token",
            "the credential token has already been used",
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
        DuplicateCredential => api_error(
            StatusCode::CONFLICT,
            "duplicate_credential",
            "this Passkey is already registered",
            request_id,
        ),
        LastCredential => api_error(
            StatusCode::CONFLICT,
            "last_credential",
            "the last Passkey of the account cannot be removed",
            request_id,
        ),
        PasskeyNotFound => api_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "the Passkey does not exist",
            request_id,
        ),
        RecentVerificationRequired => api_error(
            StatusCode::FORBIDDEN,
            "recent_verification_required",
            "a Passkey verification within the last five minutes is required",
            request_id,
        ),
        CredentialRejected => api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "credential_rejected",
            "the WebAuthn credential could not be verified",
            request_id,
        ),
        // The contract only declares 409 for these endpoints, so an unknown
        // ceremony is answered exactly like a consumed or expired one.
        CeremonyNotFound => api_error(
            StatusCode::CONFLICT,
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
