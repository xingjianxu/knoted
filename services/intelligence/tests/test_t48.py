"""Real OOXML packages exercise extraction and admission, without Office/model execution."""

import importlib.util
import json
import socket
import subprocess
import zipfile
from contextlib import contextmanager
from dataclasses import replace
from pathlib import Path

import pytest
from docx import Document
from openpyxl import Workbook
from pptx import Presentation
from pptx.util import Inches

from knoted_intelligence.parsing.models import ParseError


def parser():
    # An assertion (rather than collection failure) records the absent feature RED.
    assert importlib.util.find_spec("knoted_intelligence.parsing.office") is not None
    from knoted_intelligence.parsing import office

    return office


def rewrite(path, changes=None, extra=()):
    with zipfile.ZipFile(path) as package:
        parts = {name: package.read(name) for name in package.namelist()}
    for name, edit in (changes or {}).items():
        parts[name] = edit(parts[name]) if callable(edit) else edit
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as package:
        for name, data in parts.items():
            package.writestr(name, data)
        for name, data in extra:
            package.writestr(name, data)


def docx(tmp_path):
    path = tmp_path / "sample.docx"
    doc = Document()
    doc.add_paragraph("before")
    doc.save(path)
    return path


def test_t48_1_sheet_visibility_sparse_coordinates_and_original_name(tmp_path):
    path = tmp_path / "book.xlsx"
    book = Workbook()
    sheet = book.active
    sheet.title = "中文'原名"
    sheet["A1"] = "visible"
    sheet["B1"] = "hidden column"
    sheet["C1"] = "hidden grouped column"
    sheet.column_dimensions.group("B", "D", hidden=True)
    sheet["A2"] = "hidden row"
    sheet.row_dimensions[2].hidden = True
    sheet["E3"] = "merged"
    sheet.merge_cells("E3:F4")
    sheet["XFD1048576"] = "sparse far corner"
    book.create_sheet("hidden").sheet_state = "hidden"
    book["hidden"]["A1"] = "secret"
    book.create_sheet("very hidden").sheet_state = "veryHidden"
    book["very hidden"]["A1"] = "secret2"
    book.save(path)
    result = parser().parse_office(path)
    assert [b.text for b in result.blocks] == ["visible", "merged", "sparse far corner"]
    locations = [json.loads(b.structure) for b in result.blocks]
    assert [p["cell"] for p in locations] == ["A1", "E3", "XFD1048576"]
    assert all(p["sheet"] == "中文'原名" for p in locations)
    assert locations[1]["range"] == "E3:F4"
    assert result.page_count is None and all(b.page is None for b in result.blocks)


def test_t48_2_slide_ordinal_group_table_and_hidden_slide(tmp_path):
    path = tmp_path / "slides.pptx"
    deck = Presentation()
    deck.slides.add_slide(deck.slide_layouts[6])  # blank slide must still count
    hidden = deck.slides.add_slide(deck.slide_layouts[6])
    hidden._element.set("show", "0")
    hidden.shapes.add_textbox(0, 0, Inches(2), Inches(1)).text = "hidden slide text"
    slide = deck.slides.add_slide(deck.slide_layouts[6])
    group = slide.shapes.add_group_shape()
    group.shapes.add_textbox(0, 0, Inches(2), Inches(1)).text = "group text"
    table = slide.shapes.add_table(1, 2, 0, 0, Inches(3), Inches(1)).table
    table.cell(0, 0).text = "left"
    table.cell(0, 1).text = "right"
    deck.save(path)
    result = parser().parse_office(path)
    assert result.page_count == 3
    assert [(b.text, b.page) for b in result.blocks] == [
        ("hidden slide text", 2),
        ("group text", 3),
        ("left", 3),
        ("right", 3),
    ]
    assert all(b.bbox is None for b in result.blocks)
    assert json.loads(result.blocks[1].structure)["path"] == "shape:1/shape:1/paragraph:1"
    assert (
        json.loads(result.blocks[2].structure)["path"] == "shape:2/table/row:1/cell:1/paragraph:1"
    )


def test_t48_3_docx_order_nested_merged_tables_without_invented_pages(tmp_path):
    path = tmp_path / "document.docx"
    doc = Document()
    doc.add_paragraph("before")
    table = doc.add_table(rows=2, cols=2)
    table.cell(0, 0).merge(table.cell(0, 1)).text = "horizontal"
    table.cell(1, 0).text = "cell"
    nested = table.cell(1, 0).add_table(rows=1, cols=1)
    nested.cell(0, 0).text = "nested"
    doc.add_page_break()
    doc.add_paragraph("after")
    doc.save(path)
    result = parser().parse_office(path)
    assert [b.text for b in result.blocks] == ["before", "horizontal", "cell", "nested", "after"]
    assert json.loads(result.blocks[1].structure)["path"] == "body/table:2/row:1/cell:1/paragraph:1"
    assert "table:2/row:1/cell:1/paragraph:1" in json.loads(result.blocks[3].structure)["path"]
    assert result.page_count is None
    assert all(b.page is None and b.bbox is None for b in result.blocks)
    assert result.source_format == "docx"


