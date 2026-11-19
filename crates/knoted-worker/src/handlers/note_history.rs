//! Snapshot recovery and periodic maintenance in the actual Worker process.
use knoted_core::{
    context::AppContext,
    notes::history::{self, MaintainHistory},
};
use std::time::Duration;

/// Poll twice a minute; history's durable five-minute anchor determines which
/// notes are due. Failed startup recovery remains pending until a pass succeeds.
/// No content, database diagnostics or authorization material enters these logs.
pub async fn run(ctx: AppContext, mut stop: tokio::sync::watch::Receiver<()>) {
    let mut recover = true;
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    while super::next_tick(&mut tick, &mut stop).await {
        let Ok(Some(admission)) = ctx.try_write().await else {
            continue;
        };
        match history::maintain_history(&ctx, MaintainHistory { recover }).await {
            Ok(report) => {
                eprintln!(
                    "knoted-worker: note history pass snapshots={} pruned={} recovery={}",
                    report.snapshots, report.pruned, recover
                );
                recover = false;
            }
            Err(_) => eprintln!("knoted-worker: note history maintenance unavailable; retrying"),
        }
        if admission.finish().await.is_err() {
            eprintln!("knoted-worker: maintenance completion could not be recorded");
        }
    }
}
