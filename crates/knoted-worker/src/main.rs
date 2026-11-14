use std::path::PathBuf;
use std::process::ExitCode;

use knoted_core::config::{AppConfig, ProcessKind};

const USAGE: &str = "usage: knoted-worker [--config <PATH>]";

fn main() -> ExitCode {
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
            other => {
                eprintln!("knoted-worker: unknown argument `{other}`");
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    let path = config_path.unwrap_or_else(AppConfig::config_path_from_env);
    match AppConfig::load_for_process(&path, ProcessKind::Worker) {
        Ok(_config) => {
            println!("knoted-worker: configuration loaded");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("knoted-worker: {error}");
            ExitCode::FAILURE
        }
    }
}
