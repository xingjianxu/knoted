"""Bounded, read-only transitional OOXML text extraction (no Office engine).

This is a pure parser, not an execution handler. The execution supervisor owns
900-second deadlines/cancellation and publication. Unsupported text-bearing
features fail the entire parse; pictures, formatting and presentation notes are
outside this explicitly body-text-only extraction policy.
"""

import hashlib
import io
import json
import posixpath
import re
import struct
import zipfile
import zlib
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Literal
from xml.etree.ElementTree import Element
from xml.etree.ElementTree import ParseError as XMLParseError

from defusedxml import ElementTree as SafeXML
from defusedxml.common import DefusedXmlException

from .docling_adapter import ParsedBlock, ParsedDocument
from .models import MAX_INPUT_BYTES, ParseError

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
S = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"
P = "{http://schemas.openxmlformats.org/presentationml/2006/main}"
A = "{http://schemas.openxmlformats.org/drawingml/2006/main}"
R = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}"
REL = "{http://schemas.openxmlformats.org/package/2006/relationships}"
CT = "{http://schemas.openxmlformats.org/package/2006/content-types}"
OFFICE_REL = R[1:-1] + "/officeDocument"
FORMATS = {
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml": "docx",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml": "xlsx",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml": "pptx",
}
POLICY_REVISION = "office-ooxml-body-v2-hidden-slides-included-cache-only"


@dataclass(frozen=True)
class OfficeLimits:
    """Internal controller choices, not new product-wide resource contracts."""

    members: int = 4096
    part_bytes: int = 32 * 1024 * 1024
    expanded_bytes: int = 128 * 1024 * 1024
    compression_ratio: int = 200
    xml_nodes: int = 500_000
    xml_depth: int = 64
    text_bytes: int = 16 * 1024 * 1024
    cells: int = 100_000
    blocks: int = 100_000


DEFAULT_LIMITS = OfficeLimits()


def options_revision(limits: OfficeLimits = DEFAULT_LIMITS) -> str:
    identity = json.dumps(asdict(limits), sort_keys=True, separators=(",", ":"))
    return POLICY_REVISION + ":" + hashlib.sha256(identity.encode()).hexdigest()


def parser_summary() -> dict:
    """Artifact identity consumed by Rust; independent of supervision revision."""
    return {
        "parser_revision": POLICY_REVISION,
        "options_revision": options_revision(),
        "limits": asdict(DEFAULT_LIMITS),
    }


def _fail(code: str = "corrupt_input") -> None:
    raise ParseError(code, "Office input refused by the read-only extraction policy") from None


def _bound(value: int, limit: int) -> None:
    if value > limit:
        _fail("office_budget_exceeded")


def _part_name(name: str) -> bool:
    return bool(name) and not (
        name.startswith("/")
        or "\\" in name
        or "%" in name
        or ":" in name
        or any(p in {"", ".", ".."} for p in name.split("/"))
        or any(ord(c) < 32 for c in name)
    )


def _check_directory(data: bytes, limit: int) -> None:
    """Count actual ZIP32 directory records before ZipFile allocates ZipInfo objects.

    Only fixed headers are unpacked; names/extra/comments are skipped by length.
    ZIP64 and multi-disk archives are outside this bounded OOXML subset.
    """
    end = data.rfind(b"PK\x05\x06", max(0, len(data) - 65557))
    if end < 0 or end + 22 > len(data):
        _fail()
    disk, central_disk, disk_count, total, size, offset, comment = struct.unpack_from(
        "<4H2LH", data, end + 4
    )
    if end + 22 + comment != len(data):
        _fail()
    if (
        disk
        or central_disk
        or total == 0xFFFF
        or size == 0xFFFFFFFF
        or offset == 0xFFFFFFFF
        or data[max(0, end - 20) : end - 16] == b"PK\x06\x07"
    ):
        _fail("unsupported_format")
    if offset + size != end:
        _fail()
    cursor = offset
    count = 0
    while cursor < end:
        if cursor + 46 > end or data[cursor : cursor + 4] != b"PK\x01\x02":
            _fail()
        name, extra, note = struct.unpack_from("<3H", data, cursor + 28)
        cursor += 46 + name + extra + note
        if cursor > end:
            _fail()
        count += 1
        _bound(count, limit)
    if count != total or count != disk_count:
        _fail()