def test_formula_cache_only_never_evaluates_or_launches(tmp_path, monkeypatch):
    path = tmp_path / "formulas.xlsx"
    book = Workbook()
    sheet = book.active
    sheet["A1"] = "=1+1"
    sheet["A2"] = "=99+1"
    sheet["A3"] = "=1/0"
    book.save(path)
    rewrite(
        path,
        {
            "xl/worksheets/sheet1.xml": lambda b: b.replace(
                b"<f>1+1</f><v></v>", b"<f>1+1</f><v>2</v>"
            ).replace(
                b'<c r="A3"><f>1/0</f><v></v></c>', b'<c r="A3" t="e"><f>1/0</f><v>#DIV/0!</v></c>'
            )
        },
    )
    mod = parser()

    def forbidden(*args, **kwargs):
        pytest.fail("Office parsing must not open sockets or launch processes")

    monkeypatch.setattr(socket, "socket", forbidden)
    monkeypatch.setattr(subprocess, "Popen", forbidden)
    result = mod.parse_office(path)
    assert [b.text for b in result.blocks] == ["2", "#DIV/0!"]
    assert json.loads(result.blocks[0].structure)["value_source"] == "formula_cache"
    assert "100" not in result.full_text


@pytest.mark.parametrize(
    "change",
    [
        lambda b: b.replace(b"<w:p>", b"<w:ins><w:p>", 1).replace(b"</w:p>", b"</w:p></w:ins>", 1),
        lambda b: b.replace(b"<w:body>", b"<w:body><w:altChunk/>", 1),
        lambda b: b.replace(b"<w:body>", b"<w:body><w:txbxContent/>", 1),
    ],
)
def test_unsupported_docx_content_is_not_partial_success(tmp_path, change):
    path = docx(tmp_path)
    rewrite(path, {"word/document.xml": change})
    with pytest.raises(ParseError) as error:
        parser().parse_office(path)
    assert error.value.code == "unsupported_format"


@pytest.mark.parametrize(
    "extra,code",
    [
        (("../escape.xml", b"<x/>"), "corrupt_input"),
        (("/absolute.xml", b"<x/>"), "corrupt_input"),
        (("word\\bad.xml", b"<x/>"), "corrupt_input"),
        (("word/document.xml", b"<x/>"), "corrupt_input"),
        (("word/vbaProject.bin", b"macro"), "unsupported_format"),
        (("word/embeddings/oleObject1.bin", b"object"), "unsupported_format"),
        (("evil.xml", b'<!DOCTYPE x [<!ENTITY a "secret">]><x>&a;</x>'), "corrupt_input"),
        (("broken.xml", b"<x>"), "corrupt_input"),
    ],
)
def test_package_security_refusals(tmp_path, extra, code):
    path = docx(tmp_path)
    if extra[0] == "word/document.xml":
        with pytest.warns(UserWarning, match="Duplicate name"):
            rewrite(path, extra=[extra])
    else:
        rewrite(path, extra=[extra])
    with pytest.raises(ParseError) as error:
        parser().parse_office(path)
    assert error.value.code == code
    assert str(path) not in str(error.value)
    assert "secret" not in str(error.value)


@pytest.mark.parametrize(
    "field,value",
    [
        ("members", 1),
        ("part_bytes", 100),
        ("expanded_bytes", 100),
        ("compression_ratio", 1),
        ("xml_nodes", 4),
        ("xml_depth", 2),
        ("text_bytes", 4),
        ("blocks", 1),
    ],
)
def test_real_package_budget_exceeded_rejects_without_truncation(tmp_path, field, value):
    path = tmp_path / "limited.docx"
    doc = Document()
    doc.add_paragraph("first")
    doc.add_paragraph("second")
    doc.save(path)
    mod = parser()
    limits = replace(mod.DEFAULT_LIMITS, **{field: value})
    with pytest.raises(ParseError) as error:
        mod.parse_office(path, limits=limits)
    assert error.value.code == "office_budget_exceeded"


