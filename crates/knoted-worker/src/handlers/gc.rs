//! Image reclamation behind the Worker.
use std::time::Duration;

use knoted_core::{assets::gc::collect_unreferenced_assets, context::AppContext};
use tokio::time::{MissedTickBehavior, interval};

pub async fn run(ctx: AppContext, mut stop: tokio::sync::watch::Receiver<()>) {
    let mut tick = interval(Duration::from_secs(300));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    while super::next_tick(&mut tick, &mut stop).await {
        let Ok(Some(admission)) = ctx.try_write().await else {
            continue;
        };
        match collect_unreferenced_assets(&ctx).await {
            Ok(count) if count > 0 => eprintln!("knoted-worker: collected {count} images"),
            Ok(_) => {}
            Err(_) => eprintln!("knoted-worker: image reclamation unavailable; retrying"),
        }
        if admission.finish().await.is_err() {
            eprintln!("knoted-worker: maintenance completion could not be recorded");
        }
    }
}
