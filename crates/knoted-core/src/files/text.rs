//! Strict text decoding from a closed, digest-verified staging object.
use super::{Sha256Hex, StagedObject, capture::MAX_INPUT_BYTES, safe_fs};
use crate::context::AppContext;
use rustix::fs::{self, FileType, Mode, OFlags};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path};

pub const PARSER_REVISION: &str = "rust-text-v1";
pub const OPTIONS_REVISION: &str = "strict-unicode-locations-64k-v1";
const BLOCK_BYTES: usize = 64 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("invalid or unsupported text encoding")]
    Encoding,
    #[error("decoded NUL cannot be stored in JSONB")]
    Nul,
    #[error("input is not a closed verified staging object")]
    InvalidInput,
    #[error("input exceeds 100 MiB")]
    TooLarge,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
/// Same block shape as the Python parse artifact. Text is stored only once.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractedDocument {
    pub blocks: Vec<ExtractedBlock>,
    pub source_format: String,
    pub title: Option<String>,
    pub page_count: Option<u32>,
    pub truncated: bool,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractedBlock {
    pub ordinal: usize,
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    pub page: Option<u32>,
    pub bbox: Option<[f64; 4]>,
    pub structure: serde_json::Value,
}
/// Explicit selection: neither guessing binary encodings nor invoking code.
pub fn supports(path: &str) -> bool {
    let path = Path::new(path);
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "readme" | "license" | "makefile" | "dockerfile" | "justfile"
    ) {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "txt"
            | "md"
            | "markdown"
            | "rs"
            | "py"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cc"
            | "java"
            | "go"
            | "rb"
            | "php"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "sql"
            | "html"
            | "htm"
            | "css"
            | "scss"
            | "xml"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "ini"
            | "cfg"
            | "conf"
            | "csv"
            | "tsv"
            | "log"
            | "nix"
            | "swift"
            | "kt"
            | "kts"
            | "lua"
            | "r"
            | "pl"
            | "ex"
            | "exs"
            | "erl"
            | "hrl"
            | "vue"
            | "svelte"
    )
}
pub async fn extract_text(
    ctx: &AppContext,
    input: StagedObject,
) -> Result<ExtractedDocument, DomainError> {
    if input.size > MAX_INPUT_BYTES {
        return Err(DomainError::TooLarge);
    }
    // Closed capture keys never use a temporary suffix. Each directory and leaf
    // is opened without following links, and only a regular FD is read.
    if input.object_id.as_str().ends_with(".part") {
        return Err(DomainError::InvalidInput);
    }
    let path = ctx
        .files()
        .staging_root
        .as_ref()
        .ok_or(DomainError::InvalidInput)?
        .join("input");
    let directory = safe_fs::open_root(&path)?;
    let fd = fs::openat(
        &directory,
        input.object_id.as_str(),
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    if FileType::from_raw_mode(fs::fstat(&fd).map_err(std::io::Error::from)?.st_mode)
        != FileType::RegularFile
    {
        return Err(DomainError::InvalidInput);
    }
    let mut file = File::from(fd);
    let before = safe_fs::metadata(&file)?;
    if before.size != input.size {
        return Err(DomainError::InvalidInput);
    }
    let mut bytes = Vec::with_capacity(input.size as usize);
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_INPUT_BYTES as usize {
            return Err(DomainError::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..n]);
        tokio::task::yield_now().await;
    }
    if bytes.len() as u64 != input.size
        || Sha256Hex::digest(&bytes) != input.sha256
        || safe_fs::metadata(&file)? != before
    {
        return Err(DomainError::InvalidInput);
    }
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if bytes.len() % 2 != 0 {
            return Err(DomainError::Encoding);
        }
        let little = bytes[0] == 0xff;
        char::decode_utf16(bytes[2..].as_chunks::<2>().0.iter().map(|b| {
            if little {
                u16::from_le_bytes([b[0], b[1]])
            } else {
                u16::from_be_bytes([b[0], b[1]])
            }
        }))
        .collect::<Result<String, _>>()
        .map_err(|_| DomainError::Encoding)?
    } else {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        std::str::from_utf8(bytes)
            .map_err(|_| DomainError::Encoding)?
            .to_owned()
    };
    drop(bytes);
    if text.contains('\0') {
        return Err(DomainError::Nul);
    }
    let mut blocks = Vec::new();
    let (mut offset, mut line, mut column, mut paragraph) = (0, 1usize, 1usize, 1usize);
    let (mut line_nonwhite, mut separator) = (false, false);
    while offset < text.len() {
        let mut end = (offset + BLOCK_BYTES).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let part = &text[offset..end];
        let (start_line, start_column, start_paragraph) = (line, column, paragraph);
        for ch in part.chars() {
            if ch == '\n' {
                if !line_nonwhite {
                    separator = true;
                }
                line_nonwhite = false;
                line += 1;
                column = 1;
            } else {
                if !ch.is_whitespace() {
                    if separator {
                        paragraph += 1;
                        separator = false;
                    }
                    line_nonwhite = true;
                }
                column += 1;
            }
        }
        blocks.push(ExtractedBlock{ordinal:blocks.len(),kind:"paragraph".into(),text:part.into(),page:None,bbox:None,structure:serde_json::json!({"byte_start":offset,"byte_end":end,"line_start":start_line,"column_start":start_column,"line_end":line,"column_end":column,"paragraph_start":start_paragraph,"paragraph_end":paragraph})});
        offset = end;
        tokio::task::yield_now().await;
    }
    Ok(ExtractedDocument {
        blocks,
        source_format: "text".into(),
        title: None,
        page_count: None,
        truncated: false,
    })
}