def test_cell_budget_and_invalid_coordinates(tmp_path):
    path = tmp_path / "cells.xlsx"
    book = Workbook()
    book.active["A1"] = "one"
    book.active["B1"] = "two"
    book.save(path)
    mod = parser()
    with pytest.raises(ParseError, match="office_budget_exceeded"):
        mod.parse_office(path, limits=replace(mod.DEFAULT_LIMITS, cells=1))
    rewrite(path, {"xl/worksheets/sheet1.xml": lambda b: b.replace(b'r="B1"', b'r="XFE1"')})
    with pytest.raises(ParseError, match="corrupt_input"):
        mod.parse_office(path)


@pytest.mark.parametrize(
    "payload,suffix,code",
    [
        (b"not a zip", ".docx", "corrupt_input"),
        (bytes.fromhex("d0cf11e0a1b11ae1") + b"encrypted", ".docx", "unsupported_format"),
        (b"legacy", ".doc", "unsupported_format"),
    ],
)
def test_invalid_old_and_encrypted_inputs(tmp_path, payload, suffix, code):
    path = tmp_path / ("invalid" + suffix)
    path.write_bytes(payload)
    with pytest.raises(ParseError) as error:
        parser().parse_office(path)
    assert error.value.code == code


def test_mismatched_content_type_and_macro_type(tmp_path):
    path = docx(tmp_path)
    wrong = path.with_suffix(".xlsx")
    wrong.write_bytes(path.read_bytes())
    with pytest.raises(ParseError, match="unsupported_format"):
        parser().parse_office(wrong)
    rewrite(
        path,
        {
            "[Content_Types].xml": lambda b: b.replace(
                b"wordprocessingml.document.main+xml", b"ms-word.document.macroEnabled.main+xml"
            )
        },
    )
    with pytest.raises(ParseError, match="unsupported_format"):
        parser().parse_office(path)


@pytest.mark.parametrize("target", ["../../escape.xml", "https://invalid.example/a", "/etc/passwd"])
def test_relationship_traversal_refused(tmp_path, target):
    path = docx(tmp_path)
    rel = (
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="image" Target="' + target + '"/></Relationships>'
    )
    rewrite(path, {"word/_rels/document.xml.rels": rel.encode()})
    with pytest.raises(ParseError, match="corrupt_input"):
        parser().parse_office(path)


def test_missing_input_is_io_error_without_path_leak(tmp_path):
    path = tmp_path / "private-name.docx"
    with pytest.raises(ParseError) as error:
        parser().parse_office(path)
    assert error.value.code == "office_io_error"
    assert "private-name" not in str(error.value)


def test_revision_binds_budget_and_is_deterministic(tmp_path):
    path = docx(tmp_path)
    mod = parser()
    first = mod.parse_office(path)
    assert mod.parse_office(path) == first
    changed = mod.parse_office(path, limits=replace(mod.DEFAULT_LIMITS, cells=5))
    assert changed.options_revision != first.options_revision


def test_docx_vertical_merge_is_emitted_once(tmp_path):
    path = tmp_path / "vertical.docx"
    doc = Document()
    table = doc.add_table(rows=2, cols=2)
    table.cell(0, 0).merge(table.cell(1, 0)).text = "vertical"
    table.cell(0, 1).text = "top right"
    table.cell(1, 1).text = "bottom right"
    doc.save(path)
    result = parser().parse_office(path)
    assert [b.text for b in result.blocks] == ["vertical", "top right", "bottom right"]
    assert json.loads(result.blocks[-1].structure)["path"].endswith("row:2/cell:2/paragraph:1")


def test_cached_external_formula_is_refused(tmp_path):
    path = tmp_path / "external.xlsx"
    book = Workbook()
    book.active["A1"] = "='[remote.xlsx]Sheet1'!A1"
    book.save(path)
    rewrite(path, {"xl/worksheets/sheet1.xml": lambda b: b.replace(b"<v></v>", b"<v>42</v>")})
    with pytest.raises(ParseError, match="unsupported_format"):
        parser().parse_office(path)


def test_xlsx_dimension_is_not_used_to_expand_rectangle(tmp_path):
    path = tmp_path / "dimensions.xlsx"
    book = Workbook()
    book.active["A1"] = "one"
    book.save(path)
    rewrite(
        path,
        {
            "xl/worksheets/sheet1.xml": lambda b: b.replace(
                b'<dimension ref="A1:A1"/>', b'<dimension ref="A1:XFD1048576"/>'
            )
        },
    )
    mod = parser()
    assert mod.parse_office(path, limits=replace(mod.DEFAULT_LIMITS, cells=1)).full_text == "one"


