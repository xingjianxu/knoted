//! BGE-M3 token-bounded, deterministic chunks of immutable source snapshots.

use crate::{files::text::ExtractedDocument, search::meili::SourceType};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
use tokenizers::{Encoding, Tokenizer};
use uuid::Uuid;

pub const CHUNKER_REVISION: &str = "bge-m3-5617a9f6-512-768-64-v1";
const TOKENIZER_ENV: &str = "KNOTED_INTELLIGENCE_BGE_M3_DIR";
const TOKENIZER_BYTES: u64 = 17_098_108;
const TOKENIZER_SHA256: &str = "21106b6d7dab2952c1d496fb21d5dc9db75c28ed361a05f5020bbba27810dd08";
const TARGET: usize = 512;
pub const MAX_TOKENS: usize = 768;
const OVERLAP: usize = 64;

#[derive(Clone)]
pub struct BgeTokenizer(Arc<Tokenizer>);

impl BgeTokenizer {
    pub fn from_env() -> Result<Self, DomainError> {
        let root = std::env::var_os(TOKENIZER_ENV).ok_or(DomainError::TokenizerUnavailable)?;
        Self::load(Path::new(&root))
    }

    /// Use the machine-level read-only snapshot, checking the locked tokenizer.
    pub fn load(root: &Path) -> Result<Self, DomainError> {
        let path = root.join("tokenizer.json");
        let metadata = path
            .metadata()
            .map_err(|_| DomainError::TokenizerUnavailable)?;
        if !metadata.is_file() || metadata.len() != TOKENIZER_BYTES {
            return Err(DomainError::TokenizerUnavailable);
        }
        let bytes = std::fs::read(&path).map_err(|_| DomainError::TokenizerUnavailable)?;
        if format!("{:x}", Sha256::digest(&bytes)) != TOKENIZER_SHA256 {
            return Err(DomainError::TokenizerUnavailable);
        }
        let mut tokenizer =
            Tokenizer::from_bytes(bytes).map_err(|_| DomainError::TokenizerUnavailable)?;
        tokenizer
            .with_truncation(None)
            .map_err(|_| DomainError::TokenizerUnavailable)?
            .with_padding(None);
        Ok(Self(Arc::new(tokenizer)))
    }

    fn encode(&self, text: &str) -> Result<Encoding, DomainError> {
        self.0
            .encode(text, false)
            .map_err(|_| DomainError::TokenizerFailed)
    }

    /// Text tokens, excluding the two model input delimiters.
    pub fn count(&self, text: &str) -> Result<usize, DomainError> {
        self.0
            .encode_fast(text, false)
            .map(|result| result.len())
            .map_err(|_| DomainError::TokenizerFailed)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("BGE-M3 tokenizer snapshot unavailable")]
    TokenizerUnavailable,
    #[error("BGE-M3 tokenizer failed")]
    TokenizerFailed,
    #[error("invalid chunk source or extraction")]
    InvalidSource,
}

