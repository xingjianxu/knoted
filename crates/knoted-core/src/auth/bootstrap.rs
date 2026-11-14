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
//! Only the newest token per purpose is live: issuing supersedes the previous
//! unconsumed token of the same purpose, so running the CLI twice leaves exactly
//! one usable token and an earlier printed token cannot be replayed.
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

/// Result of [`ensure_owner`].
///
/// `created` tells the caller whether this call inserted the account or reused
/// an existing one. The bootstrap flow needs that distinction so T14 can refuse
/// `bootstrap/finish` when an owner with Passkeys already exists instead of
/// letting a stale token add a Passkey to the existing account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerAccount {
    /// The one owner account.
    pub user: User,
    /// True when this call inserted the owner row.
    pub created: bool,
}

/// Result of [`consume_bootstrap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootstrapOutcome {
    /// The owner account the token resolved to.
    pub user: User,
    /// True when this call created the owner; false when it reused one.
    pub owner_created: bool,
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
    #[error("the WebAuthn relying-party configuration is invalid")]
    Configuration,
    #[error("the WebAuthn credential could not be verified")]
    CredentialRejected,
    #[error("the ceremony does not exist")]
    CeremonyNotFound,
    #[error("the ceremony has already been used")]
    CeremonyReplayed,
    #[error("the ceremony has expired")]
    CeremonyExpired,
    #[error("the account already has a Passkey; a recovery token is required")]
    BootstrapAlreadyCompleted,
    #[error("this Passkey is already registered")]
    DuplicateCredential,
    #[error("the last Passkey of the account cannot be removed")]
    LastCredential,
    #[error("the Passkey does not exist")]
    PasskeyNotFound,
    #[error("a Passkey verification within the last five minutes is required")]
    RecentVerificationRequired,
    #[error("stored authentication state is invalid")]
    InvalidState,
    #[error(transparent)]
    Database(#[from] DbError),
}

impl From<sqlx::Error> for DomainAuthError {
    fn from(error: sqlx::Error) -> Self {
        DomainAuthError::Database(DbError::from(error))
    }
}

