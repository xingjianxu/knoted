//! T13 acceptance tests: the single owner account and the one-time CLI
//! credential tokens that bootstrap it.
//!
//! The tests run against the real local PostgreSQL test database with a
//! per-test schema, so the default parallel harness is safe. A missing database
//! is a failure, never a skip.
//!
//! Covered here: repeated bootstrap never creates a second account, expired and
//! consumed tokens are rejected, a token never reaches the logs (including the
//! CLI path and the error `Debug`/`Display`), and a bootstrap token is refused
//! once the owner exists while recovery tokens stay available. WebAuthn
//! verification and HTTP auth routes are T14/T15 and deliberately absent.

use std::sync::OnceLock;

use chrono::{Duration, TimeZone, Utc};
use knoted_api::admin::{self, TokenCommand};
use knoted_api::logging::{self, LogBuffer};
use knoted_core::auth::{self, DomainAuthError, TokenPurpose};
use knoted_core::config::{AppConfig, LogFormat, LogLevel, ProcessKind};
use knoted_core::db::testing::{TestDb, test_database_url};
use sqlx::PgPool;
use uuid::Uuid;

const UNREACHABLE_HINT: &str = "the local PostgreSQL test database must be running \
     (KNOTED_TEST_DATABASE_URL, default postgres:///knoted_test?host=/run/postgresql)";

async fn test_db() -> TestDb {
    TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"))
}

fn fixed_now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 2, 3, 4, 5, 6)
        .single()
        .expect("the fixed instant is valid")
}

async fn owner_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(pool)
        .await
        .expect("the users table must be readable")
}

/// The test database URL with the isolated schema first on `search_path`.
///
/// The CLI loads its own pool from a configuration file, so the test passes the
/// schema through PostgreSQL's `options` parameter instead of reusing the
/// `TestDb` pool. Without it the CLI would write to the shared `public` schema.
fn database_url_in_schema(db: &TestDb) -> String {
    let base = test_database_url();
    let separator = if base.contains('?') { '&' } else { '?' };
    format!(
        "{base}{separator}options[search_path]={},public",
        db.schema()
    )
}

/// Process-wide log sink, installed at most once so adding logging tests cannot
/// make the suite order-dependent.
static LOG_BUFFER: OnceLock<LogBuffer> = OnceLock::new();

fn captured_logs() -> &'static LogBuffer {
    LOG_BUFFER.get_or_init(|| {
        let buffer = LogBuffer::new();
        let sink = buffer.clone();
        logging::init_with_writer(move || sink.clone(), LogLevel::Info, LogFormat::Text)
            .expect("the test logging subscriber must install cleanly");
        buffer
    })
}

#[tokio::test]
async fn t13_1_repeated_bootstrap_creates_single_owner() {
    let db = test_db().await;
    let now = fixed_now();
    let pool = &db.pool;

    // Two tokens are issued before either is consumed. Only the newest one is
    // live: issuing supersedes the earlier token in the same transaction, so a
    // leaked or replayed first token cannot authorise anything.
    let first = auth::issue_token(pool, TokenPurpose::Bootstrap, now)
        .await
        .expect("the first bootstrap token must be issued");
    let second = auth::issue_token(pool, TokenPurpose::Bootstrap, now)
        .await
        .expect("a second bootstrap token must still be issued before an owner exists");

    assert!(
        matches!(
            auth::verify_token(pool, TokenPurpose::Bootstrap, first.plaintext(), now).await,
            Err(DomainAuthError::ConsumedToken)
        ),
        "issuing a newer token must supersede the earlier bootstrap token"
    );
    let mut tx = pool.begin().await.expect("a transaction must start");
    assert!(
        matches!(
            auth::consume_bootstrap(&mut tx, first.plaintext(), now).await,
            Err(DomainAuthError::ConsumedToken)
        ),
        "the superseded token must not create the owner"
    );
    tx.rollback()
        .await
        .expect("the failed consume must roll back");

    let mut tx = pool.begin().await.expect("a transaction must start");
    let outcome = auth::consume_bootstrap(&mut tx, second.plaintext(), now)
        .await
        .expect("the newest bootstrap token must create the owner");
    tx.commit()
        .await
        .expect("the bootstrap transaction must commit");
    assert!(
        outcome.owner_created,
        "the first successful bootstrap must report that it created the owner"
    );

    // A second `ensure_owner` reuses the row instead of inserting another one.
    let mut tx = pool.begin().await.expect("a transaction must start");
    let reused = auth::ensure_owner(&mut tx)
        .await
        .expect("ensuring the owner again must succeed");
    tx.commit()
        .await
        .expect("the ensure transaction must commit");
    assert!(!reused.created, "an existing owner must be reused");
    assert_eq!(
        outcome.user.id, reused.user.id,
        "reuse must return the same owner id"
    );

    assert_eq!(
        owner_count(pool).await,
        1,
        "the users table must hold exactly one row"
    );
}

