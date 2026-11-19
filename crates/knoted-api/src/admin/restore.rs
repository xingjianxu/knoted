//! Operator entry point for isolated restore.

use super::backup::{self, BackupManifest};
use knoted_core::config::{AppConfig, ProcessKind};
use knoted_core::context::AppContext;
use knoted_core::db::{backup::Snapshot, restore::RestoreDatabase};
use knoted_core::files::{safe_fs, staging::Sha256Hex};
use rustix::fs;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::TcpListener;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use url::Url;

pub const USAGE: &str = "usage: knoted-api admin restore --input <archive> --target <descriptor>";

pub struct RestoreCommand {
    pub input: PathBuf,
    pub target: PathBuf,
}

impl RestoreCommand {
    pub fn parse<I, S>(args: I) -> Result<Self, &'static str>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut args = args.into_iter();
        let (mut input, mut target) = (None, None);
        while let Some(flag) = args.next() {
            let slot = match flag.as_ref() {
                "--input" if input.is_none() => &mut input,
                "--target" if target.is_none() => &mut target,
                _ => return Err(USAGE),
            };
            let value = args.next().ok_or(USAGE)?;
            if value.as_ref().is_empty() || value.as_ref().starts_with("--") {
                return Err(USAGE);
            }
            *slot = Some(PathBuf::from(value.as_ref()));
        }
        Ok(Self {
            input: input.ok_or(USAGE)?,
            target: target.ok_or(USAGE)?,
        })
    }
}

fn invalid() -> io::Error {
    io::ErrorKind::InvalidData.into()
}

fn members(directory: impl AsFd) -> io::Result<BTreeSet<OsString>> {
    let mut result = BTreeSet::new();
    for entry in fs::Dir::read_from(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            result.insert(OsString::from_vec(name.to_vec()));
        }
    }
    Ok(result)
}

fn verified_copy(
    directory: &OwnedFd,
    name: &str,
    size: u64,
    sha256: &Sha256Hex,
    output: impl Write,
) -> io::Result<()> {
    let member = backup::open_regular_file(directory, name, size)?;
    let result = backup::copy_and_hash(member.take(size.saturating_add(1)), output)?;
    if result.size != size || &result.sha256 != sha256 {
        return Err(invalid());
    }
    Ok(())
}

/// Re-open no-follow members relative to held archive directories and verify
/// each copy independently when it is consumed.
pub struct VerifiedArchive {
    manifest: BackupManifest,
    root: OwnedFd,
    assets: OwnedFd,
}

impl VerifiedArchive {
    fn copy_dump(&self, output: impl Write) -> io::Result<()> {
        let digest = &self.manifest.database_dump;
        verified_copy(
            &self.root,
            backup::DATABASE_DUMP,
            digest.size,
            &digest.sha256,
            output,
        )
    }

    fn copy_assets(&self, directory: &OwnedFd) -> io::Result<()> {
        for asset in &self.manifest.assets {
            let name = asset.id.to_string();
            let mut destination = backup::private_file(directory, &name)?;
            verified_copy(
                &self.assets,
                &name,
                asset.size,
                &asset.sha256,
                &mut destination,
            )?;
            destination.sync_all()?;
        }
        fs::fsync(directory)?;
        Ok(())
    }
}

pub fn verify_archive(path: &Path) -> io::Result<VerifiedArchive> {
    let root = safe_fs::open_root(path)?;
    let expected = [backup::MANIFEST, backup::DATABASE_DUMP, backup::ASSETS].map(OsString::from);
    if members(&root)? != BTreeSet::from(expected) {
        return Err(invalid());
    }
    let stat = fs::statat(&root, backup::MANIFEST, fs::AtFlags::SYMLINK_NOFOLLOW)?;
    let manifest_file = backup::open_regular_file(
        &root,
        backup::MANIFEST,
        stat.st_size.try_into().map_err(|_| invalid())?,
    )?;
    let manifest: BackupManifest = serde_json::from_reader(manifest_file).map_err(|_| invalid())?;
    if manifest.format_version != backup::FORMAT_VERSION {
        return Err(invalid());
    }
    let mut header = [0; 5];
    backup::open_regular_file(&root, backup::DATABASE_DUMP, manifest.database_dump.size)?
        .read_exact(&mut header)?;
    if &header != b"PGDMP" {
        return Err(invalid());
    }
    let assets = safe_fs::open_collection_directory(&root, Path::new(backup::ASSETS))?;
    let mut expected = BTreeSet::new();
    for asset in &manifest.assets {
        let name = asset.id.to_string();
        if !expected.insert(OsString::from(&name)) {
            return Err(invalid());
        }
        verified_copy(&assets, &name, asset.size, &asset.sha256, io::sink())?;
    }
    if expected != members(&assets)? {
        return Err(invalid());
    }
    let verified = VerifiedArchive {
        manifest,
        root,
        assets,
    };
    verified.copy_dump(io::sink())?;
    Ok(verified)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    instance_root: PathBuf,
    app_config: PathBuf,
}