def test_hyperlink_text_is_kept_without_dereference(tmp_path, monkeypatch):
    path = tmp_path / "link.pptx"
    deck = Presentation()
    slide = deck.slides.add_slide(deck.slide_layouts[6])
    run = slide.shapes.add_textbox(0, 0, Inches(2), Inches(1)).text_frame.paragraphs[0].add_run()
    run.text = "display"
    run.hyperlink.address = "https://invalid.example/private"
    deck.save(path)
    mod = parser()
    monkeypatch.setattr(socket, "create_connection", lambda *a, **kw: pytest.fail("network access"))
    assert mod.parse_office(path).full_text == "display"


def test_non_hyperlink_external_relationship_refuses_partial_output(tmp_path):
    path = docx(tmp_path)
    rewrite(
        path,
        {
            "word/_rels/document.xml.rels": lambda b: b.replace(
                b"</Relationships>",
                b'<Relationship Id="external" Type="http://schemas.openxmlformats.org/'
                b'officeDocument/2006/relationships/image" Target="https://invalid.example/image" '
                b'TargetMode="External"/></Relationships>',
            )
        },
    )
    with pytest.raises(ParseError, match="unsupported_format"):
        parser().parse_office(path)


def test_crc_and_truncation_are_corrupt(tmp_path):
    path = docx(tmp_path)
    original = path.read_bytes()
    path.write_bytes(original[:-30])
    with pytest.raises(ParseError, match="corrupt_input"):
        parser().parse_office(path)
    path.write_bytes(original)
    # Damage only the central directory CRC: normal decompression still runs.
    raw = bytearray(original)
    offset = raw.index(b"PK\x01\x02")
    raw[offset + 16] ^= 1
    path.write_bytes(raw)
    with pytest.raises(ParseError, match="corrupt_input"):
        parser().parse_office(path)


def test_input_limit_and_exact_budget_boundary(tmp_path, monkeypatch):
    path = docx(tmp_path)
    mod = parser()
    with zipfile.ZipFile(path) as archive:
        size = sum(p.file_size for p in archive.infolist())
    assert mod.parse_office(path, limits=replace(mod.DEFAULT_LIMITS, expanded_bytes=size)).full_text
    with pytest.raises(ParseError, match="office_budget_exceeded"):
        mod.parse_office(path, limits=replace(mod.DEFAULT_LIMITS, expanded_bytes=size - 1))
    monkeypatch.setattr(mod, "MAX_INPUT_BYTES", path.stat().st_size - 1)
    with pytest.raises(ParseError, match="input_too_large"):
        mod.parse_office(path)


def test_memory_error_is_not_misclassified_as_corrupt_input(tmp_path, monkeypatch):
    path = docx(tmp_path)
    mod = parser()

    def out_of_memory(*args, **kwargs):
        raise MemoryError

    monkeypatch.setattr(Path, "open", out_of_memory)
    with pytest.raises(MemoryError):
        mod.parse_office(path)


def test_pptx_chart_fails_instead_of_publishing_missing_text(tmp_path):
    from pptx.chart.data import CategoryChartData
    from pptx.enum.chart import XL_CHART_TYPE

    path = tmp_path / "chart.pptx"
    deck = Presentation()
    slide = deck.slides.add_slide(deck.slide_layouts[6])
    data = CategoryChartData()
    data.categories = ["first", "second"]
    data.add_series("numbers", [1, 2])
    slide.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, 0, 0, Inches(3), Inches(2), data)
    deck.save(path)
    with pytest.raises(ParseError, match="unsupported_format"):
        parser().parse_office(path)


def test_missing_sheet_data_is_corrupt_not_empty_success(tmp_path):
    path = tmp_path / "missing.xlsx"
    book = Workbook()
    book.active["A1"] = "must not disappear"
    book.save(path)
    rewrite(path, {"xl/worksheets/sheet1.xml": lambda b: b.replace(b"sheetData", b"lostData")})
    with pytest.raises(ParseError, match="corrupt_input"):
        parser().parse_office(path)


def test_pptx_alternate_content_and_docx_drawing_text_refused(tmp_path):
    path = docx(tmp_path)
    rewrite(
        path,
        {
            "word/document.xml": lambda b: b.replace(
                b"<w:body>",
                b'<w:body><w:p><w:r><w:drawing><a:t xmlns:a="http://schemas.'
                b'openxmlformats.org/drawingml/2006/main">lost text</a:t></w:drawing></w:r></w:p>',
            )
        },
    )
    with pytest.raises(ParseError, match="unsupported_format"):
        parser().parse_office(path)


def test_oversized_sheet_name_cannot_amplify_location_metadata(tmp_path):
    path = tmp_path / "sheet-name.xlsx"
    book = Workbook()
    book.active["A1"] = "one"
    book.save(path)
    rewrite(
        path,
        {"xl/workbook.xml": lambda b: b.replace(b'name="Sheet"', b'name="' + b"x" * 32 + b'"')},
    )
    with pytest.raises(ParseError, match="corrupt_input"):
        parser().parse_office(path)


