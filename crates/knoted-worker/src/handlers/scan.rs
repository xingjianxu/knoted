//! Single dispatcher for the shared queue. Never run a competing kind-specific
//! runner: claim has no kind filter. New handlers belong in this match.
use crate::runner::{Handler, WorkerRunner};
use knoted_core::{
    context::AppContext,
    files::{
        notifications::{NotificationWatch, ScanSchedule},
        scanner::{self, ObserveRoot, ScanRules},
    },
    jobs::{self, ExecutionOutcome, Lease},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
pub struct Dispatcher {
    pub ctx: AppContext,
}
impl Handler for Dispatcher {
    async fn execute<'a>(&'a self, lease: &'a Lease) -> ExecutionOutcome {
        let job = match jobs::load_claimed(self.ctx.pool(), lease).await {
            Ok(Some(job)) => job,
            Ok(None) => return ExecutionOutcome::Cancelled,
            Err(_) => {
                return ExecutionOutcome::TemporaryFailure {
                    message: "job envelope unavailable".into(),
                };
            }
        };
        match job.kind.as_str() {
            scanner::SCAN_JOB_KIND => execute_scan(&self.ctx, lease).await,
            _ => ExecutionOutcome::PermanentFailure {
                message: "unsupported job kind".into(),
            },
        }
    }
    // No child process or detached blocking task is started. Dropping execute
    // interrupts traversal at its next await, before it can publish anything.
}
async fn execute_scan(ctx: &AppContext, lease: &Lease) -> ExecutionOutcome {
    let config = ctx.files();
    let Some(path) = config.root.clone().filter(|_| config.enabled) else {
        return ExecutionOutcome::PermanentFailure {
            message: "file scanning disabled".into(),
        };
    };
    let result = async {
        let report = scanner::observe_root(
            ctx,
            ObserveRoot {
                path,
                cancel: Arc::default(),
                lease: lease.clone(),
                rules: rules(ctx),
            },
        )
        .await?;
        scanner::reconcile_complete_scan(ctx, report).await
    }
    .await;
    match result {
        Ok(true) => ExecutionOutcome::Succeeded,
        Ok(false) => ExecutionOutcome::TemporaryFailure {
            message: "file scan incomplete; sources retained".into(),
        },
        Err(scanner::DomainError::Stale) => ExecutionOutcome::Cancelled,
        Err(_) => ExecutionOutcome::TemporaryFailure {
            message: "file scan unavailable; sources retained".into(),
        },
    }
}
fn rules(ctx: &AppContext) -> ScanRules {
    ScanRules {
        include_hidden: ctx.files().include_hidden.clone(),
        exclude: ctx.files().exclude.clone(),
    }
}

/// Watches kernel events every 100ms (not filesystem polling). The supplemental
/// full-scan deadline is independent of debounce and of an active scan job.
async fn schedule(ctx: AppContext) -> ! {
    let config = ctx.files();
    let root = config.root.clone().expect("enabled config validated");
    let mut clock = ScanSchedule::new(
        Instant::now(),
        Duration::from_secs(config.debounce_seconds),
        Duration::from_secs(config.rescan_seconds),
    );
    let mut watch = NotificationWatch::new().ok();
    let mut refresh_at = Instant::now();
    let mut refresh_needed = true;
    let mut last_generation = None;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let now = Instant::now();
        if watch.is_none() {
            watch = NotificationWatch::new().ok();
            refresh_needed = true;
        }
        if let Some(w) = &mut watch {
            match w.changed() {
                Ok(true) => {
                    clock.notify(now);
                    refresh_needed = true;
                }
                Ok(false) => (),
                Err(_) => {
                    clock.notify(now);
                    refresh_needed = true;
                }
            }
        }
        // Fetch identity periodically so the first successful registration can
        // install recursive watches. A replacement root is never adopted.
        if now >= refresh_at {
            refresh_at = now + Duration::from_secs(1);
            let state: Result<Option<(i64, Option<uuid::Uuid>)>, _> =
                sqlx::query_as("SELECT generation,scan_token FROM file_roots WHERE singleton")
                    .fetch_optional(ctx.pool())
                    .await;
            if let Ok(state) = state
                && state != last_generation
            {
                refresh_needed = true;
                last_generation = state;
            }
            if refresh_needed
                && let (Some(w), Ok(identity)) =
                    (&mut watch, scanner::registered_identity(&ctx).await)
            {
                match w.refresh(&root, &rules(&ctx), identity.as_ref()) {
                    Ok(()) => refresh_needed = false,
                    Err(_) => eprintln!(
                        "knoted-worker: file notifications incomplete; full scans retained"
                    ),
                }
            }
        }
        if clock.due(now) {
            match scanner::enqueue_scan(&ctx).await {
                Ok(true) => clock.enqueued(now),
                Ok(false) => (), // Busy: retain both the event and supplemental deadline.
                Err(_) => eprintln!("knoted-worker: scan scheduling unavailable"),
            }
        }
    }
}

pub async fn run(ctx: AppContext) -> ! {
    if !ctx.files().enabled {
        std::future::pending::<()>().await;
        unreachable!();
    }
    let runner = WorkerRunner::with_context(
        ctx.clone(),
        format!("files-{}", std::process::id()),
        Dispatcher { ctx: ctx.clone() },
    );
    tokio::select! {
        _=schedule(ctx)=>unreachable!(),
        _=runner.run()=>unreachable!(),
    }
}
