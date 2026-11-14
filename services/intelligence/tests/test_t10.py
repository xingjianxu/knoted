"""T10 acceptance tests: Docling + RapidOCR parsing of Chinese/English input.

These tests run the **real** parser: Docling's layout model, TableFormer and
RapidOCR are loaded from the locally provisioned, revision-locked artifacts
under ``KNOTED_INTELLIGENCE_MODELS_DIR`` (default
``/data/knoted-dev/service/models``). Nothing here mocks a model, a download or
an OCR result.

``t10_1`` additionally forbids every socket connection while the parse runs, so
a hidden Hugging Face download would fail instead of silently succeeding. If the
artifacts are absent the tests fail with the exact installation command from
``parsing/LOCK.md``; they never skip.
"""

from __future__ import annotations

import hashlib
import json
import os
import socket
import struct
import subprocess
import sys
import textwrap
import time
import uuid
import zlib
from collections.abc import Iterator
from contextlib import contextmanager
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import pytest
from fastapi.testclient import TestClient

from knoted_intelligence import executor
from knoted_intelligence.app import create_app
from knoted_intelligence.config import Settings
from knoted_intelligence.parsing import ParseError, parse_document
from knoted_intelligence.parsing import models as locked

FIXTURE_DIR = Path(__file__).resolve().parents[3] / "tests/fixtures/ocr"
SAMPLE_PNG = FIXTURE_DIR / "sample_zh_en.png"

#: Rendered on the fixture (see ``tests/fixtures/ocr/README.md``). PP-OCR drops
#: CJK spaces, so the words are asserted as substrings and never with spacing.
CHINESE_KEYWORDS = ("知识库", "合同", "笔记", "检索", "摘要", "中文识别")
ENGLISH_KEYWORDS = ("docling", "rapidocr")

EXECUTIONS_PATH = "/internal/v1/executions"


def require_artifacts() -> Path:
    """Fails with the provisioning command when the locked models are absent."""

    artifacts = locked.docling_artifacts_path(locked.default_models_dir())
    problems = locked.artifact_problems(locked.default_models_dir())
    if problems:
        pytest.fail(
            "the revision-locked OCR artifacts are not provisioned under "
            f"{artifacts}:\n  " + "\n  ".join(problems) + "\n"
            "Install them with /data/knoted-dev/service/models/ocr/fetch-models.sh "
            "(see services/intelligence/src/knoted_intelligence/parsing/LOCK.md)."
        )
    return artifacts


def parse_sample() -> Any:
    return parse_document(
        SAMPLE_PNG,
        options_revision=locked.OPTIONS_REVISION,
        models_dir=locked.default_models_dir(),
    )


# -- t10_1 -------------------------------------------------------------------


def test_t10_1_offline_parse_does_not_download(monkeypatch: pytest.MonkeyPatch) -> None:
    artifacts = require_artifacts()

    attempted: list[object] = []

    def refuse_connection(*args: object, **kwargs: object) -> None:
        attempted.append(args)
        raise AssertionError("the parser attempted a network connection")

    monkeypatch.setattr(socket.socket, "connect", refuse_connection)
    monkeypatch.setattr(socket, "create_connection", refuse_connection)

    document = parse_sample()

    assert attempted == [], "parsing must not open a socket"
    assert os.environ["HF_HUB_OFFLINE"] == "1"
    assert os.environ["TRANSFORMERS_OFFLINE"] == "1"
    assert Path(os.environ["DOCLING_ARTIFACTS_PATH"]) == artifacts

    kinds = {block.kind for block in document.blocks}
    assert {"heading", "paragraph", "table"} <= kinds, kinds
    assert document.title, "the heading is promoted to the document title"
    assert document.page_count == 1
    assert document.full_text.strip()

    # Every block carries a structural location, so a citation can point at it.
    assert all(block.page == 1 for block in document.blocks)
    assert any(block.bbox is not None for block in document.blocks)
    assert any(block.kind == "table" and "|" in block.text for block in document.blocks)

    # The lock summary is complete enough for an incident report.
    summary = locked.locked_combination_summary()
    for expected in (
        locked.DOCLING_VERSION,
        locked.RAPIDOCR_VERSION,
        locked.DOCLING_IBM_MODELS_VERSION,
        locked.LAYOUT_REVISION,
        locked.TABLEFORMER_REVISION,
        locked.RAPIDOCR_REVISION,
    ):
        assert expected in summary


