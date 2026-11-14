"""T11 acceptance tests: the real BGE-M3 dense embedding path.

These tests load the model that the host task provisioned under
``KNOTED_INTELLIGENCE_MODELS_DIR`` (default ``/data/knoted-dev/service/models``)
and run it: no model is mocked, no result is faked, and nothing is skipped when
the snapshot is missing — a missing or incomplete snapshot fails with the
``fetch-model.sh --check`` command from ``embedding/LOCK.md``.

``t11_2`` is the one exception by design: it injects **fabricated outputs** into
the validator to prove NaN/Inf/wrong-dimension refusals. That is a validation
test, not a claim about what the model returns; the real model output is only
observed in ``t11_1``/``t11_3``.

No test here asserts throughput, latency, GPU memory or retrieval quality.
"""

from __future__ import annotations

import hashlib
import json
import math
import os
import subprocess
import sys
import textwrap
import time
import uuid
from collections.abc import Iterator
from contextlib import contextmanager
from datetime import UTC, datetime, timedelta
from importlib.metadata import version as distribution_version
from pathlib import Path
from typing import Any

import numpy as np
import pytest
from fastapi.testclient import TestClient

from knoted_intelligence.app import create_app
from knoted_intelligence.config import Settings
from knoted_intelligence.embedding import bge

FIXTURE_DIR = Path(__file__).resolve().parents[3] / "tests/fixtures/embedding"
SENTENCES = FIXTURE_DIR / "sentences.json"
OCR_SAMPLE = Path(__file__).resolve().parents[3] / "tests/fixtures/ocr/sample_zh_en.png"
LOCK = Path(bge.__file__).with_name("LOCK.md")
EXECUTIONS_PATH = "/internal/v1/executions"
INPUT_OBJECT_ID = "t11-embed-input"


def require_model() -> Path:
    """Fails with the provisioning command when the locked snapshot is absent."""

    root = bge.model_dir()
    if not root.is_dir():
        pytest.fail(
            f"the BGE-M3 snapshot is not provisioned at {root}; run "
            f"{root}/fetch-model.sh --check (see embedding/LOCK.md) after installing it"
        )
    problems = bge.artifact_problems()
    if problems:
        pytest.fail(
            "the locked BGE-M3 snapshot is incomplete:\n  "
            + "\n  ".join(problems)
            + f"\nRe-run {root}/fetch-model.sh (see embedding/LOCK.md)."
        )
    return root


def fixture_texts() -> list[str]:
    payload = json.loads(SENTENCES.read_text(encoding="utf-8"))
    assert payload["schema"] == "knoted-embedding-fixture/1"
    return [item["text"] for item in payload["texts"]]


def cosine(left: list[float], right: list[float]) -> float:
    return math.fsum(a * b for a, b in zip(left, right, strict=True)) / (
        math.sqrt(math.fsum(a * a for a in left)) * math.sqrt(math.fsum(b * b for b in right))
    )


# -- t11_1 -------------------------------------------------------------------


def test_t11_1_dimension_and_normalization() -> None:
    require_model()
    texts = fixture_texts()
    assert 3 <= len(texts) <= 5, "the fixture is a small sample, not a corpus"

    vectors = bge.embed_dense(texts)

    assert len(vectors) == len(texts)
    assert all(len(vector) == bge.HIDDEN_SIZE for vector in vectors), "every row is 1024 wide"
    assert all(isinstance(value, float) for vector in vectors for value in vector)

    for row, vector in enumerate(vectors):
        norm = math.sqrt(math.fsum(value * value for value in vector))
        assert abs(norm - 1.0) <= bge.NORMALIZATION_TOLERANCE, (row, norm)
        assert all(math.isfinite(value) for value in vector), row

    # The model really loaded, on the device the environment selected, and the
    # loaded contract is the locked one (not a fallback model).
    assert bge.loaded_device() == bge.configured_device()
    contract = bge.loaded_contract()
    assert contract["hidden_size"] == bge.HIDDEN_SIZE
    assert contract["dtype"] == bge.OUTPUT_DTYPE
    assert contract["revision"] == bge.MODEL_REVISION

    # Same text twice in one batch: the outputs must agree, not merely correlate.
    repeated = bge.embed_dense([texts[0], texts[0]])
    difference = max(abs(a - b) for a, b in zip(repeated[0], repeated[1], strict=True))
    assert difference <= 1e-5, difference

    # Semantic smoke test (fixture README): the two related Chinese sentences
    # must be closer than the unrelated English one. No absolute value is
    # asserted, so a model revision update does not re-calibrate the test.
    related = cosine(vectors[0], vectors[1])
    unrelated = cosine(vectors[0], vectors[2])
    assert related > unrelated, (related, unrelated)


