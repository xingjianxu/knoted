use std::path::PathBuf;
use std::process::ExitCode;

use knoted_core::{
    config::{AppConfig, ProcessKind},
    context::AppContext,
    db,
};

const USAGE: &str = "usage: knoted-worker [--config <PATH>]";

#[tokio::main]
async fn main() -> ExitCode {
    let mut config_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                let Some(value) = args.next() else {
                    eprintln!("knoted-worker: --config requires a path");
                    return ExitCode::from(2);
                };
                config_path = Some(PathBuf::from(value));
            }
            "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("knoted-worker: unknown argument");
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    let path = config_path.unwrap_or_else(AppConfig::config_path_from_env);
    match AppConfig::load_for_process(&path, ProcessKind::Worker) {
        Ok(config) => {
            let search_api_key = config.search_api_key();
            let Some(database) = config.database else {
                eprintln!("knoted-worker: database configuration required");
                return ExitCode::FAILURE;
            };
            let pool = match db::connect(&database).await {
                Ok(pool) => pool,
                Err(_) => {
                    eprintln!("knoted-worker: database unavailable");
                    return ExitCode::FAILURE;
                }
            };
            if !matches!(db::is_ready(&pool).await, Ok(true)) {
                eprintln!("knoted-worker: schema not ready; run migrations before starting");
                return ExitCode::FAILURE;
            }
            let Ok(chat_providers) =
                knoted_core::rag::providers::ProviderRegistry::new(config.chat)
            else {
                eprintln!("knoted-worker: model provider registry unavailable");
                return ExitCode::FAILURE;
            };
            let ctx = AppContext::new(pool)
                .with_asset_root(config.assets.root)
                .with_files(config.files)
                .with_intelligence(config.intelligence)
                .with_chat_providers(std::sync::Arc::new(chat_providers))
                .with_search(config.search)
                .with_vectors_enabled(config.vectors.enabled)
                .with_search_api_key(search_api_key);
            tokio::select! {
                _ = knoted_worker::handlers::fulltext::project(ctx.clone()), if ctx.search().enabled => unreachable!(),
                _ = knoted_worker::handlers::note_history::run(ctx.clone()) => unreachable!(),
                _ = knoted_worker::handlers::trash::run(ctx.clone()) => unreachable!(),
                _ = knoted_worker::handlers::gc::run(ctx.clone()) => unreachable!(),
                _ = knoted_worker::handlers::scan::run(ctx) => ExitCode::SUCCESS,
            }
        }
        Err(error) => {
            eprintln!("knoted-worker: {error}");
            ExitCode::FAILURE
        }
    }
}