class _Package:
    def __init__(self, source, limits: OfficeLimits):
        self.limits = limits
        self.xml: dict[str, Element] = {}
        self.rels: dict[str, dict[str, tuple[str, str]]] = {}
        nodes = text_bytes = expanded = 0
        with zipfile.ZipFile(source) as archive:
            members = archive.infolist()
            _bound(len(members), limits.members)
            names = {item.filename for item in members}
            if len(names) != len(members):
                _fail()
            for item in members:
                name = item.filename
                if not _part_name(name):
                    _fail()
                if item.flag_bits & 1 or item.compress_type not in {
                    zipfile.ZIP_STORED,
                    zipfile.ZIP_DEFLATED,
                }:
                    _fail("unsupported_format")
                if any(x in name.lower() for x in ("vbaproject", "/embeddings/", "activex")):
                    _fail("unsupported_format")
                _bound(item.file_size, limits.part_bytes)
                _bound(item.file_size, max(1, item.compress_size) * limits.compression_ratio)
                expanded += item.file_size
                _bound(expanded, limits.expanded_bytes)
            expanded = 0
            for item in members:
                chunks = []
                actual = 0
                with archive.open(item) as member:
                    while chunk := member.read(64 * 1024):
                        actual += len(chunk)
                        expanded += len(chunk)
                        _bound(actual, limits.part_bytes)
                        _bound(expanded, limits.expanded_bytes)
                        chunks.append(chunk)
                if actual != item.file_size:
                    _fail()
                if not item.filename.endswith((".xml", ".rels")):
                    continue  # binary media are counted/CRC-checked, never extracted
                depth = 0
                events = SafeXML.iterparse(
                    io.BytesIO(b"".join(chunks)),
                    events=("start", "end"),
                    forbid_dtd=True,
                    forbid_entities=True,
                    forbid_external=True,
                )
                for event, element in events:
                    if event == "start":
                        depth += 1
                        nodes += 1
                        _bound(depth, limits.xml_depth)
                        _bound(nodes, limits.xml_nodes)
                    else:
                        depth -= 1
                        text_bytes += len((element.text or "").encode())
                        text_bytes += len((element.tail or "").encode())
                        _bound(text_bytes, limits.text_bytes)
                self.xml[item.filename] = events.root
        self.names = names
        for name, root in self.xml.items():
            if name.endswith(".rels"):
                self._relationships(name, root)
        types = self.get("[Content_Types].xml", CT + "Types")
        candidates = []
        for entry in types:
            content_type = entry.get("ContentType", "")
            if any(s in content_type.lower() for s in ("macroenabled", "vba", "oleobject")):
                _fail("unsupported_format")
            if content_type in FORMATS:
                part = entry.get("PartName", "").removeprefix("/")
                candidates.append((part, FORMATS[content_type]))
        if len(candidates) != 1:
            _fail("unsupported_format")
        self.main, self.format = candidates[0]
        roots = [target for kind, target in self.rels.get("", {}).values() if kind == OFFICE_REL]
        if roots != [self.main]:
            _fail()
        self.get(self.main)

    def get(self, name: str, tag: str | None = None) -> Element:
        root = self.xml.get(name)
        if root is None or (tag is not None and root.tag != tag):
            _fail()
        return root

    def _relationships(self, name: str, root: Element) -> None:
        if root.tag != REL + "Relationships":
            _fail()
        if name == "_rels/.rels":
            owner = ""
        else:
            directory, filename = posixpath.split(name)
            if not directory.endswith("/_rels"):
                _fail()
            owner = posixpath.join(directory[:-6], filename[:-5])
            if owner not in self.names:
                _fail()
        relationships = {}
        for rel in root:
            ident, kind, target = rel.get("Id"), rel.get("Type", ""), rel.get("Target", "")
            if rel.tag != REL + "Relationship" or not ident or ident in relationships:
                _fail()
            if rel.get("TargetMode") == "External":
                # Hyperlink display text is safe; never dereference its target.
                if not kind.endswith("/hyperlink"):
                    _fail("unsupported_format")
                relationships[ident] = (kind, "")
                continue
            if rel.get("TargetMode") not in {None, "Internal"}:
                _fail()
            if target.startswith("//") or any(c in target for c in ("\\", "%", ":", "#", "?")):
                _fail()
            resolved = (
                target[1:]
                if target.startswith("/")
                else posixpath.normpath(posixpath.join(posixpath.dirname(owner), target))
            )
            if not _part_name(resolved) or resolved not in self.names:
                _fail()
            relationships[ident] = (kind, resolved)
        self.rels[owner] = relationships

    def related(self, owner: str, ident: str | None, kind: str) -> str:
        relation = self.rels.get(owner, {}).get(ident)
        if relation is None or relation[0] != R[1:-1] + "/" + kind or not relation[1]:
            _fail()
        return relation[1]