def test_t11_1b_embed_execution_uses_the_locked_model(tmp_path: Path) -> None:
    """The T09 placeholder is really replaced: a child process embeds the fixture."""

    require_model()
    texts = fixture_texts()
    with running_service(tmp_path) as (client, settings):
        staged = settings.input_dir / INPUT_OBJECT_ID
        staged.write_text("\n".join(texts) + "\n", encoding="utf-8")
        execution_id = str(uuid.uuid4())
        created = client.post(
            EXECUTIONS_PATH,
            json={
                "execution_id": execution_id,
                "kind": "embed",
                "input_object_id": INPUT_OBJECT_ID,
                "input_sha256": hashlib.sha256(staged.read_bytes()).hexdigest(),
                "source_version": 7,
                "deadline": (datetime.now(UTC) + timedelta(minutes=5)).isoformat(),
                "options_revision": bge.OPTIONS_REVISION,
            },
        )
        assert created.status_code == 201, created.text
        record = wait_for_state(client, execution_id, {"succeeded", "failed"}, timeout=300.0)
        assert record["state"] == "succeeded", record
        result = client.get(f"{EXECUTIONS_PATH}/{execution_id}/result")
        assert result.status_code == 200, result.text
        artifact_info = result.json()
        artifact = settings.output_dir / execution_id / artifact_info["object_id"]
        payload = artifact.read_bytes()

    assert hashlib.sha256(payload).hexdigest() == artifact_info["sha256"]
    assert artifact_info["size"] == len(payload)
    assert artifact_info["metadata"]["kind"] == "embed"

    body = json.loads(payload)
    assert body["kind"] == "embed"
    assert body["source_version"] == 7
    assert body["model"] == bge.MODEL_REPO
    assert body["revision"] == bge.MODEL_REVISION
    assert body["dims"] == bge.HIDDEN_SIZE
    assert body["normalized"] is True
    assert body["dtype"] == bge.OUTPUT_DTYPE
    assert body["device"] == bge.configured_device()
    assert body["lock"]["model_revision"] == bge.MODEL_REVISION
    assert body["options_revision"] == bge.OPTIONS_REVISION
    assert len(body["vectors"]) == len(texts)
    assert [len(row) for row in body["vectors"]] == [bge.HIDDEN_SIZE] * len(texts)
    for row in body["vectors"]:
        norm = math.sqrt(math.fsum(value * value for value in row))
        assert abs(norm - 1.0) <= bge.NORMALIZATION_TOLERANCE, norm


# -- t11_2 -------------------------------------------------------------------


class _FakeTokenizer:
    """Counts tokens the way the real tokenizer call is shaped, nothing more."""

    def __init__(self, tokens: int) -> None:
        self.tokens = tokens

    def __call__(self, texts: list[str], **_kwargs: Any) -> dict[str, list[list[int]]]:
        return {"input_ids": [[0] * self.tokens for _ in texts]}


class _FakeModel:
    """A model stand-in used only to exercise the length and output validators."""

    def __init__(self, tokens: int, output: Any) -> None:
        self.tokenizer = _FakeTokenizer(tokens)
        self.output = output
        self.encode_calls = 0

    def encode(self, texts: list[str], **_kwargs: Any) -> Any:
        self.encode_calls += 1
        return self.output


