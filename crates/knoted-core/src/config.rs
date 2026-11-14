//! Process configuration loaded from TOML.
//!
//! The configuration model is deliberately independent of Tokio, Axum, SQLx and
//! logging runtimes: loading and validation only use `serde`, `toml` and
//! `thiserror`. Secrets are redacted from every `Debug` representation.

use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Environment variable that points at the process configuration file.
pub const CONFIG_PATH_ENV: &str = "KNOTED_CONFIG";

/// Configuration path used when [`CONFIG_PATH_ENV`] is not set.
pub const DEFAULT_CONFIG_PATH: &str = "/data/knoted-dev/service/config/knoted.toml";

/// The process a configuration is validated for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessKind {
    Api,
    Worker,
}

impl ProcessKind {
    /// Stable name used in diagnostics and configuration requirements.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProcessKind::Api => "api",
            ProcessKind::Worker => "worker",
        }
    }
}

/// HTTP listener settings, required by the API process.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: String,
}

fn default_max_connections() -> u32 {
    10
}

fn default_connect_timeout_secs() -> u64 {
    10
}

/// PostgreSQL connection settings, required by both processes.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    pub url: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
}

impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseConfig")
            .field("url", &redact_url(&self.url))
            .field("max_connections", &self.max_connections)
            .field("connect_timeout_secs", &self.connect_timeout_secs)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub format: LogFormat,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CredentialsConfig {
    pub credential_file: Option<PathBuf>,
}

/// Validated process configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub server: Option<ServerConfig>,
    pub database: Option<DatabaseConfig>,
    pub logging: LoggingConfig,
    pub credentials: CredentialsConfig,
    secrets: BTreeMap<String, String>,
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppConfig")
            .field("server", &self.server)
            .field("database", &self.database)
            .field("logging", &self.logging)
            .field("credentials", &self.credentials)
            .field("loaded_credentials", &self.secrets.len())
            .finish()
    }
}

