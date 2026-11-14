//! Minimal job runner: claim → handle → (heartbeat while running) → finish.
//!
//! The runner deliberately knows nothing about job kinds. It claims one job at a
//! time and hands the [`Lease`] to a [`Handler`], which does the work and
//! returns a [`JobOutcome`]. While the handler runs, the runner heartbeats the
//! lease; if the heartbeat fails, another worker has taken the job over, so the
//! handler's result is discarded instead of committed. That rule is the reason
//! this type exists: forgetting a heartbeat would silently let two workers
//! commit the same job.
//!
//! Handlers for the real job kinds are added by later cards; the state machine
//! that decides retries sits between [`JobOutcome`] and the database (T08).

use std::future::Future;
use std::time::Duration;

use knoted_core::db::DbError;
use knoted_core::jobs::{self, HEARTBEAT_SECONDS, JobOutcome, Lease};
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;

/// Idle delay between claim attempts when the queue is empty or a claim failed.
pub const IDLE_BACKOFF: Duration = Duration::from_secs(1);

/// Interval between lease heartbeats.
///
/// The lease lasts 90 seconds, so a third of it leaves room for two delayed
/// heartbeats before another worker may take the job over.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(HEARTBEAT_SECONDS as u64);

/// Work a claimed job. Returning an outcome is not a commit: the runner commits
/// it only if the lease is still held.
pub trait Handler: Send + Sync {
    /// Future returned by [`Handler::handle`].
    type Run<'a>: Future<Output = JobOutcome> + Send + 'a
    where
        Self: 'a;

    /// Runs the job behind `lease`.
    ///
    /// The returned future may borrow `lease` and `self`; the runner keeps both
    /// alive until the future finishes or the lease is lost.
    fn handle<'a>(&'a self, lease: &'a Lease) -> Self::Run<'a>;
}

/// Closures handle jobs too, as long as the returned future owns its data.
///
/// A closure whose future borrows the lease cannot satisfy this blanket impl,
/// so such handlers clone the token or are written as a named type.
impl<F, Fut> Handler for F
where
    F: Fn(&Lease) -> Fut + Send + Sync,
    Fut: Future<Output = JobOutcome> + Send + 'static,
{
    type Run<'a>
        = Fut
    where
        Self: 'a;

    fn handle<'a>(&'a self, lease: &'a Lease) -> Fut {
        self(lease)
    }
}

/// Errors that stop the runner from completing a job.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(
        "lease {} for job {} was lost before completion; discarding the result",
        .lease.token,
        .lease.job_id
    )]
    LeaseLost { lease: Lease },
}

/// Claims and processes jobs one at a time for a single worker identity.
pub struct WorkerRunner<H> {
    pool: PgPool,
    worker: String,
    handler: H,
    idle_backoff: Duration,
    heartbeat_interval: Duration,
}

impl<H: Handler> WorkerRunner<H> {
    /// Creates a runner for `worker`; the name is stored in `jobs.locked_by`
    /// and shows up in diagnostics.
    pub fn new(pool: PgPool, worker: impl Into<String>, handler: H) -> Self {
        WorkerRunner {
            pool,
            worker: worker.into(),
            handler,
            idle_backoff: IDLE_BACKOFF,
            heartbeat_interval: HEARTBEAT_INTERVAL,
        }
    }

    /// Overrides the idle delay, which tests shorten.
    pub fn with_idle_backoff(mut self, idle_backoff: Duration) -> Self {
        self.idle_backoff = idle_backoff;
        self
    }

    /// Overrides the heartbeat interval, which tests shorten.
    pub fn with_heartbeat_interval(mut self, heartbeat_interval: Duration) -> Self {
        self.heartbeat_interval = heartbeat_interval;
        self
    }

    /// Claims one job and processes it.
    ///
    /// Returns `Ok(None)` when no job is due. `Err(RunnerError::LeaseLost)`
    /// means the job was taken over while the handler ran; its outcome was not
    /// written.
    pub async fn run_once(&self) -> Result<Option<JobOutcome>, RunnerError> {
        let Some(lease) = jobs::claim(&self.pool, &self.worker).await? else {
            return Ok(None);
        };

        let outcome = self.execute(&lease).await?;

        if !jobs::finish_with_token(&self.pool, &lease, outcome.clone()).await? {
            return Err(RunnerError::LeaseLost { lease });
        }

        Ok(Some(outcome))
    }

    /// Runs the handler while keeping the lease alive.
    async fn execute(&self, lease: &Lease) -> Result<JobOutcome, RunnerError> {
        let handler = self.handler.handle(lease);
        tokio::pin!(handler);

        let mut ticker = tokio::time::interval(self.heartbeat_interval);
        // A handler that blocks the runtime must not make the heartbeat fire in
        // a burst afterwards: missed ticks are simply late.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // The first tick of an interval completes immediately.
        ticker.tick().await;

        loop {
            tokio::select! {
                outcome = &mut handler => return Ok(outcome),
                _ = ticker.tick() => {
                    if !jobs::heartbeat(&self.pool, lease).await? {
                        return Err(RunnerError::LeaseLost {
                            lease: lease.clone(),
                        });
                    }
                }
            }
        }
    }

    /// Claims and processes jobs forever, backing off while the queue is empty
    /// or the database is unavailable.
    ///
    /// Only cancellation (shutdown) ends this loop; failures are reported and
    /// retried after [`WorkerRunner::with_idle_backoff`] instead of ending the
    /// process.
    pub async fn run(&self) {
        loop {
            match self.run_once().await {
                Ok(Some(_)) => {}
                Ok(None) => tokio::time::sleep(self.idle_backoff).await,
                Err(error) => {
                    eprintln!("knoted-worker {}: {error}", self.worker);
                    tokio::time::sleep(self.idle_backoff).await;
                }
            }
        }
    }
}

impl<H> std::fmt::Debug for WorkerRunner<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerRunner")
            .field("worker", &self.worker)
            .field("idle_backoff", &self.idle_backoff)
            .field("heartbeat_interval", &self.heartbeat_interval)
            .finish_non_exhaustive()
    }
}
