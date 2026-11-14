//! Server-side sessions: the bearer token a browser holds after a successful
//! login, its sliding idle limit and its absolute limit.
//!
//! The plaintext token is a 256-bit CSPRNG value handed to the browser once; the
//! database stores only its SHA-256 digest, so a database read cannot be
//! replayed as a login. Every expiry decision takes `now` from the caller so the
//! injected clock decides, never `Utc::now()`.
//!
//! Two expiries apply at once. The idle limit (seven days) slides forward on
//! every request, while the absolute limit (30 days) never moves, so a session
//! cannot live forever just because it is used daily. A session is also only
//! valid until it is logged out or revoked; both are recorded in `revoked_at`.
//!
//! Transport details (cookie attributes, CSRF headers) live in
//! `knoted_api::middleware::auth`; this module is persistence only.
//!
//! Every function takes a `&mut PgConnection` instead of a pool, so the caller
//! owns the transaction boundary. T14 creates the session inside the ceremony's
//! transaction (Passkey + session commit or roll back together) and T16 replaces a
//! credential and revokes the old sessions in one transaction; neither needs a
//! second, nesting transaction from here.

use std::cmp::min;
use std::fmt;

use chrono::{DateTime, Duration, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::db::DbError;

use super::bootstrap::{generate_token, hash_token};

/// Cookie carrying the session token.
pub const SESSION_COOKIE: &str = "knoted_session";
/// Cookie carrying the CSRF token; readable by the frontend on purpose.
pub const CSRF_COOKIE: &str = "knoted_csrf";
/// Header that must repeat the CSRF token on every write request.
pub const CSRF_HEADER: &str = "x-csrf-token";
/// Idle lifetime: a session unused for this long is expired.
pub const SESSION_IDLE_TTL: Duration = Duration::days(7);
/// Absolute lifetime: a session never outlives this, however actively used.
pub const SESSION_ABSOLUTE_TTL: Duration = Duration::days(30);
/// Rows returned by [`list_sessions`] when the caller passes no limit.
pub const SESSION_PAGE_DEFAULT_LIMIT: u32 = 50;
/// Upper bound for [`list_sessions`], so a listing can never scan the table.
pub const SESSION_PAGE_MAX_LIMIT: u32 = 100;

/// A stored session.
///
/// `csrf_token_hash` is the digest the write guard compares against; it is never
/// returned to a client, and `Debug` omits it.
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    /// Row id, used to revoke or identify this session.
    pub id: Uuid,
    /// Account the session belongs to.
    pub user_id: Uuid,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// Instant of the last successful request.
    pub last_seen_at: DateTime<Utc>,
    /// Idle limit, renewed by every successful request.
    pub idle_expires_at: DateTime<Utc>,
    /// Absolute limit, fixed at creation.
    pub absolute_expires_at: DateTime<Utc>,
    /// Set once the session was logged out or revoked.
    pub revoked_at: Option<DateTime<Utc>>,
    /// Instant of the last Passkey verification, if any.
    pub last_verified_at: Option<DateTime<Utc>>,
    /// User agent recorded at creation, for the session list.
    pub user_agent: Option<String>,
    /// SHA-256 of the CSRF token; never leaves the server.
    pub csrf_token_hash: Vec<u8>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("created_at", &self.created_at)
            .field("last_seen_at", &self.last_seen_at)
            .field("idle_expires_at", &self.idle_expires_at)
            .field("absolute_expires_at", &self.absolute_expires_at)
            .field("revoked_at", &self.revoked_at)
            .field("last_verified_at", &self.last_verified_at)
            .field("user_agent", &self.user_agent)
            .finish_non_exhaustive()
    }
}