def test_t11_2_rejects_nan_inf_and_wrong_dimension(monkeypatch: pytest.MonkeyPatch) -> None:
    unit = [0.0] * (bge.HIDDEN_SIZE - 1) + [1.0]
    assert bge.validate_embeddings([unit], expected_count=1) == [unit]

    def refuses(matrix: Any, code: str, *, expected_count: int = 1) -> None:
        with pytest.raises(bge.EmbeddingError) as raised:
            bge.validate_embeddings(matrix, expected_count=expected_count)
        assert raised.value.code == code, (code, raised.value.code, str(raised.value))
        # The lock travels with the failure, like the parser's ParseError.
        assert str(raised.value).startswith("[locked: "), str(raised.value)

    # Fabricated outputs: this proves the validator, not what the model returns.
    for bad in (math.nan, math.inf, -math.inf):
        refuses([[bad] + [0.0] * (bge.HIDDEN_SIZE - 1)], "non_finite_output")
    refuses([[0.0] * (bge.HIDDEN_SIZE - 1)], "invalid_dimension")
    refuses([[0.0] * (bge.HIDDEN_SIZE + 1)], "invalid_dimension")
    refuses([0.0] * bge.HIDDEN_SIZE, "invalid_dimension", expected_count=1)
    refuses([unit, unit], "output_count_mismatch", expected_count=1)
    refuses([[0.5] * bge.HIDDEN_SIZE], "not_normalized")
    refuses([[True] + [0.0] * (bge.HIDDEN_SIZE - 1)], "invalid_output")
    refuses([[label] + [0.0] * (bge.HIDDEN_SIZE - 1) for label in ("x",)], "invalid_output")
    float64 = np.zeros((1, bge.HIDDEN_SIZE), dtype=np.float64)
    float64[0, 0] = 1.0
    refuses(float64, "invalid_output")

    # Empty batch: an explicit empty result, and the model is never touched.
    def no_model() -> Any:
        raise AssertionError("an empty batch must not load the model")

    monkeypatch.setattr(bge, "_get_model", no_model)
    assert bge.embed_dense([]) == []
    # A blank text is a caller error, refused before the model is loaded.
    for blank in ("", "   ", "\n\t"):
        with pytest.raises(bge.EmbeddingError) as blank_error:
            bge.embed_dense([blank])
        assert blank_error.value.code == "empty_text", blank_error.value

    # Overlong input: the tokenizer count is checked before encoding, so an
    # 8193-token text never reaches the GPU (or the encode call at all).
    overlong = _FakeModel(tokens=bge.MAX_SEQ_LENGTH + 1, output=[[unit]])
    monkeypatch.setattr(bge, "_get_model", lambda: overlong)
    with pytest.raises(bge.EmbeddingError) as too_long:
        bge.embed_dense(["a" * 10])
    assert too_long.value.code == "text_too_long", too_long.value
    assert overlong.encode_calls == 0

    # The boundary itself (exactly 8192 tokens) is accepted.
    at_limit = _FakeModel(tokens=bge.MAX_SEQ_LENGTH, output=np.asarray([unit], dtype="float32"))
    monkeypatch.setattr(bge, "_get_model", lambda: at_limit)
    assert bge.embed_dense(["bounded"]) == [unit]
    assert at_limit.encode_calls == 1


# -- t11_3 -------------------------------------------------------------------


