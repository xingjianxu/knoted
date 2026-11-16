//! Strict version-one note documents and lossless plain-text extraction.
//!
//! Validation never connects to the database. Only this module can construct a
//! `ValidatedDocument`; extraction consequently cannot bypass the limits or the
//! allowlists. Errors contain schema paths, never source values or serde errors.

use std::collections::HashSet;

use serde::{Deserialize, Deserializer};
use url::Url;
use uuid::Uuid;

use crate::context::AppContext;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_BLOCKS: usize = 5000;
const MAX_DEPTH: usize = 8;

/// The version is stored alongside the document, as in the HTTP contract.
#[derive(Debug)]
pub struct ValidateDocument {
    pub format_version: u32,
    pub document_json: String,
}

/// A strictly parsed, semantically validated document. No public constructor or
/// deserializer exists, so consumers must call [`validate_document`].
pub struct ValidatedDocument {
    document: NoteDocument,
    source: String,
}

impl ValidatedDocument {
    /// Original JSON, including whitespace, for persistence without data loss.
    pub fn document_json(&self) -> &str {
        &self.source
    }

    pub fn format_version(&self) -> u32 {
        1
    }
}

impl std::fmt::Debug for ValidatedDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedDocument")
            .field("format_version", &1)
            .finish_non_exhaustive()
    }
}

pub struct ExtractPlainText {
    pub document: ValidatedDocument,
}

/// Every block has an entry, including images and empty blocks. A path begins
/// with a top-level block index and continues with indexes into `children`.
#[derive(Debug, PartialEq, Eq)]
pub struct PlainTextBlock {
    pub block_id: String,
    pub path: Vec<usize>,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PlainTextDocument {
    pub text: String,
    pub blocks: Vec<PlainTextBlock>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("unsupported note format version")]
    UnsupportedVersion,
    #[error("note document limit exceeded at {path}")]
    LimitExceeded { path: String },
    #[error("invalid note document at {path}")]
    InvalidDocument { path: String },
}

impl DomainError {
    /// Suggested transport status; this domain module has no HTTP dependency.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::LimitExceeded { .. } => 413,
            _ => 422,
        }
    }
}

fn invalid(path: &str) -> DomainError {
    DomainError::InvalidDocument { path: path.into() }
}

fn limit(path: &str) -> DomainError {
    DomainError::LimitExceeded { path: path.into() }
}

// Unlike Option<T>, this type rejects explicit null. Deriving Deserialize for
// containing structs also rejects duplicate optional keys, including null keys.
#[derive(Debug, Default, serde::Serialize)]
#[serde(untagged)]
enum Optional<T> {
    #[default]
    Missing,
    Present(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Optional<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::Present)
    }
}

impl<T> Optional<T> {
    fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Missing => None,
            Self::Present(value) => Some(value),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteDocument {
    blocks: Vec<NoteBlock>,
}

#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
struct AssetId(Uuid);

impl<'de> Deserialize<'de> for AssetId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        // uuid's parser also accepts compact, braced and URN forms; the public
        // schema accepts only the standard hyphenated UUID representation.
        if raw.len() != 36
            || raw.bytes().enumerate().any(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte != b'-'
                } else {
                    !byte.is_ascii_hexdigit()
                }
            })
        {
            return Err(serde::de::Error::custom("invalid asset UUID"));
        }
        Uuid::parse_str(&raw)
            .map(Self)
            .map_err(|_| serde::de::Error::custom("invalid asset UUID"))
    }
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct NoteBlock {
    id: String,
    #[serde(rename = "type")]
    kind: BlockType,
    #[serde(default)]
    level: Optional<u8>,
    #[serde(default)]
    content: Vec<InlineNode>,
    #[serde(default)]
    asset_id: Optional<AssetId>,
    #[serde(default)]
    language: Optional<String>,
    #[serde(default)]
    checked: Optional<bool>,
    #[serde(default)]
    children: Vec<NoteBlock>,
}

#[derive(Debug, Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum BlockType {
    Heading,
    Paragraph,
    BulletListItem,
    NumberedListItem,
    CheckListItem,
    Image,
    CodeBlock,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct InlineNode {
    #[serde(rename = "type")]
    kind: InlineType,
    text: String,
    #[serde(default)]
    href: Optional<String>,
    #[serde(default)]
    marks: Vec<InlineMark>,
}

#[derive(Debug, Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum InlineType {
    Text,
    Link,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
enum InlineMark {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
}

/// Validate the raw UTF-8 byte length before parsing and return only a trusted
/// typed document. Malformed JSON and unknown schema fields use the root path:
/// serde's diagnostic can include source data, so it is deliberately discarded.
pub fn validate_document(
    _ctx: &AppContext,
    input: ValidateDocument,
) -> Result<ValidatedDocument, DomainError> {
    if input.format_version != 1 {
        return Err(DomainError::UnsupportedVersion);
    }
    if input.document_json.len() > MAX_BYTES {
        return Err(limit("$"));
    }
    check_json_nesting(&input.document_json)?;
    let document: NoteDocument =
        serde_json::from_str(&input.document_json).map_err(|_| invalid("$"))?;
    let mut ids = HashSet::new();
    let mut count = 0;
    validate_blocks(&document.blocks, 1, "$.blocks", &mut ids, &mut count)?;
    Ok(ValidatedDocument {
        document,
        source: input.document_json,
    })
}

