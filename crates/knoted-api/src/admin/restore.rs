//! Operator entry point for isolated restore.

use std::path::PathBuf;

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

use super::backup::{self, BackupManifest};
use knoted_core::files::{safe_fs, staging::Sha256Hex};
use rustix::fs;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

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
    pub manifest: BackupManifest,
    root: OwnedFd,
    assets: OwnedFd,
}

impl VerifiedArchive {
    pub fn copy_dump(&self, output: impl Write) -> io::Result<()> {
        let digest = &self.manifest.database_dump;
        verified_copy(
            &self.root,
            backup::DATABASE_DUMP,
            digest.size,
            &digest.sha256,
            output,
        )
    }

    pub fn copy_assets(&self, directory: &OwnedFd) -> io::Result<()> {
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
    let assets = safe_fs::open_directory(&root, Path::new(backup::ASSETS))?;
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