def test_t10_1b_offline_parse_in_child_process() -> None:
    """The execution child (not just the pytest process) is offline.

    Host evidence ran Docling inside a network namespace; here the child process
    forbids sockets itself and parses the fixture, so the environment the
    scheduler really spawns is covered without requiring ``unshare`` rights.
    """

    require_artifacts()
    script = textwrap.dedent(
        """
        import socket
        import sys
        from pathlib import Path

        attempts = []

        def refuse(*args, **kwargs):
            attempts.append(args)
            raise AssertionError("the parser attempted a network connection")

        socket.socket.connect = refuse
        socket.create_connection = refuse

        from knoted_intelligence.parsing import models, parse_document

        import os

        for key in ("HF_HUB_OFFLINE", "HF_DATASETS_OFFLINE", "TRANSFORMERS_OFFLINE"):
            assert os.environ.get(key) == "1", (key, os.environ.get(key))

        document = parse_document(
            Path(sys.argv[1]),
            options_revision=models.OPTIONS_REVISION,
            models_dir=models.default_models_dir(),
        )
        assert attempts == [], attempts
        assert "知识库" in document.full_text
        assert len(document.blocks) >= 3
        print("child-offline-ok")
        """
    )
    completed = subprocess.run(
        [sys.executable, "-c", script, str(SAMPLE_PNG)],
        capture_output=True,
        text=True,
        timeout=300.0,
        check=False,
    )
    assert completed.returncode == 0, completed.stderr
    assert completed.stdout.strip().endswith("child-offline-ok"), completed.stdout


# -- t10_2 -------------------------------------------------------------------


def test_t10_2_chinese_keywords_preserved(tmp_path: Path) -> None:
    require_artifacts()
    document = parse_sample()

    full_text = document.full_text
    missing = [keyword for keyword in CHINESE_KEYWORDS if keyword not in full_text]
    assert missing == [], f"OCR dropped Chinese keywords {missing}; text was {full_text!r}"

    lowered = full_text.lower()
    english_hits = [keyword for keyword in ENGLISH_KEYWORDS if keyword in lowered]
    assert english_hits, f"no English keyword survived OCR; text was {full_text!r}"
    assert "2026-11-15" in full_text, "digits and punctuation must survive as well"

    headings = [block.text for block in document.blocks if block.kind == "heading"]
    assert any("中文识别" in text for text in headings), headings

    # The same fixture also runs through the real execution protocol: the parse
    # child loads the models itself, so this proves the child wiring (not just
    # the in-process adapter) preserves the text.
    with running_service(tmp_path) as (client, settings):
        object_id = "sample-zh-en.png"
        payload = SAMPLE_PNG.read_bytes()
        (settings.input_dir / object_id).write_bytes(payload)
        execution_id = str(uuid.uuid4())
        created = client.post(
            EXECUTIONS_PATH,
            json={
                "execution_id": execution_id,
                "kind": "parse",
                "input_object_id": object_id,
                "input_sha256": hashlib.sha256(payload).hexdigest(),
                "source_version": 1,
                "deadline": (datetime.now(UTC) + timedelta(minutes=5)).isoformat(),
                "options_revision": locked.OPTIONS_REVISION,
            },
        )
        assert created.status_code == 201, created.text
        wait_for_state(client, execution_id, {"succeeded"}, timeout=180.0)

        result = client.get(f"{EXECUTIONS_PATH}/{execution_id}/result").json()
        artifact = settings.output_dir / execution_id / result["object_id"]
        parsed = json.loads(artifact.read_text(encoding="utf-8"))
        artifact_text = "\n".join(block["text"] for block in parsed["blocks"])
        assert all(keyword in artifact_text for keyword in CHINESE_KEYWORDS)
        # One copy of the text only: the blocks carry it, `full_text` stays an
        # in-process adapter field so the artifact cannot double the volume.
        assert "full_text" not in parsed, sorted(parsed)
        assert parsed["parser"]["docling"] == locked.DOCLING_VERSION
        # The requested revision and the locked revision are both recorded; a
        # worker compares them instead of trusting the request.
        assert parsed["options_revision"] == locked.OPTIONS_REVISION
        assert parsed["parser"]["options_revision"] == locked.OPTIONS_REVISION
        assert {block["type"] for block in parsed["blocks"]} >= {"heading", "table"}


