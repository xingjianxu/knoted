//! Local backup entry point and archive format shared with isolated restore.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use chrono::{DateTime, Utc};
use knoted_core::config::AppConfig;
use knoted_core::context::AppContext;
use knoted_core::db::{self, backup::Snapshot};
use knoted_core::files::{safe_fs, staging::Sha256Hex};
use knoted_core::maintenance::{self, ExclusiveGuard};
use rustix::fs::{self, FileType, Mode, OFlags, RenameFlags, ResolveFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const USAGE: &str = "usage: knoted-api admin backup --output <new-directory> [--config <PATH>]";
pub const FORMAT_VERSION: u32 = 1;
pub const MANIFEST: &str = "manifest.json";
pub const DATABASE_DUMP: &str = "database.dump";
pub const ASSETS: &str = "assets";
const WAIT_FOR_WRITES: Duration = Duration::from_secs(90);
const SNAPSHOT_DEADLINE: Duration = Duration::from_secs(60);
const LOCK_CHECK_DEADLINE: Duration = Duration::from_secs(10);
const DUMP_DEADLINE: Duration = Duration::from_secs(900);

pub struct BackupCommand {
    pub output: PathBuf,
    pub config: PathBuf,
}

impl BackupCommand {
    pub fn parse(args: &[String]) -> Result<Self, BackupError> {
        let mut config = None;
        let mut output = None;
        let mut parts = args.iter();
        while let Some(arg) = parts.next() {
            let slot = match arg.as_str() {
                "--config" if config.is_none() => &mut config,
                "--output" if output.is_none() => &mut output,
                _ => return Err(BackupError::Usage),
            };
            let value = parts
                .next()
                .filter(|arg| !arg.is_empty() && !arg.starts_with("--"))
                .ok_or(BackupError::Usage)?;
            *slot = Some(PathBuf::from(value));
        }
        Ok(Self {
            output: output.ok_or(BackupError::Usage)?,
            config: config.unwrap_or_else(AppConfig::config_path_from_env),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("expected --output <new-directory> and optional --config <path>")]
    Usage,
    #[error("the configured database is unavailable")]
    Database,
    #[error("the PostgreSQL connection settings are unsupported by the backup command")]
    Connection,
    #[error("exclusive maintenance admission unavailable or prior writes unresolved")]
    Maintenance,
    #[error(transparent)]
    Quiescence(#[from] maintenance::QuiescenceError),
    #[error(transparent)]
    Snapshot(#[from] db::backup::SnapshotError),
    #[error("backup snapshot did not respond before the deadline")]
    SnapshotTimeout,
    #[error("could not create or synchronize private backup files: {0}")]
    Archive(#[from] io::Error),
    #[error("could not read and verify registered image {0}")]
    Asset(Uuid),
    #[error("pg_dump failed or timed out; no completed archive was published")]
    Dump,
    #[error("running artifact revision is invalid")]
    Revision,
    #[error("could not encode the backup manifest")]
    Manifest,
    #[error("archive publication is uncertain; isolate this output and retry: {0}")]
    PublicationUncertain(io::Error),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    pub format_version: u32,
    pub completed_at: DateTime<Utc>,
    pub app_version: String,
    pub artifact_revision: Option<String>,
    pub source: BackupSource,
    pub migrations: Vec<BackupMigration>,
    pub database_dump: FileDigest,
    pub assets: Vec<BackupAsset>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupSource {
    pub database: String,
    pub rp_id: String,
    pub origin: String,
    pub asset_root: PathBuf,
    pub staging_root: Option<PathBuf>,
    pub file_root: Option<PathBuf>,
    pub intelligence_origin: String,
    pub search: Option<BackupSearch>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupSearch {
    pub origin: String,
    pub document_index: String,
    pub chunk_index: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupMigration {
    pub version: i64,
    pub checksum: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileDigest {
    pub size: u64,
    pub sha256: Sha256Hex,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupAsset {
    pub id: Uuid,
    pub size: u64,
    pub sha256: Sha256Hex,
}

pub async fn run(
    command: &BackupCommand,
    config: &AppConfig,
) -> Result<BackupManifest, BackupError> {
    let database = config.database().ok_or(BackupError::Database)?;
    let pool = db::connect(database)
        .await
        .map_err(|_| BackupError::Database)?;
    let ctx = AppContext::new(pool)
        .with_asset_root(config.assets.root.clone())
        .with_files(config.files.clone())
        .with_intelligence(config.intelligence.clone())
        .with_search(config.search.clone())
        .with_search_api_key(config.search_api_key())
        .with_vectors_enabled(config.vectors.enabled);
    let output = std::path::absolute(&command.output)?;
    let protected: Vec<&Path> = std::iter::once(ctx.asset_root())
        .chain(ctx.files().root.as_deref())
        .chain(ctx.files().staging_root.as_deref())
        .collect();
    let revision = artifact_revision()?;
    let mut guard = maintenance::exclusive(ctx.admission_pool(), WAIT_FOR_WRITES)
        .await
        .map_err(|_| BackupError::Maintenance)?;
    maintenance::verify_quiescence(&ctx).await?;
    check_maintenance(&mut guard).await?;
    let snapshot = tokio::time::timeout(SNAPSHOT_DEADLINE, Snapshot::open(ctx.pool()))
        .await
        .map_err(|_| BackupError::SnapshotTimeout)??;
    let environment = db::backup::dump_environment(
        &database.url,
        &ctx.pool().connect_options(),
        &snapshot.database,
    )
    .ok_or(BackupError::Connection)?;
    let root = safe_fs::create_backup_directory(&output, &protected)?;
    let dump = export_database(&root, &snapshot.id, environment, &mut guard).await?;
    let assets = copy_assets(&root, ctx.asset_root(), &snapshot.assets)?;
    let source = BackupSource {
        database: snapshot.database.clone(),
        rp_id: config.auth.rp_id.clone(),
        origin: config.auth.origin.clone(),
        asset_root: config.assets.root.clone(),
        staging_root: config.files.staging_root.clone(),
        file_root: config.files.root.clone(),
        intelligence_origin: config.intelligence.base_url.clone(),
        search: config.search.enabled.then(|| BackupSearch {
            origin: config.search.base_url.clone(),
            document_index: config.search.index_uid.clone(),
            chunk_index: config.search.rag_chunk_index_uid.clone(),
        }),
    };
    let migrations = snapshot
        .migrations
        .iter()
        .map(|(version, checksum)| BackupMigration {
            version: *version,
            checksum: checksum.clone(),
        })
        .collect();
    tokio::time::timeout(SNAPSHOT_DEADLINE, snapshot.finish())
        .await
        .map_err(|_| BackupError::SnapshotTimeout)??;
    let manifest = BackupManifest {
        format_version: FORMAT_VERSION,
        completed_at: ctx.now(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        artifact_revision: revision,
        source,
        migrations,
        database_dump: dump,
        assets,
    };
    publish_manifest(&root, &manifest, &mut guard).await?;
    Ok(manifest)
}

async fn check_maintenance(guard: &mut ExclusiveGuard) -> Result<(), BackupError> {
    tokio::time::timeout(LOCK_CHECK_DEADLINE, guard.check())
        .await
        .map_err(|_| BackupError::Maintenance)?
        .map_err(|_| BackupError::Maintenance)
}

async fn export_database(
    root: &OwnedFd,
    snapshot: &str,
    environment: [(&str, String); 5],
    guard: &mut ExclusiveGuard,
) -> Result<FileDigest, BackupError> {
    let mut file = private_file(root, DATABASE_DUMP)?;
    let mut command = tokio::process::Command::new("pg_dump");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PG") {
            command.env_remove(key);
        }
    }
    command
        .envs(environment)
        .args(["--format=custom", "--no-password", "--snapshot", snapshot])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(file.try_clone()?)
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| BackupError::Dump)?;
    let completed = tokio::time::timeout(DUMP_DEADLINE, async {
        loop {
            tokio::select! {
                report = child.wait() => break report.map_err(|_| BackupError::Dump),
                () = tokio::time::sleep(Duration::from_secs(3)) => {
                    check_maintenance(guard).await?;
                }
            }
        }
    })
    .await
    .map_err(|_| BackupError::Dump)??;
    if !completed.success() {
        return Err(BackupError::Dump);
    }
    file.sync_all()?;
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0; 5];
    file.read_exact(&mut header)
        .map_err(|_| BackupError::Dump)?;
    if &header != b"PGDMP" {
        return Err(BackupError::Dump);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(copy_and_hash(file, io::sink())?)
}

fn copy_assets(
    root: &OwnedFd,
    asset_root: &Path,
    inventory: &[db::backup::BackupAsset],
) -> Result<Vec<BackupAsset>, BackupError> {
    fs::mkdirat(root, ASSETS, Mode::RWXU).map_err(io::Error::from)?;
    let output = safe_fs::open_directory(root, Path::new(ASSETS))?;
    let mut assets = Vec::with_capacity(inventory.len());
    if !inventory.is_empty() {
        let source =
            safe_fs::open_root(asset_root).map_err(|_| BackupError::Asset(inventory[0].id))?;
        for entry in inventory {
            let copy = (|| -> io::Result<_> {
                let name = entry.id.to_string();
                let input = open_regular_file(&source, &name, entry.size)?;
                let mut destination = private_file(&output, &name)?;
                let digest = copy_and_hash(input.take(entry.size + 1), &mut destination)?;
                if digest.size != entry.size || digest.sha256 != entry.sha256 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                destination.sync_all()?;
                Ok(BackupAsset {
                    id: entry.id,
                    size: digest.size,
                    sha256: digest.sha256,
                })
            })()
            .map_err(|_| BackupError::Asset(entry.id))?;
            assets.push(copy);
        }
    }
    fs::fsync(&output).map_err(io::Error::from)?;
    Ok(assets)
}

fn open_regular_file(directory: impl AsFd, name: &str, size: u64) -> io::Result<File> {
    let fd = fs::openat2(
        &directory,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_XDEV | ResolveFlags::NO_SYMLINKS,
    )?;
    let stat = fs::fstat(&fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_nlink != 1
        || u64::try_from(stat.st_size).ok() != Some(size)
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(File::from(fd))
}

fn copy_and_hash(mut input: impl Read, mut output: impl Write) -> io::Result<FileDigest> {
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            let sha256 = Sha256Hex::parse(format!("{:x}", hash.finalize()))
                .map_err(|_| io::ErrorKind::InvalidData)?;
            return Ok(FileDigest { size, sha256 });
        }
        let bytes = &buffer[..count];
        hash.update(bytes);
        output.write_all(bytes)?;
        size += count as u64;
    }
}

fn private_file(directory: impl AsFd, name: &str) -> io::Result<File> {
    Ok(File::from(fs::openat(
        directory,
        name,
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )?))
}

async fn publish_manifest(
    root: &OwnedFd,
    manifest: &BackupManifest,
    guard: &mut ExclusiveGuard,
) -> Result<(), BackupError> {
    const INCOMPLETE: &str = "manifest.pending";
    let mut file = private_file(root, INCOMPLETE)?;
    serde_json::to_writer_pretty(&mut file, manifest).map_err(|_| BackupError::Manifest)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::fsync(root).map_err(io::Error::from)?;
    check_maintenance(guard).await?;
    fs::renameat_with(root, INCOMPLETE, root, MANIFEST, RenameFlags::NOREPLACE)
        .map_err(io::Error::from)?;
    if let Err(error) = fs::fsync(root) {
        let removed =
            fs::unlinkat(root, MANIFEST, fs::AtFlags::empty()).and_then(|()| fs::fsync(root));
        let original = io::Error::from(error);
        return Err(if removed.is_ok() {
            BackupError::Archive(original)
        } else {
            BackupError::PublicationUncertain(original)
        });
    }
    Ok(())
}

fn artifact_revision() -> Result<Option<String>, BackupError> {
    let executable = std::env::current_exe()?;
    let Some(root) = executable.parent().and_then(Path::parent) else {
        return Err(BackupError::Revision);
    };
    match std::fs::read_to_string(root.join("share/knoted/REVISION")) {
        Ok(content) => {
            let revision = content.trim();
            if revision.len() != 40
                || !revision
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err(BackupError::Revision);
            }
            Ok(Some(revision.to_owned()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