class _Output:
    def __init__(self, limits: OfficeLimits):
        self.blocks: list[ParsedBlock] = []
        self.limits = limits
        self.text_bytes = 0

    def add(self, text: str, location: dict, page: int | None = None, kind="paragraph"):
        if not text.strip():
            return
        _bound(len(self.blocks) + 1, self.limits.blocks)
        self.text_bytes += len(text.encode())
        _bound(self.text_bytes, self.limits.text_bytes)
        self.blocks.append(
            ParsedBlock(
                ordinal=len(self.blocks),
                kind=kind,
                text=text,
                page=page,
                structure=json.dumps(location, ensure_ascii=False, sort_keys=True),
            )
        )


def _text(element: Element, namespace: str) -> str:
    return "".join(
        (item.text or "")
        if item.tag == namespace + "t"
        else "\t"
        if item.tag == namespace + "tab"
        else "\n"
        for item in element.iter()
        if item.tag in {namespace + "t", namespace + "tab", namespace + "br", namespace + "cr"}
    )


def _spreadsheet_text(element: Element) -> str:
    """Read CT_Rst body text, excluding auxiliary phonetic runs and properties."""
    pieces = []
    for child in element:
        if child.tag == S + "t":
            pieces.append(child.text or "")
        elif child.tag == S + "r":
            for run_child in child:
                if run_child.tag == S + "t":
                    pieces.append(run_child.text or "")
                elif run_child.tag != S + "rPr":
                    _fail("unsupported_format")
        elif child.tag not in {S + "rPh", S + "phoneticPr"}:
            _fail("unsupported_format")
    return "".join(pieces)


def _word_text(element: Element) -> str:
    """Collect supported paragraph/run content; unknown body expressions refuse.

    Formatting and annotation anchors carry no body characters. Drawings/pictures
    are excluded by the body-text policy, after the document-wide text-box checks.
    Never flatten arbitrary descendants: ruby readings and font symbols need a
    separate interpretation and must not become partial successful body text.
    """
    characters = {
        W + "tab": "\t",
        W + "ptab": "\t",
        W + "br": "\n",
        W + "cr": "\n",
        W + "noBreakHyphen": "\u2011",
        W + "softHyphen": "\u00ad",
    }
    containers = {W + name for name in ("r", "hyperlink", "dir", "bdo")}
    non_body = {
        W + name
        for name in (
            "pPr",
            "rPr",
            "bookmarkStart",
            "bookmarkEnd",
            "proofErr",
            "permStart",
            "permEnd",
            "commentRangeStart",
            "commentRangeEnd",
            "commentReference",
            "lastRenderedPageBreak",
            "drawing",
            "pict",
        )
    }
    pieces = []
    for child in element:
        if child.tag == W + "t":
            pieces.append(child.text or "")
        elif child.tag in characters:
            pieces.append(characters[child.tag])
        elif child.tag in containers:
            pieces.append(_word_text(child))
        elif child.tag not in non_body:
            _fail("unsupported_format")
    return "".join(pieces)


def _docx(package: _Package, out: _Output):
    root = package.get(package.main, W + "document")
    unsupported = {
        W + name
        for name in (
            "ins",
            "del",
            "moveFrom",
            "moveTo",
            "altChunk",
            "txbxContent",
            "sdt",
            "footnoteReference",
            "endnoteReference",
            "fldSimple",
            "instrText",
            "object",
        )
    }
    if any(
        node.tag in unsupported or node.tag == A + "t" or node.tag.endswith("}AlternateContent")
        for node in root.iter()
    ):
        _fail("unsupported_format")
    body = root.find(W + "body")
    if body is None:
        _fail()

    def walk(container, prefix):
        for index, node in enumerate(
            (n for n in container if n.tag not in {W + "tcPr", W + "sectPr"}), 1
        ):
            if node.tag == W + "p":
                out.add(_word_text(node), {"path": f"{prefix}/paragraph:{index}"})
            elif node.tag == W + "tbl":
                table = f"{prefix}/table:{index}"
                for row_index, row in enumerate(node.findall(W + "tr"), 1):
                    before = row.find(W + "trPr/" + W + "gridBefore")
                    column = 1 + (int(before.get(W + "val")) if before is not None else 0)
                    for cell in row.findall(W + "tc"):
                        merge = cell.find(W + "tcPr/" + W + "vMerge")
                        span = cell.find(W + "tcPr/" + W + "gridSpan")
                        width = int(span.get(W + "val")) if span is not None else 1
                        if width < 1 or column < 1:
                            _fail()
                        if merge is None or merge.get(W + "val") == "restart":
                            walk(cell, f"{table}/row:{row_index}/cell:{column}")
                        elif any(_word_text(p).strip() for p in cell.iter(W + "p")):
                            _fail()  # invalid continuation must not silently lose text
                        column += width
            elif node.tag not in {W + "sectPr", W + "tcPr"}:
                _fail("unsupported_format")

    walk(body, "body")


