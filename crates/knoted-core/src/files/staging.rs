//! Object-id validated, atomically written staging storage.
//!
//! The two value types here are load-bearing beyond this module: [`ObjectId`]
//! is the only handle to a staged file (it rejects separators, `..` segments
//! and URLs) and [`Sha256Hex`] is the only accepted digest format, so the
//! internal protocol client cannot silently accept a "digest" that is not 64
//! lowercase hex characters.

use std::fmt::{self, Write as _};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

/// Longest accepted object id, in bytes.
pub const MAX_OBJECT_ID_LEN: usize = 128;

/// Suffix of the temporary file an atomic write creates.
pub const TEMP_SUFFIX: &str = ".part";

/// A staging failure.
#[derive(Debug, thiserror::Error)]
pub enum StagingError {
    /// The id is not a controlled key.
    #[error("object id {value:?} is not a controlled staging key: {reason}")]
    InvalidObjectId {
        /// The rejected value.
        value: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// The digest is not 64 lowercase hex characters.
    #[error("sha256 {value:?} is not 64 lowercase hexadecimal characters")]
    InvalidDigest {
        /// The rejected value.
        value: String,
    },
    /// The object does not exist.
    #[error("staged object {object_id:?} does not exist")]
    Missing {
        /// The missing object.
        object_id: String,
    },
    /// The object exceeds the configured limit.
    #[error("staged object {object_id:?} is {actual} bytes, above the {limit} byte limit")]
    TooLarge {
        /// The offending object.
        object_id: String,
        /// Size on disk.
        actual: u64,
        /// Configured maximum.
        limit: u64,
    },
    /// The content does not hash to the expected digest.
    #[error("staged object {object_id:?} hashes to {actual}, expected {expected}")]
    DigestMismatch {
        /// The offending object.
        object_id: String,
        /// Digest the caller expected.
        expected: String,
        /// Digest recomputed from disk.
        actual: String,
    },
    /// The content has the wrong length.
    #[error("staged object {object_id:?} is {actual} bytes, expected {expected}")]
    SizeMismatch {
        /// The offending object.
        object_id: String,
        /// Size the caller expected.
        expected: u64,
        /// Size found on disk.
        actual: u64,
    },
    /// The id resolved outside the staging root.
    #[error("object id {object_id:?} resolves outside the staging root")]
    EscapesRoot {
        /// The offending object.
        object_id: String,
    },
    /// An underlying file-system error.
    #[error("staging io error at {path}: {source}")]
    Io {
        /// Path being touched.
        path: PathBuf,
        /// The file-system error.
        #[source]
        source: io::Error,
    },
}

/// A validated staging key.
///
/// The accepted shape is the contract pattern `^(?!\.\.?$)[A-Za-z0-9._-]+$`
/// plus an explicit ban on any `..` substring: what a key may not express, a
/// caller cannot use to escape the staging root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ObjectId(String);

impl ObjectId {
    /// Validates `value` as a staging key.
    pub fn parse(value: impl Into<String>) -> Result<ObjectId, StagingError> {
        let value = value.into();
        let reason = if value.is_empty() {
            "an object id must not be empty"
        } else if value.len() > MAX_OBJECT_ID_LEN {
            "an object id is at most 128 bytes long"
        } else if value == "." || value == ".." {
            "'.' and '..' are directory names, not objects"
        } else if value.contains("..") {
            "an object id must not contain '..'"
        } else if value.contains("://") {
            "a network URL is not a staging object"
        } else if value.contains('/') || value.contains('\\') {
            "an object id must not contain a path separator"
        } else if value.contains(':') {
            "an object id must not contain ':'"
        } else if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            "an object id may only contain letters, digits, '.', '-' and '_'"
        } else {
            return Ok(ObjectId(value));
        };
        Err(StagingError::InvalidObjectId { value, reason })
    }

    /// The key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ObjectId {
    type Error = StagingError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        ObjectId::parse(value)
    }
}