fn port(origin: &str) -> Result<u16, &'static str> {
    Url::parse(origin)
        .ok()
        .and_then(|u| u.port_or_known_default())
        .ok_or("invalid service endpoint")
}

fn isolated_config(
    manifest: &BackupManifest,
    target: &Target,
    config: &AppConfig,
) -> Result<(TcpListener, TcpListener), &'static str> {
    let source = &manifest.source;
    let origin = Url::parse(&config.auth.origin).map_err(|_| "invalid restore origin")?;
    let old_origin = Url::parse(&source.origin).map_err(|_| "invalid backup origin")?;
    if origin == old_origin
        || origin.scheme() != "https"
        || knoted_core::auth::webauthn::build_webauthn(&config.auth).is_err()
        || config.auth.rp_id != source.rp_id
    {
        return Err("restore requires a separate HTTPS origin under the original RP ID");
    }
    if !target.instance_root.is_absolute()
        || !target.app_config.is_absolute()
        || target.app_config.starts_with(&target.instance_root)
        || config.assets.root != target.instance_root.join("assets")
        || config.files.staging_root.as_deref()
            != Some(target.instance_root.join("staging").as_path())
    {
        return Err("target runtime roots do not match the descriptor");
    }
    let old = [
        port(&source.intelligence_origin)?,
        port(&source.search.origin)?,
    ];
    let new = [
        port(&config.intelligence.base_url)?,
        port(&config.search.base_url)?,
    ];
    let old_indexes = [&source.search.document_index, &source.search.chunk_index];
    if new[0] == new[1]
        || new.iter().any(|p| old.contains(p))
        || old_indexes.contains(&&config.search.index_uid)
        || old_indexes.contains(&&config.search.rag_chunk_index_uid)
        || config.search.index_uid == config.search.rag_chunk_index_uid
        || !config.search.enabled
        || config.search_api_key().is_none_or(|s| s.is_empty())
    {
        return Err("restore requires separate service endpoints, credential and indexes");
    }
    let bind = config
        .server
        .as_ref()
        .ok_or("missing target server")?
        .bind
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "invalid target server")?;
    if !bind.ip().is_loopback()
        || bind.port() == 0
        || new.contains(&bind.port())
        || old.contains(&bind.port())
    {
        return Err("invalid target listener");
    }
    let api = TcpListener::bind(bind).map_err(|_| "target listener is occupied")?;
    let python_url =
        Url::parse(&config.intelligence.base_url).map_err(|_| "invalid target Python endpoint")?;
    let python_address = python_url
        .socket_addrs(|| None)
        .ok()
        .and_then(|items| items.into_iter().next())
        .ok_or("invalid target Python endpoint")?;
    let python =
        TcpListener::bind(python_address).map_err(|_| "target Python endpoint is occupied")?;
    Ok((api, python))
}

async fn empty_engine(config: &AppConfig) -> Result<(), &'static str> {
    let refused = "target search engine must be independently provisioned and empty";
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| refused)?;
    let origin = config.search.base_url.trim_end_matches('/');
    let key = config.search_api_key().ok_or(refused)?;
    for page in [
        "indexes?limit=1",
        "tasks?statuses=enqueued,processing&limit=1",
    ] {
        let body = client
            .get(format!("{origin}/{page}"))
            .bearer_auth(&key)
            .send()
            .await
            .map_err(|_| refused)?
            .error_for_status()
            .map_err(|_| refused)?
            .json::<serde_json::Value>()
            .await
            .map_err(|_| refused)?;
        if body["total"].as_u64() != Some(0)
            || !body["results"].as_array().is_some_and(Vec::is_empty)
        {
            return Err(refused);
        }
    }
    Ok(())
}

