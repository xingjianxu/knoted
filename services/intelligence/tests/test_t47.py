"""Phase A: real input admission and offline OCR; supervision belongs to Phase B."""

import socket
from pathlib import Path

import pypdfium2
import pytest
from PIL import Image

from knoted_intelligence.parsing import docling_adapter, models
from knoted_intelligence.parsing.pdf import parse_pdf_image


def parse(path: Path):
    return parse_pdf_image(path, options_revision=models.OPTIONS_REVISION)


def test_t47_1_real_pdf_page_boundary(tmp_path, monkeypatch):
    admitted = []

    def stop_before_model(root):
        admitted.append(root)
        raise RuntimeError("admission complete")

    monkeypatch.setattr(models, "require_artifacts", stop_before_model)
    for pages in (500, 501):
        path = tmp_path / f"{pages}.pdf"
        document = pypdfium2.PdfDocument.new()
        for _ in range(pages):
            document.new_page(72, 72).close()
        document.save(path)
        document.close()
        if pages == 500:
            with pytest.raises(RuntimeError, match="admission complete"):
                parse(path)
        else:
            with pytest.raises(models.ParseError) as error:
                parse(path)
            assert error.value.code == "too_many_pages"
    assert len(admitted) == 1


@pytest.mark.parametrize("format", ["GIF", "TIFF", "BMP"])
def test_t47_rejects_images_outside_whitelist(tmp_path, format):
    path = tmp_path / "staged"
    Image.new("RGB", (8, 8)).save(path, format=format)
    with pytest.raises(models.ParseError) as error:
        parse(path)
    assert error.value.code == "unsupported_format"


@pytest.mark.parametrize("payload", [b"hello", b"PK\x03\x04office", b"<html>test</html>"])
def test_t47_does_not_parse_text_office_or_unknown(tmp_path, payload):
    path = tmp_path / "staged"
    path.write_bytes(payload)
    with pytest.raises(models.ParseError) as error:
        parse(path)
    assert error.value.code == "unsupported_format"


@pytest.mark.parametrize("format", ["JPEG", "PNG", "WEBP"])
def test_t47_whitelist_reaches_model_admission(tmp_path, monkeypatch, format):
    path = tmp_path / "staged"
    Image.new("RGB", (8, 8)).save(path, format=format)

    def reached(root):
        raise RuntimeError("admitted")

    monkeypatch.setattr(models, "require_artifacts", reached)
    with pytest.raises(RuntimeError, match="admitted"):
        parse(path)


def test_t47_size_limit_precedes_models(tmp_path, monkeypatch):
    path = tmp_path / "oversized"
    with path.open("wb") as stream:
        stream.write(b"%PDF-1.7")
        stream.truncate(100 * 1024 * 1024 + 1)
    with pytest.raises(models.ParseError) as error:
        parse(path)
    assert error.value.code == "input_too_large"


def test_t47_3_real_chinese_ocr_offline(monkeypatch):
    def deny(*args, **kwargs):
        raise AssertionError("network forbidden")

    monkeypatch.setattr(socket.socket, "connect", deny)
    monkeypatch.setattr(socket, "create_connection", deny)
    path = Path(__file__).resolve().parents[3] / "tests/fixtures/ocr/sample_zh_en.png"
    result = parse(path)
    assert isinstance(result, docling_adapter.ParsedDocument)
    assert all(word in result.full_text for word in ("知识库", "合同", "检索"))
    assert result.page_count == 1
    assert all(block.page == 1 for block in result.blocks)
    assert any(block.bbox for block in result.blocks)


@pytest.mark.parametrize("width,height,accepted", [(8000, 5000, True), (8001, 5000, False)])
def test_t47_pixel_limit_before_models(tmp_path, monkeypatch, width, height, accepted):
    from test_t10 import png_header

    path = tmp_path / "staged"
    path.write_bytes(png_header(width=width, height=height))

    def reached(root):
        raise RuntimeError("admitted")

    monkeypatch.setattr(models, "require_artifacts", reached)
    if accepted:
        with pytest.raises(RuntimeError, match="admitted"):
            parse(path)
    else:
        with pytest.raises(models.ParseError) as error:
            parse(path)
        assert error.value.code == "image_too_large"
