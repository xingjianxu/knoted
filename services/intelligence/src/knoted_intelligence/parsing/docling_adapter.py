"""Docling + RapidOCR document parser.

``parse_document`` turns one staged file into a :class:`ParsedDocument` with a
title, typed blocks (``title``/``heading``/``paragraph``/``table``), structural
locations (page + bounding box, or a line number for plain text) and one
``full_text`` string.

The parser is deliberately narrow:

* only the locally provisioned, revision-locked artifacts are used — Docling
  gets an explicit ``artifacts_path`` and the hub switches are forced offline
  (:mod:`knoted_intelligence.parsing.offline`), so a missing file fails instead
  of downloading;
* the OCR engine is fixed to ``rapidocr`` + ``onnxruntime`` with
  ``lang=["ch"]`` and ``model_size="small"``, matching the artifacts on disk;
* the cheap admission checks (100 MiB, 40M pixels, 500 pages) run **before** any
  model is loaded, so an oversized input never costs a model load;
* a failure is a :class:`~knoted_intelligence.parsing.models.ParseError` whose
  message names the locked combination; it never silently falls back to another
  engine, model size or revision.

Plain UTF-8 (with or without BOM) and UTF-16 (BOM) text is read directly, which
keeps the T09 text-input path working; everything else must be a PDF or an image
format Docling's image backend understands.
"""

from __future__ import annotations

import codecs
import logging
import os
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Any, Literal

from . import models, offline

# Importing `offline` applied the switches; re-apply them here so a direct
# `import knoted_intelligence.parsing.docling_adapter` also cannot download.
offline.enforce_offline()

from docling.datamodel.base_models import ConversionStatus, InputFormat  # noqa: E402
from docling.datamodel.pipeline_options import (  # noqa: E402
    OcrMode,
    PdfPipelineOptions,
    RapidOcrOptions,
    TableFormerMode,
    TableStructureOptions,
)
from docling.document_converter import (  # noqa: E402
    DocumentConverter,
    ImageFormatOption,
    PdfFormatOption,
)
from PIL import Image  # noqa: E402

logger = logging.getLogger(__name__)

BlockKind = Literal["title", "heading", "paragraph", "table"]

#: Docling labels folded into the four block kinds the contract exposes.
_LABEL_KIND: dict[str, BlockKind] = {
    "title": "title",
    "section_header": "heading",
    "subsection_header": "heading",
    "table": "table",
    "text": "paragraph",
    "list_item": "paragraph",
    "code": "paragraph",
    "formula": "paragraph",
    "caption": "paragraph",
    "reference": "paragraph",
    "footnote": "paragraph",
}
#: Items with no useful body text for retrieval.
_SKIPPED_LABELS = {"page_header", "page_footer", "picture", "chart", "document_index"}

_MAGIC_FORMATS: tuple[tuple[bytes, str], ...] = (
    (b"%PDF-", "pdf"),
    (b"\x89PNG\r\n\x1a\n", "image"),
    (b"\xff\xd8\xff", "image"),
    (b"II*\x00", "image"),
    (b"MM\x00*", "image"),
    (b"GIF87a", "image"),
    (b"GIF89a", "image"),
)


@dataclass(frozen=True, slots=True)
class ParsedBlock:
    """One retrievable unit with its location in the source document."""

    ordinal: int
    kind: BlockKind
    text: str
    page: int | None = None
    bbox: tuple[float, float, float, float] | None = None
    #: Structure-only location (``line:12``) for formats without a page grid.
    structure: str | None = None


@dataclass(frozen=True, slots=True)
class ParsedDocument:
    title: str | None
    blocks: tuple[ParsedBlock, ...]
    full_text: str
    source_format: Literal["pdf", "image", "text"]
    page_count: int | None
    options_revision: str


