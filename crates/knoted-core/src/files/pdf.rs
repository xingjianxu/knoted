//! Locked Python PDF/image identity and strict, closed artifact consumption.
use super::{Sha256Hex, safe_fs, text::ExtractedDocument};
use crate::{
    context::AppContext,
    intelligence::{ExecutionRequest, ExecutionResult},
};
use rustix::fs::{self, FileType, Mode, OFlags};
use serde_json::Value;
use std::{fs::File, io::Read, path::Path};

pub const PARSER_REVISION: &str = "docling-pdf-image-v1";
pub const OPTIONS_REVISION: &str = "parse-v1+docling-2.150.1+layout-8f39ad3c0b4c58e9c2d2c84a38465abf757272d8+tableformer-fc0f2d45e2218ea24bce5045f58a389aed16dc23-fast+rapidocr-3.10.0-v3.10.0+ocr-onnxruntime-small-ch+onnxruntime-1.32.0";
pub const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid or incomplete parse artifact")]
    Invalid,
    #[error("parse artifact storage unavailable")]
    Io(#[from] std::io::Error),
}
pub fn supports(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "pdf" | "png" | "jpg" | "jpeg" | "webp"
    )
}
pub fn decode_artifact(
    bytes: &[u8],
    request: &ExecutionRequest,
) -> Result<ExtractedDocument, Error> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    if v["kind"] != "parse"
        || v["input_sha256"] != request.input_sha256.as_str()
        || v["source_version"].as_i64() != Some(request.source_version)
        || v["options_revision"] != request.options_revision
        || request.options_revision != OPTIONS_REVISION
        || v["parser"]["options_revision"] != OPTIONS_REVISION
        || v["truncated"] != false
    {
        return Err(Error::Invalid);
    }
    for (key, expected) in [
        ("docling", "2.150.1"),
        ("docling_ibm_models", "4.0.4"),
        ("rapidocr", "3.10.0"),
        ("onnxruntime", "1.32.0"),
        ("torch", "2.15.0"),
        ("transformers", "5.23.0"),
        ("ocr", "onnxruntime/small/ch"),
    ] {
        if v["parser"][key] != expected {
            return Err(Error::Invalid);
        }
    }
    if !v["child_pid"].as_u64().is_some_and(|pid| pid > 0) {
        return Err(Error::Invalid);
    }
    let document: ExtractedDocument =
        serde_json::from_value(v.clone()).map_err(|_| Error::Invalid)?;
    let pages = document.page_count.ok_or(Error::Invalid)?;
    if !(1..=500).contains(&pages)
        || !matches!(document.source_format.as_str(), "pdf" | "image")
        || (document.source_format == "image" && pages != 1)
    {
        return Err(Error::Invalid);
    }
    for (ordinal, block) in document.blocks.iter().enumerate() {
        if block.ordinal != ordinal
            || !matches!(
                block.kind.as_str(),
                "title" | "heading" | "paragraph" | "table"
            )
            || block.text.contains('\0')
            || block.text.is_empty()
            || !block.page.is_some_and(|p| (1..=pages).contains(&p))
            || block
                .bbox
                .is_some_and(|bbox| bbox.iter().any(|v| !v.is_finite()))
        {
            return Err(Error::Invalid);
        }
    }
    let text_length = document
        .blocks
        .iter()
        .map(|b| b.text.chars().count())
        .sum::<usize>()
        + document.blocks.len().saturating_sub(1);
    if v["text_length"].as_u64() != Some(text_length as u64) {
        return Err(Error::Invalid);
    }
    Ok(document)
}
pub async fn read_artifact(
    ctx: &AppContext,
    request: &ExecutionRequest,
    result: &ExecutionResult,
) -> Result<ExtractedDocument, Error> {
    if result.size > MAX_ARTIFACT_BYTES || result.media_type != "application/json" {
        return Err(Error::Invalid);
    }
    for (key, expected) in [
        ("kind", "parse".to_owned()),
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
    decode_artifact(&bytes, request)
}
