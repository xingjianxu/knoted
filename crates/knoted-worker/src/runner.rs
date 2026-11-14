//! Job runner: claim → execute/cancel → commit through the T08 state machine.
//!
//! The runner deliberately knows nothing about job kinds. It claims one job at a
//! time and hands the [`Lease`] to a [`Handler`], which executes the work and
//! reports an [`ExecutionOutcome`]. While the handler runs, the runner
//! heartbeats the lease. If the heartbeat fails, another worker has taken the
//! job over, so the handler's result is discarded instead of committed. If the
//! heartbeat observes `cancel_requested`, the handler is asked to cancel and
//! wait for the execution to exit; only a confirmed exit is committed, so the
//! execution slot is never released while a child may still be running.
//!
//! Retry policy, backoff and terminal-state rules live in
//! [`knoted_core::jobs::service`]; this module only drives them.

use std::future::Future;
use std::time::Duration;

use knoted_core::context::AppContext;
use knoted_core::db::DbError;
use knoted_core::jobs::service::{self, DomainJobError, ExecutionOutcome, FinishExecution};
use knoted_core::jobs::{self, HEARTBEAT_SECONDS, Job, JobOutcome, JobState, Lease};
use tokio::time::MissedTickBehavior;

/// Idle delay between claim attempts when the queue is empty or a claim failed.
pub const IDLE_BACKOFF: Duration = Duration::from_secs(1);

/// Interval between lease heartbeats.
///
/// The lease lasts 90 seconds, so a third of it leaves room for two delayed
/// heartbeats before another worker may take the job over.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(HEARTBEAT_SECONDS as u64);

/// Work a claimed job.
///
/// `execute` runs one attempt. `cancel_and_wait` is called when the database
/// reports `cancel_requested`; it must stop the execution and only return
/// `true` after the child process has exited, because the runner commits the
/// cancellation (and frees the slot) only on confirmation.
pub trait Handler: Send + Sync {
    /// Runs the work behind `lease`.
    fn execute<'a>(
        &'a self,
        lease: &'a Lease,
    ) -> impl Future<Output = ExecutionOutcome> + Send + 'a;

    /// Stops the execution and waits for it to exit.
    ///
    /// The default is for handlers without an external process: there is
    /// nothing left to wait for once the future is dropped.
    fn cancel_and_wait<'a>(&'a self, _lease: &'a Lease) -> impl Future<Output = bool> + Send + 'a {
        async { true }
    }
}

/// Closures handle jobs too, as long as the returned future owns its data.
///
/// The closure reports a [`JobOutcome`]; the runner maps it onto the richer
/// [`ExecutionOutcome`] (a `Failed` closure result is permanent, because a
/// closure has no way to describe a retryable failure).
impl<F, Fut> Handler for F
where
    F: Fn(&Lease) -> Fut + Send + Sync,
    Fut: Future<Output = JobOutcome> + Send + 'static,
{
    async fn execute<'a>(&'a self, lease: &'a Lease) -> ExecutionOutcome {
        match self(lease).await {
            JobOutcome::Succeeded => ExecutionOutcome::Succeeded,
            JobOutcome::Failed { error } => ExecutionOutcome::PermanentFailure { message: error },
            JobOutcome::Cancelled => ExecutionOutcome::Cancelled,
        }
    }
}

/// What one [`WorkerRunner::step`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum StepOutcome {
    /// The attempt was committed; `job` is the row after the transition, which
    /// may be `retry_wait` rather than terminal.
    Finished {
        job: Box<Job>,
        outcome: ExecutionOutcome,
    },
    /// A cancellation request reached the handler, but the execution exit was
    /// not confirmed. The lease is deliberately left in place: the slot stays
    /// allocated until a later attempt observes and confirms the exit.
    CancellationPending { job_id: uuid::Uuid },
    /// No job was due.
    Idle,
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
    #[error(transparent)]
    Job(#[from] DomainJobError),
}

/// Result of driving the handler while watching the lease.
enum Attempt {
    /// The handler produced an outcome.
    Ready(ExecutionOutcome),
    /// A cancellation request was observed in the database.
    CancelRequested,
    /// Another worker took the job over.
    LeaseLost,
}

/// Claims and processes jobs one at a time for a single worker identity.
pub struct WorkerRunner<H> {
    ctx: AppContext,
    worker: String,
    handler: H,
    idle_backoff: Duration,
    heartbeat_interval: Duration,
}

impl<H: Handler> WorkerRunner<H> {
    /// Creates a runner for `worker` with the system clock.
    ///
    /// The name is stored in `jobs.locked_by` and shows up in diagnostics.
    pub fn new(pool: sqlx::PgPool, worker: impl Into<String>, handler: H) -> Self {
        Self::with_context(AppContext::new(pool), worker, handler)
    }

