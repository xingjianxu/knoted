"""Real OOXML packages exercise extraction and admission, without Office/model execution."""

import importlib.util
import json
import socket
import subprocess
import zipfile
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