pub async fn run(command: &RestoreCommand) -> Result<(), &'static str> {
    let input = std::path::absolute(&command.input).map_err(|_| "invalid archive path")?;
    let archive = verify_archive(&input).map_err(|_| "archive verification failed")?;
    let target: Target = toml::from_str(
        &std::fs::read_to_string(&command.target).map_err(|_| "invalid target descriptor")?,
    )
    .map_err(|_| "invalid target descriptor")?;
    let config = AppConfig::load_for_process(&target.app_config, ProcessKind::Api)
        .map_err(|_| "invalid target app configuration")?;
    let _listeners = isolated_config(&archive.manifest, &target, &config)?;
    let source = &archive.manifest.source;
    let database = RestoreDatabase::preflight(
        &config.database().ok_or("missing target database")?.url,
        &source.database,
    )
    .await?;
    empty_engine(&config).await?;
    let mut protected = vec![input.as_path(), source.asset_root.as_path()];
    protected.extend(
        [
            source.file_root.as_deref(),
            source.staging_root.as_deref(),
            config.files.root.as_deref(),
        ]
        .into_iter()
        .flatten(),
    );
    let root = safe_fs::create_backup_directory(&target.instance_root, &protected)
        .map_err(|_| "target instance root is occupied or overlaps protected data")?;
    for name in ["assets", "staging", "python"] {
        fs::mkdirat(&root, name, fs::Mode::RWXU)
            .map_err(|_| "private directory creation failed")?;
    }
    let assets = safe_fs::open_collection_directory(&root, Path::new("assets"))
        .map_err(|_| "private asset directory unavailable")?;
    archive
        .copy_assets(&assets)
        .map_err(|_| "verified image copy failed")?;
    let mut dump = backup::private_file(&root, "install.dump")
        .map_err(|_| "private database copy unavailable")?;
    fs::unlinkat(&root, "install.dump", fs::AtFlags::empty())
        .map_err(|_| "private database copy unavailable")?;
    archive
        .copy_dump(&mut dump)
        .map_err(|_| "verified database copy failed")?;
    dump.seek(SeekFrom::Start(0))
        .map_err(|_| "private database copy unavailable")?;
    database
        .create()
        .await
        .map_err(|_| "staging database creation failed")?;
    let status = tokio::process::Command::new("pg_restore")
        .env_clear()
        .envs(database.environment.iter().cloned())
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .args([
            "--no-password",
            "--no-owner",
            "--no-privileges",
            "--exit-on-error",
            "--single-transaction",
            "--dbname",
            database.staging_name(),
        ])
        .stdin(dump)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .map_err(|_| "pg_restore unavailable")?;
    if !status.success() {
        return Err("pg_restore failed in private staging database");
    }
    let pool = database
        .pool()
        .await
        .map_err(|_| "staging database unavailable")?;
    let migrations: Vec<_> = archive
        .manifest
        .migrations
        .iter()
        .map(|m| (m.version, m.checksum.as_str()))
        .collect();
    knoted_core::db::restore::migrate_verified(&pool, &migrations).await?;
    let snapshot = Snapshot::open(&pool)
        .await
        .map_err(|_| "restored schema or image references invalid")?;
    let mut expected: Vec<_> = archive
        .manifest
        .assets
        .iter()
        .map(|a| (a.id, a.size, &a.sha256))
        .collect();
    expected.sort_by_key(|a| a.0);
    if !snapshot
        .assets
        .iter()
        .map(|a| (a.id, a.size, &a.sha256))
        .eq(expected)
    {
        return Err("restored inventory differs from manifest");
    }
    snapshot
        .finish()
        .await
        .map_err(|_| "restored inventory check failed")?;
    knoted_core::db::restore::retire_source_runtime(&pool)
        .await
        .map_err(|_| "source runtime retirement failed")?;
    let ctx = AppContext::new(pool.clone())
        .with_search(config.search.clone())
        .with_files(config.files.clone());
    while knoted_core::search::service::project_pending(&ctx)
        .await
        .map_err(|_| "target search projection failed")?
        > 0
    {}
    ctx.admission_pool().close().await;
    drop(ctx);
    knoted_core::db::restore::close_staging_pool(pool)
        .await
        .map_err(|_| "staging database disconnect failed")?;
    fs::fsync(&root).map_err(|_| "target directory durability check failed")?;
    database
        .publish()
        .await
        .map_err(|_| "database activation failed")
}
