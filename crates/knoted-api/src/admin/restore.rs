//! Operator entry point for isolated restore.

use super::backup::{self, BackupManifest};
use knoted_core::config::{AppConfig, ProcessKind};
use knoted_core::context::AppContext;
use knoted_core::db::{backup::Snapshot, restore::RestoreDatabase};
use knoted_core::files::{safe_fs, staging::Sha256Hex};
use rustix::fs;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::net::TcpListener;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use url::Url;
use uuid::Uuid;

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

    fn pin(self, target: &Path, protected: &[&Path]) -> io::Result<PinnedArchive> {
        let mut dump = scratch_file(target, protected)?;
        self.copy_dump(&mut dump)?;
        dump.rewind()?;
        let mut images = scratch_file(target, protected)?;
        for asset in &self.manifest.assets {
            verified_copy(
                &self.assets,
                &asset.id.to_string(),
                asset.size,
                &asset.sha256,
                &mut images,
            )?;
        }
        images.rewind()?;
        Ok(PinnedArchive {
            manifest: self.manifest,
            dump,
            images,
        })
    }
}

/// Create a private sibling and immediately unlink it. This keeps descriptor
/// usage bounded even for archives with many images and releases scratch bytes
/// automatically on every return or process exit.
fn scratch_file(target: &Path, protected: &[&Path]) -> io::Result<File> {
    let name = format!(".knoted-restore-{}", Uuid::new_v4());
    let location = target.with_file_name(&name);
    let mut exclusions = protected.to_vec();
    exclusions.push(target);
    let parent = safe_fs::checked_output_parent(&location, &exclusions)?;
    let file = backup::private_file(&parent, &name)?;
    fs::unlinkat(&parent, name.as_str(), fs::AtFlags::empty())?;
    Ok(file)
}

struct PinnedArchive {
    manifest: BackupManifest,
    dump: File,
    images: File,
}

impl PinnedArchive {
    fn install_images(&mut self, directory: &OwnedFd) -> io::Result<()> {
        for asset in &self.manifest.assets {
            let mut destination = backup::private_file(directory, &asset.id.to_string())?;
            let copied = io::copy(
                &mut Read::by_ref(&mut self.images).take(asset.size),
                &mut destination,
            )?;
            if copied != asset.size {
                return Err(invalid());
            }
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
) -> Result<[TcpListener; 2], &'static str> {
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
    if target.instance_root.file_name().is_none()
        || !target.instance_root.is_absolute()
        || !target.app_config.is_absolute()
        || target.app_config.starts_with(&target.instance_root)
        || config.assets.root != target.instance_root.join("assets")
        || config.files.staging_root.as_deref()
            != Some(target.instance_root.join("staging").as_path())
    {
        return Err("target runtime roots do not match the descriptor");
    }
    let source_ports = [source.api_port, port(&source.search.origin)?];
    let new_search_port = port(&config.search.base_url)?;
    if source_ports.contains(&0)
        || source_ports.contains(&new_search_port)
        || config.search.index_uid == source.search.document_index
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
        || bind.port() == new_search_port
        || source_ports.contains(&bind.port())
    {
        return Err("invalid target listener");
    }
    let api = TcpListener::bind(bind).map_err(|_| "target listener is occupied")?;
    let meili = reserve_endpoint(&config.search.base_url)?;
    Ok([api, meili])
}

fn reserve_endpoint(origin: &str) -> Result<TcpListener, &'static str> {
    let address = Url::parse(origin)
        .ok()
        .and_then(|u| u.socket_addrs(|| None).ok())
        .and_then(|items| items.into_iter().next())
        .ok_or("invalid target service endpoint")?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("invalid target service endpoint");
    }
    TcpListener::bind(address).map_err(|_| "target service endpoint is occupied")
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
    let mut protected = vec![input.clone(), source.asset_root.clone()];
    protected.extend(
        [
            source.file_root.as_deref(),
            source.staging_root.as_deref(),
            config.files.root.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(Path::to_path_buf),
    );
    let protected: Vec<_> = protected.iter().map(PathBuf::as_path).collect();
    let mut archive = archive
        .pin(&target.instance_root, &protected)
        .map_err(|_| "archive changed or private verification scratch unavailable")?;
    let root = safe_fs::create_backup_directory(&target.instance_root, &protected)
        .map_err(|_| "target instance root is occupied or overlaps protected data")?;
    for name in ["assets", "staging", "meili"] {
        fs::mkdirat(&root, name, fs::Mode::RWXU)
            .map_err(|_| "private directory creation failed")?;
    }
    let assets = safe_fs::open_collection_directory(&root, Path::new("assets"))
        .map_err(|_| "private asset directory unavailable")?;
    archive
        .install_images(&assets)
        .map_err(|_| "verified image copy failed")?;
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
        .stdin(archive.dump)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t69_1_changed_archive_never_installs_unverified_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("archive");
        let target = directory.path().join("target");
        let id = Uuid::new_v4();
        let images = archive.join(backup::ASSETS);
        std::fs::create_dir_all(&images).unwrap();
        let dump = archive.join(backup::DATABASE_DUMP);
        let image = images.join(id.to_string());
        let dump_bytes = b"PGDMP-original";
        let image_bytes = b"original image";
        std::fs::write(&dump, dump_bytes).unwrap();
        std::fs::write(&image, image_bytes).unwrap();
        let manifest = serde_json::json!({
            "format_version":2,"completed_at":"2026-01-01T00:00:00Z","app_version":"fixture",
            "artifact_revision":null,"migrations":[],
            "database_dump":{"size":dump_bytes.len(),"sha256":Sha256Hex::digest(dump_bytes)},
            "assets":[{"id":id,"size":image_bytes.len(),"sha256":Sha256Hex::digest(image_bytes)}],
            "source":{"database":"source","rp_id":"localhost","origin":"https://localhost","api_port":15602,
                "asset_root":"/original/assets","staging_root":null,"file_root":null,
                "search":{"enabled":false,
                "origin":"http://127.0.0.1:2","document_index":"docs"}}
        });
        std::fs::write(
            archive.join(backup::MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let initial = verify_archive(&archive).unwrap();
        std::fs::write(&image, b"modified image").unwrap();
        assert!(initial.pin(&target, &[&archive]).is_err());
        assert!(!target.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        std::fs::write(&image, image_bytes).unwrap();
        let mut pinned = verify_archive(&archive)
            .unwrap()
            .pin(&target, &[&archive])
            .unwrap();
        std::fs::write(&image, b"modified image").unwrap();
        std::fs::write(&dump, b"PGDMP-modified").unwrap();
        let installed = safe_fs::create_backup_directory(&target, &[&archive]).unwrap();
        pinned.install_images(&installed).unwrap();
        let mut restored = Vec::new();
        pinned.dump.read_to_end(&mut restored).unwrap();
        assert_eq!(restored, dump_bytes);
        assert_eq!(
            std::fs::read(target.join(id.to_string())).unwrap(),
            image_bytes
        );
    }
}
