//! The `knoted-api admin ...` commands.
//!
//! The API process also serves as the operator's administrative entry point, so
//! the deployment needs no second binary. The `token` command issues a
//! one-time browser credential. The `backup` command writes a local archive.
//!
//! The token is printed to stdout exactly once and never logged: stdout is the
//! operator's channel (a shell, an SSH session), while logs are collected and
//! retained. Error messages are fixed strings for the same reason.

pub mod backup;

use std::io::{self, Write};
use std::path::PathBuf;

use chrono::Utc;
use knoted_core::auth::{DomainAuthError, IssuedToken, TokenPurpose, issue_token};
use knoted_core::config::{AppConfig, ConfigError};
use knoted_core::db::{self, DbError};
use thiserror::Error;

/// Usage line for the admin subcommands.
pub const USAGE: &str =
    "usage: knoted-api admin token --purpose <bootstrap|recovery> --config <PATH>";

/// Parsed arguments of `knoted-api admin token`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenCommand {
    /// Which token to issue; the purpose is not interchangeable.
    pub purpose: TokenPurpose,
    /// Configuration file with the `[database]` section to connect with.
    pub config: PathBuf,
}

/// Failures of an admin command.
///
/// `Usage` messages are fixed strings as well: argument values are never echoed,
/// because a token pasted into the wrong position must not end up on stderr.
#[derive(Debug, Error)]
pub enum AdminError {
    /// The command line was incomplete or contained an unknown argument.
    #[error("{0}")]
    Usage(String),
    /// The configuration file could not be loaded.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The loaded configuration has no `[database]` section.
    #[error("configuration is missing required `database` settings")]
    MissingDatabase,
    #[error("write admission is paused for maintenance")]
    Maintenance,
    /// Reading the database failed before a token could be issued.
    #[error(transparent)]
    Database(#[from] DbError),
    /// Issuing the token failed, including a refused bootstrap.
    #[error(transparent)]
    Auth(#[from] DomainAuthError),
    /// Writing to stdout failed (for example a closed pipe).
    #[error("cannot write the credential token to stdout: {0}")]
    Output(#[from] io::Error),
}

impl TokenCommand {
    /// Parses the arguments that follow `admin token`.
    ///
    /// Both flags are required, so a mistyped invocation fails before a database
    /// connection is opened.
    pub fn parse<I>(args: I) -> Result<TokenCommand, AdminError>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let mut args = args.into_iter();
        let mut purpose = None;
        let mut config = None;

        while let Some(arg) = args.next() {
            match arg.as_ref() {
                "--purpose" => {
                    let Some(value) = args.next() else {
                        return Err(AdminError::Usage(
                            "--purpose requires `bootstrap` or `recovery`".to_owned(),
                        ));
                    };
                    let value = value.as_ref();
                    // The value is not echoed: it may be a token pasted into the
                    // wrong argument slot.
                    purpose = Some(TokenPurpose::parse(value).ok_or_else(|| {
                        AdminError::Usage(
                            "unknown token purpose: expected `bootstrap` or `recovery`".to_owned(),
                        )
                    })?);
                }
                "--config" => {
                    let Some(value) = args.next() else {
                        return Err(AdminError::Usage("--config requires a path".to_owned()));
                    };
                    config = Some(PathBuf::from(value.as_ref()));
                }
                _ => {
                    // Argument values are not echoed either; see above.
                    return Err(AdminError::Usage(
                        "unknown argument: expected --purpose and --config".to_owned(),
                    ));
                }
            }
        }

        let purpose =
            purpose.ok_or_else(|| AdminError::Usage("--purpose is required".to_owned()))?;
        let config = config.ok_or_else(|| AdminError::Usage("--config is required".to_owned()))?;
        Ok(TokenCommand { purpose, config })
    }
}

/// Runs `admin token`: connects with `config`, issues the token and prints it.
///
/// `config` is passed in already loaded so the caller can initialise logging
/// from the same file before this runs, and so tests can point the command at an
/// isolated schema.
pub async fn run_token_command<W>(
    command: &TokenCommand,
    config: &AppConfig,
    out: &mut W,
) -> Result<(), AdminError>
where
    W: Write,
{
    let database = config.database().ok_or(AdminError::MissingDatabase)?;
    let pool = db::connect(database).await?;
    let ctx = knoted_core::context::AppContext::new(pool);
    let guard = ctx
        .try_write()
        .await
        .map_err(DbError::from)?
        .ok_or(AdminError::Maintenance)?;
    let issued = issue_token(ctx.pool(), command.purpose, Utc::now()).await;
    guard.finish().await.map_err(DbError::from)?;
    let issued = issued?;
    write_token(out, &issued)?;
    Ok(())
}

/// Writes the one-time token and its expiry to `out`.
///
/// Exactly one line carries the token. The expiry lets the operator know how
/// long the browser has to finish the ceremony.
pub fn write_token<W>(out: &mut W, issued: &IssuedToken) -> io::Result<()>
where
    W: Write,
{
    writeln!(out, "token: {}", issued.plaintext())?;
    writeln!(out, "expires_at: {}", issued.expires_at.to_rfc3339())?;
    out.flush()
}
