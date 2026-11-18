//! Fail-closed downloads from the current approved root, never staging or paths supplied by clients.
use super::{
    capture::MAX_INPUT_BYTES,
    safe_fs::{self, Identity, Metadata},
};
use crate::context::AppContext;
use rustix::fs::{self, FileType, Mode, OFlags};
use sha2::{Digest, Sha256};
use sqlx::types::Json;
use std::{fs::File, io::Read, path::Path};
use uuid::Uuid;
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("file not found")]
    NotFound,
    #[error("file unavailable")]
    Unavailable,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
impl From<std::io::Error> for DomainError {
    fn from(_: std::io::Error) -> Self {
        Self::Unavailable
    }
}
pub struct ReadContent {
    pub id: Uuid,
}
#[derive(Debug)]
pub struct FileContent {
    pub bytes: Vec<u8>,
    pub name: String,
    pub version: i64,
}
#[derive(Clone, PartialEq, sqlx::FromRow)]
struct Root {
    configured_path: String,
    identity: Option<Json<Identity>>,
    generation: i64,
    last_complete_generation: Option<i64>,
    scan_token: Option<Uuid>,
    paused_reason: Option<String>,
}
#[derive(Clone, PartialEq, sqlx::FromRow)]
struct Source {
    root_id: Uuid,
    relative_path: String,
    size: i64,
    sha256: String,
    content_version: i64,
    status: String,
    content_fresh: bool,
    observation: Json<Metadata>,
    captured_observation: Option<Json<Metadata>>,
    observation_version: i64,
    captured_observation_version: Option<i64>,
}
const ROOT: &str = "SELECT configured_path,identity,generation,last_complete_generation,scan_token,paused_reason FROM file_roots WHERE id=$1";
const SOURCE: &str = "SELECT root_id,relative_path,size,sha256,content_version,status,content_fresh,observation,captured_observation,observation_version,captured_observation_version FROM file_sources WHERE id=$1";
impl Source {
    fn validate(&self) -> Result<(), DomainError> {
        if self.status == "missing" {
            return Err(DomainError::NotFound);
        }
        if !self.content_fresh
            || self.captured_observation.as_ref() != Some(&self.observation)
            || self.captured_observation_version != Some(self.observation_version)
            || self.size < 0
            || self.size as u64 > MAX_INPUT_BYTES
            || self.observation.0.size != self.size as u64
        {
            return Err(DomainError::Unavailable);
        }
        Ok(())
    }
    fn open(&self, root: &Root) -> Result<File, DomainError> {
        let path = Path::new(&self.relative_path);
        if self
            .relative_path
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
            || path.is_absolute()
        {
            return Err(DomainError::Unavailable);
        }
        let root_fd = safe_fs::open_root(Path::new(&root.configured_path))?;
        if root.identity.as_ref().map(|i| &i.0) != Some(&safe_fs::metadata(&root_fd)?.identity) {
            return Err(DomainError::Unavailable);
        }
        let parent =
            safe_fs::open_directory(&root_fd, path.parent().ok_or(DomainError::Unavailable)?)?;
        let fd = fs::openat(
            &parent,
            path.file_name().ok_or(DomainError::Unavailable)?,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?;
        if FileType::from_raw_mode(fs::fstat(&fd).map_err(std::io::Error::from)?.st_mode)
            != FileType::RegularFile
            || safe_fs::metadata(&fd)? != self.observation.0
        {
            return Err(DomainError::Unavailable);
        }
        Ok(File::from(fd))
    }
}
fn verify_path(source: &Source, root: &Root, file: &File) -> Result<(), DomainError> {
    if safe_fs::metadata(file)? != source.observation.0 {
        return Err(DomainError::Unavailable);
    }
    let current = source.open(root)?;
    if safe_fs::metadata(current)? != safe_fs::metadata(file)? {
        return Err(DomainError::Unavailable);
    }
    Ok(())
}
pub async fn read_content(
    ctx: &AppContext,
    input: ReadContent,
) -> Result<FileContent, DomainError> {
    let source: Source = sqlx::query_as(SOURCE)
        .bind(input.id)
        .fetch_optional(ctx.pool())
        .await?
        .ok_or(DomainError::NotFound)?;
    source.validate()?;
    let root: Root = sqlx::query_as(ROOT)
        .bind(source.root_id)
        .fetch_optional(ctx.pool())
        .await?
        .ok_or(DomainError::Unavailable)?;
    if !ctx.files().enabled
        || ctx.files().root.as_deref() != Some(Path::new(&root.configured_path))
        || root.identity.is_none()
        || root.paused_reason.is_some()
        || root.scan_token.is_some()
        || root.last_complete_generation != Some(root.generation)
    {
        return Err(DomainError::Unavailable);
    }
    let s = source.clone();
    let r = root.clone();
    let (bytes, file) = tokio::task::spawn_blocking(move || read_verified(&s, &r))
        .await
        .map_err(|_| DomainError::Unavailable)??;
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        // Match scanner/capture lock ordering; annotation-only version changes are allowed.
        let current_root: Root = sqlx::query_as(&format!("{ROOT} FOR UPDATE"))
            .bind(source.root_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(DomainError::Unavailable)?;
        let current_source: Source = sqlx::query_as(&format!("{SOURCE} FOR UPDATE"))
            .bind(input.id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(DomainError::NotFound)?;
        current_source.validate()?;
        if root != current_root || source != current_source {
            return Err(DomainError::Unavailable);
        }
        let version: i64 = sqlx::query_scalar("SELECT version FROM file_sources WHERE id=$1")
            .bind(input.id)
            .fetch_one(&mut *tx)
            .await?;
        tokio::task::spawn_blocking(move || verify_path(&current_source, &current_root, &file))
            .await
            .map_err(|_| DomainError::Unavailable)??;
        Ok(FileContent {
            bytes,
            name: source
                .relative_path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .into(),
            version,
        })
    }
    .await;
    match result {
        Ok(v) => {
            tx.commit().await?;
            Ok(v)
        }
        Err(e) => {
            tx.rollback().await?;
            Err(e)
        }
    }
}

fn read_verified(s: &Source, r: &Root) -> Result<(Vec<u8>, File), DomainError> {
    let mut file = s.open(r)?;
    let mut bytes = Vec::with_capacity(s.size as usize);
    let mut buffer = [0u8; 64 * 1024];
    let mut hash = Sha256::new();
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        if bytes.len() as u64 + n as u64 > MAX_INPUT_BYTES || bytes.len() + n > s.size as usize {
            return Err(DomainError::Unavailable);
        }
        #[cfg(test)]
        AFTER_CHUNK.with(|hook| {
            if let Some(action) = hook.borrow_mut().take() {
                action();
            }
        });
        hash.update(&buffer[..n]);
        bytes.extend_from_slice(&buffer[..n]);
    }
    if bytes.len() as i64 != s.size || format!("{:x}", hash.finalize()) != s.sha256 {
        return Err(DomainError::Unavailable);
    }
    verify_path(s, r, &file)?;
    Ok((bytes, file))
}

#[cfg(test)]
thread_local! { static AFTER_CHUNK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t49_2_read_in_progress_changes_are_rejected() {
        std::fs::create_dir_all("/data/knoted-dev/tasks/t49/tmp").unwrap();
        for mode in ["overwrite", "truncate", "append", "rename", "symlink"] {
            let dir = tempfile::tempdir_in("/data/knoted-dev/tasks/t49/tmp").unwrap();
            let path = dir.path().join("sample");
            let bytes = vec![b'a'; 128 * 1024];
            std::fs::write(&path, &bytes).unwrap();
            let metadata = safe_fs::metadata(File::open(&path).unwrap()).unwrap();
            let source = Source {
                root_id: Uuid::new_v4(),
                relative_path: "sample".into(),
                size: bytes.len() as i64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                content_version: 1,
                status: "present".into(),
                content_fresh: true,
                observation: Json(metadata.clone()),
                captured_observation: Some(Json(metadata)),
                observation_version: 1,
                captured_observation_version: Some(1),
            };
            let root = Root {
                configured_path: dir.path().to_str().unwrap().into(),
                identity: Some(Json(
                    safe_fs::metadata(safe_fs::open_root(dir.path()).unwrap())
                        .unwrap()
                        .identity,
                )),
                generation: 1,
                last_complete_generation: Some(1),
                scan_token: None,
                paused_reason: None,
            };
            assert_eq!(read_verified(&source, &root).unwrap().0, bytes);
            AFTER_CHUNK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    use std::io::Write;
                    match mode {
                        "overwrite" => std::fs::write(&path, vec![b'b'; 128 * 1024]).unwrap(),
                        "truncate" => std::fs::OpenOptions::new()
                            .write(true)
                            .open(&path)
                            .unwrap()
                            .set_len(0)
                            .unwrap(),
                        "append" => std::fs::OpenOptions::new()
                            .append(true)
                            .open(&path)
                            .unwrap()
                            .write_all(b"new")
                            .unwrap(),
                        _ => {
                            let old = path.with_file_name("old");
                            std::fs::rename(&path, &old).unwrap();
                            if mode == "symlink" {
                                std::os::unix::fs::symlink(old, path).unwrap()
                            } else {
                                std::fs::write(path, vec![b'a'; 128 * 1024]).unwrap()
                            }
                        }
                    }
                }))
            });
            assert!(
                matches!(read_verified(&source, &root), Err(DomainError::Unavailable)),
                "{mode}"
            );
        }
    }
}