#[tokio::test]
async fn t13_2_expired_and_consumed_tokens_rejected() {
    let db = test_db().await;
    let now = fixed_now();
    let pool = &db.pool;

    let expired = auth::issue_token(pool, TokenPurpose::Bootstrap, now)
        .await
        .expect("a bootstrap token must be issued");
    sqlx::query("UPDATE credential_tokens SET expires_at = $1 WHERE id = $2")
        .bind(now - Duration::seconds(1))
        .bind(expired.id)
        .execute(pool)
        .await
        .expect("the test must be able to age the token");

    assert!(
        matches!(
            auth::verify_token(pool, TokenPurpose::Bootstrap, expired.plaintext(), now).await,
            Err(DomainAuthError::ExpiredToken)
        ),
        "an expired token must be rejected by verify_token"
    );

    let mut tx = pool.begin().await.expect("a transaction must start");
    assert!(
        matches!(
            auth::consume_bootstrap(&mut tx, expired.plaintext(), now).await,
            Err(DomainAuthError::ExpiredToken)
        ),
        "an expired token must be rejected by consume_bootstrap"
    );
    tx.rollback()
        .await
        .expect("the failed consume must roll back");

    let usable = auth::issue_token(pool, TokenPurpose::Bootstrap, now)
        .await
        .expect("a fresh bootstrap token must be issued");
    let mut tx = pool.begin().await.expect("a transaction must start");
    auth::consume_bootstrap(&mut tx, usable.plaintext(), now)
        .await
        .expect("the fresh token must be consumable");
    tx.commit()
        .await
        .expect("the bootstrap transaction must commit");

    assert!(
        matches!(
            auth::verify_token(pool, TokenPurpose::Bootstrap, usable.plaintext(), now).await,
            Err(DomainAuthError::ConsumedToken)
        ),
        "a consumed token must be rejected by verify_token"
    );

    let mut tx = pool.begin().await.expect("a transaction must start");
    assert!(
        matches!(
            auth::consume_bootstrap(&mut tx, usable.plaintext(), now).await,
            Err(DomainAuthError::ConsumedToken)
        ),
        "a consumed token must be rejected by consume_bootstrap"
    );
    tx.rollback()
        .await
        .expect("the failed consume must roll back");

    // Purpose is part of the token's identity: a recovery token must never open
    // the bootstrap flow, and vice versa.
    let recovery = auth::issue_token(pool, TokenPurpose::Recovery, now)
        .await
        .expect("a recovery token must be issued");
    assert!(
        matches!(
            auth::verify_token(pool, TokenPurpose::Bootstrap, recovery.plaintext(), now).await,
            Err(DomainAuthError::InvalidToken)
        ),
        "a recovery token must not be accepted for the bootstrap purpose"
    );
}