/// A freshly created session and the two plaintext secrets that authorise it.
///
/// This is the only moment the plaintext exists. `Debug` is implemented by hand
/// because the derived version would print both tokens into any log line that
/// formatted the value.
pub struct CreatedSession {
    /// The stored session.
    pub session: Session,
    /// Plaintext session token, to be placed in an `HttpOnly` cookie.
    pub token: String,
    /// Plaintext CSRF token, to be placed in a readable cookie and echoed in
    /// the [`CSRF_HEADER`] on writes.
    pub csrf_token: String,
}

impl fmt::Debug for CreatedSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreatedSession")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

/// Columns read into a [`Session`], in struct order.
const SESSION_COLUMNS: &str = "id, user_id, created_at, last_seen_at, idle_expires_at, \
     absolute_expires_at, revoked_at, last_verified_at, user_agent, csrf_token_hash";

type SessionRow = (
    Uuid,
    Uuid,
    DateTime<Utc>,
    DateTime<Utc>,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<String>,
    Vec<u8>,
);

fn row_to_session(row: SessionRow) -> Session {
    let (
        id,
        user_id,
        created_at,
        last_seen_at,
        idle_expires_at,
        absolute_expires_at,
        revoked_at,
        last_verified_at,
        user_agent,
        csrf_token_hash,
    ) = row;
    Session {
        id,
        user_id,
        created_at,
        last_seen_at,
        idle_expires_at,
        absolute_expires_at,
        revoked_at,
        last_verified_at,
        user_agent,
        csrf_token_hash,
    }
}

/// Creates a session for `user_id`.
///
/// Every write in this module takes a `&mut PgConnection` rather than a pool, so
/// the caller chooses the transaction boundary. T14 finishes a ceremony as one
/// transaction — consume the ceremony, consume the credential token, insert the
/// Passkey, create the session, revoke the replaced sessions — and T16 does the
/// same for recovery. Pass the transaction itself (`&mut tx`) or a pooled
/// connection (`&mut *pool.acquire().await?`); never open a second transaction
/// inside these functions.
///
/// `verified` records a Passkey verification in the same insert (T14 passes
/// `true` after a login ceremony), which is what lets T16 require a recent
/// verification before sensitive operations. Only digests are stored; the
/// plaintext tokens exist in the returned [`CreatedSession`] and nowhere else.
pub async fn create_session(
    executor: &mut PgConnection,
    user_id: Uuid,
    now: DateTime<Utc>,
    user_agent: Option<&str>,
    verified: bool,
) -> Result<CreatedSession, DbError> {
    let id = Uuid::new_v4();
    let token = generate_token();
    let csrf_token = generate_token();
    let idle_expires_at = now + SESSION_IDLE_TTL;
    let absolute_expires_at = now + SESSION_ABSOLUTE_TTL;
    let last_verified_at = verified.then_some(now);

    sqlx::query(
        "INSERT INTO sessions (id, user_id, token_hash, csrf_token_hash, created_at, last_seen_at, \
         idle_expires_at, absolute_expires_at, last_verified_at, user_agent) \
         VALUES ($1, $2, $3, $4, $5, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(user_id)
    .bind(hash_token(&token).as_slice())
    .bind(hash_token(&csrf_token).as_slice())
    .bind(now)
    .bind(idle_expires_at)
    .bind(absolute_expires_at)
    .bind(last_verified_at)
    .bind(user_agent)
    .execute(&mut *executor)
    .await?;

    tracing::info!(
        target: "knoted_core::auth",
        session_id = %id,
        user_id = %user_id,
        %idle_expires_at,
        %absolute_expires_at,
        "created session"
    );

    Ok(CreatedSession {
        session: Session {
            id,
            user_id,
            created_at: now,
            last_seen_at: now,
            idle_expires_at,
            absolute_expires_at,
            revoked_at: None,
            last_verified_at,
            user_agent: user_agent.map(str::to_owned),
            csrf_token_hash: hash_token(&csrf_token).to_vec(),
        },
        token,
        csrf_token,
    })
}

/// Resolves `token` to a live session, renewing its idle limit.
///
/// Returns `None` for an unknown, revoked or expired token. A live session gets
/// `last_seen_at = now` and `idle_expires_at = min(now + 7d, absolute_expires_at)`
/// written back; the renewal is capped so activity cannot extend the absolute
/// limit.
pub async fn resolve_session(
    executor: &mut PgConnection,
    token: &str,
    now: DateTime<Utc>,
) -> Result<Option<Session>, DbError> {
    let hash = hash_token(token);
    let row: Option<SessionRow> = sqlx::query_as(&format!(
        "SELECT {SESSION_COLUMNS} FROM sessions WHERE token_hash = $1"
    ))
    .bind(hash.as_slice())
    .fetch_optional(&mut *executor)
    .await?;

    let Some(row) = row else {
        tracing::debug!(
            target: "knoted_core::auth",
            reason = "unknown_token",
            "session rejected"
        );
        return Ok(None);
    };

    let mut session = row_to_session(row);
    if let Some(reason) = rejection_reason(&session, now) {
        tracing::debug!(
            target: "knoted_core::auth",
            session_id = %session.id,
            reason,
            "session rejected"
        );
        return Ok(None);
    }

    let idle_expires_at = min(now + SESSION_IDLE_TTL, session.absolute_expires_at);
    sqlx::query("UPDATE sessions SET last_seen_at = $2, idle_expires_at = $3 WHERE id = $1")
        .bind(session.id)
        .bind(now)
        .bind(idle_expires_at)
        .execute(&mut *executor)
        .await?;
    session.last_seen_at = now;
    session.idle_expires_at = idle_expires_at;

    Ok(Some(session))
}

/// Why a stored session must not be used at `now`.
fn rejection_reason(session: &Session, now: DateTime<Utc>) -> Option<&'static str> {
    if session.revoked_at.is_some() {
        return Some("revoked");
    }
    if session.idle_expires_at <= now {
        return Some("idle_expired");
    }
    if session.absolute_expires_at <= now {
        return Some("absolute_expired");
    }
    None
}

