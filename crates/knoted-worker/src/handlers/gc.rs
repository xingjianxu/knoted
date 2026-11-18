//! Image and execution staging reclamation behind the Worker.
use std::time::Duration;

use knoted_core::{
    assets::gc::collect_unreferenced_assets, context::AppContext, files::gc::collect_staging,
};
use tokio::time::{MissedTickBehavior, interval};

pub async fn run(ctx: AppContext) -> ! {
    let mut tick = interval(Duration::from_secs(300));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match collect_unreferenced_assets(&ctx).await {
            Ok(count) if count > 0 => eprintln!("knoted-worker: collected {count} images"),
            Ok(_) => {}
            Err(_) => eprintln!("knoted-worker: image reclamation unavailable; retrying"),
        }
        match collect_staging(&ctx).await {
            Ok(count) if count > 0 => eprintln!("knoted-worker: collected {count} staging objects"),
            Ok(_) => {}
            Err(_) => eprintln!("knoted-worker: staging reclamation unavailable; retrying"),
        }
    }
}