_CELL = re.compile(r"([A-Z]{1,3})([1-9][0-9]{0,6})\Z")


def _coordinate(address: str) -> tuple[int, int]:
    match = _CELL.fullmatch(address)
    if match is None:
        _fail()
    column = 0
    for letter in match[1]:
        column = column * 26 + ord(letter) - 64
    row = int(match[2])
    if column > 16384 or row > 1048576:
        _fail()
    return column, row


def _xlsx(package: _Package, out: _Output):
    root = package.get(package.main, S + "workbook")
    if root.find(S + "externalReferences") is not None:
        _fail("unsupported_format")
    strings = []
    for kind, target in package.rels.get(package.main, {}).values():
        if kind.endswith("/sharedStrings"):
            strings = [_spreadsheet_text(si) for si in package.get(target, S + "sst")]
    sheets = root.find(S + "sheets")
    if sheets is None:
        _fail()
    cell_count = 0
    sheet_names = set()
    for sheet in sheets:
        title = sheet.get("name")
        if not title or len(title) > 31 or title in sheet_names:
            _fail()
        sheet_names.add(title)
        part = package.related(package.main, sheet.get(R + "id"), "worksheet")
        xml = package.get(part, S + "worksheet")
        if xml.find(S + "sheetData") is None:
            _fail()
        visible = sheet.get("state", "visible") == "visible"
        if sheet.get("state", "visible") not in {"visible", "hidden", "veryHidden"}:
            _fail()
        # Ignore declared dimension: iterate only actual XML cells, never a rectangle.
        hidden_deltas = [0] * 16386
        for col in xml.findall(S + "cols/" + S + "col"):
            start, end = int(col.get("min", "0")), int(col.get("max", "0"))
            if not 1 <= start <= end <= 16384:
                _fail()
            if col.get("hidden") in {"1", "true"}:
                hidden_deltas[start] += 1
                hidden_deltas[end + 1] -= 1
        # Bounded column domain: O(columns + ranges + cells), not ranges * cells.
        hidden_columns = set()
        active = 0
        for column in range(1, 16385):
            active += hidden_deltas[column]
            if active:
                hidden_columns.add(column)
        merges = {}
        for merge in xml.findall(S + "mergeCells/" + S + "mergeCell"):
            reference = merge.get("ref", "")
            start, end = reference.split(":")
            left, top = _coordinate(start)
            right, bottom = _coordinate(end)
            if left > right or top > bottom or start in merges:
                _fail()
            merges[start] = reference
        seen = set()
        rows_seen = set()
        for row in xml.findall(S + "sheetData/" + S + "row"):
            row_index = int(row.get("r", "0"))
            if not 1 <= row_index <= 1048576 or row_index in rows_seen:
                _fail()
            rows_seen.add(row_index)
            row_visible = visible and row.get("hidden") not in {"1", "true"}
            for cell in row.findall(S + "c"):
                cell_count += 1
                _bound(cell_count, out.limits.cells)
                address = cell.get("r", "")
                column, actual_row = _coordinate(address)
                if actual_row != row_index or address in seen:
                    _fail()
                seen.add(address)
                formula = cell.find(S + "f")
                if formula is not None and any(c in (formula.text or "") for c in ("[", "]", "|")):
                    _fail("unsupported_format")
                if not row_visible or column in hidden_columns:
                    continue
                value = cell.find(S + "v")
                cell_type = cell.get("t", "n")
                text = value.text if value is not None and value.text is not None else ""
                if cell_type == "inlineStr" and (value is not None or formula is not None):
                    _fail()
                if formula is not None and not text:
                    continue  # missing cache is absent, never computed or changed to zero
                if cell_type == "s":
                    index = int(text)
                    if not 0 <= index < len(strings):
                        _fail()
                    text = strings[index]
                elif cell_type == "inlineStr":
                    inline = cell.find(S + "is")
                    if inline is None:
                        if len(cell) or (cell.text or "").strip():
                            _fail()
                        text = ""
                    else:
                        text = _spreadsheet_text(inline)
                elif cell_type not in {"n", "b", "e", "str", "d"}:
                    _fail("unsupported_format")
                location = {"sheet": title, "cell": address}
                if address in merges:
                    location["range"] = merges[address]
                if formula is not None:
                    location["value_source"] = "formula_cache"
                out.add(text, location)


