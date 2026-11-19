//! Full-text publication jobs.
//!
//! The protocol (projection, ledger, engine adapter) lives in
//! `knoted_core::search::meili` so it is testable without the worker crate; this
//! module only registers the job kinds with the shared queue dispatcher.
use knoted_core::{
    context::AppContext,
    jobs::{ExecutionOutcome, Job, Lease},
};

pub use knoted_core::search::meili::{CLEANUP_JOB_KIND, PUBLISH_JOB_KIND};

pub async fn execute(ctx: &AppContext, lease: &Lease, job: &Job) -> ExecutionOutcome {
    knoted_core::search::meili::execute(ctx, lease, job).await
}

/// A publication only holds in-flight HTTP requests; dropping the future is the
/// exit, so the cancellation is always confirmed.
pub async fn cancel_and_wait(ctx: &AppContext, lease: &Lease) -> bool {
    knoted_core::search::meili::cancel_and_wait(ctx, lease).await
}

/// Runs independently of file scanning so application records can be indexed
/// even with no source directory configured.
pub async fn project(ctx: AppContext, mut stop: tokio::sync::watch::Receiver<()>) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while super::next_tick(&mut interval, &mut stop).await {
        let Ok(Some(admission)) = ctx.try_write().await else {
            continue;
        };
        if knoted_core::search::service::project_pending(&ctx)
            .await
            .is_err()
        {
            eprintln!("knoted-worker: full-text source projection temporarily unavailable");
        }
        if admission.finish().await.is_err() {
            eprintln!("knoted-worker: maintenance completion could not be recorded");
        }
    }
}
