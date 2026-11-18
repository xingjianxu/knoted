"""Chunk-aware embedding on the existing staged execution protocol."""

from __future__ import annotations

import hashlib
import json
from datetime import UTC, datetime, timedelta
from uuid import uuid4

import pytest
from fastapi.testclient import TestClient

from knoted_intelligence.app import create_app
from knoted_intelligence.config import Settings
from knoted_intelligence.embedding import bge
from knoted_intelligence.executor import _compute


def chunk_spec(tmp_path, texts):
    chunks = [{"id": str(uuid4()), "text": text} for text in texts]
    manifest = {"chunk_set_id": str(uuid4()), "chunks": chunks}
    staged = tmp_path / "t58-chunk-input"
    staged.write_text(json.dumps(manifest), encoding="utf-8")
    return (
        {
            "kind": "embed",
            "source_version": 2,
            "options_revision": f"embed-chunks-v1+bge-m3-{bge.MODEL_REVISION}-dense-1024",
            "input_path": str(staged),
            "input_sha256": hashlib.sha256(staged.read_bytes()).hexdigest(),
        },
        manifest,
    )


@pytest.mark.parametrize("bad", [[1.0], [float("nan")] + [0.0] * 1023])
def test_t58_1_invalid_batch_cannot_produce_a_partial_artifact(tmp_path, monkeypatch, bad):
    spec, _ = chunk_spec(tmp_path, ["first"] * 8 + ["last"])
    count = 0

    def encode(texts):
        nonlocal count
        count += 1
        return [[1.0] + [0.0] * 1023 for _ in texts] if count == 1 else [bad]

    monkeypatch.setattr(bge, "embed_dense", encode)
    with pytest.raises(bge.EmbeddingError, match="(values|finite)"):
        _compute(spec)
    assert count == 2


def test_t58_2_staged_execution_replays_the_same_id(tmp_path):
    staging = tmp_path / "staging" / "input"
    staging.mkdir(parents=True)
    spec, _ = chunk_spec(staging, ["a\nmultiline\nchunk"])
    settings = Settings(
        state_dir=tmp_path / "state", staging_dir=staging.parent, child_hold_ms=1200
    )
    request = {key: value for key, value in spec.items() if key != "input_path"} | {
        "execution_id": str(uuid4()),
        "input_object_id": "t58-chunk-input",
        "deadline": (datetime.now(UTC) + timedelta(minutes=1)).isoformat(),
    }
    with TestClient(create_app(settings)) as client:
        first = client.post("/internal/v1/executions", json=request)
        second = client.post("/internal/v1/executions", json=request)
        assert first.status_code == 201, first.text
        assert second.status_code == 200, second.text
        assert first.json()["execution_id"] == second.json()["execution_id"]
        wrong = client.post(
            "/internal/v1/executions",
            json=request | {"options_revision": request["options_revision"] + "-new"},
        )
        assert wrong.status_code == 409, wrong.text


def test_t58_3_multiline_identity_and_locked_revision_are_preserved(tmp_path, monkeypatch):
    texts = ["第一行\n第二行"] + [f"text {i}" for i in range(8)]
    spec, manifest = chunk_spec(tmp_path, texts)
    calls = []

    def encode(batch):
        calls.append(batch)
        return [[1.0] + [0.0] * 1023 for _ in batch]

    monkeypatch.setattr(bge, "embed_dense", encode)
    monkeypatch.setattr(bge, "loaded_device", lambda: "cpu")
    output = _compute(spec)
    assert calls == [texts[:8], texts[8:]]
    assert output["chunk_set_id"] == manifest["chunk_set_id"]
    assert output["chunk_ids"] == [chunk["id"] for chunk in manifest["chunks"]]
    assert output["revision"] == bge.MODEL_REVISION
    assert len(output["vectors"]) == 9
    with pytest.raises(bge.EmbeddingError, match="revision"):
        _compute(spec | {"options_revision": spec["options_revision"] + "-unknown"})
    assert len(calls) == 2


def test_t58_real_staged_chunk_retains_identity_in_completed_artifact(tmp_path):
    from time import monotonic, sleep

    settings = Settings(state_dir=tmp_path / "state", staging_dir=tmp_path / "staging")
    settings.prepare()
    spec, manifest = chunk_spec(settings.input_dir, ["会议纪要\n包含保留的换行位置。"])
    execution_id = str(uuid4())
    request = {key: value for key, value in spec.items() if key != "input_path"} | {
        "execution_id": execution_id,
        "input_object_id": "t58-chunk-input",
        "deadline": (datetime.now(UTC) + timedelta(minutes=3)).isoformat(),
    }
    endpoint = f"/internal/v1/executions/{execution_id}"
    with TestClient(create_app(settings)) as client:
        response = client.post("/internal/v1/executions", json=request)
        assert response.status_code == 201, response.text
        cutoff = monotonic() + 80
        while monotonic() < cutoff:
            record = client.get(endpoint).json()
            if record["state"] == "succeeded":
                break
            assert record["state"] in {"queued", "running"}, record
            sleep(0.1)
        else:
            pytest.fail("embedding execution did not complete")
        result = client.get(endpoint + "/result").json()
        raw = (settings.output_dir / execution_id / result["object_id"]).read_bytes()
        assert hashlib.sha256(raw).hexdigest() == result["sha256"]
        payload = json.loads(raw)
        assert payload["chunk_ids"] == [manifest["chunks"][0]["id"]]
        assert payload["chunk_set_id"] == manifest["chunk_set_id"]
        assert payload["revision"] == bge.MODEL_REVISION
        assert len(payload["vectors"][0]) == bge.HIDDEN_SIZE
