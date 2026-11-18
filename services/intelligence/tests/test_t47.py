"""PDF/image admission, real offline OCR and execution exit confirmation."""

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


def test_t47_wire_strict_exit_proof():
    import json

    from pydantic import ValidationError

    from knoted_intelligence.app import ExecutionRecord

    root = Path(__file__).resolve().parents[3] / "api/examples"
    value = json.loads((root / "valid/execution-record-exit-confirmed.json").read_text())
    legacy = dict(value)
    legacy.pop("compute_exit_confirmed")
    assert ExecutionRecord.model_validate(legacy).compute_exit_confirmed is False
    for state in ("succeeded", "failed", "cancelled", "interrupted"):
        for proof in (True, False):
            record = ExecutionRecord.model_validate(
                dict(value, state=state, compute_exit_confirmed=proof)
            )
            assert record.compute_exit_confirmed is proof
    for invalid in (1, "true", None):
        with pytest.raises(ValidationError):
            ExecutionRecord.model_validate(dict(value, compute_exit_confirmed=invalid))
    for state in ("queued", "running", "cancel_requested"):
        with pytest.raises(ValidationError):
            ExecutionRecord.model_validate(dict(value, state=state))


def test_t47_plain_ledger_open_does_not_invent_recovery(tmp_path):
    from datetime import UTC, datetime, timedelta

    from knoted_intelligence.ledger import Ledger

    path = tmp_path / "executions.db"
    first = Ledger(path)
    first.open()
    first.insert(
        execution_id="legacy",
        kind="parse",
        input_object_id="input",
        input_sha256="0" * 64,
        source_version=1,
        options_revision="v1",
        deadline=datetime.now(UTC) + timedelta(seconds=60),
        now=datetime.now(UTC),
    )
    first.mark_started("legacy", datetime.now(UTC))
    second = Ledger(path)
    second.open()
    try:
        assert second.get("legacy").state == "running"
        assert second.get("legacy").compute_exit_confirmed is False
    finally:
        second.close()
        first.close()


def test_t47_expired_first_materialize_is_durable_and_deadline_immutable(tmp_path):
    import hashlib
    from datetime import UTC, datetime, timedelta
    from uuid import uuid4

    from fastapi.testclient import TestClient

    from knoted_intelligence.app import create_app
    from knoted_intelligence.config import Settings

    settings = Settings(state_dir=tmp_path / "state", staging_dir=tmp_path / "staging")
    request = dict(
        execution_id=str(uuid4()),
        kind="parse",
        input_object_id="missing",
        input_sha256=hashlib.sha256(b"missing").hexdigest(),
        source_version=1,
        options_revision="v1",
        deadline=(datetime.now(UTC) - timedelta(seconds=1)).isoformat(),
    )
    with TestClient(create_app(settings)) as client:
        result = client.post("/internal/v1/executions", json=request)
        assert result.status_code == 201
        assert result.json()["state"] == "failed"
        assert result.json()["compute_exit_confirmed"] is True
        assert client.post("/internal/v1/executions", json=request).status_code == 200
        changed = dict(request, deadline=(datetime.now(UTC) + timedelta(seconds=60)).isoformat())
        assert client.post("/internal/v1/executions", json=changed).status_code == 409
    with TestClient(create_app(settings)) as client:
        replay = client.post("/internal/v1/executions", json=request)
        assert replay.json()["compute_exit_confirmed"] is True


def test_t47_single_instance_lock_and_legacy_fail_closed(tmp_path):
    from fastapi.testclient import TestClient

    from knoted_intelligence.app import create_app
    from knoted_intelligence.config import Settings

    settings = Settings(state_dir=tmp_path / "state", staging_dir=tmp_path / "staging")
    with TestClient(create_app(settings)):
        with pytest.raises(RuntimeError, match="already owned"):
            with TestClient(create_app(settings)):
                pass