# -- t10_3 -------------------------------------------------------------------


def test_t10_3_failure_reports_locked_combination(tmp_path: Path) -> None:
    require_artifacts()
    summary_parts = (
        locked.DOCLING_VERSION,
        locked.RAPIDOCR_VERSION,
        locked.LAYOUT_REVISION,
        locked.TABLEFORMER_REVISION,
    )

    truncated = tmp_path / "truncated.png"
    truncated.write_bytes(SAMPLE_PNG.read_bytes()[:64])
    with pytest.raises(ParseError) as corrupt:
        parse_document(
            truncated,
            options_revision=locked.OPTIONS_REVISION,
            models_dir=locked.default_models_dir(),
        )
    assert corrupt.value.code in {"corrupt_input", "unsupported_format"}
    # The summary leads the message, so a downstream truncation can never cut
    # the lock report off the end.
    assert str(corrupt.value).startswith(f"[locked: {locked.locked_combination_summary()}]"), str(
        corrupt.value
    )
    for part in summary_parts:
        assert part in str(corrupt.value), str(corrupt.value)
    assert "docling" in str(corrupt.value).lower()

    unsupported = tmp_path / "payload.bin"
    unsupported.write_bytes(bytes(range(256)) * 4)
    with pytest.raises(ParseError) as garbage:
        parse_document(
            unsupported,
            options_revision=locked.OPTIONS_REVISION,
            models_dir=locked.default_models_dir(),
        )
    assert garbage.value.code == "unsupported_format"
    for part in summary_parts:
        assert part in str(garbage.value), str(garbage.value)

    # The same failure reaches the worker as a terminal `failed` record with the
    # locked combination in the message, never as a silent re-selection.
    payload = SAMPLE_PNG.read_bytes()[:64]
    with running_service(tmp_path) as (client, settings):
        object_id = "truncated.png"
        (settings.input_dir / object_id).write_bytes(payload)
        execution_id = str(uuid.uuid4())
        created = client.post(
            EXECUTIONS_PATH,
            json={
                "execution_id": execution_id,
                "kind": "parse",
                "input_object_id": object_id,
                "input_sha256": hashlib.sha256(payload).hexdigest(),
                "source_version": 1,
                "deadline": (datetime.now(UTC) + timedelta(minutes=5)).isoformat(),
                "options_revision": locked.OPTIONS_REVISION,
            },
        )
        assert created.status_code == 201, created.text

        record = wait_for_state(client, execution_id, {"failed"}, timeout=120.0)
        assert record["error"]["code"] == "execution_failed", record
        message = record["error"]["message"]
        for part in summary_parts:
            assert part in message, message
        assert client.get(f"{EXECUTIONS_PATH}/{execution_id}/result").status_code == 422