def test_overlapping_hidden_column_ranges_keep_visibility(tmp_path):
    path = tmp_path / "overlap.xlsx"
    book = Workbook()
    for column in "ABCDE":
        book.active[column + "1"] = column
    book.save(path)
    rewrite(
        path,
        {
            "xl/worksheets/sheet1.xml": lambda b: b.replace(
                b"<sheetData>",
                b'<cols><col min="2" max="3" hidden="1"/>'
                b'<col min="3" max="4" hidden="true"/></cols><sheetData>',
            )
        },
    )
    result = parser().parse_office(path)
    assert [b.text for b in result.blocks] == ["A", "E"]


@pytest.mark.parametrize("storage", ["inline", "shared"])
@pytest.mark.parametrize("rich", [False, True])
def test_xlsx_phonetic_runs_are_not_cell_body(tmp_path, storage, rich):
    path = tmp_path / "phonetic.xlsx"
    book = Workbook()
    book.active["A1"] = "placeholder"
    book.save(path)
    text = "<r><rPr><b/></rPr><t>東</t></r><r><t>京</t></r>" if rich else "<t>東京</t>"
    text += '<rPh sb="0" eb="2"><t>とうきょう</t></rPh><phoneticPr fontId="0"/>'
    body = text.encode()
    if storage == "inline":
        rewrite(
            path,
            {
                "xl/worksheets/sheet1.xml": lambda b: b.replace(
                    b"<is><t>placeholder</t></is>", b"<is>" + body + b"</is>"
                )
            },
        )
    else:
        rewrite(
            path,
            {
                "xl/worksheets/sheet1.xml": lambda b: b.replace(
                    b't="inlineStr"><is><t>placeholder</t></is>', b't="s"><v>0</v>'
                ),
                "xl/_rels/workbook.xml.rels": lambda b: b.replace(
                    b"</Relationships>",
                    b'<Relationship Id="shared" Type="http://schemas.'
                    b'openxmlformats.org/officeDocument/2006/relationships/sharedStrings" '
                    b'Target="sharedStrings.xml"/></Relationships>',
                ),
                "[Content_Types].xml": lambda b: b.replace(
                    b"</Types>",
                    b'<Override PartName="/xl/sharedStrings.xml" ContentType="application/'
                    b'vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>',
                ),
            },
            extra=[
                (
                    "xl/sharedStrings.xml",
                    b'<sst xmlns="http://schemas.openxmlformats.org/'
                    b'spreadsheetml/2006/main"><si>' + body + b"</si></sst>",
                )
            ],
        )
    result = parser().parse_office(path)
    assert result.full_text == "東京"
    assert json.loads(result.blocks[0].structure) == {"sheet": "Sheet", "cell": "A1"}


@pytest.mark.parametrize(
    "node,character",
    [
        ("<w:noBreakHyphen/>", "\u2011"),
        ("<w:softHyphen/>", "\u00ad"),
        ('<w:ptab w:alignment="center" w:relativeTo="margin" w:leader="none"/>', "\t"),
    ],
)
def test_docx_inline_character_nodes_preserve_body(tmp_path, node, character):
    path = docx(tmp_path)
    rewrite(
        path,
        {
            "word/document.xml": lambda b: b.replace(
                b"<w:t>before</w:t>", b"<w:t>left</w:t>" + node.encode() + b"<w:t>right</w:t>"
            )
        },
    )
    assert parser().parse_office(path).full_text == "left" + character + "right"


@pytest.mark.parametrize(
    "node",
    [
        '<w:sym w:font="Wingdings" w:char="F04A"/>',
        "<w:ruby><w:rt><w:r><w:t>reading</w:t></w:r></w:rt>"
        "<w:rubyBase><w:r><w:t>body</w:t></w:r></w:rubyBase></w:ruby>",
        "<w:pgNum/>",
        "<w:contentPart/>",
        "<w:unexpectedBody><w:t>unknown</w:t></w:unexpectedBody>",
    ],
)
def test_docx_unmapped_inline_body_refuses_partial_success(tmp_path, node):
    path = docx(tmp_path)
    rewrite(
        path,
        {
            "word/document.xml": lambda b: b.replace(
                b"<w:t>before</w:t>", b"<w:t>before</w:t>" + node.encode()
            )
        },
    )
    with pytest.raises(ParseError, match="unsupported_format"):
        parser().parse_office(path)