#[tokio::test]
async fn t13_3_token_never_logged() {
    let db = test_db().await;
    let logs = captured_logs();
    let now = fixed_now();
    let pool = &db.pool;

    let issued = auth::issue_token(pool, TokenPurpose::Bootstrap, now)
        .await
        .expect("a bootstrap token must be issued");
    let plaintext = issued.plaintext().to_owned();

    let debug = format!("{issued:?}");
    assert!(
        !debug.contains(&plaintext),
        "the issued token's Debug output must not contain the plaintext: {debug}"
    );

    // A failing verification takes the same error path an operator would see.
    let error = auth::verify_token(pool, TokenPurpose::Recovery, &plaintext, now)
        .await
        .expect_err("a bootstrap token must not verify for recovery");
    assert!(
        !format!("{error}").contains(&plaintext),
        "the error message must not echo the token"
    );
    assert!(
        !format!("{error:?}").contains(&plaintext),
        "the error Debug output must not echo the token"
    );

    // A token pasted into the wrong argument slot must not be echoed either.
    let usage_error =
        TokenCommand::parse(["--purpose", &plaintext, "--config", "/tmp/unused.toml"])
            .expect_err("a token is not a valid purpose");
    assert!(
        !format!("{usage_error}").contains(&plaintext),
        "a usage error must not echo the argument value"
    );

    // The CLI handler loads its own pool from a configuration file; point it at
    // this test's isolated schema and capture its stdout.
    let tempdir = tempfile::tempdir().expect("a temporary directory must be created");
    let config_path = tempdir.path().join("knoted.toml");
    std::fs::write(
        &config_path,
        format!(
            "[server]\nbind = \"127.0.0.1:0\"\n\n[database]\nurl = \"{}\"\n",
            database_url_in_schema(&db)
        ),
    )
    .expect("the test configuration must be written");
    let config = AppConfig::load_for_process(&config_path, ProcessKind::Api)
        .expect("the test configuration must load for the API process");
    let command = TokenCommand::parse([
        "--purpose",
        "bootstrap",
        "--config",
        config_path
            .to_str()
            .expect("the temporary path must be UTF-8"),
    ])
    .expect("the admin token arguments must parse");

    let mut output = Vec::new();
    admin::run_token_command(&command, &config, &mut output)
        .await
        .expect("the CLI token command must succeed");
    let output = String::from_utf8(output).expect("the CLI output must be UTF-8");
    let printed = output
        .lines()
        .find_map(|line| line.strip_prefix("token: "))
        .expect("the CLI must print the token on its own line")
        .to_owned();
    assert_eq!(
        output.matches(&printed).count(),
        1,
        "the CLI must print the token exactly once: {output}"
    );

    // The printed token is real: it must verify against the database.
    let record = auth::verify_token(pool, TokenPurpose::Bootstrap, &printed, Utc::now())
        .await
        .expect("the printed token must be usable");
    assert_eq!(record.purpose, TokenPurpose::Bootstrap);

    let logs = logs.contents();
    // Positive assertion: removing the issuance log would otherwise make the
    // redaction assertion pass vacuously.
    assert!(
        logs.contains("issued credential token"),
        "issuing a token must be logged without the secret: {logs}"
    );
    assert!(
        !logs.contains(&plaintext),
        "the issued token must never reach the logs: {logs}"
    );
    assert!(
        !logs.contains(&printed),
        "the CLI-issued token must never reach the logs: {logs}"
    );
}

#[tokio::test]
async fn t13_4_bootstrap_refused_when_owner_exists() {
    let db = test_db().await;
    let now = fixed_now();
    let pool = &db.pool;

    let bootstrap = auth::issue_token(pool, TokenPurpose::Bootstrap, now)
        .await
        .expect("the first bootstrap token must be issued");
    let mut tx = pool.begin().await.expect("a transaction must start");
    auth::consume_bootstrap(&mut tx, bootstrap.plaintext(), now)
        .await
        .expect("the bootstrap token must create the owner");
    tx.commit()
        .await
        .expect("the bootstrap transaction must commit");

    let error = auth::issue_token(pool, TokenPurpose::Bootstrap, now)
        .await
        .expect_err("bootstrap must be refused once the owner exists");
    assert!(
        matches!(error, DomainAuthError::Conflict),
        "the refusal must be an explicit domain conflict, got {error:?}"
    );
    let bootstrap_tokens: i64 =
        sqlx::query_scalar("SELECT count(*) FROM credential_tokens WHERE purpose = 'bootstrap'")
            .fetch_one(pool)
            .await
            .expect("credential_tokens must be readable");
    assert_eq!(
        bootstrap_tokens, 1,
        "a refused bootstrap must not leave a second token behind"
    );

    let recovery = auth::issue_token(pool, TokenPurpose::Recovery, now)
        .await
        .expect("recovery must stay available once the owner exists");
    let record = auth::verify_token(pool, TokenPurpose::Recovery, recovery.plaintext(), now)
        .await
        .expect("the recovery token must verify");
    assert_eq!(record.purpose, TokenPurpose::Recovery);
}