def _pptx(package: _Package, out: _Output) -> int:
    root = package.get(package.main, P + "presentation")
    slides = root.find(P + "sldIdLst")
    if slides is None:
        return 0

    def paragraphs(container, prefix, page):
        for index, para in enumerate(container.findall(A + "p"), 1):
            out.add(_text(para, A), {"path": f"{prefix}/paragraph:{index}"}, page)

    def shapes(tree, prefix, page):
        index = 0
        for shape in tree:
            if shape.tag in {P + "nvGrpSpPr", P + "grpSpPr", P + "extLst"}:
                continue
            index += 1
            path = f"{prefix}shape:{index}"
            if shape.tag == P + "grpSp":
                shapes(shape, path + "/", page)
            elif shape.tag == P + "sp":
                body = shape.find(P + "txBody")
                if body is not None:
                    paragraphs(body, path, page)
            elif shape.tag == P + "graphicFrame":
                table = shape.find(A + "graphic/" + A + "graphicData/" + A + "tbl")
                if table is None:
                    _fail("unsupported_format")
                for r, row in enumerate(table.findall(A + "tr"), 1):
                    for c, cell in enumerate(row.findall(A + "tc"), 1):
                        if cell.get("hMerge") in {"1", "true"} or cell.get("vMerge") in {
                            "1",
                            "true",
                        }:
                            if _text(cell, A).strip():
                                _fail()
                            continue
                        body = cell.find(A + "txBody")
                        if body is not None:
                            paragraphs(body, f"{path}/table/row:{r}/cell:{c}", page)
            elif shape.tag not in {P + "pic", P + "cxnSp"}:
                _fail("unsupported_format")

    seen = set()
    for page, slide in enumerate(slides, 1):
        part = package.related(package.main, slide.get(R + "id"), "slide")
        if part in seen:
            _fail()
        seen.add(part)
        xml = package.get(part, P + "sld")
        if any(n.tag.endswith("}AlternateContent") for n in xml.iter()):
            _fail("unsupported_format")
        tree = xml.find(P + "cSld/" + P + "spTree")
        if tree is None:
            _fail()
        shapes(tree, "", page)
    return len(slides)


def parse_office(
    path: Path | str, *, limits: OfficeLimits = DEFAULT_LIMITS, staged: bool = False
) -> ParsedDocument:
    """Parse a staged OOXML file; never writes artifacts or returns truncated text.

    Staged object names are opaque; the Worker checks the original source format.
    The options identity binds policy and admission budgets. IO errors remain
    distinct from corrupt data; MemoryError and programming failures propagate.
    No exception text from the input, XML parser or OS enters ParseError detail.
    """
    path = Path(path)
    if not staged and path.suffix.lower() not in {".docx", ".xlsx", ".pptx", ""}:
        _fail("unsupported_format")
    try:
        with path.open("rb") as source:
            # One descriptor and bounded snapshot; avoid stat/open races and growth.
            data = source.read(MAX_INPUT_BYTES + 1)
        if len(data) > MAX_INPUT_BYTES:
            _fail("input_too_large")
        if data.startswith(bytes.fromhex("d0cf11e0a1b11ae1")):
            _fail("unsupported_format")
        _check_directory(data, limits.members)
        package = _Package(io.BytesIO(data), limits)
        if not staged and path.suffix and path.suffix.lower() != "." + package.format:
            _fail("unsupported_format")
        out = _Output(limits)
        page_count = None
        source_format: Literal["docx", "xlsx", "pptx"] = package.format
        if source_format == "docx":
            _docx(package, out)
        elif source_format == "xlsx":
            _xlsx(package, out)
        else:
            page_count = _pptx(package, out)
        return ParsedDocument(
            title=None,
            blocks=tuple(out.blocks),
            full_text="\n".join(b.text for b in out.blocks),
            source_format=source_format,
            page_count=page_count,
            options_revision=options_revision(limits),
        )
    except (
        zipfile.BadZipFile,
        XMLParseError,
        DefusedXmlException,
        zlib.error,
        EOFError,
        ValueError,
        KeyError,
        TypeError,
    ):
        _fail()
    except OSError:
        _fail("office_io_error")