def test_t11_3_model_revision_recorded(monkeypatch: pytest.MonkeyPatch) -> None:
    root = require_model()
    lock = LOCK.read_text(encoding="utf-8")
    summary = bge.lock_summary()

    # The human-readable lock, the constants and the machine-readable manifest
    # all carry the same revision; the local snapshot has no .git to read it from.
    locked = f"{bge.MODEL_REPO}@{bge.MODEL_REVISION}"
    assert locked in lock, locked
    assert bge.MODEL_REVISION == bge.locked_model_revision()
    assert summary["model_revision"] == bge.MODEL_REVISION
    assert summary["options_revision"] == bge.OPTIONS_REVISION
    assert bge.OPTIONS_REVISION in lock

    manifest = bge.manifest()
    assert manifest["schema"] == bge.MANIFEST_SCHEMA
    assert manifest["repo"] == bge.MODEL_REPO
    assert manifest["revision"] == bge.MODEL_REVISION
    assert manifest["runtime_contract"] == {
        "embedding_dim": bge.HIDDEN_SIZE,
        "normalize_embeddings": bge.NORMALIZE_EMBEDDINGS,
        "max_seq_length": bge.MAX_SEQ_LENGTH,
        "dtype": bge.OUTPUT_DTYPE,
    }

    # Every locked file, size and digest is in both the manifest and LOCK.md.
    manifest_files = {entry["path"]: entry for entry in manifest["files"]}
    for relative, size, sha256 in bge.EXPECTED_FILES:
        entry = manifest_files[relative]
        assert (entry["size"], entry["sha256"]) == (size, sha256), relative
        candidate = root / relative
        assert candidate.stat().st_size == size, relative
        assert relative in lock and sha256 in lock, relative

    # The tokenizer revision lives in the same snapshot; there is no second
    # download that could drift.
    assert bge.TOKENIZER_REVISION == bge.MODEL_REVISION
    assert bge.TOKENIZER_REVISION in lock

    # GPU selection is explicit: UUID through the environment, never index 0/1.
    assert bge.GPU_UUID_ENV in lock
    assert bge.CUDA_VISIBLE_DEVICES_ENV in lock
    assert bge.configured_device() == (
        "cuda" if os.environ.get(bge.GPU_UUID_ENV, "").strip() else "cpu"
    )
    if os.environ.get(bge.GPU_UUID_ENV, "").strip():
        assert os.environ[bge.CUDA_VISIBLE_DEVICES_ENV] == os.environ[bge.GPU_UUID_ENV].strip()

    # An index is not a UUID: the service refuses it instead of picking a card.
    for index in ("0", "1"):
        monkeypatch.setenv(bge.GPU_UUID_ENV, index)
        with pytest.raises(bge.EmbeddingError) as invalid_uuid:
            bge.configured_device()
        assert invalid_uuid.value.code == "invalid_gpu_uuid", invalid_uuid.value
        assert bge.CUDA_VISIBLE_DEVICES_ENV in str(invalid_uuid.value)
    monkeypatch.undo()

    # The lock also names what is deliberately unused.
    assert "sparse" in lock.lower() and "colbert" in lock.lower()

    # The actually loaded model agrees with the lock, and the installed runtime
    # is the pinned one.
    contract = bge.loaded_contract()
    assert contract["revision"] == bge.MODEL_REVISION
    assert contract["tokenizer_revision"] == bge.TOKENIZER_REVISION
    assert contract["hidden_size"] == bge.HIDDEN_SIZE
    assert contract["max_seq_length"] == bge.MAX_SEQ_LENGTH
    assert contract["device"] == bge.configured_device()

    assert distribution_version("sentence-transformers") == bge.SENTENCE_TRANSFORMERS_VERSION
    assert distribution_version("transformers") == bge.TRANSFORMERS_VERSION
    assert distribution_version("safetensors") == bge.SAFETENSORS_VERSION
    installed_torch = distribution_version("torch")
    assert installed_torch.split("+")[0] == bge.TORCH_VERSION, installed_torch
    assert contract["versions"]["torch"] == installed_torch


# -- t11_4 -------------------------------------------------------------------


def test_t11_4_offline_load_without_network() -> None:
    """A fresh interpreter cannot open a socket and still loads the local model."""

    require_model()
    script = textwrap.dedent(
        """
        import json
        import math
        import os
        import socket

        attempts = []

        def refuse(*args, **kwargs):
            attempts.append(args)
            raise AssertionError("the embedder attempted a network connection")

        socket.socket.connect = refuse
        socket.create_connection = refuse

        from knoted_intelligence.embedding import bge

        for key in ("HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE", "HF_HUB_DISABLE_TELEMETRY"):
            assert os.environ.get(key) == "1", (key, os.environ.get(key))

        vectors = bge.embed_dense(
            ["离线加载 smoke test", "second text for the offline batch"]
        )
        assert attempts == [], attempts
        assert len(vectors) == 2
        assert all(len(row) == bge.HIDDEN_SIZE for row in vectors)
        norms = [math.sqrt(math.fsum(value * value for value in row)) for row in vectors]
        assert all(abs(norm - 1.0) <= bge.NORMALIZATION_TOLERANCE for norm in norms), norms
        contract = bge.loaded_contract()
        print(json.dumps({
            "device": contract["device"],
            "revision": contract["revision"],
            "hidden_size": contract["hidden_size"],
            "norms": norms,
            "sockets": len(attempts),
        }))
        """
    )
    environment = dict(os.environ)
    # A cold hub cache proves the load comes from the snapshot, not a cached copy.
    cache = Path(environment.get("TMPDIR", "/tmp")) / f"t11-hf-cache-{uuid.uuid4().hex}"
    environment["HF_HOME"] = str(cache)
    environment["SENTENCE_TRANSFORMERS_HOME"] = str(cache)
    result = subprocess.run(
        [sys.executable, "-c", script],
        capture_output=True,
        text=True,
        env=environment,
        timeout=300,
        check=False,
    )
    assert result.returncode == 0, result.stderr[-4000:]
    report = json.loads(result.stdout.strip().splitlines()[-1])
    assert report["revision"] == bge.MODEL_REVISION
    assert report["hidden_size"] == bge.HIDDEN_SIZE
    assert report["sockets"] == 0
    assert not cache.exists() or not any(cache.rglob("*")), (
        "the model must not be copied to a hub cache"
    )


