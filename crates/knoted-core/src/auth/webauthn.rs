//! WebAuthn Passkey ceremonies and their persistence.
//!
//! `webauthn-rs` performs every cryptographic check (challenge, origin, RP ID,
//! signature, counter); this module only owns the ceremony lifecycle and the
//! PostgreSQL rows. The state produced by `start_passkey_registration` and
//! `start_passkey_authentication` is stored server-side in `ceremonies.state`
//! and is never sent to the browser, so enabling
//! `danger-allow-state-serialisation` does not turn the client into the keeper
//! of its own challenge.
//!
//! A ceremony lives for [`CEREMONY_TTL`] (five minutes) and is consumed exactly
//! once by a conditional `UPDATE` inside the caller's transaction. The
//! predicate checks `consumed_at IS NULL AND expires_at > now`, so two
//! concurrent finishes cannot both win and a replayed `ceremony_id` is rejected
//! before any credential or session is written.
//!
//! The relying party is fixed by [`AuthConfig`]: the RP ID is a bare host and
//! the origin is matched exactly, including its port. Subdomains and other
//! ports are not accepted.
//!
//! As with the rest of `knoted-core`, no Axum type appears here and every
//! expiry decision uses the caller's injected clock.

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sqlx::{PgConnection, Postgres, Transaction};
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::{
    AuthenticationResult, CreationChallengeResponse, Passkey, PasskeyAuthentication,
    PasskeyRegistration, RequestChallengeResponse, Webauthn, WebauthnBuilder,
};

pub use webauthn_rs::prelude::{PublicKeyCredential, RegisterPublicKeyCredential};

use crate::config::AuthConfig;
use crate::db::DbError;

use super::bootstrap::{DomainAuthError, consume_bootstrap_token};

/// Human-readable relying-party name shown by authenticators.
pub const RP_NAME: &str = "Knoted";
/// How long a registration or authentication ceremony stays usable.
pub const CEREMONY_TTL: Duration = Duration::minutes(5);

/// Why a ceremony exists.
///
/// The values are the database check constraint's vocabulary for T14; the
/// recovery and add-Passkey purposes are added with their own routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeremonyPurpose {
    /// First registration of the owner's Passkey.
    Bootstrap,
    /// Passkey login for the existing owner.
    Login,
}

impl CeremonyPurpose {
    /// The database value of this purpose.
    pub fn as_str(&self) -> &'static str {
        match self {
            CeremonyPurpose::Bootstrap => "bootstrap",
            CeremonyPurpose::Login => "login",
        }
    }
}

