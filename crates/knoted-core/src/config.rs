//! Process configuration loaded from TOML.
//!
//! Loading is independent of process runtimes. File storage validation resolves
//! existing ancestors to reject aliases into read-only sources. Secrets are
//! redacted from every `Debug` representation.

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

fn default_rp_id() -> String {
    "wlab.132140.xyz".to_owned()
}

fn default_origin() -> String {
    "https://wlab.132140.xyz:15601".to_owned()
}

/// WebAuthn relying-party settings and the one origin allowed to write.
///
/// `rp_id` is the WebAuthn RP ID and never contains a port; the port belongs to
/// `origin`, which is compared exactly against the `Origin` header of every
/// write request. Both have the deployment's defaults, so an existing
/// configuration file stays valid without an `[auth]` section.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AuthConfig {
    pub rp_id: String,
    pub origin: String,
}

impl Default for AuthConfig {
    fn default() -> AuthConfig {
        AuthConfig {
            rp_id: default_rp_id(),
            origin: default_origin(),
        }
    }
}

/// Private asset storage. Relative roots are rejected before process startup.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AssetsConfig {
    pub root: PathBuf,
}
impl Default for AssetsConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/data/knoted-dev/service/assets"),
        }
    }
}

/// Internal execution service; no external origin or inline credentials.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct IntelligenceConfig {
    pub base_url: String,
}
impl Default for IntelligenceConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:15603".into(),
        }
    }
}
impl IntelligenceConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let valid = url::Url::parse(&self.base_url).ok().is_some_and(|u| {
            u.scheme() == "http"
                && u.username().is_empty()
                && u.password().is_none()
                && u.path() == "/"
                && u.query().is_none()
                && u.fragment().is_none()
                && match u.host() {
                    Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                    Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                    _ => false,
                }
        });
        if valid {
            Ok(())
        } else {
            Err(ConfigError::Invalid {
                field: "intelligence.base_url",
                message: "requires an HTTP loopback origin without credentials or a path".into(),
            })
        }
    }
}

/// Full-text publication target. Disabled unless configured; an API key is
/// referenced by credential name, never inlined into the configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SearchConfig {
    pub enabled: bool,
    pub base_url: String,
    pub index_uid: String,
    /// Independent generation for lexical RAG chunks.
    pub rag_chunk_index_uid: String,
    /// Credential name in the shared credential file, `None` for an engine that
    /// accepts unauthenticated loopback traffic.
    pub api_key_credential: Option<String>,
}
impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: "http://127.0.0.1:15605".into(),
            index_uid: "knoted_documents".into(),
            rag_chunk_index_uid: "knoted_rag_chunks".into(),
            api_key_credential: None,
        }
    }
}
impl SearchConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: &'static str| ConfigError::Invalid {
            field: "search",
            message: message.into(),
        };
        // Same shape as the internal execution service: loopback HTTP only, no
        // credentials in the URL and no path to append requests to.
        let valid = url::Url::parse(&self.base_url).ok().is_some_and(|u| {
            u.scheme() == "http"
                && u.username().is_empty()
                && u.password().is_none()
                && u.path() == "/"
                && u.query().is_none()
                && u.fragment().is_none()
                && match u.host() {
                    Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                    Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                    _ => false,
                }
        });
        if !valid {
            return Err(invalid(
                "base_url requires an HTTP loopback IP-literal origin without credentials or a path",
            ));
        }
        // Both generation UIDs become URL path segments.
        for uid in [&self.index_uid, &self.rag_chunk_index_uid] {
            if uid.is_empty()
                || uid.len() > 400
                || !uid
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err(invalid(
                    "index_uid and rag_chunk_index_uid require 1-400 alphanumeric, `-` or `_` characters",
                ));
            }
        }
        if self.index_uid == self.rag_chunk_index_uid {
            return Err(invalid("document and chunk index UIDs must differ"));
        }
        Ok(())
    }
}

/// Optional server-side Office preview. Configuration contains a path, never a JWT secret.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficeConfig {
    pub document_server_origin: String,
    pub jwt_secret_file: PathBuf,
}

impl OfficeConfig {
    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        if !is_canonical_https_origin(&self.document_server_origin) {
            return Err(ConfigError::Invalid {
                field: "office.document_server_origin",
                message: "requires a canonical HTTPS origin without a path or credentials".into(),
            });
        }
        if !self.jwt_secret_file.is_absolute() {
            return Err(ConfigError::Invalid {
                field: "office.jwt_secret_file",
                message: "requires an absolute file path".into(),
            });
        }
        Ok(())
    }
}

pub(crate) fn is_canonical_https_origin(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| url.scheme() == "https" && url.origin().ascii_serialization() == value)
}

