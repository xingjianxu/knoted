//! Task trash maintenance, independent of note-history recovery.
use knoted_core::{
    context::AppContext,
    tasks::lifecycle::{self, PurgeTrash},
};
use std::time::Duration;

/// Run immediately at startup and every thirty seconds thereafter. Only a
/// category and count enter logs: never task bodies or database diagnostics.
pub async fn run(ctx: AppContext, mut stop: tokio::sync::watch::Receiver<()>) {
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    while super::next_tick(&mut tick, &mut stop).await {
        let Ok(Some(admission)) = ctx.try_write().await else {
            continue;
        };
        match lifecycle::purge_trash(&ctx, PurgeTrash {}).await {
            Ok(report) => eprintln!("knoted-worker: task trash pass purged={}", report.purged),
            Err(_) => eprintln!("knoted-worker: task trash maintenance unavailable; retrying"),
        }
        if admission.finish().await.is_err() {
            eprintln!("knoted-worker: maintenance completion could not be recorded");
        }
    }
}