def test_docx_ruby_at_paragraph_level_refuses_reading_pollution(tmp_path):
    path = docx(tmp_path)
    rewrite(
        path,
        {
            "word/document.xml": lambda b: b.replace(
                b"</w:r></w:p>",
                b"</w:r><w:ruby><w:rt><w:r><w:t>reading</w:t></w:r></w:rt>"
                b"<w:rubyBase><w:r><w:t>body</w:t></w:r></w:rubyBase></w:ruby></w:p>",
                1,
            )
        },
    )
    with pytest.raises(ParseError, match="unsupported_format"):
        parser().parse_office(path)


def test_docx_hyperlink_and_markup_keep_body_in_order(tmp_path):
    path = docx(tmp_path)
    rewrite(
        path,
        {
            "word/document.xml": lambda b: b.replace(
                b"<w:r><w:t>before</w:t></w:r>",
                b'<w:bookmarkStart w:id="1" w:name="here"/>'
                b'<w:hyperlink w:anchor="here"><w:r><w:rPr><w:b/></w:rPr><w:t>linked</w:t>'
                b"<w:tab/><w:t>text</w:t><w:br/><w:t>line</w:t></w:r></w:hyperlink>"
                b'<w:bookmarkEnd w:id="1"/>',
            )
        },
    )
    assert parser().parse_office(path).full_text == "linked\ttext\nline"


@pytest.mark.parametrize("format", ["docx", "xlsx", "pptx"])
def test_t48_execution_dispatch_uses_office_identity_on_opaque_staging(tmp_path, format):
    import hashlib

    from knoted_intelligence.executor import _compute

    path = tmp_path / "opaque-staged-object.bin"
    if format == "docx":
        doc = Document()
        doc.add_paragraph("Office中文")
        doc.save(path)
    elif format == "xlsx":
        book = Workbook()
        book.active.title = "原表名"
        book.active["C3"] = "Office中文"
        book.save(path)
    else:
        deck = Presentation()
        slide = deck.slides.add_slide(deck.slide_layouts[6])
        slide.shapes.add_textbox(0, 0, Inches(2), Inches(1)).text = "Office中文"
        deck.save(path)
    mod = parser()
    spec = dict(
        kind="parse",
        input_path=str(path),
        source_version=7,
        input_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
        options_revision=mod.options_revision(),
    )
    result = _compute(spec)
    assert result["source_format"] == format
    assert result["blocks"][0]["text"] == "Office中文"
    assert result["options_revision"] == mod.options_revision()
    assert result["parser"] == {
        "parser_revision": mod.POLICY_REVISION,
        "options_revision": mod.options_revision(),
        "limits": __import__("dataclasses").asdict(mod.DEFAULT_LIMITS),
    }
    assert result["text_length"] == 8
    assert result["source_version"] == 7 and result["truncated"] is False
    assert result["page_count"] == (1 if format == "pptx" else None)


# Real loopback service, guardian and SQLite; no mocked execution state.


@contextmanager
def office_http(root, hold_ms=0):
    import os
    import sys

    import httpx
    from test_t47_http import wait_for

    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    env = os.environ | {
        "KNOTED_INTELLIGENCE_HOST": "127.0.0.1",
        "KNOTED_INTELLIGENCE_PORT": str(port),
        "KNOTED_INTELLIGENCE_STATE_DIR": str(root / "state"),
        "KNOTED_INTELLIGENCE_STAGING_DIR": str(root / "staging"),
        "KNOTED_INTELLIGENCE_CHILD_HOLD_MS": str(hold_ms),
        "KNOTED_INTELLIGENCE_CHILD_IGNORE_SIGTERM": "1",
        "KNOTED_INTELLIGENCE_CANCEL_GRACE_SECONDS": "0.1",
        "CUDA_VISIBLE_DEVICES": "",
        "KNOTED_EMBED_GPU_UUID": "",
        "KNOTED_INTELLIGENCE_DOCLING_DEVICE": "cpu",
    }
    root.mkdir(parents=True, exist_ok=True)
    with (root / "http.log").open("a") as log:
        process = subprocess.Popen(
            [sys.executable, "-I", "-B", "-c", "from knoted_intelligence.app import main;main()"],
            env=env,
            stdout=log,
            stderr=log,
        )
        with httpx.Client(base_url=f"http://127.0.0.1:{port}", timeout=30) as client:

            def ready():
                assert process.poll() is None, "owned Office test service exited"
                try:
                    return client.get("/internal/v1/health").status_code == 200
                except httpx.TransportError:
                    return False

            try:
                wait_for(ready)
                yield client
            finally:
                if process.poll() is None:
                    process.terminate()
                process.wait(timeout=30)