/// Row read by [`load_ceremony`]: owner, token, state, expiry, consumption.
type CeremonyLookupRow = (
    Option<Uuid>,
    Option<Uuid>,
    Value,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

/// Row returned by [`consume_ceremony`]'s conditional update.
type CeremonyConsumeRow = (Option<Uuid>, Option<Uuid>, Value, DateTime<Utc>);

/// A server-side ceremony row, before it is consumed.
#[derive(Debug, Clone)]
pub struct Ceremony {
    /// Row id returned to the browser so it can name the ceremony on finish.
    pub id: Uuid,
    /// Account the ceremony belongs to.
    pub user_id: Uuid,
    /// Bootstrap/recovery token that authorised the ceremony, if any.
    pub credential_token_id: Option<Uuid>,
    /// Server-only `webauthn-rs` state.
    pub state: Value,
    /// Instant after which the ceremony is rejected.
    pub expires_at: DateTime<Utc>,
}

/// Builds the relying-party instance from the process configuration.
///
/// `rp_id` must be an effective domain of `origin`; the builder rejects anything
/// else, so a typo fails here instead of at the first login. Subdomains are
/// disabled and the origin keeps its port, which is what makes the match exact.
pub fn build_webauthn(config: &AuthConfig) -> Result<Webauthn, DomainAuthError> {
    let origin = Url::parse(&config.origin).map_err(|_| DomainAuthError::Configuration)?;
    WebauthnBuilder::new(&config.rp_id, &origin)
        .map_err(|_| DomainAuthError::Configuration)?
        .rp_name(RP_NAME)
        .allow_subdomains(false)
        .build()
        .map_err(|_| DomainAuthError::Configuration)
}

/// Starts a Passkey registration ceremony.
///
/// No credentials are excluded: `bootstrap/finish` is only allowed while the
/// owner has none, and an existing one is rejected there before this challenge
/// could add a second.
pub fn start_registration(
    webauthn: &Webauthn,
    user: Uuid,
    user_name: &str,
    display_name: &str,
) -> Result<(CreationChallengeResponse, PasskeyRegistration), DomainAuthError> {
    webauthn
        .start_passkey_registration(user, user_name, display_name, None)
        .map_err(|_| DomainAuthError::CredentialRejected)
}

/// Completes a Passkey registration using the server-held ceremony state.
///
/// Every failure — wrong origin, wrong RP ID hash, bad signature, altered
/// challenge — is [`DomainAuthError::CredentialRejected`], which the route maps
/// to `422` without writing anything.
pub fn finish_registration(
    webauthn: &Webauthn,
    credential: &RegisterPublicKeyCredential,
    state: &Value,
) -> Result<Passkey, DomainAuthError> {
    let registration: PasskeyRegistration =
        serde_json::from_value(state.clone()).map_err(|_| DomainAuthError::InvalidState)?;
    webauthn
        .finish_passkey_registration(credential, &registration)
        .map_err(|_| DomainAuthError::CredentialRejected)
}

/// Starts a Passkey authentication ceremony for the owner's credentials.
pub fn start_authentication(
    webauthn: &Webauthn,
    passkeys: &[Passkey],
) -> Result<(RequestChallengeResponse, PasskeyAuthentication), DomainAuthError> {
    webauthn
        .start_passkey_authentication(passkeys)
        .map_err(|_| DomainAuthError::CredentialRejected)
}

/// Completes a Passkey authentication using the server-held ceremony state.
pub fn finish_authentication(
    webauthn: &Webauthn,
    credential: &PublicKeyCredential,
    state: &Value,
) -> Result<AuthenticationResult, DomainAuthError> {
    let authentication: PasskeyAuthentication =
        serde_json::from_value(state.clone()).map_err(|_| DomainAuthError::InvalidState)?;
    webauthn
        .finish_passkey_authentication(credential, &authentication)
        .map_err(|_| DomainAuthError::CredentialRejected)
}

/// Stores the serialised challenge state and returns the new ceremony id.
///
/// The row carries the account and, for a bootstrap ceremony, the id of the
/// token that authorised it. The plaintext token is never stored.
pub async fn create_ceremony(
    executor: &mut PgConnection,
    purpose: CeremonyPurpose,
    user_id: Uuid,
    credential_token_id: Option<Uuid>,
    state: &Value,
    now: DateTime<Utc>,
) -> Result<Uuid, DomainAuthError> {
    let id = Uuid::new_v4();
    let expires_at = now + CEREMONY_TTL;
    sqlx::query(
        "INSERT INTO ceremonies \
         (id, purpose, user_id, credential_token_id, state, created_at, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(purpose.as_str())
    .bind(user_id)
    .bind(credential_token_id)
    .bind(state)
    .bind(now)
    .bind(expires_at)
    .execute(&mut *executor)
    .await?;
    Ok(id)
}

/// Reads a live ceremony without consuming it.
///
/// `finish` calls this first because the credential must be verified before the
/// transaction starts; [`consume_ceremony`] is still the authority on single
/// use, so a concurrent finish that loses the race writes nothing.
pub async fn load_ceremony(
    executor: &mut PgConnection,
    id: Uuid,
    purpose: CeremonyPurpose,
    now: DateTime<Utc>,
) -> Result<Ceremony, DomainAuthError> {
    let row: Option<CeremonyLookupRow> = sqlx::query_as(
        "SELECT user_id, credential_token_id, state, expires_at, consumed_at \
         FROM ceremonies WHERE id = $1 AND purpose = $2",
    )
    .bind(id)
    .bind(purpose.as_str())
    .fetch_optional(&mut *executor)
    .await?;

    let Some((user_id, credential_token_id, state, expires_at, consumed_at)) = row else {
        return Err(DomainAuthError::CeremonyNotFound);
    };
    if consumed_at.is_some() {
        return Err(DomainAuthError::CeremonyReplayed);
    }
    if expires_at <= now {
        return Err(DomainAuthError::CeremonyExpired);
    }
    let user_id = user_id.ok_or(DomainAuthError::InvalidState)?;

    Ok(Ceremony {
        id,
        user_id,
        credential_token_id,
        state,
        expires_at,
    })
}

/// Atomically consumes a ceremony and returns its stored state.
///
/// The statement is the single-use gate: only an unconsumed, unexpired row is
/// updated, and the caller's transaction rolls the update back if any later
/// write fails. Zero rows are classified into a replay, an expiry or a missing
/// ceremony so the route can answer `409` (or `404` for an unknown id).
pub async fn consume_ceremony(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    purpose: CeremonyPurpose,
    now: DateTime<Utc>,
) -> Result<Ceremony, DomainAuthError> {
    let row: Option<CeremonyConsumeRow> = sqlx::query_as(
        "UPDATE ceremonies SET consumed_at = $3 \
         WHERE id = $1 AND purpose = $2 \
           AND consumed_at IS NULL AND expires_at > $3 \
         RETURNING user_id, credential_token_id, state, expires_at",
    )
    .bind(id)
    .bind(purpose.as_str())
    .bind(now)
    .fetch_optional(&mut **tx)
    .await?;

    let Some((user_id, credential_token_id, state, expires_at)) = row else {
        return Err(classify_ceremony_rejection(tx, id, purpose, now).await?);
    };
    let user_id = user_id.ok_or(DomainAuthError::InvalidState)?;

    Ok(Ceremony {
        id,
        user_id,
        credential_token_id,
        state,
        expires_at,
    })
}

/// Finds out why the conditional ceremony update matched nothing.
async fn classify_ceremony_rejection(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    purpose: CeremonyPurpose,
    now: DateTime<Utc>,
) -> Result<DomainAuthError, DbError> {
    let row: Option<(DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT expires_at, consumed_at FROM ceremonies WHERE id = $1 AND purpose = $2",
    )
    .bind(id)
    .bind(purpose.as_str())
    .fetch_optional(&mut **tx)
    .await?;

    Ok(match row {
        None => DomainAuthError::CeremonyNotFound,
        Some((_, Some(_))) => DomainAuthError::CeremonyReplayed,
        Some((expires_at, None)) if expires_at <= now => DomainAuthError::CeremonyExpired,
        Some((_, None)) => DomainAuthError::CeremonyReplayed,
    })
}

/// Returns the owner's id when the single account exists.
pub async fn find_owner(executor: &mut PgConnection) -> Result<Option<Uuid>, DomainAuthError> {
    let id: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users")
        .fetch_optional(&mut *executor)
        .await?;
    Ok(id.map(|(id,)| id))
}

/// Whether the owner already has at least one Passkey.
///
/// `bootstrap/finish` uses this to refuse adding a credential to an existing
/// account even when a stale bootstrap token is still live. This is the
/// account-takeover guard from the T13 review: only the first registration is
/// allowed through the bootstrap flow.
pub async fn owner_has_passkey(
    executor: &mut PgConnection,
    user_id: Uuid,
) -> Result<bool, DomainAuthError> {
    let (exists,): (bool,) =
        sqlx::query_as("SELECT EXISTS (SELECT 1 FROM passkey_credentials WHERE user_id = $1)")
            .bind(user_id)
            .fetch_one(&mut *executor)
            .await?;
    Ok(exists)
}

/// Loads the account's Passkeys for an authentication challenge.
pub async fn load_passkeys(
    executor: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<Passkey>, DomainAuthError> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT passkey FROM passkey_credentials \
         WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user_id)
    .fetch_all(&mut *executor)
    .await?;

    rows.into_iter()
        .map(|value| serde_json::from_value(value).map_err(|_| DomainAuthError::InvalidState))
        .collect()
}

/// Persists a freshly registered Passkey and returns its row id.
///
/// `credential_id` stores the raw bytes (unique per installation) and `passkey`
/// the serialised `webauthn-rs` credential, including its counter. Transports
/// are the browser's hints, not a security property.
pub async fn insert_passkey(
    executor: &mut PgConnection,
    user_id: Uuid,
    passkey: &Passkey,
    name: &str,
    transports: &[String],
    now: DateTime<Utc>,
) -> Result<Uuid, DomainAuthError> {
    let id = Uuid::new_v4();
    let value = serde_json::to_value(passkey).map_err(|_| DomainAuthError::InvalidState)?;
    sqlx::query(
        "INSERT INTO passkey_credentials \
         (id, user_id, credential_id, passkey, transports, name, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(user_id)
    .bind(passkey.cred_id().as_slice())
    .bind(value)
    .bind(transports)
    .bind(name)
    .bind(now)
    .execute(&mut *executor)
    .await?;
    Ok(id)
}

/// Applies an authentication result to the stored Passkey.
///
/// The credential is located by id, its counter/backup flags are updated via
/// `webauthn-rs`, and `last_used_at` records the successful login. Returns
/// `false` when the credential row no longer exists, so the route can reject the
/// login instead of minting a session for a deleted Passkey.
pub async fn update_passkey_after_authentication(
    executor: &mut PgConnection,
    result: &AuthenticationResult,
    now: DateTime<Utc>,
) -> Result<bool, DomainAuthError> {
    let row: Option<(Uuid, Value)> =
        sqlx::query_as("SELECT id, passkey FROM passkey_credentials WHERE credential_id = $1")
            .bind(result.cred_id().as_slice())
            .fetch_optional(&mut *executor)
            .await?;

    let Some((id, value)) = row else {
        return Ok(false);
    };
    let mut passkey: Passkey =
        serde_json::from_value(value).map_err(|_| DomainAuthError::InvalidState)?;
    if passkey.update_credential(result).is_none() {
        return Ok(false);
    }
    let updated = serde_json::to_value(&passkey).map_err(|_| DomainAuthError::InvalidState)?;
    sqlx::query("UPDATE passkey_credentials SET passkey = $2, last_used_at = $3 WHERE id = $1")
        .bind(id)
        .bind(updated)
        .bind(now)
        .execute(&mut *executor)
        .await?;
    Ok(true)
}

/// Consumes the bootstrap token bound to a ceremony.
///
/// Re-exported here so the route can finish the ceremony through one module
/// without confusing token and ceremony concerns. The conditional update and
/// owner handling stay in [`super::bootstrap`], including the `owner_created`
/// distinction required by the T13 review.
pub async fn consume_ceremony_token(
    tx: &mut Transaction<'_, Postgres>,
    ceremony: &Ceremony,
    now: DateTime<Utc>,
) -> Result<super::bootstrap::BootstrapOutcome, DomainAuthError> {
    let token_id = ceremony
        .credential_token_id
        .ok_or(DomainAuthError::InvalidState)?;
    consume_bootstrap_token(tx, token_id, now).await
}
