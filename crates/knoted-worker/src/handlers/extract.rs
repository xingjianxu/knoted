//! Rust text extraction uses the same queue and success publication fence.
use knoted_core::{
    context::AppContext,
    files::{
        extraction::{self, ExtractionInput},
        office, text,
    },
    jobs::{ExecutionOutcome, Lease},
};
pub async fn execute(
    ctx: &AppContext,
    lease: &Lease,
    job: &knoted_core::jobs::Job,
) -> ExecutionOutcome {
    if job.payload["parser_revision"].as_str() == Some(office::RETIRED_DOCX_PARSER_REVISION) {
        // Move this lease onto the runner's cancellation path. It keeps the
        // slot until every persisted execution has a matching exit receipt.
        loop {
            if sqlx::query(
                "UPDATE jobs SET state='cancel_requested',updated_at=now()
                 WHERE id=$1 AND lease_token=$2 AND state='running'",
            )
            .bind(lease.job_id)
            .bind(lease.token)
            .execute(ctx.pool())
            .await
            .is_ok()
            {
                // A non-cancel outcome makes the runner observe the durable
                // request and confirm it rather than reporting exit ourselves.
                return temporary("DOCX parser revision retired");
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
    if !ctx.files().enabled {
        return ExecutionOutcome::Cancelled;
    }
    let Ok(input) = ExtractionInput::from_job(job) else {
        return ExecutionOutcome::PermanentFailure {
            message: "invalid extraction envelope".into(),
        };
    };
    if let Some((parser_revision, options_revision)) = job
        .payload
        .get("parser_revision")
        .and_then(|v| v.as_str())
        .and_then(extraction::python_identity)
    {
        return execute_python(ctx, lease, &input, parser_revision, options_revision).await;
    }
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

async fn execute_python(
    ctx: &AppContext,
    lease: &Lease,
    input: &ExtractionInput,
    parser_revision: &str,
    options_revision: &str,
) -> ExecutionOutcome {
    use knoted_core::{
        files::pdf,
        intelligence::{ExecutionKind, ExecutionRequest, ExecutionState, IntelligenceClient},
    };
    let client = match IntelligenceClient::new(&ctx.intelligence().base_url) {
        Ok(client) => client,
        Err(_) => return temporary("intelligence configuration unavailable"),
    };
    let attempts = match extraction::load_attempts(ctx.pool(), lease.job_id).await {
        Ok(attempts) => attempts,
        Err(_) => return temporary("attempt ledger unavailable"),
    };
    for previous in attempts.iter().filter(|a| a.lease_token != lease.token) {
        if client.settle_execution(&previous.request).await.is_err() {
            return temporary("previous execution exit unconfirmed");
        }
    }
    let Ok(staged) = input.staged_object() else {
        return permanent("invalid staging input");
    };
    let request = ExecutionRequest {
        execution_id: uuid::Uuid::new_v4(),
        kind: ExecutionKind::Parse,
        input_object_id: staged.object_id,
        input_sha256: staged.sha256,
        source_version: input.content_version,
        deadline: ctx.now() + std::time::Duration::from_secs(900),
        options_revision: options_revision.into(),
    };
    let attempt = match extraction::register_attempt(ctx.pool(), lease, request).await {
        Ok(attempt) => attempt,
        Err(extraction::DomainError::Stale) => return ExecutionOutcome::Cancelled,
        Err(_) => return temporary("attempt registration unavailable"),
    };
    let request = &attempt.request;
    if client.create_execution(request).await.is_err() {
        return temporary("execution submission unconfirmed");
    }
    loop {
        let record = match client.get_execution(request.execution_id).await {
            Ok(record) => record,
            Err(_) => return temporary("execution observation unavailable"),
        };
        if record.verify_echo(request).is_err() {
            return permanent("execution echo mismatch");
        }
        if record.state.is_terminal() {
            if record.verify_resource_exit(request).is_err() {
                return temporary("execution exit unconfirmed");
            }
            if record.state != ExecutionState::Succeeded {
                let code = record
                    .error
                    .as_ref()
                    .map(|e| e.code.as_str())
                    .unwrap_or("execution_failed");
                return execution_failure(code);
            }
            if record.verify_fresh(request, ctx.now()).is_err() {
                return permanent("deadline_exceeded");
            }
            let result = match client.get_result(request.execution_id).await {
                Ok(result) => result,
                Err(_) => return temporary("result unavailable"),
            };
            let parsed = if request.options_revision == office::OPTIONS_REVISION {
                office::read_artifact(ctx, request, &result).await
            } else {
                pdf::read_artifact(ctx, request, &result).await
            };
            let document = match parsed {
                Ok(document) => document,
                Err(pdf::Error::Io(_)) => return temporary("artifact unavailable"),
                Err(_) => return permanent("invalid parse artifact"),
            };
            if office::format_for_parser(parser_revision)
                .is_some_and(|expected| document.source_format != expected)
            {
                return permanent("unsupported_format");
            }
            return match extraction::publish_python(ctx, lease, input, &document, &attempt).await {
                Ok(()) => ExecutionOutcome::Succeeded,
                Err(extraction::DomainError::Stale) => ExecutionOutcome::Cancelled,
                Err(extraction::DomainError::DeadlineExceeded) => permanent("deadline_exceeded"),
                Err(_) => temporary("extraction publication unavailable"),
            };
        }
        if ctx.now() >= request.deadline {
            return if client.settle_execution(request).await.is_ok() {
                permanent("deadline_exceeded")
            } else {
                temporary("deadline exit unconfirmed")
            };
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
fn execution_failure(code: &str) -> ExecutionOutcome {
    if matches!(
        code,
        "corrupt_input"
            | "unsupported_format"
            | "invalid_input"
            | "empty_input"
            | "input_too_large"
            | "office_budget_exceeded"
            | "image_too_large"
            | "too_many_pages"
            | "artifact_too_large"
            | "output_too_large"
            | "deadline_exceeded"
            | "input_digest_mismatch"
    ) {
        permanent(code)
    } else {
        temporary(code)
    }
}
fn temporary(message: &str) -> ExecutionOutcome {
    ExecutionOutcome::TemporaryFailure {
        message: message.into(),
    }
}
fn permanent(message: &str) -> ExecutionOutcome {
    ExecutionOutcome::PermanentFailure {
        message: message.into(),
    }
}

pub async fn cancel_and_wait(ctx: &AppContext, lease: &Lease) -> bool {
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        cancel_registered(ctx, lease),
    )
    .await
    .unwrap_or(false)
}
async fn cancel_registered(ctx: &AppContext, lease: &Lease) -> bool {
    use knoted_core::intelligence::IntelligenceClient;
    let Ok(attempts) = extraction::load_attempts(ctx.pool(), lease.job_id).await else {
        return false;
    };
    let Ok(client) = IntelligenceClient::new(&ctx.intelligence().base_url) else {
        return false;
    };
    let current: Option<uuid::Uuid> =
        match sqlx::query_scalar("SELECT lease_token FROM jobs WHERE id=$1")
            .bind(lease.job_id)
            .fetch_one(ctx.pool())
            .await
        {
            Ok(token) => token,
            Err(_) => return false,
        };
    for attempt in attempts
        .iter()
        .filter(|a| current == Some(lease.token) || a.lease_token == lease.token)
    {
        if client.settle_execution(&attempt.request).await.is_err() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t48_office_safe_codes_preserve_permanent_and_finite_retry_classes() {
        assert!(matches!(
            execution_failure("office_budget_exceeded"),
            ExecutionOutcome::PermanentFailure { .. }
        ));
        assert!(matches!(
            execution_failure("office_io_error"),
            ExecutionOutcome::TemporaryFailure { .. }
        ));
    }
    #[test]
    fn t47_machine_codes_preserve_permanent_input_limits_and_retryable_models() {
        for code in [
            "output_too_large",
            "too_many_pages",
            "image_too_large",
            "corrupt_input",
        ] {
            assert!(matches!(
                execution_failure(code),
                ExecutionOutcome::PermanentFailure { .. }
            ));
        }
        for code in [
            "models_missing",
            "execution_failed",
            "unrecognized_future_code",
        ] {
            assert!(matches!(
                execution_failure(code),
                ExecutionOutcome::TemporaryFailure { .. }
            ));
        }
    }
}