def parse_document(
    path: Path | str,
    *,
    options_revision: str,
    models_dir: Path | str | None = None,
) -> ParsedDocument:
    """Parses one staged document with the locked offline stack.

    ``options_revision`` is echoed into the result so the worker can bind the
    artifact to the option revision it asked for. ``models_dir`` overrides
    ``KNOTED_INTELLIGENCE_MODELS_DIR``; it defaults to the configured tree.
    """

    source = Path(path)
    if not source.is_file():
        raise models.ParseError("input_missing", "input is not a readable regular file")
    size = source.stat().st_size
    if size == 0:
        raise models.ParseError("empty_input", "input is empty")
    models.check_input_size(size)

    root = Path(models_dir) if models_dir is not None else models.default_models_dir()
    source_format = _detect_format(source)
    if source_format == "text":
        # The text path needs no model, so it also works for the T09 fixture.
        return _parse_text(source, options_revision)

    if source_format == "image":
        width, height = _image_size(source)
        models.check_image_pixels(width, height)
        page_count = 1
    else:
        page_count = _pdf_page_count(source)
        models.check_page_count(page_count)

    artifacts = models.require_artifacts(root)
    converter = _converter(str(artifacts))
    try:
        result = converter.convert(str(source))
    except models.ParseError:  # pragma: no cover - nothing inside raises it yet
        raise
    except Exception as exc:  # noqa: BLE001 - docling raises a wide range of types
        # Only the class travels: a docling message may name a cache path.
        raise models.ParseError(
            "corrupt_input", f"docling could not parse the input ({type(exc).__name__})"
        ) from exc

    status = getattr(result, "status", None)
    if status is not None and status != ConversionStatus.SUCCESS:
        raise models.ParseError(
            "corrupt_input",
            f"docling finished with status {getattr(status, 'value', status)}; "
            "a partial document is not published",
        )
    return _document_to_parsed(result.document, source_format, page_count, options_revision)


@lru_cache(maxsize=2)
def _converter(artifacts_path: str) -> DocumentConverter:
    """Builds (and reuses) the converter for one artifacts tree.

    The cache is per process; every execution runs in its own child process, so
    it only saves the model load when a caller parses twice. The key is the
    artifacts path, so two model trees never share a converter.
    """

    if models.TABLEFORMER_MODE != "fast":  # pragma: no cover - guarded by LOCK.md
        raise AssertionError(f"unsupported locked TableFormer mode {models.TABLEFORMER_MODE!r}")
    os.environ["DOCLING_ARTIFACTS_PATH"] = artifacts_path
    ocr_options = RapidOcrOptions(
        backend=models.RAPIDOCR_BACKEND,
        lang=list(models.RAPIDOCR_LANG),
        model_size=models.RAPIDOCR_MODEL_SIZE,
        # An image has no text layer, and a scanned PDF may have a broken one.
        mode=OcrMode.FULL_PAGE,
    )
    pipeline_options = PdfPipelineOptions(
        artifacts_path=Path(artifacts_path),
        do_ocr=True,
        ocr_options=ocr_options,
        do_table_structure=True,
        table_structure_options=TableStructureOptions(
            do_cell_matching=True,
            mode=TableFormerMode.FAST,
        ),
        generate_page_images=False,
        generate_picture_images=False,
    )
    logger.info(
        "loading the locked parser: artifacts=%s docling=%s rapidocr=%s",
        artifacts_path,
        models.DOCLING_VERSION,
        models.RAPIDOCR_VERSION,
    )
    return DocumentConverter(
        format_options={
            InputFormat.IMAGE: ImageFormatOption(pipeline_options=pipeline_options),
            InputFormat.PDF: PdfFormatOption(pipeline_options=pipeline_options),
        }
    )


def _detect_format(path: Path) -> Literal["pdf", "image", "text"]:
    with path.open("rb") as handle:
        head = handle.read(16)
    for magic, source_format in _MAGIC_FORMATS:
        if head.startswith(magic):
            return source_format  # type: ignore[return-value]
    if head[:4] == b"RIFF" and head[8:12] == b"WEBP":
        return "image"
    # BMP's magic is just "BM"; require the zeroed reserved fields so a text
    # file starting with "BM" is not routed to the image backend.
    if head[:2] == b"BM" and head[6:10] == b"\x00\x00\x00\x00":
        return "image"
    if head.startswith((codecs.BOM_UTF16_LE, codecs.BOM_UTF16_BE, codecs.BOM_UTF8)):
        return "text"
    # Extension-less UTF-8 text is a supported input (the T09 staged objects
    # are extension-less), so try the whole file before declaring it unsupported.
    try:
        path.read_bytes().decode("utf-8")
    except UnicodeDecodeError as exc:
        raise models.ParseError(
            "unsupported_format",
            "input is neither a PDF/image nor UTF-8/UTF-16 text",
        ) from exc
    return "text"


