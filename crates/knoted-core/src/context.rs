//! The process-wide application context shared by every route handler.
//!
//! `AppContext` owns the PostgreSQL pool and the clock, so handlers depend on
//! one injectable object instead of process globals. Tests replace the clock
//! with [`FixedClock`] to make time-dependent behaviour deterministic; the API
//! crate never constructs a second, parallel context.
//!
//! This module is transport agnostic: no Axum type appears here, per the
//! architecture rule that `knoted-core` owns persistence and the API crate owns
//! transport.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;

/// Source of the current time.
///
/// Implementations must be cheap and thread-safe: every request may call this.
pub trait Clock: Send + Sync {
    /// Current instant in UTC.
    fn now(&self) -> DateTime<Utc>;
}

/// Wall clock backed by the operating system.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock that only moves when a test moves it.
///
/// `FixedClock` is cloneable and every clone observes the same instant, so a
/// test can keep one handle while handlers hold a type-erased clone.
#[derive(Debug, Clone)]
pub struct FixedClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl FixedClock {
    /// Starts the clock at `now`.
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    /// Moves the clock forward by `by`.
    pub fn advance(&self, by: Duration) {
        let mut now = self.lock();
        *now += by;
    }

    /// Sets the clock to `now`, for tests that jump backwards or far ahead.
    pub fn set(&self, now: DateTime<Utc>) {
        *self.lock() = now;
    }

    fn lock(&self) -> MutexGuard<'_, DateTime<Utc>> {
        // A test that panics while holding the lock must not wedge the clock;
        // the stored instant is still a valid value.
        self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.lock()
    }
}

/// Shared state handed to every request handler.
#[derive(Clone)]
pub struct AppContext {
    pool: PgPool,
    clock: Arc<dyn Clock>,
    asset_root: std::path::PathBuf,
    files: crate::config::FilesConfig,
    intelligence: crate::config::IntelligenceConfig,
    search: crate::config::SearchConfig,
    vectors_enabled: bool,
    search_api_key: Option<String>,
    chat_providers: Option<Arc<crate::rag::providers::ProviderRegistry>>,
    // One admitted generation per instance. Keep its terminal receipt across
    // the worker runner dropping the provider future to drive cancellation.
    chat_terminal_receipt: Arc<Mutex<Option<(uuid::Uuid, uuid::Uuid)>>>,
    office: Option<Arc<crate::files::office::PreviewStore>>,
}

impl AppContext {
    /// Builds a context with the system clock.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            clock: Arc::new(SystemClock),
            asset_root: crate::config::AssetsConfig::default().root,
            files: crate::config::FilesConfig::default(),
            intelligence: crate::config::IntelligenceConfig::default(),
            search: crate::config::SearchConfig::default(),
            vectors_enabled: false,
            search_api_key: None,
            chat_providers: None,
            chat_terminal_receipt: Arc::default(),
            office: None,
        }
    }

    /// Replaces the clock, keeping the pool.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Injects private storage for an instance or isolated test.
    pub fn with_asset_root(mut self, root: impl Into<std::path::PathBuf>) -> Self {
        self.asset_root = root.into();
        self
    }

    pub fn asset_root(&self) -> &std::path::Path {
        &self.asset_root
    }

    pub fn with_intelligence(mut self, config: crate::config::IntelligenceConfig) -> Self {
        self.intelligence = config;
        self
    }
    pub fn intelligence(&self) -> &crate::config::IntelligenceConfig {
        &self.intelligence
    }

    pub fn with_files(mut self, config: crate::config::FilesConfig) -> Self {
        self.files = config;
        self
    }
    pub fn files(&self) -> &crate::config::FilesConfig {
        &self.files
    }

    pub fn with_vectors_enabled(mut self, enabled: bool) -> Self {
        self.vectors_enabled = enabled;
        self
    }
    pub fn vectors_enabled(&self) -> bool {
        self.vectors_enabled
    }

    pub fn with_search(mut self, config: crate::config::SearchConfig) -> Self {
        self.search = config;
        self
    }

    /// Full-text publication target; disabled by default.
    pub fn search(&self) -> &crate::config::SearchConfig {
        &self.search
    }

    pub fn with_chat_providers(
        mut self,
        providers: Arc<crate::rag::providers::ProviderRegistry>,
    ) -> Self {
        self.chat_providers = Some(providers);
        self
    }

    pub fn chat_providers(&self) -> Option<&crate::rag::providers::ProviderRegistry> {
        self.chat_providers.as_deref()
    }

    pub(crate) fn remember_chat_terminal(&self, lease: &crate::jobs::Lease) {
        *self
            .chat_terminal_receipt
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((lease.job_id, lease.token));
    }

    pub(crate) fn received_chat_terminal(&self, lease: &crate::jobs::Lease) -> bool {
        *self
            .chat_terminal_receipt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            == Some((lease.job_id, lease.token))
    }

    /// Installs the search engine's API key from the credential file. The value
    /// never appears in `Debug` output or logs.
    pub fn with_search_api_key(mut self, key: Option<String>) -> Self {
        self.search_api_key = key;
        self
    }

    pub fn search_api_key(&self) -> Option<&str> {
        self.search_api_key.as_deref()
    }

    /// Install the optional API-local Office state. An unreadable credential
    /// disables only Office previews; it does not prevent the API from starting.
    pub fn with_office(
        mut self,
        config: Option<&crate::config::OfficeConfig>,
        auth: &crate::config::AuthConfig,
    ) -> Self {
        self.office = config
            .and_then(|config| crate::files::office::PreviewStore::new(config, auth))
            .map(Arc::new);
        self
    }

    pub(crate) fn office(&self) -> Option<&Arc<crate::files::office::PreviewStore>> {
        self.office.as_ref()
    }

    /// The PostgreSQL pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Current instant according to the injected clock.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }
}

impl fmt::Debug for AppContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppContext")
            .field("pool", &self.pool)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::{AppContext, Clock, FixedClock};
    use chrono::{Duration, Utc};
    use sqlx::postgres::PgPoolOptions;

    fn lazy_pool() -> sqlx::PgPool {
        // Never connected: these tests only need a pool value, not a database.
        PgPoolOptions::new()
            .connect_lazy("postgres://knoted_test@127.0.0.1:1/knoted_test")
            .expect("a lazy pool is built without connecting")
    }

    #[test]
    fn t06_fixed_clock_advances_and_is_shared() {
        let start = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        let clock = FixedClock::new(start);
        let handle = clock.clone();

        assert_eq!(clock.now(), start);
        handle.advance(Duration::seconds(90));
        assert_eq!(clock.now(), start + Duration::seconds(90));
        assert_eq!(handle.now(), clock.now(), "clones share one instant");
        handle.set(start);
        assert_eq!(clock.now(), start);
    }

    #[tokio::test]
    async fn t06_app_context_exposes_injected_clock_and_pool() {
        let start = Utc.with_ymd_and_hms(2026, 6, 7, 8, 9, 10).unwrap();
        let fixed = std::sync::Arc::new(FixedClock::new(start));
        let pool = lazy_pool();
        let ctx = AppContext::new(pool.clone()).with_clock(fixed.clone());

        assert_eq!(ctx.now(), start);
        fixed.advance(Duration::minutes(5));
        assert_eq!(ctx.now(), start + Duration::minutes(5));
        assert_eq!(
            ctx.pool().options().get_max_connections(),
            pool.options().get_max_connections()
        );
    }
}
