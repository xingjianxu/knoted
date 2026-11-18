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
