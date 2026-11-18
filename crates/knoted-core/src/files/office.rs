//! Office identity and strict source locations, independent of PDF page limits.
use super::{artifact, pdf::Error, text::ExtractedDocument};
use crate::{
    context::AppContext,
    intelligence::{ExecutionRequest, ExecutionResult},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;
pub const PARSER_REVISION: &str = "office-ooxml-body-v2-hidden-slides-included-cache-only";
pub const OPTIONS_REVISION: &str = "office-ooxml-body-v2-hidden-slides-included-cache-only:66f5d3305d4cda1ad981b12e95d75f4c4f4efcc31e5ca45a8985ead92f65da8a";
// The source extension is a durable selector: renames cannot reuse a success
// (or exhausted failure budget) produced for a different expected format.
pub const RETIRED_DOCX_PARSER_REVISION: &str =
    "office-ooxml-body-v2-hidden-slides-included-cache-only:docx";
pub const DOCX_PARSER_REVISION: &str = "office-ooxml-body-v3:docx";
pub const XLSX_PARSER_REVISION: &str =
    "office-ooxml-body-v2-hidden-slides-included-cache-only:xlsx";
pub const PPTX_PARSER_REVISION: &str =
    "office-ooxml-body-v2-hidden-slides-included-cache-only:pptx";
pub fn format_for_parser(parser: &str) -> Option<&'static str> {
    match parser {
        DOCX_PARSER_REVISION => Some("docx"),
        XLSX_PARSER_REVISION => Some("xlsx"),
        PPTX_PARSER_REVISION => Some("pptx"),
        _ => None,
    }
}
pub fn identity(parser: &str) -> Option<(&'static str, &'static str)> {
    [
        DOCX_PARSER_REVISION,
        XLSX_PARSER_REVISION,
        PPTX_PARSER_REVISION,
    ]
    .into_iter()
    .find(|p| *p == parser)
    .map(|p| (p, OPTIONS_REVISION))
}
pub fn identity_for_path(path: &str) -> Option<(&'static str, &'static str)> {
    let parser = match Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "docx" => DOCX_PARSER_REVISION,
        "xlsx" => XLSX_PARSER_REVISION,
        "pptx" => PPTX_PARSER_REVISION,
        _ => return None,
    };
    identity(parser)
}
pub fn supports(path: &str) -> bool {
    identity_for_path(path).is_some()
}
fn parser_summary(source_format: &str) -> Value {
    let parser_revision = if source_format == "docx" {
        DOCX_PARSER_REVISION
    } else {
        PARSER_REVISION
    };
    json!({"parser_revision":parser_revision,"options_revision":OPTIONS_REVISION,
        "limits":{"members":4096,"part_bytes":33554432,"expanded_bytes":134217728,"compression_ratio":200,
        "xml_nodes":500000,"xml_depth":64,"text_bytes":16777216,"cells":100000,"blocks":100000}})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathLocation {
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CellLocation {
    sheet: String,
    cell: String,
    range: Option<String>,
    value_source: Option<String>,
}
fn coordinate(address: &str) -> Option<(u32, u32)> {
    let split = address.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = address.split_at(split);
    if letters.is_empty()
        || letters.len() > 3
        || digits.starts_with('0')
        || !digits.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let mut column = 0;
    for c in letters.bytes() {
        if !c.is_ascii_uppercase() {
            return None;
        }
        column = column * 26 + u32::from(c - b'A') + 1;
    }
    let row = digits.parse::<u32>().ok()?;
    (column <= 16384 && (1..=1048576).contains(&row)).then_some((column, row))
}
fn numbered(part: &str, name: &str) -> bool {
    part.strip_prefix(name)
        .and_then(|s| s.strip_prefix(':'))
        .is_some_and(|s| {
            !s.starts_with('0')
                && s.bytes().all(|b| b.is_ascii_digit())
                && s.parse::<u32>().is_ok_and(|n| n > 0)
        })
}
fn path_valid(path: &str, format: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    let mut index = 0;
    if format == "docx" {
        if parts.first() != Some(&"body") {
            return false;
        }
        index = 1;
    } else {
        while index < parts.len() && numbered(parts[index], "shape") {
            index += 1;
        }
        if index == 0 {
            return false;
        }
    }
    while index < parts.len() {
        if numbered(parts[index], "paragraph") {
            return index + 1 == parts.len();
        }
        let table = if format == "docx" {
            numbered(parts[index], "table")
        } else {
            parts[index] == "table"
        };
        if !table
            || index + 3 >= parts.len()
            || !numbered(parts[index + 1], "row")
            || !numbered(parts[index + 2], "cell")
        {
            return false;
        }
        index += 3;
        if format == "pptx" {
            return index + 1 == parts.len() && numbered(parts[index], "paragraph");
        }
    }
    false
}
fn location_valid(value: &Value, format: &str) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    if format != "xlsx" {
        return serde_json::from_str::<PathLocation>(text)
            .is_ok_and(|p| path_valid(&p.path, format));
    }
    let Ok(cell) = serde_json::from_str::<CellLocation>(text) else {
        return false;
    };
    let Some(start) = coordinate(&cell.cell) else {
        return false;
    };
    if cell.sheet.is_empty()
        || cell.sheet.chars().count() > 31
        || cell.sheet.contains('\0')
        || cell
            .value_source
            .as_deref()
            .is_some_and(|s| s != "formula_cache")
    {
        return false;
    }
    if let Some(range) = cell.range {
        let Some((first, last)) = range.split_once(':') else {
            return false;
        };
        if first != cell.cell
            || !coordinate(last).is_some_and(|end| end.0 >= start.0 && end.1 >= start.1)
        {
            return false;
        }
    }
    true
}
pub fn decode_artifact(
    bytes: &[u8],
    request: &ExecutionRequest,
) -> Result<ExtractedDocument, Error> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    if v["kind"] != "parse"
        || v["input_sha256"] != request.input_sha256.as_str()
        || v["source_version"].as_i64() != Some(request.source_version)
        || request.options_revision != OPTIONS_REVISION
        || v["options_revision"] != OPTIONS_REVISION
        || v["truncated"] != false
        || !v["child_pid"].as_u64().is_some_and(|p| p > 0)
    {
        return Err(Error::Invalid);
    }
    let document: ExtractedDocument =
        serde_json::from_value(v.clone()).map_err(|_| Error::Invalid)?;
    if v["parser"] != parser_summary(&document.source_format) {
        return Err(Error::Invalid);
    }
    match document.source_format.as_str() {
        "docx" | "xlsx" if document.page_count.is_none() => {}
        "pptx" if document.page_count.is_some_and(|n| n <= 4096) => {}
        _ => return Err(Error::Invalid),
    }
    if document.blocks.len() > 100000 {
        return Err(Error::Invalid);
    }
    let mut characters = 0;
    let mut text_bytes = 0;
    for (ordinal, block) in document.blocks.iter().enumerate() {
        if block.ordinal != ordinal
            || block.kind != "paragraph"
            || block.text.trim().is_empty()
            || block.text.contains('\0')
            || block.bbox.is_some()
            || !location_valid(&block.structure, &document.source_format)
        {
            return Err(Error::Invalid);
        }
        if document.source_format == "pptx" {
            if !block
                .page
                .is_some_and(|page| page > 0 && page <= document.page_count.unwrap_or(0))
            {
                return Err(Error::Invalid);
            }
        } else if block.page.is_some() {
            return Err(Error::Invalid);
        }
        characters += block.text.chars().count();
        text_bytes += block.text.len();
    }
    if text_bytes > 16 * 1024 * 1024
        || v["text_length"].as_u64()
            != Some((characters + document.blocks.len().saturating_sub(1)) as u64)
    {
        return Err(Error::Invalid);
    }
    Ok(document)
}
pub async fn read_artifact(
    ctx: &AppContext,
    request: &ExecutionRequest,
    result: &ExecutionResult,
) -> Result<ExtractedDocument, Error> {
    let bytes = artifact::read_bytes(ctx, request, result).await?;
    decode_artifact(&bytes, request)
}