def office_request(path, seconds=60):
    import hashlib
    from datetime import UTC, datetime, timedelta
    from uuid import uuid4

    return dict(
        execution_id=str(uuid4()),
        kind="parse",
        input_object_id=path.name,
        input_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
        source_version=7,
        options_revision=parser().options_revision(),
        deadline=(datetime.now(UTC) + timedelta(seconds=seconds)).isoformat(),
    )


def office_terminal(client, request):
    from test_t47_http import wait_for

    def completed():
        record = client.get(f"/internal/v1/executions/{request['execution_id']}").json()
        return record if record.get("compute_exit_confirmed") else None

    return wait_for(completed, seconds=60)


def test_t48_real_http_manifest_success_and_permanent_codes(tmp_path):
    import hashlib
    import sys

    fixtures = tmp_path / "fixtures"
    script = Path(__file__).resolve().parents[3] / "crates/knoted-core/tests/fixtures/t48_office.py"
    subprocess.run([sys.executable, "-I", "-B", str(script), str(fixtures)], check=True)
    with office_http(tmp_path) as client:
        inputs = tmp_path / "staging/input"
        for format in ("docx", "xlsx", "pptx"):
            path = inputs / ("opaque-" + format)  # no filename hint reaches parser
            path.write_bytes((fixtures / ("sample." + format)).read_bytes())
            request = office_request(path)
            assert client.post("/internal/v1/executions", json=request).status_code == 201
            record = office_terminal(client, request)
            assert record["state"] == "succeeded", record
            result = client.get(f"/internal/v1/executions/{request['execution_id']}/result").json()
            raw = (
                tmp_path / "staging/output" / request["execution_id"] / result["object_id"]
            ).read_bytes()
            assert len(raw) == result["size"]
            assert hashlib.sha256(raw).hexdigest() == result["sha256"]
            artifact = json.loads(raw)
            assert artifact["source_format"] == format
            assert artifact["options_revision"] == request["options_revision"]
            assert artifact["parser"] == parser().parser_summary()
            assert artifact["source_version"] == 7 and artifact["truncated"] is False
            assert artifact["page_count"] == (2 if format == "pptx" else None)
            if format == "xlsx":
                assert json.loads(artifact["blocks"][0]["structure"])["sheet"] == "中文'原名"
            if format == "pptx":
                assert artifact["blocks"][0]["page"] == 2
            for key in ("options_revision", "input_sha256"):
                assert result["metadata"][key] == request[key]
            assert result["metadata"]["source_version"] == "7"
            # Terminal cancel conflict cannot revoke successful exit proof.
            assert (
                client.post(f"/internal/v1/executions/{request['execution_id']}/cancel").status_code
                == 409
            )
            assert client.post("/internal/v1/executions", json=request).json()[
                "compute_exit_confirmed"
            ]
        budget = tmp_path / "budget.docx"
        budget.write_bytes((fixtures / "sample.docx").read_bytes())
        rewrite(budget, extra=[("bomb.xml", b"<x>" + b"a" * 60000 + b"</x>")])
        for payload, code in [
            (budget.read_bytes(), "office_budget_exceeded"),
            (b"broken", "corrupt_input"),
            (bytes.fromhex("d0cf11e0a1b11ae1"), "unsupported_format"),
        ]:
            path = inputs / "broken"
            path.write_bytes(payload)
            request = office_request(path)
            assert client.post("/internal/v1/executions", json=request).status_code == 201
            record = office_terminal(client, request)
            assert record["state"] == "failed" and record["error"]["code"] == code
            assert (
                client.get(f"/internal/v1/executions/{request['execution_id']}/result").status_code
                == 422
            )


def test_t48_real_http_cancel_deadline_restart_and_new_attempt(tmp_path):
    import sys

    from test_t47_http import wait_for

    from knoted_intelligence.supervision import inspect

    sentinel = subprocess.Popen([sys.executable, "-I", "-B", "-c", "import time;time.sleep(120)"])
    try:
        with office_http(tmp_path, hold_ms=60000) as client:
            path = tmp_path / "staging/input/office"
            doc = Document()
            doc.add_paragraph("retry中文")
            doc.save(path)
            cancelled = office_request(path)
            assert client.post("/internal/v1/executions", json=cancelled).status_code == 201
            marker = tmp_path / "staging/output" / cancelled["execution_id"] / "child-status.json"
            wait_for(marker.exists)
            pid = json.loads(marker.read_text())["pid"]
            response = client.post(f"/internal/v1/executions/{cancelled['execution_id']}/cancel")
            assert response.status_code == 200
            assert office_terminal(client, cancelled)["state"] == "cancelled"
            assert not Path(f"/proc/{pid}").exists()
            expired = office_request(path, seconds=1)
            assert client.post("/internal/v1/executions", json=expired).status_code == 201
            record = office_terminal(client, expired)
            assert record["state"] == "failed" and record["error"]["code"] == "deadline_exceeded"
            active = office_request(path)
            assert client.post("/internal/v1/executions", json=active).status_code == 201
            wait_for(
                (tmp_path / "staging/output" / active["execution_id"] / "child-status.json").exists
            )
        assert inspect(tmp_path / "state")["clean"]
        assert sentinel.poll() is None
        with office_http(tmp_path) as client:
            old = office_terminal(client, active)
            assert old["state"] in {"cancelled", "interrupted"}
            assert client.post("/internal/v1/executions", json=active).json()[
                "compute_exit_confirmed"
            ]
            retry = office_request(path)
            assert retry["execution_id"] != active["execution_id"]
            assert client.post("/internal/v1/executions", json=retry).status_code == 201
            assert office_terminal(client, retry)["state"] == "succeeded"
    finally:
        sentinel.terminate()
        sentinel.wait(timeout=5)


