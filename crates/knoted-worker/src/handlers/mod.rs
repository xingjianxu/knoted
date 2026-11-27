//! Background maintenance handlers.
pub mod gc;
pub mod note_history;
pub mod scan;
pub mod trash;

pub mod fulltext;

use tokio::{sync::watch, time::Interval};

async fn next_tick(tick: &mut Interval, stop: &mut watch::Receiver<()>) -> bool {
    tokio::select! {
        biased;
        _ = stop.changed() => false,
        _ = tick.tick() => true,
    }
}

pub async fn run(ctx: knoted_core::context::AppContext) {
    let (shutdown, stop) = watch::channel(());
    tokio::join!(
        async {
            scan::run(ctx.clone()).await;
            drop(shutdown);
        },
        async {
            if ctx.search().enabled {
                fulltext::project(ctx.clone(), stop.clone()).await;
            }
        },
        note_history::run(ctx.clone(), stop.clone()),
        trash::run(ctx.clone(), stop.clone()),
        gc::run(ctx.clone(), stop.clone())
    );
}