impl From<ObjectId> for String {
    fn from(value: ObjectId) -> String {
        value.0
    }
}

impl AsRef<str> for ObjectId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A lowercase hexadecimal SHA-256 digest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256Hex(String);

impl Sha256Hex {
    /// Validates an existing digest string.
    pub fn parse(value: impl Into<String>) -> Result<Sha256Hex, StagingError> {
        let value = value.into();
        if value.len() == 64
            && value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Ok(Sha256Hex(value));
        }
        Err(StagingError::InvalidDigest { value })
    }

    /// Computes the digest of `bytes`.
    pub fn digest(bytes: &[u8]) -> Sha256Hex {
        Sha256Hex(to_hex(&Sha256::digest(bytes)))
    }

    /// The digest as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Sha256Hex {
    type Error = StagingError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Sha256Hex::parse(value)
    }
}

impl From<Sha256Hex> for String {
    fn from(value: Sha256Hex) -> String {
        value.0
    }
}

/// What the store knows about one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedObject {
    /// The object's key.
    pub object_id: ObjectId,
    /// Digest of the stored bytes.
    pub sha256: Sha256Hex,
    /// Length of the stored bytes.
    pub size: u64,
}

/// A flat, object-id addressed staging directory.
///
/// Every write is atomic: the bytes go to a temporary file in the same
/// directory, are flushed and fsynced, and are then renamed over the
/// destination. A reader therefore sees either the previous object or the
/// complete new one, never a partial write.
#[derive(Debug, Clone)]
pub struct StagingStore {
    root: PathBuf,
    max_object_bytes: u64,
}

impl StagingStore {
    /// Opens (or creates) a staging directory.
    ///
    /// `max_object_bytes` bounds both written and read objects; a caller that
    /// stages an oversized artifact gets [`StagingError::TooLarge`] instead of a
    /// partial file.
    pub fn new(
        root: impl Into<PathBuf>,
        max_object_bytes: u64,
    ) -> Result<StagingStore, StagingError> {
        let store = StagingStore {
            root: root.into(),
            max_object_bytes,
        };
        store.ensure_root()?;
        Ok(store)
    }

    /// The staging root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Largest object, in bytes, the store accepts.
    pub fn max_object_bytes(&self) -> u64 {
        self.max_object_bytes
    }

    /// Absolute path of `object_id`, guaranteed inside the root.
    pub fn path(&self, object_id: &ObjectId) -> Result<PathBuf, StagingError> {
        self.resolve(object_id)
    }

    /// Atomically writes `bytes` as `object_id` and returns its digest and size.
    pub fn write(&self, object_id: &ObjectId, bytes: &[u8]) -> Result<StagedObject, StagingError> {
        let size = bytes.len() as u64;
        if size > self.max_object_bytes {
            return Err(StagingError::TooLarge {
                object_id: object_id.as_str().to_owned(),
                actual: size,
                limit: self.max_object_bytes,
            });
        }
        self.ensure_root()?;
        let destination = self.resolve(object_id)?;
        let temporary = destination.with_file_name(format!(
            ".{}.{}{TEMP_SUFFIX}",
            object_id.as_str(),
            Uuid::new_v4().simple()
        ));
        if let Err(source) = self.write_temporary(&temporary, bytes) {
            let _ = fs::remove_file(&temporary);
            return Err(StagingError::Io {
                path: temporary,
                source,
            });
        }
        if let Err(source) = fs::rename(&temporary, &destination) {
            let _ = fs::remove_file(&temporary);
            return Err(StagingError::Io {
                path: destination,
                source,
            });
        }
        Ok(StagedObject {
            object_id: object_id.clone(),
            sha256: Sha256Hex::digest(bytes),
            size,
        })
    }

