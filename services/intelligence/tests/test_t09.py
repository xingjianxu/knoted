"""T09 acceptance tests: the internal execution protocol and staging objects.

These tests drive the real application, the real SQLite ledger and **real child
processes**: the only thing replaced is the compute handler, which is a
deliberate placeholder until T10/T11. Nothing here mocks a state transition, a
process exit or a `wait()`.

Each test starts the service with a temporary state/staging tree, so no test
touches `/data/knoted-dev/service` and no default path is written from the
repository.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import signal
import sqlite3
import threading
import time
import uuid
from collections.abc import Iterator
from contextlib import contextmanager
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import pytest
from fastapi.testclient import TestClient

from knoted_intelligence import staging
from knoted_intelligence.app import create_app
from knoted_intelligence.config import Settings
from knoted_intelligence.executor import CHILD_STATUS_FILE
from knoted_intelligence.ledger import Ledger

EXECUTIONS_PATH = "/internal/v1/executions"
INPUT_OBJECT_ID = "note-t09"
INPUT_TEXT = "Knoted T09 input\nsecond line\nthird line\n"
SHA256_PATTERN_LEN = 64
PARSE_OPTIONS = "parse-options-t09"


def digest_text(text: str = INPUT_TEXT) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


@contextmanager
def start_service(tmp_path: Path, **overrides: Any) -> Iterator[Service]:
    """Starts a fresh service on its own temporary state and staging tree."""

    settings = Settings(
        state_dir=tmp_path / "state",
        staging_dir=tmp_path / "staging",
        **overrides,
    )
    settings.prepare()
    (settings.input_dir / INPUT_OBJECT_ID).write_text(INPUT_TEXT, encoding="utf-8")
    app = create_app(settings)
    with TestClient(app) as client:
        yield Service(client, settings)


class Service:
    """Thin client around the ASGI app plus helpers to inspect its side effects."""

    def __init__(self, client: TestClient, settings: Settings) -> None:
        self.client = client
        self.settings = settings
        self.deadlines: dict[str, datetime] = {}

    # -- protocol calls ------------------------------------------------------

    def create(
        self,
        *,
        execution_id: str | None = None,
        kind: str = "parse",
        object_id: str = INPUT_OBJECT_ID,
        sha256: str | None = None,
        source_version: int = 1,
        deadline: datetime | None = None,
        options_revision: str = PARSE_OPTIONS,
    ):
        execution_id = execution_id or str(uuid.uuid4())
        if deadline is None:
            deadline = self.deadlines.setdefault(
                execution_id, datetime.now(UTC) + timedelta(minutes=5)
            )
        body = {
            "execution_id": execution_id,
            "kind": kind,
            "input_object_id": object_id,
            "input_sha256": sha256 or digest_text(),
            "source_version": source_version,
            "deadline": (deadline or datetime.now(UTC) + timedelta(minutes=5)).isoformat(),
            "options_revision": options_revision,
        }
        return self.client.post(EXECUTIONS_PATH, json=body)

    def get(self, execution_id: str):
        return self.client.get(f"{EXECUTIONS_PATH}/{execution_id}")

    def cancel(self, execution_id: str):
        return self.client.post(f"{EXECUTIONS_PATH}/{execution_id}/cancel")

    def result(self, execution_id: str):
        return self.client.get(f"{EXECUTIONS_PATH}/{execution_id}/result")

    def health(self):
        return self.client.get("/internal/v1/health")

    # -- inspection ----------------------------------------------------------

    def wait(
        self,
        execution_id: str,
        states: set[str],
        *,
        timeout: float = 20.0,
    ) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        last: dict[str, Any] | None = None
        while time.monotonic() < deadline:
            response = self.get(execution_id)
            assert response.status_code == 200, response.text
            last = response.json()
            if last["state"] in states:
                return last
            time.sleep(0.02)
        raise AssertionError(f"execution {execution_id} never reached {states}; last record {last}")

    def child_status(self, execution_id: str) -> dict[str, Any]:
        path = self.output_dir(execution_id) / CHILD_STATUS_FILE
        assert path.is_file(), f"child status {path} is missing"
        return json.loads(path.read_text(encoding="utf-8"))

    def wait_for_child(self, execution_id: str, *, timeout: float = 20.0) -> dict[str, Any]:
        path = self.output_dir(execution_id) / CHILD_STATUS_FILE
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if path.is_file():
                try:
                    status = json.loads(path.read_text(encoding="utf-8"))
                except json.JSONDecodeError:  # the child is mid-rewrite
                    time.sleep(0.02)
                    continue
                if status.get("phase") == "running":
                    return status
            time.sleep(0.02)
        raise AssertionError(f"child for {execution_id} never reported itself running")

    def output_dir(self, execution_id: str) -> Path:
        return self.settings.output_dir / execution_id

    def ledger_state(self, execution_id: str) -> str | None:
        # A separate read-only connection sees the committed transition even
        # while the request that performs it is still blocked. WAL makes this
        # safe; it is also the only way to observe `cancel_requested` without
        # racing the cancellation request itself.
        uri = f"file:{self.settings.ledger_path}?mode=ro"
        connection = sqlite3.connect(uri, uri=True, timeout=5.0)
        try:
            row = connection.execute(
                "SELECT state FROM executions WHERE execution_id = ?", (execution_id,)
            ).fetchone()
        finally:
            connection.close()
        return row[0] if row else None

    def execution_count(self) -> int:
        uri = f"file:{self.settings.ledger_path}?mode=ro"
        connection = sqlite3.connect(uri, uri=True, timeout=5.0)
        try:
            return int(connection.execute("SELECT COUNT(*) FROM executions").fetchone()[0])
        finally:
            connection.close()


# -- tests -------------------------------------------------------------------


@pytest.mark.parametrize(
    "object_id",
    [
        "../x",
        "a/b",
        "..",
        ".",
        "http://evil",
        "https://evil.example/object",
        "a\\b",
        "/absolute",
        "sub/../../escape",
        "a..b",
        "",
        # 129 bytes: one over the shared limit (see t09_10 for the parity check).
        "a" * 129,
    ],
)
def test_t09_1_path_and_url_object_ids_rejected(tmp_path: Path, object_id: str) -> None:
    with start_service(tmp_path) as service:
        response = service.create(object_id=object_id)

        assert response.status_code in (400, 422), response.text
        body = response.json()
        assert body["code"], "the rejection carries a stable code"
        assert body["request_id"], "the rejection carries a request id"
        # An error body must never describe the deployment layout.
        assert str(tmp_path) not in response.text
        assert "/staging" not in response.text
        assert service.execution_count() == 0, "a rejected id must not create an execution"
        assert not service.settings.output_dir.exists() or not any(
            service.settings.output_dir.iterdir()
        ), "a rejected id must not create staging output"


def test_t09_2_duplicate_id_does_not_restart(tmp_path: Path) -> None:
    with start_service(tmp_path) as service:
        execution_id = str(uuid.uuid4())
        first = service.create(execution_id=execution_id)
        assert first.status_code == 201, first.text
        assert first.json()["state"] in {"queued", "running"}

        finished = service.wait(execution_id, {"succeeded"})
        child_before = service.child_status(execution_id)
        artifact_path = service.output_dir(execution_id) / f"{execution_id}-parse.json"
        artifact_before = artifact_path.read_bytes()

        replay = service.create(execution_id=execution_id)
        assert replay.status_code == 200, replay.text
        record = replay.json()
        assert record["state"] == "succeeded"
        assert record["created_at"] == finished["created_at"]
        assert record["started_at"] == finished["started_at"]
        assert record["updated_at"] == finished["updated_at"]

        child_after = service.child_status(execution_id)
        assert child_after["pid"] == child_before["pid"], "the compute ran once, in one child"
        assert child_after["started_at"] == child_before["started_at"]
        artifact_after = artifact_path.read_bytes()
        assert artifact_after == artifact_before

        # Same id, different parameters: the stored execution is not what was asked.
        conflict = service.create(execution_id=execution_id, sha256="0" * SHA256_PATTERN_LEN)
        assert conflict.status_code == 409, conflict.text
        assert conflict.json()["code"] == "execution_conflict"
        assert service.execution_count() == 1


def test_t09_2_duplicate_while_running_reuses_the_running_child(tmp_path: Path) -> None:
    with start_service(tmp_path, max_slots=1, child_hold_ms=60_000) as service:
        execution_id = str(uuid.uuid4())
        assert service.create(execution_id=execution_id).status_code == 201
        service.wait(execution_id, {"running"})
        child_before = service.wait_for_child(execution_id)

        replay = service.create(execution_id=execution_id)
        assert replay.status_code == 200, replay.text
        assert replay.json()["state"] == "running"
        child_after = service.wait_for_child(execution_id)
        assert child_after["pid"] == child_before["pid"], "no second child was started"

        cancelled = service.cancel(execution_id)
        assert cancelled.status_code == 200, cancelled.text
        assert cancelled.json()["state"] == "cancelled"


def test_t09_3_restart_marks_running_interrupted(tmp_path: Path) -> None:
    with start_service(tmp_path) as service:
        settings = service.settings
        now = datetime.now(UTC)
        queued_id = str(uuid.uuid4())
        running_id = str(uuid.uuid4())
        cancelling_id = str(uuid.uuid4())

        ledger = Ledger(settings.ledger_path)
        ledger.open()
        # A queued row has no owner left after a restart either: the in-memory
        # queue is gone and the worker retries with a new execution id.
        ledger.insert(
            execution_id=queued_id,
            kind="parse",
            input_object_id=INPUT_OBJECT_ID,
            input_sha256=digest_text(),
            source_version=1,
            options_revision=PARSE_OPTIONS,
            deadline=now + timedelta(minutes=5),
            now=now,
        )
        for execution_id in (running_id, cancelling_id):
            ledger.insert(
                execution_id=execution_id,
                kind="parse",
                input_object_id=INPUT_OBJECT_ID,
                input_sha256=digest_text(),
                source_version=1,
                options_revision=PARSE_OPTIONS,
                deadline=now + timedelta(minutes=5),
                now=now,
            )
            assert ledger.mark_started(execution_id, now)
        assert ledger.mark_cancel_requested(cancelling_id, now)
        ledger.close()

        assert service.get(queued_id).json()["state"] == "queued"
        assert service.get(running_id).json()["state"] == "running"
        assert service.get(cancelling_id).json()["state"] == "cancel_requested"

    # The process that owned those rows is gone; a restart must not resume them.
    with start_service(tmp_path) as restarted:
        for execution_id in (queued_id, running_id, cancelling_id):
            record = restarted.get(execution_id).json()
            assert record["state"] == "interrupted"
            assert record["finished_at"] is not None
            assert record["error"]["code"] == "interrupted"

        # `interrupted` is a retryable terminal state: same id is never reused.
        assert restarted.cancel(running_id).status_code == 409
        result = restarted.result(running_id)
        assert result.status_code == 422
        assert result.json()["code"] == "execution_not_succeeded"
        # Legacy records have no exit receipt. Business interruption is not resource proof.
        assert all(
            restarted.get(eid).json()["compute_exit_confirmed"] is False
            for eid in (queued_id, running_id, cancelling_id)
        )
        assert restarted.health().json() == {"status": "degraded", "active_executions": 0}


def test_t09_4_cancel_kills_process_group_and_waits(tmp_path: Path) -> None:
    # Four real cancellations in one run. The child installs its SIGTERM handler
    # before it announces `running`, so `wait_for_child` only ever observes a
    # child that can already report the signal; repeating the cycle is the
    # flakiness check for that ordering.
    attempts = 4
    with start_service(tmp_path, max_slots=1, child_hold_ms=60_000) as service:
        for attempt in range(attempts):
            execution_id = str(uuid.uuid4())
            assert service.create(execution_id=execution_id).status_code == 201
            service.wait(execution_id, {"running"})

            child = service.wait_for_child(execution_id)
            pid = child["pid"]
            assert child["pgid"] == pid, "the execution child must lead its own process group"
            os.kill(pid, 0)  # still alive before the cancel

            box: dict[str, Any] = {}

            def request_cancel(execution_id: str = execution_id, box: dict = box) -> None:
                box["response"] = service.cancel(execution_id)

            thread = threading.Thread(target=request_cancel, name=f"t09-cancel-{attempt}")
            thread.start()

            observed: list[str] = []
            deadline = time.monotonic() + 20.0
            while time.monotonic() < deadline:
                state = service.ledger_state(execution_id)
                if state and (not observed or observed[-1] != state):
                    observed.append(state)
                if state == "cancelled":
                    break
                time.sleep(0.01)

            thread.join(timeout=20.0)
            assert not thread.is_alive(), "cancel must return once the exit is confirmed"

            response = box["response"]
            assert response.status_code == 200, response.text
            record = response.json()
            assert record["state"] == "cancelled"
            assert record["started_at"] is not None
            assert record["finished_at"] is not None
            assert record["error"]["code"] == "cancelled"
            assert "code 143" in record["error"]["message"], record["error"]["message"]

            assert "cancel_requested" in observed, (attempt, observed)
            assert observed[-1] == "cancelled", observed

            child_after = service.child_status(execution_id)
            assert child_after["phase"] == "terminating"
            assert child_after["signal"] == signal.SIGTERM

            # The supervisor reaped the child, so the pid and its group are gone.
            with pytest.raises(ProcessLookupError):
                os.kill(pid, 0)
            with pytest.raises(ProcessLookupError):
                os.killpg(pid, 0)

            # A cancelled execution is terminal, so a later cancel is a conflict.
            # Repeated cancels *while* a cancellation is in flight answer 200: a
            # duplicate request during the kill must not fail the caller.
            again = service.cancel(execution_id)
            assert again.status_code == 409, again.text
            assert again.json()["code"] == "execution_finished"
            assert service.result(execution_id).status_code == 422

        assert service.execution_count() == attempts


def test_t09_4b_sigkill_escalates_when_child_ignores_sigterm(tmp_path: Path) -> None:
    # A child that ignores SIGTERM is a real failure mode (a stuck parser, a
    # hung accelerator call). The supervisor must escalate to SIGKILL, still
    # wait for the exit, and only then record `cancelled`.
    with start_service(
        tmp_path,
        max_slots=1,
        child_hold_ms=60_000,
        child_ignore_sigterm=True,
        cancel_grace_seconds=0.3,
    ) as service:
        execution_id = str(uuid.uuid4())
        assert service.create(execution_id=execution_id).status_code == 201
        service.wait(execution_id, {"running"})
        child = service.wait_for_child(execution_id)
        pid = child["pid"]

        response = service.cancel(execution_id)
        assert response.status_code == 200, response.text
        record = response.json()
        assert record["state"] == "cancelled"
        assert record["error"]["code"] == "cancelled"
        assert "signal 9" in record["error"]["message"], record["error"]["message"]

        # The child never handled SIGTERM, so its status stays `running`; the
        # escalation, not a graceful stop, is what ended it.
        assert service.child_status(execution_id)["phase"] == "running"
        with pytest.raises(ProcessLookupError):
            os.kill(pid, 0)
        with pytest.raises(ProcessLookupError):
            os.killpg(pid, 0)
        assert service.result(execution_id).status_code == 422


def test_t09_4c_symlinked_input_cannot_escape_staging(tmp_path: Path) -> None:
    if not hasattr(os, "symlink"):  # pragma: no cover - Linux only in CI
        pytest.skip("symlinks are unavailable on this platform")
    with start_service(tmp_path) as service:
        link = service.settings.input_dir / "escape-link"
        os.symlink("/etc/passwd", link)
        execution_id = str(uuid.uuid4())
        created = service.create(execution_id=execution_id, object_id="escape-link")
        assert created.status_code == 201, created.text

        record = service.wait(execution_id, {"failed"})
        assert record["error"]["code"] == "input_invalid"
        assert record["started_at"] is None
        assert str(service.settings.input_dir) not in record["error"]["message"]
        assert not service.output_dir(execution_id).exists()


def test_t09_5_deadline_and_digest_mismatch_fail_without_running(tmp_path: Path) -> None:
    with start_service(tmp_path) as service:
        expired_id = str(uuid.uuid4())
        expired = service.create(
            execution_id=expired_id,
            deadline=datetime.now(UTC) - timedelta(minutes=1),
        )
        assert expired.status_code == 201, expired.text
        record = service.wait(expired_id, {"failed"})
        assert record["error"]["code"] == "deadline_exceeded"
        assert record["started_at"] is None, "an expired execution never starts"
        assert not service.output_dir(expired_id).exists()

        mismatched_id = str(uuid.uuid4())
        mismatched = service.create(
            execution_id=mismatched_id,
            sha256="1" * SHA256_PATTERN_LEN,
        )
        assert mismatched.status_code == 201, mismatched.text
        record = service.wait(mismatched_id, {"failed"})
        assert record["error"]["code"] == "input_digest_mismatch"
        assert record["started_at"] is None, "a mismatched input is never computed"
        assert not service.output_dir(mismatched_id).exists()

        assert service.result(expired_id).status_code == 422
        assert service.result(mismatched_id).status_code == 422
        assert service.execution_count() == 2


def test_t09_6_result_is_registered_and_verifiable(tmp_path: Path) -> None:
    with start_service(tmp_path, child_hold_ms=400) as service:
        execution_id = str(uuid.uuid4())
        created = service.create(execution_id=execution_id)
        assert created.status_code == 201, created.text

        pending = service.result(execution_id)
        assert pending.status_code == 409, pending.text
        assert pending.json()["code"] == "execution_not_finished"

        service.wait(execution_id, {"succeeded"})
        response = service.result(execution_id)
        assert response.status_code == 200, response.text
        result = response.json()

        artifact = service.settings.output_dir / execution_id / result["object_id"]
        payload = artifact.read_bytes()
        assert result["sha256"] == hashlib.sha256(payload).hexdigest()
        assert result["size"] == len(payload)
        assert result["media_type"] == "application/json"
        assert result["metadata"]["source_version"] == "1"
        assert result["metadata"]["input_sha256"] == digest_text()
        assert result["metadata"]["options_revision"] == PARSE_OPTIONS
        # The contract types metadata as `{[k: string]: string}`: an object with
        # string values, never a null or a nested value.
        assert isinstance(result["metadata"], dict)
        assert all(
            isinstance(key, str) and isinstance(value, str)
            for key, value in result["metadata"].items()
        )

        document = json.loads(payload)
        assert document["kind"] == "parse"
        assert document["input_sha256"] == digest_text()
        assert document["blocks"][0]["text"] == "Knoted T09 input"
        assert document["blocks"][2]["text"] == "third line"

        # The atomic write leaves no temporary file behind.
        leftovers = [path.name for path in artifact.parent.iterdir() if path.name.endswith(".part")]
        assert leftovers == []
        assert [path.name for path in artifact.parent.iterdir() if path.name.startswith(".")] == []

        # Reading the result twice returns the same registered digest.
        assert service.result(execution_id).json() == result


def test_t09_7_unknown_execution_and_health(tmp_path: Path) -> None:
    with start_service(tmp_path) as service:
        unknown = str(uuid.uuid4())
        missing = service.get(unknown)
        assert missing.status_code == 404
        assert missing.json()["code"] == "execution_not_found"
        assert service.cancel(unknown).status_code == 404
        assert service.result(unknown).status_code == 404

        health = service.health()
        assert health.status_code == 200
        assert health.json() == {"status": "ok", "active_executions": 0}

        execution_id = str(uuid.uuid4())
        assert service.create(execution_id=execution_id).status_code == 201
        service.wait(execution_id, {"succeeded"})

        finished = service.cancel(execution_id)
        assert finished.status_code == 409
        assert finished.json()["code"] == "execution_finished"
        assert service.health().json()["active_executions"] == 0


def test_t09_8_invalid_uuid_is_a_validation_error(tmp_path: Path) -> None:
    with start_service(tmp_path) as service:
        bad = "not-a-uuid"
        responses = [
            service.client.get(f"{EXECUTIONS_PATH}/{bad}"),
            service.client.post(f"{EXECUTIONS_PATH}/{bad}/cancel"),
            service.client.get(f"{EXECUTIONS_PATH}/{bad}/result"),
            service.client.post(
                EXECUTIONS_PATH,
                json={
                    "execution_id": bad,
                    "kind": "parse",
                    "input_object_id": INPUT_OBJECT_ID,
                    "input_sha256": digest_text(),
                    "source_version": 1,
                    "deadline": (datetime.now(UTC) + timedelta(minutes=5)).isoformat(),
                    "options_revision": PARSE_OPTIONS,
                },
            ),
        ]
        for response in responses:
            assert response.status_code == 422, response.text
            payload = response.json()
            assert payload["code"] == "validation_failed"
            assert payload["field_errors"], "the field that failed is named"
        assert service.execution_count() == 0


def test_t09_9_deadline_and_source_version_are_bounded(tmp_path: Path) -> None:
    with start_service(tmp_path) as service:
        # A naive timestamp has no absolute meaning; guessing UTC would move the
        # expiry of a result by the local offset.
        naive = service.create(deadline=datetime(2026, 1, 2, 3, 4, 5))
        assert naive.status_code == 422, naive.text
        assert naive.json()["code"] == "validation_failed"
        assert any("timezone" in error["message"] for error in naive.json()["field_errors"]), (
            naive.json()
        )

        # The worker stores source_version in an i64; wider values cannot round-trip.
        overflow = service.create(source_version=2**63)
        assert overflow.status_code == 422, overflow.text
        assert overflow.json()["code"] == "validation_failed"

        assert service.create(source_version=2**63 - 1).status_code == 201
        assert service.execution_count() == 1


def test_t09_10_object_id_limit_matches_rust() -> None:
    assert staging.MAX_OBJECT_ID_LEN == 128
    rust_source = Path(__file__).resolve().parents[3] / "crates/knoted-core/src/files/staging.rs"
    rust_code = rust_source.read_text(encoding="utf-8")
    declared = re.search(r"MAX_OBJECT_ID_LEN: usize = (\d+)", rust_code)
    assert declared, f"{rust_source} no longer declares MAX_OBJECT_ID_LEN"
    assert int(declared.group(1)) == staging.MAX_OBJECT_ID_LEN, (
        "the Python and Rust object-id limits diverged"
    )

    assert staging.ensure_object_id("a" * staging.MAX_OBJECT_ID_LEN)
    with pytest.raises(staging.StagingError):
        staging.ensure_object_id("a" * (staging.MAX_OBJECT_ID_LEN + 1))