    /// Creates a runner with an explicit context, so tests can inject a fixed
    /// clock and retry scheduling stays deterministic.
    pub fn with_context(ctx: AppContext, worker: impl Into<String>, handler: H) -> Self {
        WorkerRunner {
            ctx,
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

    /// Claims one job, drives it through the state machine and reports what
    /// happened.
    ///
    /// Unlike [`WorkerRunner::run_once`], this reports a pending cancellation
    /// separately from an idle queue and returns the committed job row.
    pub async fn step(&self) -> Result<StepOutcome, RunnerError> {
        let Some(lease) = jobs::claim(self.ctx.pool(), &self.worker).await? else {
            return Ok(StepOutcome::Idle);
        };

        // A takeover of a cancellation request must not execute the work again;
        // it continues where the dead worker stopped: confirming the exit.
        if lease.state == JobState::CancelRequested {
            return self.confirm_cancel(&lease).await;
        }

        let outcome = match self.drive_handler(&lease).await? {
            Attempt::Ready(outcome) => outcome,
            Attempt::CancelRequested => return self.confirm_cancel(&lease).await,
            Attempt::LeaseLost => return Err(RunnerError::LeaseLost { lease }),
        };

        // The handler may have finished between two heartbeats. Check once more
        // so a cancellation that arrived in that window is not overwritten by a
        // result the state machine would have to reject anyway.
        if outcome != ExecutionOutcome::Cancelled {
            match jobs::heartbeat_state(self.ctx.pool(), &lease).await? {
                None => return Err(RunnerError::LeaseLost { lease }),
                Some(JobState::CancelRequested) => {
                    return self.confirm_cancel(&lease).await;
                }
                Some(_) => {}
            }
        }

        self.commit(&lease, outcome, false).await
    }

    /// Claims one job and processes it.
    ///
    /// This is a compatibility view over [`WorkerRunner::step`] for callers that
    /// only need the reported outcome: every non-terminal result (a scheduled
    /// retry or a pending cancellation) looks like `None`, exactly like an idle
    /// queue. New code should use `step` and inspect [`StepOutcome`].
    pub async fn run_once(&self) -> Result<Option<JobOutcome>, RunnerError> {
        match self.step().await? {
            StepOutcome::Finished { outcome, .. } => Ok(Some(legacy_outcome(outcome))),
            StepOutcome::CancellationPending { .. } | StepOutcome::Idle => Ok(None),
        }
    }

    /// Runs the handler while keeping the lease alive and watching for
    /// cancellation.
    async fn drive_handler(&self, lease: &Lease) -> Result<Attempt, RunnerError> {
        let handler = self.handler.execute(lease);
        tokio::pin!(handler);

        let mut ticker = tokio::time::interval(self.heartbeat_interval);
        // A handler that blocks the runtime must not make the heartbeat fire in
        // a burst afterwards: missed ticks are simply late.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // The first tick of an interval completes immediately.
        ticker.tick().await;

        loop {
            tokio::select! {
                outcome = &mut handler => return Ok(Attempt::Ready(outcome)),
                _ = ticker.tick() => {
                    match jobs::heartbeat_state(self.ctx.pool(), lease).await? {
                        None => return Ok(Attempt::LeaseLost),
                        Some(JobState::CancelRequested) => return Ok(Attempt::CancelRequested),
                        Some(_) => {}
                    }
                }
            }
        }
    }

    /// Asks the handler to stop and commits the cancellation only once the
    /// execution confirms its exit.
    async fn confirm_cancel(&self, lease: &Lease) -> Result<StepOutcome, RunnerError> {
        if !self.handler.cancel_and_wait(lease).await {
            // No confirmation: keep the lease. Returning without finishing is
            // what preserves the slot; the lease is recovered by a takeover
            // after it expires, which repeats the same confirmation attempt.
            return Ok(StepOutcome::CancellationPending {
                job_id: lease.job_id,
            });
        }

        self.commit(lease, ExecutionOutcome::Cancelled, true).await
    }

    /// Applies the state machine and maps a still-pending cancellation back to
    /// [`StepOutcome::CancellationPending`] instead of reporting it finished.
    async fn commit(
        &self,
        lease: &Lease,
        outcome: ExecutionOutcome,
        cancel_confirmed: bool,
    ) -> Result<StepOutcome, RunnerError> {
        let job = service::finish_execution(
            &self.ctx,
            FinishExecution {
                job_id: lease.job_id,
                lease_token: lease.token,
                outcome: outcome.clone(),
                cancel_confirmed,
            },
        )
        .await
        .map_err(|error| runner_error(lease, error))?;

        if job.state == JobState::CancelRequested.as_str() {
            return Ok(StepOutcome::CancellationPending { job_id: job.id });
        }

        Ok(StepOutcome::Finished {
            job: Box::new(job),
            outcome,
        })
    }

    /// Claims and processes jobs forever, backing off while the queue is empty
    /// or the database is unavailable.
    ///
    /// Only cancellation (shutdown) ends this loop; failures are reported and
    /// retried after [`WorkerRunner::with_idle_backoff`] instead of ending the
    /// process.
    pub async fn run(&self) {
        loop {
            match self.step().await {
                Ok(StepOutcome::Finished { .. }) => {}
                Ok(StepOutcome::CancellationPending { .. }) | Ok(StepOutcome::Idle) => {
                    tokio::time::sleep(self.idle_backoff).await;
                }
                Err(error) => {
                    eprintln!("knoted-worker {}: {error}", self.worker);
                    tokio::time::sleep(self.idle_backoff).await;
                }
            }
        }
    }
}

/// Maps the runner's richer outcome onto the T07 `JobOutcome` view.
fn legacy_outcome(outcome: ExecutionOutcome) -> JobOutcome {
    match outcome {
        ExecutionOutcome::Succeeded => JobOutcome::Succeeded,
        ExecutionOutcome::Cancelled => JobOutcome::Cancelled,
        ExecutionOutcome::TemporaryFailure { message }
        | ExecutionOutcome::PermanentFailure { message } => JobOutcome::Failed { error: message },
    }
}

/// Preserves [`RunnerError::LeaseLost`] for the lease errors the runner already
/// handles, and keeps database failures in their own variant.
fn runner_error(lease: &Lease, error: DomainJobError) -> RunnerError {
    match error {
        DomainJobError::LeaseLost { .. } => RunnerError::LeaseLost {
            lease: lease.clone(),
        },
        DomainJobError::Database(error) => RunnerError::Database(error),
        other => RunnerError::Job(other),
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
