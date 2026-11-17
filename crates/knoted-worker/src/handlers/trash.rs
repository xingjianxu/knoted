//! Task trash maintenance, independent of note-history recovery.
use knoted_core::{
    context::AppContext,
    tasks::lifecycle::{self, PurgeTrash},
};
use std::time::Duration;

/// Run immediately at startup and every thirty seconds thereafter. Only a
/// category and count enter logs: never task bodies or database diagnostics.
pub async fn run(ctx: AppContext) -> ! {
    loop {
        match lifecycle::purge_trash(&ctx, PurgeTrash {}).await {
            Ok(report) => eprintln!("knoted-worker: task trash pass purged={}", report.purged),
            Err(_) => eprintln!("knoted-worker: task trash maintenance unavailable; retrying"),
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}
