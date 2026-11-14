//! The single account and the one-time tokens that authorise creating or
//! recovering it.
//!
//! Knoted has exactly one owner and no public registration. An operator runs
//! `knoted-api admin token --purpose bootstrap` on the server, hands the printed
//! token to the browser once, and the browser exchanges it for a Passkey
//! ceremony. This module owns the token lifecycle; WebAuthn verification and
//! sessions are T14/T15, which is why the tables for them already exist in
//! `migrations/0003_t13_auth.sql` but no code here reads them.
//!
//! Secrets never leave this module: [`IssuedToken`] exposes the plaintext only
//! through [`IssuedToken::plaintext`] for the CLI to print, its `Debug` output
//! is redacted, and every error carries a fixed message instead of the token.
//! Only the SHA-256 digest is persisted.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::db::DbError;

/// How long an issued token stays valid.
pub const TOKEN_TTL: Duration = Duration::minutes(10);

/// What a credential token authorises.
///
/// The values are the database check constraint's vocabulary and are never
/// interchangeable: a recovery token cannot open the bootstrap flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenPurpose {
    /// Creates the owner account. Only issued while no owner exists.
    Bootstrap,
    /// Replaces the owner's Passkeys when the old ones are lost.
    Recovery,
}

impl TokenPurpose {
    /// Every purpose, in the order the CLI lists them.
    pub const ALL: [TokenPurpose; 2] = [TokenPurpose::Bootstrap, TokenPurpose::Recovery];

    /// The database value of this purpose.
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenPurpose::Bootstrap => "bootstrap",
            TokenPurpose::Recovery => "recovery",
        }
    }

    /// Parses a database or CLI value, rejecting anything else.
    pub fn parse(value: &str) -> Option<TokenPurpose> {
        match value {
            "bootstrap" => Some(TokenPurpose::Bootstrap),
            "recovery" => Some(TokenPurpose::Recovery),
            _ => None,
        }
    }
}

/// A freshly issued token: the only moment the plaintext exists.
///
/// `Debug` is implemented by hand because the derived version would print the
/// secret and any `tracing` call that formatted the value would leak it.
pub struct IssuedToken {
    /// Row id of the stored digest.
    pub id: Uuid,
    /// Purpose the token was issued for.
    pub purpose: TokenPurpose,
    /// Issue instant, as recorded in the database.
    pub created_at: DateTime<Utc>,
    /// Instant after which the token is rejected.
    pub expires_at: DateTime<Utc>,
    token: String,
}