# -- t11_5 -------------------------------------------------------------------


def test_t11_5_gpu_slots_serialize_parse_and_embed(tmp_path: Path) -> None:
    """One GPU slot: a parse and an embed execution never run together.

    The parser is pinned to the CPU here, but the deployment rule is about the
    Python *slot*: OCR and embeddings share card index 1 and must not resident
    both models at once. ``KNOTED_INTELLIGENCE_GPU_SLOTS`` defaults to 1 and is
    acquired by every parse/embed execution before its child starts. The two
    executions are submitted back-to-back while the first is still running; the
    ledger's ``started_at``/``finished_at`` intervals must therefore not
    overlap. Without the shared slot they start within milliseconds and overlap.
    """

    require_model()
    assert OCR_SAMPLE.is_file(), OCR_SAMPLE
    texts = fixture_texts()[:2]

    with running_service(tmp_path, max_slots=2, child_hold_ms=1_500) as (client, settings):
        assert settings.gpu_slots == 1
        parse_object = "t11-5-parse"
        embed_object = "t11-5-embed"
        (settings.input_dir / parse_object).write_bytes(OCR_SAMPLE.read_bytes())
        embed_input = settings.input_dir / embed_object
        embed_input.write_text("\n".join(texts) + "\n", encoding="utf-8")

        parse_id = str(uuid.uuid4())
        created_parse = client.post(
            EXECUTIONS_PATH,
            json={
                "execution_id": parse_id,
                "kind": "parse",
                "input_object_id": parse_object,
                "input_sha256": hashlib.sha256(OCR_SAMPLE.read_bytes()).hexdigest(),
                "source_version": 1,
                "deadline": (datetime.now(UTC) + timedelta(minutes=10)).isoformat(),
                "options_revision": "parse-t11-5",
            },
        )
        assert created_parse.status_code == 201, created_parse.text

        embed_id = str(uuid.uuid4())
        created_embed = client.post(
            EXECUTIONS_PATH,
            json={
                "execution_id": embed_id,
                "kind": "embed",
                "input_object_id": embed_object,
                "input_sha256": hashlib.sha256(embed_input.read_bytes()).hexdigest(),
                "source_version": 1,
                "deadline": (datetime.now(UTC) + timedelta(minutes=10)).isoformat(),
                "options_revision": bge.OPTIONS_REVISION,
            },
        )
        assert created_embed.status_code == 201, created_embed.text

        parse_record = wait_for_state(client, parse_id, {"succeeded", "failed"}, timeout=300.0)
        embed_record = wait_for_state(client, embed_id, {"succeeded", "failed"}, timeout=300.0)

    assert parse_record["state"] == "succeeded", parse_record
    assert embed_record["state"] == "succeeded", embed_record

    def interval(record: dict[str, Any]) -> tuple[datetime, datetime, datetime]:
        created = datetime.fromisoformat(record["created_at"])
        started = datetime.fromisoformat(record["started_at"])
        finished = datetime.fromisoformat(record["finished_at"])
        assert started <= finished, record
        return created, started, finished

    parse_created, parse_started, parse_finished = interval(parse_record)
    embed_created, embed_started, embed_finished = interval(embed_record)

    # Each request was accepted while the other was still in flight, so an
    # overlap would be observed, not merely possible.
    assert embed_created < parse_finished, (embed_created, parse_finished)
    assert parse_created < embed_finished, (parse_created, embed_finished)

    overlap = parse_started <= embed_finished and embed_started <= parse_finished
    assert not overlap, (
        "parse and embed overlapped while the GPU slot is 1: "
        f"parse={parse_started.isoformat()}..{parse_finished.isoformat()} "
        f"embed={embed_started.isoformat()}..{embed_finished.isoformat()}"
    )


# -- helpers -----------------------------------------------------------------


@contextmanager
def running_service(tmp_path: Path, **overrides: Any) -> Iterator[tuple[TestClient, Settings]]:
    options: dict[str, Any] = {"max_slots": 1, **overrides}
    settings = Settings(
        state_dir=tmp_path / "state",
        staging_dir=tmp_path / "staging",
        **options,
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
