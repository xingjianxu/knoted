//! Passkey credential management and account recovery.
//!
//! T14 owns the WebAuthn cryptography and the ceremony primitives; this module
//! owns the account-level rules around an already verified credential: which
//! Passkeys the owner has, adding one, removing one, and the single transaction
//! that replaces every credential and session during recovery.
//!
//! Two invariants live here:
//!
//! - An authenticated account must keep at least one Passkey, so removing the
//!   last one is refused. Recovery is the only flow allowed to replace the whole
//!   set, because it installs the new credential first and deletes the old ones
//!   afterwards in the same transaction.
//! - A sensitive credential operation requires a Passkey verification within
//!   [`CREDENTIAL_VERIFICATION_WINDOW`] of the caller's injected clock.
//!
//! As with the rest of `knoted-core`, no Axum type appears here and every
//! expiry decision uses the caller's `now`.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;
use webauthn_rs::prelude::Passkey;

use crate::db::DbError;

use super::bootstrap::{DomainAuthError, TokenPurpose, TokenRecord};
use super::sessions::{self, CreatedSession, Session};
use super::webauthn::{CeremonyPurpose, consume_ceremony, find_owner};

/// How recently a session must have verified a Passkey for a credential write.
pub const CREDENTIAL_VERIFICATION_WINDOW: Duration = Duration::minutes(5);

/// One stored Passkey, without the server-side public key or counter state.
///
/// `credential_id` stays raw bytes here; the route encodes it as base64url,
/// which is the only representation the browser ever sees. The serialised
/// `webauthn-rs` credential (public key, counter, backup flags) is deliberately
/// absent from this type, so a listing cannot leak it by accident.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyRecord {
    /// Row id, used to delete the credential.
    pub id: Uuid,
    /// Raw WebAuthn credential id bytes.
    pub credential_id: Vec<u8>,
    /// Human-readable name chosen by the owner.
    pub name: String,
    /// Browser transport hints.
    pub transports: Vec<String>,
    /// When the credential was registered.
    pub created_at: DateTime<Utc>,
    /// When the credential last authenticated, if ever.
    pub last_used_at: Option<DateTime<Utc>>,
}

impl PasskeyRecord {
    /// The contract's `credential_id`: base64url without padding.
    ///
    /// URL-safe base64 is what a browser can compare against
    /// `PublicKeyCredential.rawId` without an extra transformation.
    pub fn credential_id_base64(&self) -> String {
        URL_SAFE_NO_PAD.encode(&self.credential_id)
    }
}

/// Columns read into a [`PasskeyRecord`], in struct order.
const PASSKEY_COLUMNS: &str = "id, credential_id, name, transports, created_at, last_used_at";