    /// Reads the object's bytes, refusing anything above the size limit.
    pub fn read(&self, object_id: &ObjectId) -> Result<Vec<u8>, StagingError> {
        let path = self.resolve(object_id)?;
        let metadata = fs::metadata(&path).map_err(|source| match source.kind() {
            io::ErrorKind::NotFound => StagingError::Missing {
                object_id: object_id.as_str().to_owned(),
            },
            _ => StagingError::Io {
                path: path.clone(),
                source,
            },
        })?;
        if metadata.len() > self.max_object_bytes {
            return Err(StagingError::TooLarge {
                object_id: object_id.as_str().to_owned(),
                actual: metadata.len(),
                limit: self.max_object_bytes,
            });
        }
        fs::read(&path).map_err(|source| StagingError::Io { path, source })
    }

    /// Recomputes the digest and size currently on disk.
    pub fn stat(&self, object_id: &ObjectId) -> Result<StagedObject, StagingError> {
        let path = self.resolve(object_id)?;
        let mut file = File::open(&path).map_err(|source| match source.kind() {
            io::ErrorKind::NotFound => StagingError::Missing {
                object_id: object_id.as_str().to_owned(),
            },
            _ => StagingError::Io {
                path: path.clone(),
                source,
            },
        })?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut size = 0_u64;
        loop {
            let read = file.read(&mut buffer).map_err(|source| StagingError::Io {
                path: path.clone(),
                source,
            })?;
            if read == 0 {
                break;
            }
            size += read as u64;
            if size > self.max_object_bytes {
                return Err(StagingError::TooLarge {
                    object_id: object_id.as_str().to_owned(),
                    actual: size,
                    limit: self.max_object_bytes,
                });
            }
            hasher.update(&buffer[..read]);
        }
        Ok(StagedObject {
            object_id: object_id.clone(),
            sha256: Sha256Hex(to_hex(&hasher.finalize())),
            size,
        })
    }

    /// Recomputes the object and compares it with what the caller expects.
    ///
    /// This is the verification step a published result must pass: the bytes on
    /// disk have to match the digest and size the service reported.
    pub fn verify(&self, expected: &StagedObject) -> Result<StagedObject, StagingError> {
        let actual = self.stat(&expected.object_id)?;
        if actual.size != expected.size {
            return Err(StagingError::SizeMismatch {
                object_id: expected.object_id.as_str().to_owned(),
                expected: expected.size,
                actual: actual.size,
            });
        }
        if actual.sha256 != expected.sha256 {
            return Err(StagingError::DigestMismatch {
                object_id: expected.object_id.as_str().to_owned(),
                expected: expected.sha256.as_str().to_owned(),
                actual: actual.sha256.as_str().to_owned(),
            });
        }
        Ok(actual)
    }

    fn write_temporary(&self, temporary: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = File::create(temporary)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()
    }

    fn ensure_root(&self) -> Result<(), StagingError> {
        fs::create_dir_all(&self.root).map_err(|source| StagingError::Io {
            path: self.root.clone(),
            source,
        })
    }

    fn resolve(&self, object_id: &ObjectId) -> Result<PathBuf, StagingError> {
        self.ensure_root()?;
        let root = fs::canonicalize(&self.root).map_err(|source| StagingError::Io {
            path: self.root.clone(),
            source,
        })?;
        let candidate = root.join(object_id.as_str());
        // Object ids contain no separators, so the parent must be the root. The
        // explicit check keeps that invariant a property of the store rather
        // than of the validator alone.
        if candidate.parent() != Some(root.as_path()) {
            return Err(StagingError::EscapesRoot {
                object_id: object_id.as_str().to_owned(),
            });
        }
        if candidate.exists() {
            let canonical = fs::canonicalize(&candidate).map_err(|source| StagingError::Io {
                path: candidate.clone(),
                source,
            })?;
            if !canonical.starts_with(&root) {
                return Err(StagingError::EscapesRoot {
                    object_id: object_id.as_str().to_owned(),
                });
            }
        }
        Ok(candidate)
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing into a String cannot fail.
        write!(&mut hex, "{byte:02x}").expect("writing into a String cannot fail");
    }
    hex
}
