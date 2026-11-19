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
        let Some(admission) = self.ctx.try_write().await.map_err(DbError::from)? else {
            return Ok(StepOutcome::Idle);
        };
        let result = self.step_admitted().await;
        admission.finish().await.map_err(DbError::from)?;
        result
    }

    async fn step_admitted(&self) -> Result<StepOutcome, RunnerError> {
        let Some(lease) = jobs::claim(self.ctx.pool(), &self.worker).await? else {
            return Ok(StepOutcome::Idle);
        };

        self.process_lease(lease).await
    }

    async fn process_lease(&self, lease: Lease) -> Result<StepOutcome, RunnerError> {
        // A takeover of a cancellation request must not execute the work again;
        // it continues where the dead worker stopped: confirming the exit.
        if lease.state == JobState::CancelRequested {
            return self.confirm_cancel(&lease).await;
        }

        let attempt = match self.drive_handler(&lease).await {
            Ok(attempt) => attempt,
            Err(error) => {
                let _ = self.handler.cancel_and_wait(&lease).await;
                return Err(error);
            }
        };
        let outcome = match attempt {
            Attempt::Ready(outcome) => outcome,
            Attempt::CancelRequested => return self.confirm_cancel(&lease).await,
            Attempt::LeaseLost => {
                let _ = self.handler.cancel_and_wait(&lease).await;
                return Err(RunnerError::LeaseLost { lease });
            }
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
    /// only need the reported outcome: only a terminal result (`succeeded`,
    /// `failed`, `cancelled`) is returned. A scheduled retry and a pending
    /// cancellation both look like `None`, exactly like an idle queue, because
    /// the job did not reach a final state. New code should use `step` and
    /// inspect [`StepOutcome`].
    pub async fn run_once(&self) -> Result<Option<JobOutcome>, RunnerError> {
        match self.step().await? {
            StepOutcome::Finished { job, outcome } => match JobState::parse(&job.state) {
                Some(state) if state.is_terminal() => Ok(Some(legacy_outcome(outcome))),
                _ => Ok(None),
            },
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
                    // A publishing handler may hold the jobs row lock. Keep
                    // driving it while heartbeat SQL waits for that same row;
                    // otherwise neither future can release the other's wait.
                    // No detached task: returning/dropping drive_handler also
                    // drops the in-flight heartbeat and handler futures.
                    tokio::select! {
                        outcome = &mut handler => return Ok(Attempt::Ready(outcome)),
                        state = jobs::heartbeat_state(self.ctx.pool(), lease) => {
                            match state? {
                                None => return Ok(Attempt::LeaseLost),
                                Some(JobState::CancelRequested) => return Ok(Attempt::CancelRequested),
                                Some(_) => {}
                            }
                        }
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

        self.finish_confirmed_cancel(lease).await
    }

    /// Writes `cancelled` after the handler confirmed the exit.
    async fn finish_confirmed_cancel(&self, lease: &Lease) -> Result<StepOutcome, RunnerError> {
        let job = self.write(lease, ExecutionOutcome::Cancelled, true).await?;
        if job.state == JobState::CancelRequested.as_str() {
            // The confirmation write missed (for example the lease moved
            // between the read and the write): keep the slot and let a later
            // attempt retry the confirmation.
            return Ok(StepOutcome::CancellationPending { job_id: job.id });
        }
        Ok(StepOutcome::Finished {
            job: Box::new(job),
            outcome: ExecutionOutcome::Cancelled,
        })
    }

    /// Applies the state machine and maps a still-pending cancellation back to
    /// [`StepOutcome::CancellationPending`] instead of reporting it finished.
    async fn commit(
        &self,
        lease: &Lease,
        outcome: ExecutionOutcome,
        cancel_confirmed: bool,
    ) -> Result<StepOutcome, RunnerError> {
        let job = self.write(lease, outcome.clone(), cancel_confirmed).await?;

        if job.state == JobState::CancelRequested.as_str() {
            if cancel_confirmed {
                return Ok(StepOutcome::CancellationPending { job_id: job.id });
            }
            // A cancellation arrived after the last heartbeat. The handler has
            // finished, but the contract still needs an explicit exit
            // confirmation before the lease may be released.
            if !self.handler.cancel_and_wait(lease).await {
                return Ok(StepOutcome::CancellationPending { job_id: job.id });
            }
            return self.finish_confirmed_cancel(lease).await;
        }

        Ok(StepOutcome::Finished {
            job: Box::new(job),
            outcome,
        })
    }

    /// Sends one outcome to the state machine.
    async fn write(
        &self,
        lease: &Lease,
        outcome: ExecutionOutcome,
        cancel_confirmed: bool,
    ) -> Result<Job, RunnerError> {
        service::finish_execution(
            &self.ctx,
            FinishExecution {
                job_id: lease.job_id,
                lease_token: lease.token,
                outcome,
                cancel_confirmed,
            },
        )
        .await
        .map_err(|error| runner_error(lease, error))
    }

    /// Claims and processes jobs forever, backing off while the queue is empty
    /// or the database is unavailable.
    ///
    /// Only cancellation (shutdown) ends this loop; failures are reported and
    /// retried after [`WorkerRunner::with_idle_backoff`] instead of ending the
    /// process.
    pub async fn run(&self) {
        self.run_until(async {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM listener");
            tokio::select! { _ = term.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        })
        .await;
    }

    /// Stop admission before dropping the active future, then consult the
    /// durable attempts owned by this worker. No Drop future is an exit proof.
    pub async fn run_until(&self, shutdown: impl Future<Output = ()> + Send) {
        tokio::pin!(shutdown);
        loop {
            // Finish admission and claim even when the signal arrives between
            // sending SQL and receiving its result. The returned lease then
            // identifies exactly which execution may require cancellation.
            let result = match self.ctx.try_write().await {
                Ok(Some(guard)) => {
                    let claimed = tokio::select! {
                        biased;
                        _ = &mut shutdown => {
                            self.shutdown_admitted(guard).await;
                            return;
                        }
                        // Never cancel a claim already sent to PostgreSQL.
                        _ = std::future::ready(()) => jobs::claim(self.ctx.pool(), &self.worker).await,
                    };
                    let result = match claimed {
                        Ok(Some(lease)) => tokio::select! {
                            biased;
                            _ = &mut shutdown => {
                                self.shutdown_admitted(guard).await;
                                return;
                            }
                            result = self.process_lease(lease) => result,
                        },
                        Ok(None) => Ok(StepOutcome::Idle),
                        Err(error) => Err(RunnerError::Database(error)),
                    };
                    match guard.finish().await {
                        Ok(()) => result,
                        Err(e) => Err(RunnerError::Database(e.into())),
                    }
                }
                Ok(None) => Ok(StepOutcome::Idle),
                Err(e) => Err(RunnerError::Database(e.into())),
            };
            if let Err(error) = &result {
                eprintln!("worker attempt failed: {error}");
            }
            if !matches!(result, Ok(StepOutcome::Finished { .. })) {
                tokio::select! {
                    _ = &mut shutdown => {
                        if let Ok(Some(guard)) = self.ctx.try_write().await {
                            self.shutdown_admitted(guard).await;
                        }
                        return;
                    }
                    _ = tokio::time::sleep(self.idle_backoff) => {}
                }
            }
        }
    }

    async fn shutdown_admitted(&self, guard: knoted_core::maintenance::WriteGuard) {
        let owned: Result<Vec<Job>, _> =
            sqlx::query_as("SELECT * FROM jobs WHERE locked_by=$1 AND lease_token IS NOT NULL")
                .bind(&self.worker)
                .fetch_all(self.ctx.pool())
                .await;
        let Ok(jobs) = owned else {
            return;
        };
        for job in jobs {
            if let Some(lease) = job.lease()
                && !self.handler.cancel_and_wait(&lease).await
            {
                // The durable mark prevents backup even if this process exits.
                return;
            }
        }
        if guard.finish().await.is_err() {
            eprintln!("knoted-worker: shutdown completion could not be recorded");
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