type PasskeyRow = (
    Uuid,
    Vec<u8>,
    String,
    Vec<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

fn row_to_record(row: PasskeyRow) -> PasskeyRecord {
    let (id, credential_id, name, transports, created_at, last_used_at) = row;
    PasskeyRecord {
        id,
        credential_id,
        name,
        transports,
        created_at,
        last_used_at,
    }
}

/// Rows returned by [`list_passkeys`] when the caller passes no limit.
pub const PASSKEY_PAGE_DEFAULT_LIMIT: u32 = 50;
/// Upper bound for [`list_passkeys`], so a listing can never scan the table.
pub const PASSKEY_PAGE_MAX_LIMIT: u32 = 100;

/// One page of the owner's Passkeys, newest first.
///
/// `next_cursor` is the keyset position `(created_at, id)` of the last returned
/// row, and is only set when another row exists. Encoding it for the contract's
/// opaque `next_cursor` string is a DTO concern and belongs to the route.
pub struct PasskeyList {
    /// The credentials in this page, newest first.
    pub items: Vec<PasskeyRecord>,
    /// Keyset position to resume after, or `None` on the last page.
    pub next_cursor: Option<(DateTime<Utc>, Uuid)>,
}

/// Lists one page of the owner's Passkeys, newest first.
///
/// The ordering is total (`created_at DESC, id DESC`) and the cursor is a
/// keyset position, so two credentials registered in the same instant are still
/// paged without overlap or gaps. The query reads one row beyond the page to
/// tell whether a next page exists, which is what makes `next_cursor` truthful
/// instead of a guess. A zero limit is clamped to one row and a limit above
/// [`PASSKEY_PAGE_MAX_LIMIT`] is capped, so a listing is always bounded.
pub async fn list_passkeys(
    executor: &mut PgConnection,
    user_id: Uuid,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    limit: Option<u32>,
) -> Result<PasskeyList, DomainAuthError> {
    let (cursor_created_at, cursor_id) = match cursor {
        Some((created_at, id)) => (Some(created_at), Some(id)),
        None => (None, None),
    };
    let limit = i64::from(
        limit
            .unwrap_or(PASSKEY_PAGE_DEFAULT_LIMIT)
            .clamp(1, PASSKEY_PAGE_MAX_LIMIT),
    );
    let rows: Vec<PasskeyRow> = sqlx::query_as(&format!(
        "SELECT {PASSKEY_COLUMNS} FROM passkey_credentials \
         WHERE user_id = $1 \
           AND ($2::timestamptz IS NULL \
                OR (created_at, id) < ($2::timestamptz, $3::uuid)) \
         ORDER BY created_at DESC, id DESC LIMIT $4"
    ))
    .bind(user_id)
    .bind(cursor_created_at)
    .bind(cursor_id)
    .bind(limit + 1)
    .fetch_all(&mut *executor)
    .await
    .map_err(DbError::from)?;
    let mut items: Vec<PasskeyRecord> = rows.into_iter().map(row_to_record).collect();
    let next_cursor = if items.len() as i64 > limit {
        items.truncate(limit as usize);
        items.last().map(|record| (record.created_at, record.id))
    } else {
        None
    };
    Ok(PasskeyList { items, next_cursor })
}

/// Stores a newly registered Passkey and returns its record.
///
/// The caller has already verified the credential with `webauthn-rs`; this
/// function only persists it. A `credential_id` that is already stored is
/// [`DomainAuthError::DuplicateCredential`], which the route answers with `409`:
/// the same authenticator cannot be registered twice, and the unique constraint
/// is the authority, so a concurrent double-finish cannot both win.
///
/// The executor is a connection rather than a pool so the caller owns the
/// transaction; pass `&mut *tx` to commit the credential with the consumed
/// ceremony, or a pooled connection for a standalone write.
pub async fn add_passkey(
    executor: &mut PgConnection,
    user_id: Uuid,
    passkey: &Passkey,
    name: &str,
    transports: &[String],
    now: DateTime<Utc>,
) -> Result<PasskeyRecord, DomainAuthError> {
    let id = Uuid::new_v4();
    let value = serde_json::to_value(passkey).map_err(|_| DomainAuthError::InvalidState)?;
    let inserted = sqlx::query(
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
    .await;

    if let Err(error) = inserted {
        let error = DbError::from(error);
        if is_unique_violation(&error) {
            return Err(DomainAuthError::DuplicateCredential);
        }
        return Err(DomainAuthError::Database(error));
    }

    Ok(PasskeyRecord {
        id,
        credential_id: passkey.cred_id().as_slice().to_vec(),
        name: name.to_owned(),
        transports: transports.to_vec(),
        created_at: now,
        last_used_at: None,
    })
}

/// Removes one of the owner's Passkeys.
///
/// The row is locked for the duration of the transaction, so two concurrent
/// removals cannot both count `2` and delete both credentials: the loser
/// re-reads the committed state, sees a single row and is refused with
/// [`DomainAuthError::LastCredential`]. A credential that belongs to another
/// account, or no longer exists, is [`DomainAuthError::PasskeyNotFound`], which
/// keeps the endpoint from confirming ids it does not own.
pub async fn remove_passkey(
    pool: &PgPool,
    user_id: Uuid,
    passkey_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), DomainAuthError> {
    let mut tx = pool.begin().await.map_err(DbError::from)?;
    let ids: Vec<(Uuid,)> =
        sqlx::query_as("SELECT id FROM passkey_credentials WHERE user_id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(DbError::from)?;

    if !ids.iter().any(|(id,)| *id == passkey_id) {
        tx.rollback().await.map_err(DbError::from)?;
        return Err(DomainAuthError::PasskeyNotFound);
    }
    if ids.len() <= 1 {
        tx.rollback().await.map_err(DbError::from)?;
        return Err(DomainAuthError::LastCredential);
    }

    sqlx::query("DELETE FROM passkey_credentials WHERE id = $1 AND user_id = $2")
        .bind(passkey_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
    tx.commit().await.map_err(DbError::from)?;

    tracing::info!(
        target: "knoted_core::auth",
        user_id = %user_id,
        passkey_id = %passkey_id,
        %now,
        "removed passkey"
    );
    Ok(())
}

/// The result of a completed recovery.
///
/// The new session is returned exactly like a login's, and the counts make the
/// replacement observable without a second query in the route.
#[derive(Debug)]
pub struct RecoveryOutcome {
    /// The freshly created, verified session.
    pub session: CreatedSession,
    /// How many old credentials were deleted.
    pub removed_credentials: u64,
    /// How many old sessions were revoked.
    pub revoked_sessions: u64,
}

/// The credential a recovery installs, with the metadata the row needs.
///
/// Grouped so the recovery call stays readable: the new credential, its name and
/// its transport hints always belong together.
pub struct NewPasskey<'a> {
    /// The verified WebAuthn credential.
    pub passkey: &'a Passkey,
    /// Name chosen by the owner.
    pub name: &'a str,
    /// Browser transport hints, already filtered to the contract's enum.
    pub transports: &'a [String],
}

/// Replaces every Passkey and session of the owner in one transaction.
///
/// The order is deliberate and is the whole safety property of recovery:
///
/// 1. the Recovery ceremony is consumed (single use, five-minute expiry);
/// 2. the ceremony must be bound to the same recovery token the finish request
///    presented, so a ceremony cannot be replayed with another token;
/// 3. the recovery token is consumed (single use, ten-minute expiry);
/// 4. the new credential is inserted, and only then are the old credentials
///    deleted;
/// 5. every old session is revoked and a new verified session is created.
///
/// Every step runs in the caller's transaction, which the route commits only
/// after this function returns; the caller rolls it back on any error, so the
/// owner never loses the working credential or session because a recovery
/// attempt failed half way.
pub async fn complete_recovery(
    tx: &mut Transaction<'_, Postgres>,
    token: &TokenRecord,
    ceremony_id: Uuid,
    new_passkey: &NewPasskey<'_>,
    user_agent: Option<&str>,
    now: DateTime<Utc>,
) -> Result<RecoveryOutcome, DomainAuthError> {
    if token.purpose != TokenPurpose::Recovery {
        return Err(DomainAuthError::InvalidToken);
    }

    let ceremony = consume_ceremony(tx, ceremony_id, CeremonyPurpose::Recovery, now).await?;
    if ceremony.credential_token_id != Some(token.id) {
        return Err(DomainAuthError::InvalidToken);
    }
    let Some(owner_id) = find_owner(&mut *tx).await? else {
        return Err(DomainAuthError::InvalidToken);
    };
    if owner_id != ceremony.user_id {
        return Err(DomainAuthError::InvalidToken);
    }

    consume_recovery_token(tx, token.id, now).await?;

    let record = add_passkey(
        &mut *tx,
        owner_id,
        new_passkey.passkey,
        new_passkey.name,
        new_passkey.transports,
        now,
    )
    .await?;
    let removed_credentials =
        sqlx::query("DELETE FROM passkey_credentials WHERE user_id = $1 AND id <> $2")
            .bind(owner_id)
            .bind(record.id)
            .execute(&mut **tx)
            .await
            .map_err(DbError::from)?
            .rows_affected();

    let revoked_sessions = sessions::revoke_all_sessions_for_user(&mut *tx, owner_id, now).await?;
    let session = sessions::create_session(&mut *tx, owner_id, now, user_agent, true).await?;

    tracing::info!(
        target: "knoted_core::auth",
        user_id = %owner_id,
        passkey_id = %record.id,
        removed_credentials,
        revoked_sessions,
        "completed account recovery"
    );

    Ok(RecoveryOutcome {
        session,
        removed_credentials,
        revoked_sessions,
    })
}

/// Requires a Passkey verification within [`CREDENTIAL_VERIFICATION_WINDOW`].
///
/// A missing verification, one older than the window, or one with a future
/// timestamp (clock skew) is [`DomainAuthError::RecentVerificationRequired`],
/// which the route answers with `403 recent_verification_required`.
pub fn require_recent_verification(
    session: &Session,
    now: DateTime<Utc>,
) -> Result<(), DomainAuthError> {
    if sessions::recently_verified(session, now, CREDENTIAL_VERIFICATION_WINDOW) {
        Ok(())
    } else {
        Err(DomainAuthError::RecentVerificationRequired)
    }
}

/// Consumes a recovery token by row id, with the same single-use semantics as
/// the bootstrap token.
async fn consume_recovery_token(
    tx: &mut Transaction<'_, Postgres>,
    token_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), DomainAuthError> {
    let consumed: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE credential_tokens SET consumed_at = $2 \
         WHERE id = $1 AND purpose = 'recovery' \
           AND consumed_at IS NULL AND expires_at > $2 \
         RETURNING id",
    )
    .bind(token_id)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DbError::from)?;
    if consumed.is_some() {
        return Ok(());
    }

    let row: Option<(DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT expires_at, consumed_at FROM credential_tokens \
         WHERE id = $1 AND purpose = 'recovery'",
    )
    .bind(token_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DbError::from)?;

    let error = match row {
        None => DomainAuthError::InvalidToken,
        Some((_, Some(_))) => DomainAuthError::ConsumedToken,
        Some((expires_at, None)) if expires_at <= now => DomainAuthError::ExpiredToken,
        Some((_, None)) => DomainAuthError::ConsumedToken,
    };
    tracing::debug!(
        target: "knoted_core::auth",
        token_id = %token_id,
        reason = error.reason(),
        "recovery token rejected"
    );
    Err(error)
}

/// Whether a database error is a unique-constraint violation.
fn is_unique_violation(error: &DbError) -> bool {
    matches!(
        error,
        DbError::Sqlx(sqlx::Error::Database(database))
            if database.code().as_deref() == Some("23505")
    )
}