/// Disabled unless explicitly configured. Sources remain read-only; staging
/// must live outside the observed tree.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FilesConfig {
    pub enabled: bool,
    pub root: Option<PathBuf>,
    pub staging_root: Option<PathBuf>,
    pub debounce_seconds: u64,
    pub rescan_seconds: u64,
    pub include_hidden: Vec<PathBuf>,
    pub exclude: Vec<PathBuf>,
}
impl Default for FilesConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            root: None,
            staging_root: None,
            debounce_seconds: 2,
            rescan_seconds: 600,
            include_hidden: vec![],
            exclude: vec![],
        }
    }
}
impl FilesConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        let invalid = || {
            ConfigError::Invalid { field: "files", message: "requires absolute separate root/staging paths, positive intervals and safe relative rules".into() }
        };
        if self.debounce_seconds == 0 || self.rescan_seconds == 0 {
            return Err(invalid());
        }
        if !(crate::files::scanner::ScanRules {
            include_hidden: self.include_hidden.clone(),
            exclude: self.exclude.clone(),
        })
        .valid()
        {
            return Err(invalid());
        }
        for path in [&self.root, &self.staging_root].into_iter().flatten() {
            if !path.is_absolute()
                || path
                    .components()
                    .any(|p| matches!(p, std::path::Component::ParentDir))
            {
                return Err(invalid());
            }
        }
        if self.enabled {
            let (Some(root), Some(staging)) = (&self.root, &self.staging_root) else {
                return Err(invalid());
            };
            let resolved_root = resolve_existing(root).map_err(|_| invalid())?;
            let resolved_staging = resolve_existing(staging).map_err(|_| invalid())?;
            if resolved_staging.starts_with(&resolved_root)
                || resolved_root.starts_with(&resolved_staging)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

// Resolve existing ancestors too: the configured staging leaf may not exist
// yet, while one of its ancestors is an alias into the read-only source tree.
fn resolve_existing(path: &Path) -> std::io::Result<PathBuf> {
    resolve_existing_at(path, 0)
}
fn resolve_existing_at(path: &Path, depth: usize) -> std::io::Result<PathBuf> {
    if depth > 40 {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Ok(target) = std::fs::read_link(path) {
                let target = if target.is_absolute() {
                    target
                } else {
                    path.parent()
                        .ok_or(std::io::ErrorKind::InvalidInput)?
                        .join(target)
                };
                return resolve_existing_at(&target, depth + 1);
            }
            let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
            Ok(resolve_existing_at(parent, depth + 1)?
                .join(path.file_name().ok_or(std::io::ErrorKind::InvalidInput)?))
        }
        Err(error) => Err(error),
    }
}