/// PostgreSQL SQLSTATE for a violated `CHECK` constraint.
fn is_check_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database) if database.code().as_deref() == Some("23514")
    )
}

#[tokio::test]
async fn t13_5_ceremony_token_binding_enforced() {
    let db = test_db().await;
    let now = fixed_now();
    let pool = &db.pool;
    let expires_at = now + Duration::minutes(5);

    // A recovery ceremony is whatever authorised it: without a token it must
    // not exist, so `recovery/finish` can re-check the same token in its
    // transaction instead of trusting the request body.
    let token = auth::issue_token(pool, TokenPurpose::Recovery, now)
        .await
        .expect("a recovery token must be issued");

    let error = sqlx::query(
        "INSERT INTO ceremonies (id, purpose, state, expires_at) \
         VALUES ($1, 'bootstrap', '{}'::jsonb, $2)",
    )
    .bind(Uuid::new_v4())
    .bind(expires_at)
    .execute(pool)
    .await
    .expect_err("a bootstrap ceremony without a credential token must be rejected");
    assert!(
        is_check_violation(&error),
        "the binding must be enforced by a CHECK constraint, got {error:?}"
    );

    // A login ceremony comes from a session, so attaching a CLI token is a
    // contradiction the database must refuse as well.
    let error = sqlx::query(
        "INSERT INTO ceremonies (id, purpose, credential_token_id, state, expires_at) \
         VALUES ($1, 'login', $2, '{}'::jsonb, $3)",
    )
    .bind(Uuid::new_v4())
    .bind(token.id)
    .bind(expires_at)
    .execute(pool)
    .await
    .expect_err("a login ceremony bound to a credential token must be rejected");
    assert!(
        is_check_violation(&error),
        "the binding must be enforced by a CHECK constraint, got {error:?}"
    );

    // The legal combinations still insert.
    sqlx::query(
        "INSERT INTO ceremonies (id, purpose, state, expires_at) \
         VALUES ($1, 'login', '{}'::jsonb, $2)",
    )
    .bind(Uuid::new_v4())
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("a login ceremony without a token must be accepted");
    sqlx::query(
        "INSERT INTO ceremonies (id, purpose, credential_token_id, state, expires_at) \
         VALUES ($1, 'recovery', $2, '{}'::jsonb, $3)",
    )
    .bind(Uuid::new_v4())
    .bind(token.id)
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("a recovery ceremony bound to its token must be accepted");

    let ceremonies: i64 = sqlx::query_scalar("SELECT count(*) FROM ceremonies")
        .fetch_one(pool)
        .await
        .expect("ceremonies must be readable");
    assert_eq!(ceremonies, 2, "only the two legal ceremonies must exist");
}

#[tokio::test]
async fn t13_6_concurrent_issue_keeps_single_live_token() {
    let db = test_db().await;
    let now = fixed_now();
    let pool = &db.pool;

    // Two CLI invocations race. The supersede only sees committed rows, so
    // issuance is serialised per purpose with a transaction-scoped advisory
    // lock; exactly one of the two tokens must stay usable.
    let (left, right) = tokio::join!(
        auth::issue_token(pool, TokenPurpose::Bootstrap, now),
        auth::issue_token(pool, TokenPurpose::Bootstrap, now),
    );
    let left = left.expect("concurrent issuance must not fail");
    let right = right.expect("concurrent issuance must not fail");

    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM credential_tokens WHERE purpose = 'bootstrap' \
         AND consumed_at IS NULL AND expires_at > $1",
    )
    .bind(now)
    .fetch_one(pool)
    .await
    .expect("credential_tokens must be readable");
    assert_eq!(live, 1, "at most one live token per purpose");

    let left_live = auth::verify_token(pool, TokenPurpose::Bootstrap, left.plaintext(), now)
        .await
        .is_ok();
    let right_live = auth::verify_token(pool, TokenPurpose::Bootstrap, right.plaintext(), now)
        .await
        .is_ok();
    assert!(
        left_live ^ right_live,
        "exactly one of the concurrently issued tokens must remain usable \
         (left={left_live}, right={right_live})"
    );
}
