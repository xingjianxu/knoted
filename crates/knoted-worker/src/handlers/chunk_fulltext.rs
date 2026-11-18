//! Chunk build and lexical publication use the shared dispatcher and job queue.

use knoted_core::{context::AppContext, search::chunks};
use std::time::Duration;

pub use chunks::{BUILD_JOB_KIND, PUBLISH_JOB_KIND, execute};

pub async fn schedule(ctx: AppContext) -> ! {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        if chunks::enqueue_missing(&ctx).await.is_err() {
            eprintln!("knoted-worker: chunk discovery unavailable");
        }
    }
}