impl IssuedToken {
    /// The one-time plaintext token. Only the CLI may print this.
    pub fn plaintext(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for IssuedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedToken")
            .field("id", &self.id)
            .field("purpose", &self.purpose)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Row shape of `credential_tokens` read by [`verify_token`].
type TokenRow = (
    Uuid,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

/// A stored token that passed every validity check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRecord {
    /// Row id of the stored digest.
    pub id: Uuid,
    /// Purpose the token was issued for.
    pub purpose: TokenPurpose,
    /// Issue instant.
    pub created_at: DateTime<Utc>,
    /// Instant after which the token is rejected.
    pub expires_at: DateTime<Utc>,
    /// Set once the token was consumed; always `None` here.
    pub consumed_at: Option<DateTime<Utc>>,
}

/// The single owner account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct User {
    /// Account id; stable for the lifetime of the installation.
    pub id: Uuid,
    /// When the account was first created.
    pub created_at: DateTime<Utc>,
}

/// Why a token was rejected or an account operation failed.
///
/// Every message is fixed: none contains the token, its digest or any database
/// row, so these errors are safe to log and to return from the CLI.
#[derive(Debug, thiserror::Error)]
pub enum DomainAuthError {
    #[error("the credential token is not valid")]
    InvalidToken,
    #[error("the credential token has expired")]
    ExpiredToken,
    #[error("the credential token has already been used")]
    ConsumedToken,
    #[error("the account already exists; a recovery token is required")]
    Conflict,
    #[error(transparent)]
    Database(#[from] DbError),
}

impl DomainAuthError {
    /// Stable machine-readable reason for logs, never containing the token.
    pub fn reason(&self) -> &'static str {
        match self {
            DomainAuthError::InvalidToken => "invalid_token",
            DomainAuthError::ExpiredToken => "expired_token",
            DomainAuthError::ConsumedToken => "consumed_token",
            DomainAuthError::Conflict => "conflict",
            DomainAuthError::Database(_) => "database_error",
        }
    }
}

/// SHA-256 digest of `token`.
///
/// Only this value is stored; the API and CLI never persist the plaintext.
pub fn hash_token(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// Generates a 256-bit token from the operating system CSPRNG.
///
/// The encoding is URL-safe base64 without padding, so the value survives a
/// copy/paste into a URL, a shell argument and a JSON body unchanged.
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Issues a new one-time token valid for [`TOKEN_TTL`].
///
/// A bootstrap token is refused with [`DbError::OwnerAlreadyExists`] once the
/// owner exists, which is what keeps the CLI from offering a second way to
/// create the account. The check runs inside the same `INSERT`, so two
/// concurrent CLI invocations cannot both slip past a read-then-write race.
pub async fn issue_token(
    pool: &PgPool,
    purpose: TokenPurpose,
    now: DateTime<Utc>,
) -> Result<IssuedToken, DbError> {
    let id = Uuid::new_v4();
    let token = generate_token();
    let hash = hash_token(&token);
    let expires_at = now + TOKEN_TTL;

    let row: Option<(DateTime<Utc>,)> = sqlx::query_as(
        "INSERT INTO credential_tokens (id, purpose, token_hash, created_at, expires_at) \
         SELECT $1, $2, $3, $4, $5 \
         WHERE $2 = 'recovery' OR NOT EXISTS (SELECT 1 FROM users) \
         RETURNING created_at",
    )
    .bind(id)
    .bind(purpose.as_str())
    .bind(hash.as_slice())
    .bind(now)
    .bind(expires_at)
    .fetch_optional(pool)
    .await?;

    let Some((created_at,)) = row else {
        // The insert was filtered out, which for a bootstrap token means an
        // owner exists. Nothing was written, so no second token remains.
        return Err(DbError::OwnerAlreadyExists);
    };

    tracing::info!(
        purpose = purpose.as_str(),
        token_id = %id,
        %expires_at,
        "issued credential token"
    );

    Ok(IssuedToken {
        id,
        purpose,
        created_at,
        expires_at,
        token,
    })
}

/// Verifies `plaintext` without consuming it.
///
/// Used by the options step of the WebAuthn ceremonies (T14): it establishes
/// that the operator-issued token is genuine before a challenge is created,
/// while [`consume_bootstrap`] is what burns it on finish. A token of the other
/// purpose is [`DomainAuthError::InvalidToken`].
pub async fn verify_token(
    pool: &PgPool,
    purpose: TokenPurpose,
    plaintext: &str,
    now: DateTime<Utc>,
) -> Result<TokenRecord, DomainAuthError> {
    let hash = hash_token(plaintext);
    let row: Option<TokenRow> = sqlx::query_as(
        "SELECT id, purpose, created_at, expires_at, consumed_at \
         FROM credential_tokens WHERE token_hash = $1 AND purpose = $2",
    )
    .bind(hash.as_slice())
    .bind(purpose.as_str())
    .fetch_optional(pool)
    .await
    .map_err(DbError::from)?;

    let Some((id, stored_purpose, created_at, expires_at, consumed_at)) = row else {
        tracing::debug!(
            purpose = purpose.as_str(),
            reason = DomainAuthError::InvalidToken.reason(),
            "credential token rejected"
        );
        return Err(DomainAuthError::InvalidToken);
    };

    if consumed_at.is_some() {
        tracing::debug!(
            purpose = purpose.as_str(),
            token_id = %id,
            reason = DomainAuthError::ConsumedToken.reason(),
            "credential token rejected"
        );
        return Err(DomainAuthError::ConsumedToken);
    }
    if expires_at <= now {
        tracing::debug!(
            purpose = purpose.as_str(),
            token_id = %id,
            reason = DomainAuthError::ExpiredToken.reason(),
            "credential token rejected"
        );
        return Err(DomainAuthError::ExpiredToken);
    }

    Ok(TokenRecord {
        id,
        purpose: TokenPurpose::parse(&stored_purpose).ok_or(DomainAuthError::InvalidToken)?,
        created_at,
        expires_at,
        consumed_at,
    })
}

/// Consumes a bootstrap token and returns the owner, creating it if needed.
///
/// Everything happens in the caller's transaction: the conditional `UPDATE`
/// marks the digest consumed, and only then is the owner ensured. A failed
/// ceremony that rolls back therefore leaves the token usable, and a repeated
/// finish with a second token returns the existing owner instead of creating a
/// second account.
pub async fn consume_bootstrap(
    tx: &mut Transaction<'_, Postgres>,
    plaintext: &str,
    now: DateTime<Utc>,
) -> Result<User, DomainAuthError> {
    let hash = hash_token(plaintext);
    // Single statement, single consumption: the predicate re-checks both the
    // expiration and `consumed_at` under the row lock, so two concurrent
    // finishes cannot both win.
    let consumed: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE credential_tokens SET consumed_at = $2 \
         WHERE token_hash = $1 AND purpose = 'bootstrap' \
           AND consumed_at IS NULL AND expires_at > $2 \
         RETURNING id",
    )
    .bind(hash.as_slice())
    .bind(now)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DbError::from)?;

