"""Document parsing backends.

The public surface is :func:`parse_document`, :class:`ParsedDocument`,
:class:`ParsedBlock` and :class:`ParseError`. ``models`` holds the locked
versions, revisions, artifact paths and admission limits; ``LOCK.md`` records
the host evidence for every value and how a failure is reported.
"""

from __future__ import annotations

from .docling_adapter import ParsedBlock, ParsedDocument, parse_document
from .models import OPTIONS_REVISION, ParseError

__all__ = [
    "OPTIONS_REVISION",
    "ParseError",
    "ParsedBlock",
    "ParsedDocument",
    "parse_document",
]
