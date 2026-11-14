//! Knoted API process entry point.
//!
//! Responsibilities are deliberately narrow: parse `--config`, load and
//! validate the API configuration, initialise logging, open the PostgreSQL
//! pool, and serve the router from [`knoted_api::build_router`]. Business
//! routes live in the library crate.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use knoted_api::admin::{self, TokenCommand};
use knoted_api::build_router;
use knoted_api::logging;
use knoted_core::config::{AppConfig, ProcessKind};
use knoted_core::context::AppContext;
use knoted_core::db;

const USAGE: &str = "usage: knoted-api [--config <PATH>]\n       \
     knoted-api admin token --purpose <bootstrap|recovery> --config <PATH>";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `admin` is a subcommand, not a flag: dispatch before the server parser
    // sees the arguments, so an unknown admin flag never looks like a server
    // flag.
    if args.first().is_some_and(|arg| arg == "admin") {
        return run_admin(&args[1..]).await;
    }
    run_server(args).await
}

async fn run_server(args: Vec<String>) -> ExitCode {
    let mut config_path = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                let Some(value) = args.next() else {
                    eprintln!("knoted-api: --config requires a path");
                    return ExitCode::from(2);
                };
                config_path = Some(PathBuf::from(value));
            }
            "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => {
                // The value is never echoed: a token pasted into the wrong
                // argument slot would otherwise reach stderr and the logs.
                eprintln!("knoted-api: unknown argument");
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    let path = config_path.unwrap_or_else(AppConfig::config_path_from_env);
    let config = match AppConfig::load_for_process(&path, ProcessKind::Api) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("knoted-api: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = logging::init(config.logging.level, config.logging.format) {
        eprintln!("knoted-api: {error}");
        return ExitCode::FAILURE;
    }

    // `load_for_process` already rejected configurations without these
    // sections. Re-check instead of `expect`: a future change to validation
    // must produce a clear diagnostic, never a panic in a service process.
    let Some(database) = config.database() else {
        eprintln!("knoted-api: configuration is missing required `database` settings");
        return ExitCode::FAILURE;
    };
    let Some(server) = &config.server else {
        eprintln!("knoted-api: configuration is missing required `server.bind`");
        return ExitCode::FAILURE;
    };
    let bind: SocketAddr = match server.bind.parse() {
        Ok(bind) => bind,
        Err(_) => {
            eprintln!(
                "knoted-api: invalid `server.bind` `{}`: expected host:port",
                server.bind
            );
            return ExitCode::FAILURE;
        }
    };

    let pool = match db::connect(database).await {
        Ok(pool) => pool,
        Err(error) => {
            tracing::error!(error = %error, "database connection failed");
            return ExitCode::FAILURE;
        }
    };
    let ctx = AppContext::new(pool);

    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(%bind, error = %error, "cannot bind the API listener");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(%bind, "knoted-api listening");

    if let Err(error) = axum::serve(listener, build_router(ctx)).await {
        tracing::error!(error = %error, "the API server stopped with an error");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run_admin(args: &[String]) -> ExitCode {
    let Some(subcommand) = args.first() else {
        eprintln!("knoted-api: admin requires a subcommand");
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    match subcommand.as_str() {
        "token" => run_admin_token(&args[1..]).await,
        "--help" | "help" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            // Same rule as the server parser: no argument values in output.
            eprintln!("knoted-api: unknown admin command");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

async fn run_admin_token(args: &[String]) -> ExitCode {
    let command = match TokenCommand::parse(args) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("knoted-api: {error}");
            eprintln!("{}", admin::USAGE);
            return ExitCode::from(2);
        }
    };

    let config = match AppConfig::load_for_process(&command.config, ProcessKind::Api) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("knoted-api: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = logging::init(config.logging.level, config.logging.format) {
        eprintln!("knoted-api: {error}");
        return ExitCode::FAILURE;
    }

    // stdout is the operator's channel: the token is printed once here and
    // nowhere else. Errors and logs carry fixed messages, never the token.
    let mut stdout = io::stdout().lock();
    match admin::run_token_command(&command, &config, &mut stdout).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "admin token command failed");
            eprintln!("knoted-api: {error}");
            ExitCode::FAILURE
        }
    }
}
