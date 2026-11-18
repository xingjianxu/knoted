"""T47 PDF/image admission; supervision and publication remain caller-owned."""

from pathlib import Path

from . import docling_adapter, models
from .docling_adapter import ParsedDocument


def parse_pdf_image(
    path: Path | str,
    *,
    options_revision: str,
    models_dir: Path | str | None = None,
) -> ParsedDocument:
    """Accept only staged PDF/JPEG/PNG/WebP bytes, never text or Office fallbacks.

    Reuse the locked adapter's pre-model page/pixel admission and shared blocks.
    This function does not implement a deadline or process exit guarantee.
    """
    source = Path(path)
    if not source.is_file():
        raise models.ParseError("input_missing", "input is not a readable regular file")
    size = source.stat().st_size
    if size == 0:
        raise models.ParseError("empty_input", "input is empty")
    models.check_input_size(size)
    with source.open("rb") as stream:
        head = stream.read(16)
    supported = head.startswith((b"%PDF-", b"\x89PNG\r\n\x1a\n", b"\xff\xd8\xff"))
    supported |= head[:4] == b"RIFF" and head[8:12] == b"WEBP"
    if not supported:
        raise models.ParseError("unsupported_format", "expected PDF, JPEG, PNG or WebP input")
    return docling_adapter.parse_document(
        source, options_revision=options_revision, models_dir=models_dir
    )