def test_t47_2_deadline_reaps_compute_without_partial_publication(tmp_path):
    import hashlib
    import time
    from datetime import UTC, datetime, timedelta
    from uuid import uuid4

    from fastapi.testclient import TestClient

    from knoted_intelligence.app import create_app
    from knoted_intelligence.config import Settings
    from knoted_intelligence.supervision import inspect

    settings = Settings(
        state_dir=tmp_path / "state",
        staging_dir=tmp_path / "staging",
        child_hold_ms=60000,
        child_ignore_sigterm=True,
        cancel_grace_seconds=0.1,
    )
    settings.prepare()
    (settings.input_dir / "input").write_bytes(b"never published")
    execution_id = str(uuid4())
    with TestClient(create_app(settings)) as client:
        response = client.post(
            "/internal/v1/executions",
            json=dict(
                execution_id=execution_id,
                kind="parse",
                input_object_id="input",
                input_sha256=hashlib.sha256(b"never published").hexdigest(),
                source_version=1,
                options_revision="test",
                deadline=(datetime.now(UTC) + timedelta(seconds=0.5)).isoformat(),
            ),
        )
        assert response.status_code == 201
        end = time.monotonic() + 15
        while time.monotonic() < end:
            row = client.get(f"/internal/v1/executions/{execution_id}").json()
            if row["compute_exit_confirmed"]:
                break
            time.sleep(0.02)
        assert row["compute_exit_confirmed"] is True
        assert row["state"] == "failed"
        assert row["error"]["code"] == "deadline_exceeded"
        assert client.get(f"/internal/v1/executions/{execution_id}/result").status_code == 422
    assert inspect(settings.state_dir)["clean"] is True
    assert not list(settings.output_dir.rglob("*-parse.json"))


def test_t47_corrupt_receipt_never_echoes_previously_confirmed_true(tmp_path):
    import sqlite3
    from datetime import UTC, datetime, timedelta
    from uuid import uuid4

    from fastapi.testclient import TestClient

    from knoted_intelligence.app import create_app
    from knoted_intelligence.config import Settings

    settings = Settings(state_dir=tmp_path / "state", staging_dir=tmp_path / "staging")
    request = dict(
        execution_id=str(uuid4()),
        kind="parse",
        input_object_id="absent",
        input_sha256="0" * 64,
        source_version=1,
        options_revision="v1",
        deadline=(datetime.now(UTC) - timedelta(seconds=1)).isoformat(),
    )
    with TestClient(create_app(settings)) as client:
        assert client.post("/internal/v1/executions", json=request).json()["compute_exit_confirmed"]
        with sqlite3.connect(settings.state_dir / "executions.db") as db:
            db.execute("UPDATE supervision SET receipt='{}'")
        assert client.get(f"/internal/v1/executions/{request['execution_id']}").status_code == 503
        assert client.post("/internal/v1/executions", json=request).status_code == 503
        assert client.get("/internal/v1/health").json()["status"] == "degraded"


def test_t47_cancel_between_input_check_and_started_has_never_started_proof(tmp_path, monkeypatch):
    import hashlib
    import time
    from datetime import UTC, datetime, timedelta
    from uuid import uuid4

    from knoted_intelligence.config import Settings
    from knoted_intelligence.executor import ExecutionSpec, Scheduler
    from knoted_intelligence.ledger import Ledger

    settings = Settings(state_dir=tmp_path / "state", staging_dir=tmp_path / "staging")
    settings.prepare()
    ledger = Ledger(settings.state_dir / "executions.db")
    ledger.open()
    scheduler = Scheduler(ledger, settings)
    (settings.input_dir / "source").write_bytes(b"hello")
    original = ledger.mark_started

    def cancel_first(execution_id, now):
        scheduler._supervision.request_stop(execution_id)
        return original(execution_id, now)

    monkeypatch.setattr(ledger, "mark_started", cancel_first)
    execution_id = str(uuid4())
    scheduler.submit(
        ExecutionSpec(
            execution_id,
            "parse",
            "source",
            hashlib.sha256(b"hello").hexdigest(),
            1,
            "test",
            datetime.now(UTC) + timedelta(seconds=30),
        )
    )
    try:
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            row = scheduler.record(execution_id)
            if row.compute_exit_confirmed:
                break
            time.sleep(0.02)
        assert row.state == "cancelled" and row.compute_exit_confirmed
        assert scheduler._supervision.receipt(execution_id)["reason"] == "never_started"
    finally:
        scheduler.shutdown()
        ledger.close()