// A valid depth-eight schema needs at most 20 JSON containers (including inline
// marks). This generous bound is checked iteratively before serde allocates a
// recursive tree, bounding parsing, validation and drop stack usage even for
// malicious input. Braces inside strings and escaped quotes are not containers.
fn check_json_nesting(raw: &str) -> Result<(), DomainError> {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in raw.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > 32 {
                        return Err(limit("$"));
                    }
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    Ok(())
}

fn validate_blocks<'a>(
    blocks: &'a [NoteBlock],
    depth: usize,
    path: &str,
    ids: &mut HashSet<&'a str>,
    count: &mut usize,
) -> Result<(), DomainError> {
    if !blocks.is_empty() && depth > MAX_DEPTH {
        return Err(limit(path));
    }
    for (index, block) in blocks.iter().enumerate() {
        let path = format!("{path}[{index}]");
        *count += 1;
        if *count > MAX_BLOCKS {
            return Err(limit(&path));
        }
        if !(1..=64).contains(&block.id.chars().count()) || !ids.insert(&block.id) {
            return Err(invalid(&format!("{path}.id")));
        }
        match block.level.as_ref() {
            Some(level) if !(1..=3).contains(level) => {
                return Err(invalid(&format!("{path}.level")));
            }
            None if block.kind == BlockType::Heading => {
                return Err(invalid(&format!("{path}.level")));
            }
            _ => {}
        }
        if block.kind == BlockType::Image && block.asset_id.as_ref().is_none() {
            return Err(invalid(&format!("{path}.asset_id")));
        }
        for (index, inline) in block.content.iter().enumerate() {
            let href_path = format!("{path}.content[{index}].href");
            match inline.href.as_ref() {
                Some(href) if !safe_href(href) => return Err(invalid(&href_path)),
                None if inline.kind == InlineType::Link => return Err(invalid(&href_path)),
                _ => {}
            }
        }
        validate_blocks(
            &block.children,
            depth + 1,
            &format!("{path}.children"),
            ids,
            count,
        )?;
    }
    Ok(())
}

fn safe_href(href: &str) -> bool {
    // URL parsers normalize controls, surrounding whitespace and scheme case.
    // Check the original spelling before parsing. Scheme case is insignificant
    // in URI syntax; the parser's normalized scheme is safe after this check.
    if !uri_spelling(href) {
        return false;
    }
    let Some((_, remainder)) = href.split_once(':') else {
        return false;
    };
    let Ok(url) = Url::parse(href) else {
        return false;
    };
    match url.scheme() {
        "http" | "https" => http_uri_components(remainder) && url.host_str().is_some(),
        "mailto" => !remainder.contains(['[', ']']) && !url.path().is_empty(),
        _ => false,
    }
}

fn http_uri_components(remainder: &str) -> bool {
    let Some(rest) = remainder.strip_prefix("//") else {
        return false;
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, suffix) = rest.split_at(end);
    if authority.is_empty() || suffix.contains(['[', ']']) {
        return false;
    }
    let (userinfo, host) = authority.rsplit_once('@').unwrap_or(("", authority));
    // Raw brackets belong only to an IPv6 host literal, never userinfo,
    // path, query or fragment. Url::parse already validated the IPv6 host.
    !userinfo.contains(['[', ']', '@']) && (host.starts_with('[') || !host.contains(['[', ']']))
}

// The schema uses URI, not IRI: hosts must use punycode and Unicode paths
// must be percent encoded. WHATWG URL parsing alone would silently encode or
// normalize invalid spelling, so preserve the stricter original contract.
fn uri_spelling(href: &str) -> bool {
    let bytes = href.as_bytes();
    let mut index = 0;
    let mut fragment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            if !bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
                || !bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit)
            {
                return false;
            }
            index += 3;
            continue;
        }
        if byte == b'#' {
            if fragment {
                return false;
            }
            fragment = true;
        }
        if !byte.is_ascii_alphanumeric() && !b"-._~:/?#[]@!$&'()*+,;=".contains(&byte) {
            return false;
        }
        index += 1;
    }
    true
}

/// Extract in preorder, concatenating inline text literally and separating
/// every block (including empty blocks) with exactly one newline.
pub fn extract_plain_text(
    _ctx: &AppContext,
    input: ExtractPlainText,
) -> Result<PlainTextDocument, DomainError> {
    let mut blocks = Vec::new();
    extract_blocks(
        &input.document.document.blocks,
        &mut Vec::new(),
        &mut blocks,
    );
    let text = blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(PlainTextDocument { text, blocks })
}

fn extract_blocks(blocks: &[NoteBlock], path: &mut Vec<usize>, output: &mut Vec<PlainTextBlock>) {
    for (index, block) in blocks.iter().enumerate() {
        path.push(index);
        output.push(PlainTextBlock {
            block_id: block.id.clone(),
            path: path.clone(),
            text: block
                .content
                .iter()
                .map(|inline| inline.text.as_str())
                .collect(),
        });
        extract_blocks(&block.children, path, output);
        path.pop();
    }
}
