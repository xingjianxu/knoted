//! Bounded, digest-verified reads shared by Python execution adapters.
use super::{Sha256Hex, safe_fs};
use crate::{
    context::AppContext,
    intelligence::{ExecutionRequest, ExecutionResult},
};
use rustix::fs::{self, FileType, Mode, OFlags};
use std::{fs::File, io::Read};
pub const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid or incomplete execution artifact")]
    Invalid,
    #[error("execution artifact storage unavailable")]
    Io(#[from] std::io::Error),
}
pub(crate) async fn read_bytes(
    ctx: &AppContext,
    request: &ExecutionRequest,
    result: &ExecutionResult,
) -> Result<Vec<u8>, Error> {
    if result.size > MAX_ARTIFACT_BYTES || result.media_type != "application/json" {
        return Err(Error::Invalid);
    }
    for (key, expected) in [
        ("kind", request.kind.to_string()),
        ("source_version", request.source_version.to_string()),
        ("input_sha256", request.input_sha256.to_string()),
        ("options_revision", request.options_revision.clone()),
    ] {
        if result.metadata.get(key) != Some(&expected) {
            return Err(Error::Invalid);
        }
    }
    let root = ctx
        .files()
        .staging_root
        .as_ref()
        .ok_or(Error::Invalid)?
        .join("output")
        .join(request.execution_id.to_string());
    let dir = safe_fs::open_root(&root)?;
    let fd = fs::openat(
        &dir,
        result.object_id.as_str(),
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    if FileType::from_raw_mode(fs::fstat(&fd).map_err(std::io::Error::from)?.st_mode)
        != FileType::RegularFile
    {
        return Err(Error::Invalid);
    }
    let mut file = File::from(fd);
    let before = safe_fs::metadata(&file)?;
    if before.size != result.size {
        return Err(Error::Invalid);
    }
    let mut bytes = Vec::new();
    let mut buffer = vec![0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if bytes.len() as u64 + count as u64 > result.size {
            return Err(Error::Invalid);
        }
        bytes.extend_from_slice(&buffer[..count]);
        tokio::task::yield_now().await;
    }
    if bytes.len() as u64 != result.size
        || Sha256Hex::digest(&bytes) != result.sha256
        || safe_fs::metadata(&file)? != before
    {
        return Err(Error::Invalid);
    }
    Ok(bytes)
}