pub struct ChunkConfig {
    pub revision: String,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            revision: CHUNKER_REVISION.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkSource {
    pub source_type: SourceType,
    pub source_id: Uuid,
    pub source_version: i64,
    pub extraction_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChunkLocation {
    pub block_ordinal: usize,
    /// A UTF-8 range relative to the original extracted block.
    pub byte_start: usize,
    pub byte_end: usize,
    pub page: Option<u32>,
    pub bbox: Option<[f64; 4]>,
    pub structure: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub id: Uuid,
    pub ordinal: usize,
    pub text: String,
    pub token_count: usize,
    pub locations: Vec<ChunkLocation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkSet {
    pub id: Uuid,
    pub source: ChunkSource,
    pub chunker_revision: String,
    pub chunks: Vec<Chunk>,
}

fn stable_id(parts: &[&[u8]]) -> Uuid {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    let bytes: [u8; 16] = digest.finalize()[..16]
        .try_into()
        .expect("fixed digest length");
    uuid::Builder::from_custom_bytes(bytes).into_uuid()
}

impl ChunkSet {
    fn push(&mut self, text: String, token_count: usize, locations: Vec<ChunkLocation>) {
        let ordinal = self.chunks.len();
        self.chunks.push(Chunk {
            id: stable_id(&[
                b"knoted/chunk",
                self.id.as_bytes(),
                &(ordinal as u64).to_be_bytes(),
            ]),
            ordinal,
            text,
            token_count,
            locations,
        });
    }
}

struct Fragment<'a> {
    text: &'a str,
    tokens: usize,
    location: ChunkLocation,
}

/// Prefer a block boundary, and only split through original text when necessary.
/// The same page / sheet can share a chunk. Its component ranges remain distinct.
pub fn chunk_document(
    document: &ExtractedDocument,
    source: ChunkSource,
    config: &ChunkConfig,
    tokenizer: &BgeTokenizer,
) -> Result<ChunkSet, DomainError> {
    if !matches!(
        source.source_type,
        SourceType::File | SourceType::Note | SourceType::Task
    ) || source.source_version <= 0
        || !(1..=128).contains(&config.revision.len())
        || document.truncated
    {
        return Err(DomainError::InvalidSource);
    }
    let id = stable_id(&[
        b"knoted/chunk-set",
        source.source_type.as_str().as_bytes(),
        source.source_id.as_bytes(),
        &source.source_version.to_be_bytes(),
        source.extraction_id.as_bytes(),
        config.revision.as_bytes(),
    ]);
    let mut set = ChunkSet {
        id,
        source,
        chunker_revision: config.revision.clone(),
        chunks: Vec::new(),
    };
    let mut pending = String::new();
    let mut locations: Vec<ChunkLocation> = Vec::new();
    let mut pending_tokens = 0;
    for block in &document.blocks {
        if block.text.trim().is_empty() {
            continue;
        }
        if block.text.contains('\0') {
            return Err(DomainError::InvalidSource);
        }
        let structure = match block.structure.as_str() {
            Some(value) => match serde_json::from_str::<Value>(value) {
                Ok(parsed) if parsed.is_object() => parsed,
                _ => block.structure.clone(),
            },
            None => block.structure.clone(),
        };
        let location = ChunkLocation {
            block_ordinal: block.ordinal,
            byte_start: 0,
            byte_end: block.text.len(),
            page: block.page,
            bbox: block.bbox,
            structure,
        };
        let independent = block.kind == "activity";
        split_block(&block.text, &location, tokenizer, |fragment| {
            let at_boundary = locations.last().is_some_and(|previous| {
                previous.page != fragment.location.page
                    || previous.structure["sheet"] != fragment.location.structure["sheet"]
                    || previous.structure["slide"] != fragment.location.structure["slide"]
            });
            let heading = matches!(block.kind.as_str(), "heading" | "title");
            let split =
                fragment.location.byte_start > 0 || fragment.location.byte_end < block.text.len();
            if !pending.is_empty() {
                let section_boundary = at_boundary || independent || split || heading;
                if !section_boundary && pending_tokens < TARGET {
                    let end = pending.len();
                    pending.push('\n');
                    pending.push_str(fragment.text);
                    let joined_tokens = tokenizer.count(&pending)?;
                    if joined_tokens <= MAX_TOKENS {
                        pending_tokens = joined_tokens;
                        locations.push(fragment.location);
                        return Ok(());
                    }
                    pending.truncate(end);
                }
                let overlap = if section_boundary {
                    None
                } else {
                    overlap_tail(
                        &pending,
                        locations.last().ok_or(DomainError::InvalidSource)?,
                        tokenizer,
                    )?
                };
                set.push(
                    std::mem::take(&mut pending),
                    pending_tokens,
                    std::mem::take(&mut locations),
                );
                if let Some((mut text, location)) = overlap {
                    text.push('\n');
                    text.push_str(fragment.text);
                    let tokens = tokenizer.count(&text)?;
                    if tokens <= MAX_TOKENS {
                        pending = text;
                        pending_tokens = tokens;
                        locations.push(location);
                        locations.push(fragment.location);
                        return Ok(());
                    }
                }
            }
            if independent || split {
                set.push(
                    fragment.text.into(),
                    fragment.tokens,
                    vec![fragment.location],
                );
                pending_tokens = 0;
            } else {
                pending.push_str(fragment.text);
                pending_tokens = fragment.tokens;
                locations.push(fragment.location);
            }
            Ok(())
        })?;
    }
    if !pending.is_empty() {
        set.push(pending, pending_tokens, locations);
    }
    Ok(set)
}

fn overlap_tail(
    pending: &str,
    location: &ChunkLocation,
    tokenizer: &BgeTokenizer,
) -> Result<Option<(String, ChunkLocation)>, DomainError> {
    // Overlap the final block only, preserving an earlier block boundary.
    let length = location.byte_end - location.byte_start;
    let tail = &pending[pending.len() - length..];
    let encoding = tokenizer.encode(tail)?;
    let offsets = encoding.get_offsets();
    let first = offsets.len().saturating_sub(OVERLAP);
    if first == 0 && !offsets.is_empty() {
        return Ok(Some((tail.into(), location.clone())));
    }
    for &(start, _) in &offsets[first..] {
        if tail.is_char_boundary(start)
            && start < tail.len()
            && (1..=OVERLAP).contains(&tokenizer.count(&tail[start..])?)
        {
            let mut location = location.clone();
            location.byte_start += start;
            return Ok(Some((tail[start..].into(), location)));
        }
    }
    Ok(None)
}

fn split_block(
    text: &str,
    location: &ChunkLocation,
    tokenizer: &BgeTokenizer,
    mut emit: impl FnMut(Fragment<'_>) -> Result<(), DomainError>,
) -> Result<(), DomainError> {
    let encoding = tokenizer.encode(text)?;
    let offsets = encoding.get_offsets();
    if offsets.is_empty() {
        return Ok(());
    }
    if offsets.len() <= MAX_TOKENS {
        return emit(Fragment {
            text,
            tokens: offsets.len(),
            location: location.clone(),
        });
    }

    let mut token_start = 0;
    let mut byte_start = 0;
    while token_start < offsets.len() {
        let should_split = offsets.len() - token_start > MAX_TOKENS;
        let mut token_end = if should_split {
            token_start + TARGET
        } else {
            offsets.len()
        };
        let (byte_end, tokens) = loop {
            let mut byte_end = if token_end == offsets.len() {
                text.len()
            } else {
                offsets[token_end].0
            };
            while !text.is_char_boundary(byte_end) && byte_end > byte_start {
                byte_end -= 1;
            }
            // Several normalized tokens may map to one original code point.
            if byte_end <= byte_start {
                token_end += 1;
                continue;
            }
            let count = tokenizer.count(&text[byte_start..byte_end])?;
            if count <= MAX_TOKENS {
                break (byte_end, count);
            }
            if token_end <= token_start + 1 {
                return Err(DomainError::TokenizerFailed);
            }
            token_end = token_start + (token_end - token_start) / 2;
        };
        let mut range = location.clone();
        range.byte_start = byte_start;
        range.byte_end = byte_end;
        if tokens > 0 {
            emit(Fragment {
                text: &text[byte_start..byte_end],
                tokens,
                location: range,
            })?;
        }
        if token_end == offsets.len() {
            break;
        }
        let mut next = (token_end.saturating_sub(OVERLAP)).max(token_start + 1);
        while next < token_end
            && (offsets[next].0 <= byte_start || !text.is_char_boundary(offsets[next].0))
        {
            next += 1;
        }
        token_start = next;
        byte_start = if next == token_end {
            byte_end
        } else {
            offsets[next].0
        };
    }
    Ok(())
}