/// Validated process configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub server: Option<ServerConfig>,
    pub database: Option<DatabaseConfig>,
    pub logging: LoggingConfig,
    pub credentials: CredentialsConfig,
    pub auth: AuthConfig,
    pub assets: AssetsConfig,
    pub files: FilesConfig,
    pub intelligence: IntelligenceConfig,
    pub search: SearchConfig,
    pub office: Option<OfficeConfig>,
    secrets: BTreeMap<String, String>,
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppConfig")
            .field("server", &self.server)
            .field("database", &self.database)
            .field("logging", &self.logging)
            .field("credentials", &self.credentials)
            .field("auth", &self.auth)
            .field("assets", &self.assets)
            .field("files", &self.files)
            .field("intelligence", &self.intelligence)
            .field("search", &self.search)
            .field("office_configured", &self.office.is_some())
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
        let file: ConfigFile = toml::from_str(&contents).map_err(|source| {
            // The TOML error's Display (and its source chain) echoes the raw
            // offending line, which can be a `database.url` with a password.
            // Keep only the location, never the payload.
            let (line, column) = source
                .span()
                .map(|span| line_column(&contents, span.start))
                .unwrap_or((0, 0));
            ConfigError::Parse {
                path: path.to_path_buf(),
                line,
                column,
            }
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

    /// Search engine API key named by `search.api_key_credential`.
    ///
    /// `None` means the engine accepts unauthenticated loopback traffic; a named
    /// credential that is missing from the loaded file also yields `None` rather
    /// than failing the whole process, matching the optional external services.
    pub fn search_api_key(&self) -> Option<String> {
        self.search
            .api_key_credential
            .as_deref()
            .and_then(|name| self.credential(name))
            .map(str::to_owned)
    }

    /// Resolves the configuration path from `KNOTED_CONFIG` or the default.
    pub fn config_path_from_env() -> PathBuf {
        match std::env::var_os(CONFIG_PATH_ENV) {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => PathBuf::from(DEFAULT_CONFIG_PATH),
        }
    }

    /// Database settings, when the file declares them.
    ///
    /// [`AppConfig::load_for_process`] has already rejected configurations
    /// without a database section, so process entry points can call this and
    /// treat `None` as a bug instead of re-implementing the check.
    pub fn database(&self) -> Option<&DatabaseConfig> {
        self.database.as_ref()
    }

    /// WebAuthn relying-party and allowed-origin settings.
    pub fn auth(&self) -> &AuthConfig {
        &self.auth
    }

    fn from_file(file: ConfigFile) -> Result<AppConfig, ConfigError> {
        if let Some(database) = &file.database {
            validate_database(database)?;
        }
        validate_auth(&file.auth)?;
        file.files.validate()?;
        file.intelligence.validate()?;
        file.search.validate()?;
        if let Some(office) = &file.office {
            office.validate()?;
        }
        if file.files.enabled {
            let source = resolve_existing(file.files.root.as_ref().expect("validated root"))
                .map_err(|_| ConfigError::Invalid {
                    field: "files.root",
                    message: "cannot resolve existing ancestors".into(),
                })?;
            let assets = resolve_existing(&file.assets.root).map_err(|_| ConfigError::Invalid {
                field: "assets.root",
                message: "cannot resolve existing ancestors".into(),
            })?;
            if source.starts_with(&assets) || assets.starts_with(&source) {
                return Err(ConfigError::Invalid {
                    field: "files.root",
                    message: "must be separate from private asset storage".into(),
                });
            }
        }
        if !file.assets.root.is_absolute() {
            return Err(ConfigError::Invalid {
                field: "assets.root",
                message: "must be an absolute path".to_owned(),
            });
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
            auth: file.auth,
            assets: file.assets,
            files: file.files,
            intelligence: file.intelligence,
            search: file.search,
            office: file.office,
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
    #[serde(default)]
    auth: AuthConfig,
    #[serde(default)]
    assets: AssetsConfig,
    #[serde(default)]
    files: FilesConfig,
    #[serde(default)]
    intelligence: IntelligenceConfig,
    #[serde(default)]
    search: SearchConfig,
    office: Option<OfficeConfig>,
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

fn validate_auth(auth: &AuthConfig) -> Result<(), ConfigError> {
    // A WebAuthn RP ID is a bare host: `localhost` or a registered domain. It
    // never carries a scheme, a port or a path, and whitespace is always a typo.
    if auth.rp_id.is_empty()
        || auth.rp_id.contains('/')
        || auth.rp_id.contains(':')
        || auth.rp_id.chars().any(char::is_whitespace)
    {
        return Err(ConfigError::Invalid {
            field: "auth.rp_id",
            message: "must be a bare host name without scheme, port, path or whitespace".to_owned(),
        });
    }

    // The origin is compared verbatim against the `Origin` header, so it must be
    // the canonical `https://host[:port]` form a browser sends: no credentials,
    // path, query, fragment or trailing slash. Parsing it here turns a typo into
    // a startup failure instead of silently rejecting every write request.
    let Ok(origin) = url::Url::parse(&auth.origin) else {
        return Err(invalid_origin());
    };
    let canonical = origin.scheme() == "https"
        && origin.host_str().is_some_and(|host| !host.is_empty())
        && origin.path() == "/"
        && origin.query().is_none()
        && origin.fragment().is_none()
        && origin.username().is_empty()
        && origin.password().is_none();
    // A browser never writes the default https port into `Origin`, and `url`
    // normalises `:443` away, so an explicit default port in the configuration
    // would silently never match. Reject it at load time instead.
    if !canonical || auth.origin.ends_with('/') || auth.origin.ends_with(":443") {
        return Err(invalid_origin());
    }
    Ok(())
}

fn invalid_origin() -> ConfigError {
    ConfigError::Invalid {
        field: "auth.origin",
        message: "must be `https://host[:port]` without credentials, path, query, \
                  fragment, trailing slash or the default `:443` port"
            .to_owned(),
    }
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
/// `Debug` intentionally omits source payloads for the same reason. Parse
/// failures keep only a line/column, because `toml::de::Error` echoes the
/// offending line and may carry a database password.
#[derive(thiserror::Error)]
pub enum ConfigError {
    #[error("read config file {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "parse config file {}: invalid TOML syntax at line {line} column {column}",
        path.display()
    )]
    Parse {
        path: PathBuf,
        line: usize,
        column: usize,
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

/// One-based line and column for a byte offset in `contents`.
fn line_column(contents: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(contents.len());
    let before = &contents[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit_once('\n')
        .map(|(_, tail)| tail.chars().count())
        .unwrap_or_else(|| before.chars().count())
        + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::line_column;

    #[test]
    fn line_column_is_one_based_and_char_aware() {
        assert_eq!(line_column("abc", 0), (1, 1));
        assert_eq!(line_column("abc", 3), (1, 4));
        // A byte offset inside multibyte text must still count characters.
        assert_eq!(line_column("中文信息", 3), (1, 2));
        assert_eq!(line_column("中\nabc", 5), (2, 2));
        assert_eq!(line_column("abc", 99), (1, 4), "offsets clamp to the end");
    }
}