def test_t10_3b_missing_models_reported_via_execution(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A long `models_missing` message survives the child -> record round trip.

    The message lists the missing files *after* the lock summary; this is the
    case where head truncation used to cut the versions and revisions off.
    """

    empty_models = tmp_path / "empty-models"
    empty_models.mkdir()
    monkeypatch.setenv(locked.MODELS_DIR_ENV, str(empty_models))

    with running_service(tmp_path) as (client, settings):
        object_id = "sample-zh-en.png"
        payload = SAMPLE_PNG.read_bytes()
        (settings.input_dir / object_id).write_bytes(payload)
        execution_id = str(uuid.uuid4())
        created = client.post(
            EXECUTIONS_PATH,
            json={
                "execution_id": execution_id,
                "kind": "parse",
                "input_object_id": object_id,
                "input_sha256": hashlib.sha256(payload).hexdigest(),
                "source_version": 1,
                "deadline": (datetime.now(UTC) + timedelta(minutes=5)).isoformat(),
                "options_revision": locked.OPTIONS_REVISION,
            },
        )
        assert created.status_code == 201, created.text

        record = wait_for_state(client, execution_id, {"failed"}, timeout=120.0)

    assert record["error"]["code"] == "execution_failed", record
    message = record["error"]["message"]
    assert "models_missing" in message, message
    # The whole summary, not a prefix of it, reaches the record.
    assert f"[locked: {locked.locked_combination_summary()}]" in message, message
    assert "RapidOcr/PP-OCRv6_rec_small.onnx: missing" in message, message


# -- t10_4 -------------------------------------------------------------------


def test_t10_4_limits_enforced(tmp_path: Path) -> None:
    assert locked.MAX_INPUT_BYTES == 100 * 1024 * 1024
    assert locked.MAX_IMAGE_PIXELS == 40_000_000
    assert locked.MAX_PDF_PAGES == 500

    # Byte limit: the same check the parser applies, with a synthetic size so
    # the test never writes a 100 MiB file.
    locked.check_input_size(locked.MAX_INPUT_BYTES)
    with pytest.raises(ParseError) as oversized:
        locked.check_input_size(locked.MAX_INPUT_BYTES + 1)
    assert oversized.value.code == "input_too_large"
    assert "100 MiB" in str(oversized.value)

    # Pixel limit: a 62-byte PNG whose IHDR claims 60M pixels. The parser reads
    # the header and refuses before decoding any pixel data.
    bomb = tmp_path / "pixel-bomb.png"
    bomb.write_bytes(png_header(width=10_000, height=6_000))
    with pytest.raises(ParseError) as too_many_pixels:
        parse_document(
            bomb,
            options_revision=locked.OPTIONS_REVISION,
            models_dir=locked.default_models_dir(),
        )
    assert too_many_pixels.value.code == "image_too_large"
    assert "40" in str(too_many_pixels.value) and "pixel" in str(too_many_pixels.value).lower()

    # A PDF page can be huge without a huge file: a 1 KB PDF with a 20000x20000
    # pt MediaBox (~400M pixels at 72 dpi) is refused by the same pixel limit.
    huge_pdf = tmp_path / "huge-media-box.pdf"
    huge_pdf.write_bytes(minimal_pdf(width=20_000, height=20_000))
    assert huge_pdf.stat().st_size < 4_096
    empty_models = tmp_path / "empty-models"
    empty_models.mkdir()
    with pytest.raises(ParseError) as pdf_bomb:
        parse_document(
            huge_pdf,
            options_revision=locked.OPTIONS_REVISION,
            models_dir=empty_models,
        )
    assert pdf_bomb.value.code == "image_too_large", pdf_bomb.value
    assert "pdf page 1" in str(pdf_bomb.value), str(pdf_bomb.value)

    # A normal page passes the pixel check and only then reports the (empty)
    # model tree, which proves the order: admission first, model load second.
    normal_pdf = tmp_path / "normal.pdf"
    normal_pdf.write_bytes(minimal_pdf(width=612, height=792))
    with pytest.raises(ParseError) as no_models:
        parse_document(
            normal_pdf,
            options_revision=locked.OPTIONS_REVISION,
            models_dir=empty_models,
        )
    assert no_models.value.code == "models_missing", no_models.value

    # The 100 MiB branch goes through the real parse_document entry point; a
    # sparse file costs no disk space and is rejected before it is read.
    oversized_text = tmp_path / "oversized.txt"
    with oversized_text.open("wb") as handle:
        handle.truncate(locked.MAX_INPUT_BYTES + 1)
    with pytest.raises(ParseError) as sparse_oversized:
        parse_document(
            oversized_text,
            options_revision=locked.OPTIONS_REVISION,
            models_dir=empty_models,
        )
    assert sparse_oversized.value.code == "input_too_large"
    assert oversized_text.stat().st_blocks * 512 < oversized_text.stat().st_size

    # The boundary itself is accepted by the checks (40M pixels exactly).
    locked.check_image_pixels(8_000, 5_000)
    with pytest.raises(ParseError):
        locked.check_image_pixels(8_000, 5_001)

    # Page limit uses the same pure check, so no 501-page document is needed.
    locked.check_page_count(locked.MAX_PDF_PAGES)
    with pytest.raises(ParseError) as too_many_pages:
        locked.check_page_count(locked.MAX_PDF_PAGES + 1)
    assert too_many_pages.value.code == "too_many_pages"


def test_t10_4b_artifact_volume_is_bounded(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    assert locked.MAX_ARTIFACT_BYTES == 128 * 1024 * 1024
    locked.check_artifact_size(locked.MAX_ARTIFACT_BYTES)
    with pytest.raises(ParseError) as oversized:
        locked.check_artifact_size(locked.MAX_ARTIFACT_BYTES + 1)
    assert oversized.value.code == "output_too_large"
    assert "artifact" in str(oversized.value).lower()

    # The real handler applies the check to the serialized artifact. A tiny
    # synthetic limit exercises the real `_compute` path without a huge input.
    source = tmp_path / "artifact-volume.txt"
    source.write_text("Knoted artifact volume\nsecond line\n", encoding="utf-8")
    spec = {
        "kind": "parse",
        "input_path": str(source),
        "input_sha256": "0" * 64,
        "source_version": 1,
        "options_revision": locked.OPTIONS_REVISION,
    }
    monkeypatch.setattr(locked, "MAX_ARTIFACT_BYTES", 32)
    with pytest.raises(ParseError) as artifact_too_large:
        executor._compute(spec)
    assert artifact_too_large.value.code == "output_too_large", artifact_too_large.value
    assert str(artifact_too_large.value).startswith("[locked: ")


# -- helpers -----------------------------------------------------------------


def png_header(*, width: int, height: int) -> bytes:
    """A structurally valid PNG signature + IHDR + IEND but no pixel data."""

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    ihdr = chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    return b"\x89PNG\r\n\x1a\n" + ihdr + chunk(b"IEND", b"")


def minimal_pdf(*, width: int, height: int) -> bytes:
    """A ~600 byte one-page PDF with the given MediaBox, no content."""

    bodies = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        (
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] "
            "/Resources << >> /Contents 4 0 R >>"
        ).encode("ascii"),
        b"<< /Length 0 >>\nstream\n\nendstream",
    ]
    payload = bytearray(b"%PDF-1.7\n")
    offsets: list[int] = []
    for number, body in enumerate(bodies, start=1):
        offsets.append(len(payload))
        payload += f"{number} 0 obj\n".encode("ascii") + body + b"\nendobj\n"
    xref_offset = len(payload)
    payload += f"xref\n0 {len(bodies) + 1}\n".encode("ascii")
    payload += b"0000000000 65535 f \n"
    for offset in offsets:
        payload += f"{offset:010d} 00000 n \n".encode("ascii")
    payload += (
        f"trailer\n<< /Size {len(bodies) + 1} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n"
    ).encode("ascii")
    return bytes(payload)


@contextmanager
def running_service(tmp_path: Path) -> Iterator[tuple[TestClient, Settings]]:
    settings = Settings(
        state_dir=tmp_path / "state",
        staging_dir=tmp_path / "staging",
        max_slots=1,
    )
    settings.prepare()
    with TestClient(create_app(settings)) as client:
        yield client, settings


def wait_for_state(
    client: TestClient, execution_id: str, states: set[str], *, timeout: float
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    last: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        response = client.get(f"{EXECUTIONS_PATH}/{execution_id}")
        assert response.status_code == 200, response.text
        last = response.json()
        if last["state"] in states:
            return last
        time.sleep(0.05)
    raise AssertionError(f"execution {execution_id} never reached {states}; last {last}")
