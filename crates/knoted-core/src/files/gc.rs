//! Remove only named objects backed by an expired, confirmed execution.
use std::{collections::BTreeSet, io, os::fd::OwnedFd, path::Path};

use chrono::Duration;
use rustix::fs::{self, AtFlags, Dir, FileType};
use uuid::Uuid;

use crate::{
    context::AppContext,
    intelligence::{ExecutionKind, ExecutionState, IntelligenceClient, IntelligenceError},
    rag::embedding::staged_input_id,
};

use super::safe_fs;

#[derive(Debug, thiserror::Error)]
pub enum CollectionError {
    #[error("staging collection storage unavailable")]
    Storage(#[from] io::Error),
    #[error("staging execution status unavailable")]
    Intelligence(#[from] IntelligenceError),
}

fn open_directory(dir: &OwnedFd, name: &str) -> io::Result<Option<OwnedFd>> {
    match safe_fs::open_collection_directory(dir, Path::new(name)) {
        Ok(fd) => Ok(Some(fd)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn canonical_id(name: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(name).ok()?;
    (name == id.hyphenated().to_string()).then_some(id)
}

fn embed_id(name: &str) -> Option<Uuid> {
    let (id, digest) = name
        .strip_prefix("embed-")?
        .strip_suffix(".json")?
        .rsplit_once('-')?;
    super::Sha256Hex::parse(digest).ok()?;
    canonical_id(id)
}

fn execution_ids(input: Option<&OwnedFd>, output: Option<&OwnedFd>) -> io::Result<BTreeSet<Uuid>> {
    let mut ids = BTreeSet::new();
    if let Some(dir) = input {
        for entry in Dir::read_from(dir)? {
            let entry = entry?;
            if entry.file_type() == FileType::RegularFile
                && let Ok(name) = entry.file_name().to_str()
                && let Some(id) = embed_id(name)
            {
                ids.insert(id);
            }
        }
    }
    if let Some(dir) = output {
        for entry in Dir::read_from(dir)? {
            let entry = entry?;
            if entry.file_type() == FileType::Directory
                && let Ok(name) = entry.file_name().to_str()
                && let Some(id) = canonical_id(name)
            {
                ids.insert(id);
            }
        }
    }
    Ok(ids)
}

pub async fn collect_staging(ctx: &AppContext) -> Result<usize, CollectionError> {
    let Some(root) = ctx.files().staging_root.as_deref() else {
        return Ok(0);
    };
    let root = safe_fs::open_collection_root(ctx, root)?;
    let input = open_directory(&root, "input")?;
    let output = open_directory(&root, "output")?;
    let ids = execution_ids(input.as_ref(), output.as_ref())?;
    if ids.is_empty() {
        return Ok(0);
    }

    let client = IntelligenceClient::from_context(ctx)?;
    let cutoff = ctx.now() - Duration::hours(24);
    let mut removed = 0;
    let mut failure = None;
    for id in ids {
        let Ok(record) = client.get_execution(id).await else {
            // Missing proof, including 404 and temporary unavailability,
            // cannot authorize reclaiming a possibly live object.
            continue;
        };
        if record.execution_id != id
            || !record.state.is_terminal()
            || !record.compute_exit_confirmed
            || record.finished_at.is_none_or(|finished| finished > cutoff)
        {
            continue;
        }
        let result =
            remove_execution_objects(&client, &record, input.as_ref(), output.as_ref()).await;
        match result {
            Ok(count) => removed += count,
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(removed), Err)
}

async fn remove_execution_objects(
    client: &IntelligenceClient,
    record: &crate::intelligence::ExecutionRecord,
    input: Option<&OwnedFd>,
    output: Option<&OwnedFd>,
) -> Result<usize, CollectionError> {
    let id = record.execution_id;
    let mut removed = 0;
    if record.kind == ExecutionKind::Embed
        && let Some(input) = input
    {
        let name = staged_input_id(id, &record.input_sha256);
        removed += usize::from(safe_fs::unlink_regular(input, &name)?);
    }
    if let Some(output) = output
        && let Some(dir) = open_directory(output, &id.to_string())?
    {
        let artifact = format!("{id}-{}.json", record.kind);
        if record.state == ExecutionState::Succeeded {
            let result = client.get_result(id).await?;
            let echoes = [
                ("kind", record.kind.to_string()),
                ("source_version", record.source_version.to_string()),
                ("input_sha256", record.input_sha256.to_string()),
                ("options_revision", record.options_revision.clone()),
            ];
            if result.media_type != "application/json"
                || result.object_id.as_str() != artifact
                || echoes
                    .iter()
                    .any(|(key, value)| result.metadata.get(*key) != Some(value))
            {
                return Err(io::Error::from(io::ErrorKind::InvalidData).into());
            }
        }
        // Output names come from the controlled Python writer. Cancel or
        // deadline can win after the artifact write, and a killed writer can
        // leave an interrupted atomic rename with the same known target.
        for entry in Dir::read_from(&dir).map_err(io::Error::from)? {
            let entry = entry.map_err(io::Error::from)?;
            if let Ok(name) = entry.file_name().to_str()
                && [artifact.as_str(), "child-status.json"]
                    .iter()
                    .any(|target| name == *target || is_partial_output(name, target))
            {
                removed += usize::from(safe_fs::unlink_regular(&dir, name)?);
            }
        }
        match fs::unlinkat(output, id.to_string(), AtFlags::REMOVEDIR) {
            Ok(()) | Err(rustix::io::Errno::NOENT | rustix::io::Errno::NOTEMPTY) => (),
            Err(error) => return Err(io::Error::from(error).into()),
        }
    }
    Ok(removed)
}

fn is_partial_output(name: &str, target: &str) -> bool {
    name.strip_prefix('.')
        .and_then(|name| name.strip_prefix(target))
        .and_then(|suffix| suffix.strip_prefix('.'))
        .and_then(|suffix| suffix.strip_suffix(".part"))
        .is_some_and(|suffix| {
            suffix.len() == 32
                && suffix
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
}