impl DomainAuthError {
    /// Stable machine-readable reason for logs, never containing the token.
    pub fn reason(&self) -> &'static str {
        match self {
            DomainAuthError::InvalidToken => "invalid_token",
            DomainAuthError::ExpiredToken => "expired_token",
            DomainAuthError::ConsumedToken => "consumed_token",
            DomainAuthError::Conflict => "conflict",
            DomainAuthError::Configuration => "configuration_error",
            DomainAuthError::CredentialRejected => "credential_rejected",
            DomainAuthError::CeremonyNotFound => "ceremony_not_found",
            DomainAuthError::CeremonyReplayed => "ceremony_replayed",
            DomainAuthError::CeremonyExpired => "ceremony_expired",
            DomainAuthError::BootstrapAlreadyCompleted => "bootstrap_already_completed",
            DomainAuthError::DuplicateCredential => "duplicate_credential",
            DomainAuthError::LastCredential => "last_credential",
            DomainAuthError::PasskeyNotFound => "passkey_not_found",
            DomainAuthError::RecentVerificationRequired => "recent_verification_required",
            DomainAuthError::InvalidState => "invalid_state",
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
/// copy/paste into a URL, a shell argument, a cookie and a JSON body unchanged.
/// Shared with [`super::sessions`], which issues the same shape of secret.
pub(crate) fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Issues a new one-time token valid for [`TOKEN_TTL`].
///
/// At most one token per purpose is live: issuing a token consumes every
/// earlier unconsumed, unexpired token of the same purpose in the same
/// transaction. Running the CLI twice therefore leaves exactly one usable
/// token, so a token printed earlier cannot be replayed by someone who saw it.
/// Expired rows are left untouched; they are already unusable and keep their
/// truthful "expired, not consumed" state.
///
/// A bootstrap token is refused with [`DomainAuthError::Conflict`] once the
/// owner exists, which is what keeps the CLI from offering a second way to
/// create the account. The check runs inside the same `INSERT`, so two
/// concurrent CLI invocations cannot both slip past a read-then-write race.
///
/// Business-time decisions (expiry) use `now`, the injected clock; the row's
/// `created_at` is also written from `now`, while `users.created_at` uses the
/// database default because the account row is not created here.
pub async fn issue_token(
    pool: &PgPool,
    purpose: TokenPurpose,
    now: DateTime<Utc>,
) -> Result<IssuedToken, DomainAuthError> {
    let id = Uuid::new_v4();
    let token = generate_token();
    let hash = hash_token(&token);
    let expires_at = now + TOKEN_TTL;

    let mut tx = pool.begin().await.map_err(DbError::from)?;

    // Serialise issuance per purpose. The supersede below only sees committed
    // rows, so without a lock two concurrent CLI runs could each leave a live
    // token. The advisory lock is transaction-scoped, released on commit or
    // rollback, and never blocks a different purpose (a hash collision only
    // adds harmless serialisation).
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('knoted.credential_tokens'), hashtext($1))")
        .bind(purpose.as_str())
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;

    // Supersede the previous live token of the same purpose. The UPDATE and the
    // INSERT share a transaction, so a refused bootstrap does not invalidate the
    // caller's existing token either.
    sqlx::query(
        "UPDATE credential_tokens SET consumed_at = $2 \
         WHERE purpose = $1 AND consumed_at IS NULL AND expires_at > $2",
    )
    .bind(purpose.as_str())
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(DbError::from)?;

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
    .fetch_optional(&mut *tx)
    .await
    .map_err(DbError::from)?;

    let Some((created_at,)) = row else {
        // The insert was filtered out, which for a bootstrap token means an
        // owner exists. Roll back so the supersede above is undone and the
        // operator keeps the token it already has.
        tx.rollback().await.map_err(DbError::from)?;
        return Err(DomainAuthError::Conflict);
    };

    tx.commit().await.map_err(DbError::from)?;

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
) -> Result<BootstrapOutcome, DomainAuthError> {
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

    ensure_owner(tx)
        .await
        .map(|owner| BootstrapOutcome {
            user: owner.user,
            owner_created: owner.created,
        })
        .map_err(DomainAuthError::from)
}

/// Consumes a bootstrap token by its stored row id.
///
/// `bootstrap/options` verifies the plaintext token, binds its row id to the
/// ceremony (`ceremonies.credential_token_id`) and never keeps the plaintext.
/// `bootstrap/finish` therefore has only the id to work with, while still
/// needing the same single-use, expiry-checked consumption as
/// [`consume_bootstrap`]. The conditional `UPDATE` is the authority: it burns the
/// token and ensures the owner in the caller's transaction, so a failure rolls
/// both back together.
pub async fn consume_bootstrap_token(
    tx: &mut Transaction<'_, Postgres>,
    token_id: Uuid,
    now: DateTime<Utc>,
) -> Result<BootstrapOutcome, DomainAuthError> {
    let consumed: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE credential_tokens SET consumed_at = $2 \
         WHERE id = $1 AND purpose = 'bootstrap' \
           AND consumed_at IS NULL AND expires_at > $2 \
         RETURNING id",
    )
    .bind(token_id)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DbError::from)?;

    if consumed.is_none() {
        let error = classify_bootstrap_rejection_by_id(tx, token_id, now).await?;
        tracing::debug!(
            purpose = TokenPurpose::Bootstrap.as_str(),
            token_id = %token_id,
            reason = error.reason(),
            "credential token rejected"
        );
        return Err(error);
    }

    ensure_owner(tx)
        .await
        .map(|owner| BootstrapOutcome {
            user: owner.user,
            owner_created: owner.created,
        })
        .map_err(DomainAuthError::from)
}

/// Same classification as [`classify_bootstrap_rejection`], keyed by row id.
async fn classify_bootstrap_rejection_by_id(
    tx: &mut Transaction<'_, Postgres>,
    token_id: Uuid,
    now: DateTime<Utc>,
) -> Result<DomainAuthError, DbError> {
    let row: Option<(DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT expires_at, consumed_at FROM credential_tokens \
         WHERE id = $1 AND purpose = 'bootstrap'",
    )
    .bind(token_id)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(match row {
        None => DomainAuthError::InvalidToken,
        Some((_, Some(_))) => DomainAuthError::ConsumedToken,
        Some((expires_at, None)) if expires_at <= now => DomainAuthError::ExpiredToken,
        Some((_, None)) => DomainAuthError::ConsumedToken,
    })
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
///
/// `created_at` is the database clock (`DEFAULT now()`), not the injected
/// clock: the account row is infrastructure state and the injectable clock only
/// decides business expiry (which token is still valid). Tests that need a
/// deterministic account timestamp assert on relationships, not the instant.
pub async fn ensure_owner(tx: &mut Transaction<'_, Postgres>) -> Result<OwnerAccount, DbError> {
    let inserted: Option<(Uuid, DateTime<Utc>)> = sqlx::query_as(
        "INSERT INTO users (id) VALUES ($1) ON CONFLICT (singleton) DO NOTHING \
             RETURNING id, created_at",
    )
    .bind(Uuid::new_v4())
    .fetch_optional(&mut **tx)
    .await?;

    if let Some((id, created_at)) = inserted {
        return Ok(OwnerAccount {
            user: User { id, created_at },
            created: true,
        });
    }

    // The insert conflicted: another transaction (or an earlier call) owns the
    // row, so reuse it instead of creating a second account.
    let (id, created_at): (Uuid, DateTime<Utc>) =
        sqlx::query_as("SELECT id, created_at FROM users")
            .fetch_one(&mut **tx)
            .await?;
    Ok(OwnerAccount {
        user: User { id, created_at },
        created: false,
    })
}