impl AppConfig {
    /// Reads, parses and validates a configuration file.
    ///
    /// Process-specific sections are not required; use
    /// [`AppConfig::load_for_process`] when starting an API or worker process.
    ///
    /// A relative `credentials.credential_file` is resolved against the process
    /// working directory, not against the configuration file's directory;
    /// deployment configurations should use absolute paths.
    pub fn load(path: impl AsRef<Path>) -> Result<AppConfig, ConfigError> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let file: ConfigFile = toml::from_str(&contents).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_file(file)
    }

    /// Loads and additionally verifies the requirements of `process`.
    pub fn load_for_process(
        path: impl AsRef<Path>,
        process: ProcessKind,
    ) -> Result<AppConfig, ConfigError> {
        let config = Self::load(path)?;
        config.require_process(process)?;
        Ok(config)
    }

    /// Returns a credential value loaded from the configured credential file.
    pub fn credential(&self, name: &str) -> Option<&str> {
        self.secrets.get(name).map(String::as_str)
    }

    /// Resolves the configuration path from `KNOTED_CONFIG` or the default.
    pub fn config_path_from_env() -> PathBuf {
        match std::env::var_os(CONFIG_PATH_ENV) {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => PathBuf::from(DEFAULT_CONFIG_PATH),
        }
    }

    fn from_file(file: ConfigFile) -> Result<AppConfig, ConfigError> {
        if let Some(database) = &file.database {
            validate_database(database)?;
        }
        let secrets = match &file.credentials.credential_file {
            Some(path) => load_credentials(path)?,
            None => BTreeMap::new(),
        };
        // A `[server]` section without `bind` is the same diagnostic as an
        // absent section: the API requires `server.bind`.
        let server = file
            .server
            .and_then(|raw| raw.bind.map(|bind| ServerConfig { bind }));

        Ok(AppConfig {
            server,
            database: file.database,
            logging: file.logging,
            credentials: file.credentials,
            secrets,
        })
    }

    fn require_process(&self, process: ProcessKind) -> Result<(), ConfigError> {
        if self.database.is_none() {
            return Err(ConfigError::Missing {
                field: "database",
                process: process.as_str(),
            });
        }
        if process == ProcessKind::Api {
            let server = self.server.as_ref().ok_or(ConfigError::Missing {
                field: "server.bind",
                process: process.as_str(),
            })?;
            server
                .bind
                .parse::<SocketAddr>()
                .map_err(|_| ConfigError::Invalid {
                    field: "server.bind",
                    message: format!("`{}` is not a socket address (host:port)", server.bind),
                })?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServer {
    bind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    server: Option<RawServer>,
    database: Option<DatabaseConfig>,
    #[serde(default)]
    logging: LoggingConfig,
    #[serde(default)]
    credentials: CredentialsConfig,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialFile {
    secrets: BTreeMap<String, String>,
}

fn validate_database(database: &DatabaseConfig) -> Result<(), ConfigError> {
    if database.url.is_empty()
        || !(database.url.starts_with("postgres://") || database.url.starts_with("postgresql://"))
    {
        return Err(ConfigError::Invalid {
            field: "database.url",
            message: "must start with `postgres://` or `postgresql://`".to_owned(),
        });
    }
    if !(1..=64).contains(&database.max_connections) {
        return Err(ConfigError::Invalid {
            field: "database.max_connections",
            message: format!("must be between 1 and 64, got {}", database.max_connections),
        });
    }
    if !(1..=300).contains(&database.connect_timeout_secs) {
        return Err(ConfigError::Invalid {
            field: "database.connect_timeout_secs",
            message: format!(
                "must be between 1 and 300, got {}",
                database.connect_timeout_secs
            ),
        });
    }
    Ok(())
}

fn load_credentials(path: &Path) -> Result<BTreeMap<String, String>, ConfigError> {
    let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::CredentialRead {
        path: path.to_path_buf(),
        source,
    })?;
    let file: CredentialFile =
        toml::from_str(&contents).map_err(|_| ConfigError::CredentialParse {
            path: path.to_path_buf(),
        })?;
    Ok(file.secrets)
}

/// Replaces the password in `scheme://user:password@host` with `***`.
fn redact_url(url: &str) -> String {
    redact_password_parameters(&redact_userinfo(url))
}

fn redact_userinfo(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_owned();
    };
    let authority_start = scheme_end + "://".len();
    let rest = &url[authority_start..];
    let Some(at) = rest.rfind('@') else {
        return url.to_owned();
    };
    let userinfo = &rest[..at];
    match userinfo.find(':') {
        Some(colon) => format!(
            "{}{}:***@{}",
            &url[..authority_start],
            &userinfo[..colon],
            &rest[at + 1..]
        ),
        None => format!("{}***@{}", &url[..authority_start], &rest[at + 1..]),
    }
}

/// Replaces `password=...` query (or key/value) values with `***`.
fn redact_password_parameters(value: &str) -> String {
    const KEY: &str = "password=";
    let mut redacted = String::with_capacity(value.len());
    let mut remaining = value;
    loop {
        let Some(index) = remaining.to_ascii_lowercase().find(KEY) else {
            redacted.push_str(remaining);
            return redacted;
        };
        redacted.push_str(&remaining[..index]);
        redacted.push_str("password=***");
        let after = &remaining[index + KEY.len()..];
        let end = after.find(['&', '#']).unwrap_or(after.len());
        remaining = &after[end..];
    }
}

/// Configuration loading failures.
///
/// `Display` never includes credential values or TOML source fragments, and
/// `Debug` intentionally omits source payloads for the same reason.
#[derive(thiserror::Error)]
pub enum ConfigError {
    #[error("read config file {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parse config file {}: invalid TOML syntax", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("missing required config field `{field}` for {process}")]
    Missing {
        field: &'static str,
        process: &'static str,
    },
    #[error("invalid config field `{field}`: {message}")]
    Invalid {
        field: &'static str,
        message: String,
    },
    #[error("read credential file {}: {source}", path.display())]
    CredentialRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid credential file {}", path.display())]
    CredentialParse { path: PathBuf },
}

impl fmt::Debug for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, .. } => f
                .debug_struct("Read")
                .field("path", path)
                .finish_non_exhaustive(),
            Self::Parse { path, .. } => f
                .debug_struct("Parse")
                .field("path", path)
                .finish_non_exhaustive(),
            Self::Missing { field, process } => f
                .debug_struct("Missing")
                .field("field", field)
                .field("process", process)
                .finish(),
            Self::Invalid { field, message } => f
                .debug_struct("Invalid")
                .field("field", field)
                .field("message", message)
                .finish(),
            Self::CredentialRead { path, .. } => f
                .debug_struct("CredentialRead")
                .field("path", path)
                .finish_non_exhaustive(),
            Self::CredentialParse { path } => f
                .debug_struct("CredentialParse")
                .field("path", path)
                .finish(),
        }
    }
}