/// Revokes one session owned by `user_id`; returns whether it was live.
///
/// The account id is part of the predicate, exactly like [`revoke_session`], so a
/// session id from another account (or from a tampered request) cannot revoke
/// someone else's session. Idempotent: logging out twice is not an error, the
/// second call reports `false`. Other sessions of the same account, including
/// other devices, are untouched.
pub async fn logout(
    executor: &mut PgConnection,
    user_id: Uuid,
    session_id: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, DbError> {
    let revoked: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE sessions SET revoked_at = $3 \
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL RETURNING id",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(now)
    .fetch_optional(&mut *executor)
    .await?;

    if revoked.is_some() {
        tracing::info!(
            target: "knoted_core::auth",
            session_id = %session_id,
            user_id = %user_id,
            "revoked session"
        );
    }
    Ok(revoked.is_some())
}

/// Lists the account's live sessions, newest first.
///
/// Expired sessions (idle or absolute, judged against `now`) and revoked ones are
/// omitted, so the result is exactly what the browser may still use. `limit` is
/// clamped to `1..=SESSION_PAGE_MAX_LIMIT` and defaults to
/// `SESSION_PAGE_DEFAULT_LIMIT`; the ordering is total (`created_at DESC, id
/// DESC`), which is the stable order a keyset cursor needs.
///
/// The contract's `next_cursor` encoding is a DTO concern and belongs to the
/// route layer (T14); this function consumes the decoded `(created_at, id)` pair
/// and guarantees the bounded, deterministically ordered page the cursor is
/// derived from. Only rows strictly older in that total order are returned, so
/// walking page after page neither repeats nor skips a row.
pub async fn list_sessions(
    executor: &mut PgConnection,
    user_id: Uuid,
    now: DateTime<Utc>,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    limit: Option<u32>,
) -> Result<Vec<Session>, DbError> {
    let (cursor_created_at, cursor_id) = match cursor {
        Some((created_at, id)) => (Some(created_at), Some(id)),
        None => (None, None),
    };
    let limit = i64::from(
        limit
            .unwrap_or(SESSION_PAGE_DEFAULT_LIMIT)
            .clamp(1, SESSION_PAGE_MAX_LIMIT),
    );
    let rows: Vec<SessionRow> = sqlx::query_as(&format!(
        "SELECT {SESSION_COLUMNS} FROM sessions \
         WHERE user_id = $1 AND revoked_at IS NULL \
           AND idle_expires_at > $2 AND absolute_expires_at > $2 \
           AND ($3::timestamptz IS NULL \
                OR (created_at, id) < ($3::timestamptz, $4::uuid)) \
         ORDER BY created_at DESC, id DESC LIMIT $5"
    ))
    .bind(user_id)
    .bind(now)
    .bind(cursor_created_at)
    .bind(cursor_id)
    .bind(limit)
    .fetch_all(&mut *executor)
    .await?;
    Ok(rows.into_iter().map(row_to_session).collect())
}

/// Revokes a session belonging to `user_id`; returns whether it changed.
///
/// The account id is part of the predicate, so one account cannot revoke
/// another's session by guessing its id. Already revoked sessions are left as
/// they are, which makes a repeated delete a no-op instead of an error.
pub async fn revoke_session(
    executor: &mut PgConnection,
    user_id: Uuid,
    session_id: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, DbError> {
    let revoked: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE sessions SET revoked_at = $3 \
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL RETURNING id",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(now)
    .fetch_optional(&mut *executor)
    .await?;

    if revoked.is_some() {
        tracing::info!(
            target: "knoted_core::auth",
            session_id = %session_id,
            user_id = %user_id,
            "revoked session"
        );
    }
    Ok(revoked.is_some())
}

/// Revokes every live session of `user_id`; returns how many changed.
///
/// This is the recovery primitive: T16 replaces a lost Passkey and must end every
/// existing session at the same moment, inside the same transaction that installs
/// the new credential. Idempotent: a second call changes nothing and reports `0`.
pub async fn revoke_all_sessions_for_user(
    executor: &mut PgConnection,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> Result<u64, DbError> {
    let revoked = sqlx::query(
        "UPDATE sessions SET revoked_at = $2 WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(now)
    .execute(&mut *executor)
    .await?
    .rows_affected();

    tracing::info!(
        target: "knoted_core::auth",
        user_id = %user_id,
        revoked,
        "revoked every session of the account"
    );
    Ok(revoked)
}

/// Records a Passkey verification on a live session.
///
/// Returns `false` when the session is missing or already revoked, so the caller
/// can tell that the verification was not recorded instead of assuming success.
/// T14 calls this after a successful login/add-Passkey ceremony; T16 then gates
/// credential deletion and recovery on [`recently_verified`].
pub async fn mark_verified(
    executor: &mut PgConnection,
    session_id: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, DbError> {
    let updated = sqlx::query(
        "UPDATE sessions SET last_verified_at = $2 \
         WHERE id = $1 AND revoked_at IS NULL \
           AND idle_expires_at > $2 AND absolute_expires_at > $2",
    )
    .bind(session_id)
    .bind(now)
    .execute(&mut *executor)
    .await?
    .rows_affected();
    if updated == 0 {
        tracing::warn!(
            target: "knoted_core::auth",
            session_id = %session_id,
            "cannot mark a missing or revoked session as verified"
        );
    }
    Ok(updated > 0)
}

/// Whether the session verified a Passkey within `window` before `now`.
///
/// The window is exclusive: a verification exactly `window` ago is stale. A
/// timestamp in the future (clock skew) does not count either, so a skewed row
/// cannot widen the window.
pub fn recently_verified(session: &Session, now: DateTime<Utc>, window: Duration) -> bool {
    session
        .last_verified_at
        .is_some_and(|verified| verified <= now && now - verified < window)
}