def _decode_text(path: Path) -> str:
    raw = path.read_bytes()
    if raw.startswith((codecs.BOM_UTF16_LE, codecs.BOM_UTF16_BE)):
        return raw.decode("utf-16")
    if raw.startswith(codecs.BOM_UTF8):
        return raw.decode("utf-8-sig")
    # A NUL byte in non-UTF-16 data means this is a binary format we did not
    # recognise, not text that happens to be valid UTF-8.
    if b"\x00" in raw:
        raise models.ParseError(
            "unsupported_format", "input contains NUL bytes but is not a supported format"
        )
    return raw.decode("utf-8")


def _parse_text(path: Path, options_revision: str) -> ParsedDocument:
    text = _decode_text(path).replace("\r\n", "\n").replace("\r", "\n")
    blocks: list[ParsedBlock] = []
    for line_number, line in enumerate(text.split("\n"), start=1):
        stripped = line.strip()
        if not stripped:
            continue
        blocks.append(
            ParsedBlock(
                ordinal=len(blocks),
                kind="paragraph",
                text=stripped,
                structure=f"line:{line_number}",
            )
        )
    return ParsedDocument(
        title=None,
        blocks=tuple(blocks),
        full_text=text.strip("\n"),
        source_format="text",
        page_count=None,
        options_revision=options_revision,
    )


def _image_size(path: Path) -> tuple[int, int]:
    try:
        with Image.open(path) as image:
            return image.size
    except Exception as exc:  # noqa: BLE001 - PIL raises OSError and subclasses
        raise models.ParseError(
            "corrupt_input", f"image header could not be read ({type(exc).__name__})"
        ) from exc


def _pdf_page_count(path: Path) -> int:
    import pypdfium2

    try:
        document = pypdfium2.PdfDocument(str(path))
    except Exception as exc:  # noqa: BLE001 - pdfium raises its own error type
        raise models.ParseError(
            "corrupt_input", f"pdf could not be opened ({type(exc).__name__})"
        ) from exc
    try:
        return len(document)
    finally:
        document.close()


def _document_to_parsed(
    document: Any,
    source_format: Literal["pdf", "image"],
    page_count: int,
    options_revision: str,
) -> ParsedDocument:
    blocks: list[ParsedBlock] = []
    for item, _level in document.iterate_items():
        label = str(getattr(item, "label", "")).lower()
        if label in _SKIPPED_LABELS:
            continue
        kind = _LABEL_KIND.get(label)
        if kind is None:
            kind = "paragraph"
        if kind == "table":
            text = _table_markdown(item, document)
        else:
            text = (getattr(item, "text", "") or "").strip()
        if not text:
            continue
        page, bbox = _provenance(item)
        blocks.append(
            ParsedBlock(
                ordinal=len(blocks),
                kind=kind,
                text=text,
                page=page,
                bbox=bbox,
            )
        )

    title = getattr(document, "title", None) or next(
        (block.text for block in blocks if block.kind in {"title", "heading"}),
        None,
    )
    pages = getattr(document, "pages", None)
    if pages:
        page_count = len(pages)
    return ParsedDocument(
        title=title,
        blocks=tuple(blocks),
        full_text="\n".join(block.text for block in blocks),
        source_format=source_format,
        page_count=page_count,
        options_revision=options_revision,
    )


def _table_markdown(item: Any, document: Any) -> str:
    try:
        return item.export_to_markdown(document).strip()
    except Exception:  # noqa: BLE001 - degraded table is better than a crash
        logger.warning("a table could not be exported as markdown", exc_info=True)
        return (getattr(item, "text", "") or "").strip()


def _provenance(item: Any) -> tuple[int | None, tuple[float, float, float, float] | None]:
    provenance = getattr(item, "prov", None) or ()
    if not provenance:
        return None, None
    first = provenance[0]
    page = getattr(first, "page_no", None)
    box = getattr(first, "bbox", None)
    bbox: tuple[float, float, float, float] | None = None
    if box is not None:
        try:
            left, top, right, bottom = box.as_tuple()
        except AttributeError:  # pragma: no cover - older docling bbox objects
            left, top, right, bottom = box.l, box.t, box.r, box.b
        bbox = (
            round(float(left), 2),
            round(float(top), 2),
            round(float(right), 2),
            round(float(bottom), 2),
        )
    return (int(page) if page else None), bbox