def test_t48_empty_inline_string_preserves_visible_neighbor(tmp_path):
    path = tmp_path / "empty.xlsx"
    book = Workbook()
    book.active["A1"] = "expected visible"
    book.active["B1"] = ""
    book.save(path)
    assert [b.text for b in parser().parse_office(path).blocks] == ["expected visible"]


@pytest.mark.parametrize("content", ["<v>3</v>", "<f>1+2</f>", "<is><t>text</t></is><v>3</v>"])
def test_t48_inline_string_conflicting_values_refused(tmp_path, content):
    path = tmp_path / "conflict.xlsx"
    book = Workbook()
    book.active["A1"] = "expected visible"
    book.active["B1"] = ""
    book.save(path)
    rewrite(
        path,
        {
            "xl/worksheets/sheet1.xml": lambda data: data.replace(
                b'<c r="B1" t="inlineStr"></c>',
                f'<c r="B1" t="inlineStr">{content}</c>'.encode(),
            )
        },
    )
    with pytest.raises(ParseError) as error:
        parser().parse_office(path)
    assert error.value.code == "corrupt_input"


@pytest.mark.parametrize("declared", [4097, 1])
def test_t48_member_budget_precedes_zipinfo_allocation(tmp_path, monkeypatch, declared):
    import struct

    path = tmp_path / "many.docx"
    with zipfile.ZipFile(path, "w") as archive:
        for index in range(4097):
            archive.writestr(f"empty{index}", b"")
    data = bytearray(path.read_bytes())
    end = data.rfind(b"PK\x05\x06")
    struct.pack_into("<HH", data, end + 8, declared, declared)
    path.write_bytes(data)
    office = parser()

    def refuse_full_directory(*args, **kwargs):
        pytest.fail("over-budget directory reached full ZipFile construction")

    monkeypatch.setattr(office.zipfile, "ZipFile", refuse_full_directory)
    with pytest.raises(ParseError) as error:
        office.parse_office(path)
    assert error.value.code == "office_budget_exceeded"


def test_t48_directory_exact_member_limit_with_variable_fields(tmp_path):
    path = docx(tmp_path)
    with zipfile.ZipFile(path) as archive:
        count = len(archive.infolist())
    with zipfile.ZipFile(path, "a") as archive:
        archive.comment = b"archive comment"
        for index in range(4096 - count):
            member = zipfile.ZipInfo(f"media/unused{index}.bin")
            member.extra = b"\xfe\xca\x02\x00ok"
            member.comment = b"member comment"
            archive.writestr(member, b"")
    assert parser().parse_office(path).full_text == "before"


@pytest.mark.parametrize("damage", ["count", "length", "zip64", "multidisk"])
def test_t48_directory_rejects_malformed_or_unsupported_headers(tmp_path, damage):
    import struct

    path = docx(tmp_path)
    data = bytearray(path.read_bytes())
    end = data.rfind(b"PK\x05\x06")
    expected = "corrupt_input"
    if damage == "count":
        struct.pack_into("<HH", data, end + 8, 1, 1)
    elif damage == "length":
        central = struct.unpack_from("<L", data, end + 16)[0]
        struct.pack_into("<HHH", data, central + 28, 65535, 65535, 65535)
    elif damage == "zip64":
        struct.pack_into("<L", data, end + 16, 0xFFFFFFFF)
        expected = "unsupported_format"
    else:
        struct.pack_into("<H", data, end + 4, 1)
        expected = "unsupported_format"
    path.write_bytes(data)
    with pytest.raises(ParseError) as error:
        parser().parse_office(path)
    assert error.value.code == expected
