//! Rust text extraction uses the same queue and success publication fence.
use knoted_core::{
    context::AppContext,
    files::{
        extraction::{self, ExtractionInput},
        text,
    },
    jobs::{ExecutionOutcome, Lease},
};
pub async fn execute(
    ctx: &AppContext,
    lease: &Lease,
    job: &knoted_core::jobs::Job,
) -> ExecutionOutcome {
    if !ctx.files().enabled {
        return ExecutionOutcome::Cancelled;
    }
    let Ok(input) = ExtractionInput::from_job(job) else {
        return ExecutionOutcome::PermanentFailure {
            message: "invalid extraction envelope".into(),
        };
    };
    let Ok(staged) = input.staged_object() else {
        return ExecutionOutcome::PermanentFailure {
            message: "invalid extraction input".into(),
        };
    };
    let document = match tokio::time::timeout(
        std::time::Duration::from_secs(900),
        text::extract_text(ctx, staged),
    )
    .await
    {
        Ok(Ok(doc)) => doc,
        Ok(Err(text::DomainError::Io(_))) => {
            return ExecutionOutcome::TemporaryFailure {
                message: "staging input unavailable".into(),
            };
        }
        Ok(Err(e)) => {
            return ExecutionOutcome::PermanentFailure {
                message: e.to_string(),
            };
        }
        Err(_) => {
            return ExecutionOutcome::PermanentFailure {
                message: "text extraction exceeded 900 seconds".into(),
            };
        }
    };
    match extraction::publish(ctx, lease, &input, &document).await {
        Ok(()) => ExecutionOutcome::Succeeded,
        Err(extraction::DomainError::Stale) => ExecutionOutcome::Cancelled,
        Err(_) => ExecutionOutcome::TemporaryFailure {
            message: "extraction publication unavailable".into(),
        },
    }
}
