//! Chunk build and lexical publication use the shared dispatcher and job queue.

use knoted_core::{context::AppContext, search::chunks};
use std::time::Duration;

pub use chunks::{BUILD_JOB_KIND, PUBLISH_JOB_KIND, execute};

pub async fn schedule(ctx: AppContext, mut stop: tokio::sync::watch::Receiver<()>) {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while super::next_tick(&mut tick, &mut stop).await {
        let Ok(Some(admission)) = ctx.try_write().await else {
            continue;
        };
        if chunks::enqueue_missing(&ctx).await.is_err() {
            eprintln!("knoted-worker: chunk discovery unavailable");
        }
        if admission.finish().await.is_err() {
            eprintln!("knoted-worker: maintenance completion could not be recorded");
        }
    }
}