    if consumed.is_none() {
        let error = classify_bootstrap_rejection(tx, hash.as_slice(), now).await?;
        tracing::debug!(
            purpose = TokenPurpose::Bootstrap.as_str(),
            reason = error.reason(),
            "credential token rejected"
        );
        return Err(error);
    }

    ensure_owner(tx).await.map_err(DomainAuthError::from)
}

/// Finds out why the conditional `UPDATE` matched nothing.
///
/// The reasons are distinguished here, after the fact, instead of reading the
/// row first and updating second: the update is the authority on whether the
/// token was consumable, so a concurrent consumer cannot make this path report
/// success.
async fn classify_bootstrap_rejection(
    tx: &mut Transaction<'_, Postgres>,
    hash: &[u8],
    now: DateTime<Utc>,
) -> Result<DomainAuthError, DbError> {
    let row: Option<(DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT expires_at, consumed_at FROM credential_tokens \
         WHERE token_hash = $1 AND purpose = 'bootstrap'",
    )
    .bind(hash)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(match row {
        None => DomainAuthError::InvalidToken,
        Some((_, Some(_))) => DomainAuthError::ConsumedToken,
        Some((expires_at, None)) if expires_at <= now => DomainAuthError::ExpiredToken,
        // The row exists, is unconsumed and unexpired yet the update lost:
        // another transaction consumed it between the two statements.
        Some((_, None)) => DomainAuthError::ConsumedToken,
    })
}

/// Returns the owner account, creating it when `users` is empty.
///
/// `ON CONFLICT (singleton) DO NOTHING` makes the insert idempotent even when
/// two transactions race, and `UNIQUE (singleton)`/`CHECK (singleton)` in the
/// migration make a second row impossible. The follow-up `SELECT` is therefore
/// guaranteed to see exactly the one owner.
pub async fn ensure_owner(tx: &mut Transaction<'_, Postgres>) -> Result<User, DbError> {
    sqlx::query("INSERT INTO users (id) VALUES ($1) ON CONFLICT (singleton) DO NOTHING")
        .bind(Uuid::new_v4())
        .execute(&mut **tx)
        .await?;

    let (id, created_at): (Uuid, DateTime<Utc>) =
        sqlx::query_as("SELECT id, created_at FROM users")
            .fetch_one(&mut **tx)
            .await?;
    Ok(User { id, created_at })
}
