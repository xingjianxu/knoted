//! Snapshot recovery and periodic maintenance in the actual Worker process.
use knoted_core::{
    context::AppContext,
    notes::history::{self, MaintainHistory},
};
use std::time::Duration;

/// Poll twice a minute; history's durable five-minute anchor determines which
/// notes are due. Failed startup recovery remains pending until a pass succeeds.
/// No content, database diagnostics or authorization material enters these logs.
pub async fn run(ctx: AppContext) -> ! {
    let mut recover = true;
    loop {
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
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}
