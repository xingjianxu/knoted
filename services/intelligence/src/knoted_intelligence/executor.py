"""Execution scheduling and gated child computation.

A separate lightweight guardian exclusively signals and reaps each complete
compute subtree. The API persists launch authorization before GO, retains its
slots until the guardian receipt validates, and closes an owner pipe on shutdown.
The child loads locked parse/embed models only after its private gate opens.
A timeout, leader exit or interrupted ledger state alone never releases a slot.
"""

from __future__ import annotations

import json
import logging
import os
import signal
import subprocess
import sys
import threading
import time
import uuid
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from . import staging
from .config import Settings
from .ledger import (
    IN_FLIGHT_STATES,
    TERMINAL_STATES,
    ExecutionErrorDetail,
    ExecutionResultDetail,
    ExecutionRow,
    Ledger,
    LedgerConflict,
)
from .supervision import Supervision

logger = logging.getLogger(__name__)

QUEUED = "queued"
RUNNING = "running"
SUCCEEDED = "succeeded"
FAILED = "failed"
CANCEL_REQUESTED = "cancel_requested"
CANCELLED = "cancelled"
INTERRUPTED = "interrupted"

#: Bounded wait while polling for a free slot, so a queued cancellation is seen.
SLOT_POLL_SECONDS = 0.02
#: How long a cancel request waits for the supervisor to confirm the exit.
CANCEL_CONFIRM_SECONDS = 15.0
#: Grace period the child itself grants after SIGTERM, so an observer can see
#: ``cancel_requested`` before the row becomes ``cancelled``.
TERMINATE_NOTICE_SECONDS = 0.3

#: Name of the child's self-reported status file inside the output directory.
CHILD_STATUS_FILE = "child-status.json"

#: Execution kinds that use the Python GPU slot. Parse (OCR) and embed share one
#: card; both acquire the same semaphore before a child starts, so the two
#: model families are never resident at the same time.
GPU_KINDS = frozenset({"parse", "embed"})

_CHILD_BOOTSTRAP = (
    "import sys;"
    "from knoted_intelligence.executor import child_main;"
    "raise SystemExit(child_main(sys.argv[1]))"
)


class ExecutionFailure(Exception):
    """Base class for protocol-level refusals; the HTTP layer maps the code."""

    code = "execution_error"


class ExecutionUnavailable(ExecutionFailure):
    code = "execution_unavailable"


class ExecutionNotFound(ExecutionFailure):
    code = "execution_not_found"


class ExecutionConflict(ExecutionFailure):
    code = "execution_conflict"


class ExecutionFinished(ExecutionFailure):
    code = "execution_finished"


class ExecutionNotFinished(ExecutionFailure):
    code = "execution_not_finished"


class ExecutionNotSucceeded(ExecutionFailure):
    code = "execution_not_succeeded"


@dataclass(frozen=True, slots=True)
class ExecutionSpec:
    """The validated part of an ``ExecutionRequest`` the scheduler needs."""

    execution_id: str
    kind: str
    input_object_id: str
    input_sha256: str
    source_version: int
    options_revision: str
    deadline: datetime


@dataclass(slots=True)
class _Entry:
    """Supervisor-side handles for one in-flight execution."""

    execution_id: str
    cancel_event: threading.Event = field(default_factory=threading.Event)
    confirmed: threading.Event = field(default_factory=threading.Event)
    process: subprocess.Popen[bytes] | None = None
    thread: threading.Thread | None = None
    token: str = ""
    owner_fd: int | None = None


class Scheduler:
    """Owns the slots, the child processes and the ledger transitions."""

    def __init__(self, ledger: Ledger, settings: Settings) -> None:
        self._ledger = ledger
        self._settings = settings
        self._slots = threading.BoundedSemaphore(settings.max_slots)
        # Separate from `_slots`: `max_slots` bounds all children, while this
        # one bounds GPU-class children (parse + embed) to the configured card
        # budget. A thread always takes `_slots` first, then this one.
        self._gpu_slots = threading.BoundedSemaphore(settings.gpu_slots)
        self._lock = threading.RLock()
        self._entries: dict[str, _Entry] = {}
        self._unresolved: set[str] = set()
        self._closed = False
        self._closed_at = 0.0
        self._supervision = Supervision(ledger.path)

    def recover(self) -> None:
        # Only called after taking the instance lock. Ordinary Ledger.open is inert.
        rows = self._ledger.all_rows()
        self._unresolved.update(row.execution_id for row in rows)
        for row in rows:
            if self._supervision.phase(row.execution_id) == "queued":
                with self._supervision.connect() as db:
                    token = db.execute(
                        "SELECT token FROM supervision WHERE execution_id=?", (row.execution_id,)
                    ).fetchone()[0]
                self._supervision.complete(row.execution_id, token, "never_started", None)
        self._ledger.interrupt_unfinished(_now())
        self.reconcile()

    def reconcile(self) -> bool:
        # Historical confirmed rows were validated once at recovery. Runtime
        # polling visits only unresolved occupancy; requested rows still get
        # independent receipt validation in _require().
        uncertain = False
        with self._lock:
            for execution_id in self._unresolved.difference(self._entries):
                if self._ledger.confirm_exit(execution_id):
                    self._unresolved.discard(execution_id)
                else:
                    uncertain = True
        return uncertain

    def degraded(self) -> bool:
        uncertain = self.reconcile()
        with self._lock:
            return uncertain or any(
                entry.process is not None
                and entry.process.poll() is not None
                and not self._supervision.receipt(entry.execution_id)
                for entry in self._entries.values()
            )

    # -- public API ----------------------------------------------------------

    def submit(self, spec: ExecutionSpec) -> tuple[ExecutionRow, bool]:
        """Accepts a new execution, or returns the stored record unchanged.

        Returns ``(record, created)`` where ``created`` is ``False`` for a
        repeated id with identical parameters — the original work is never
        restarted. Different parameters raise :class:`ExecutionConflict`.
        """

        now = _now()
        with self._lock:
            existing = self._ledger.get(spec.execution_id)
            if existing is not None:
                return self._replay(existing, spec)
            if self._closed or self.degraded():
                raise ExecutionUnavailable("execution resources are not confirmed available")
            token = str(uuid.uuid4())
            try:
                row = self._ledger.insert(
                    execution_id=spec.execution_id,
                    kind=spec.kind,
                    input_object_id=spec.input_object_id,
                    input_sha256=spec.input_sha256,
                    source_version=spec.source_version,
                    options_revision=spec.options_revision,
                    deadline=spec.deadline,
                    now=now,
                    supervision_token=token,
                )
            except LedgerConflict:
                # A concurrent insert of the same id can land here; the stored
                # row decides, exactly as in the duplicate path above.
                existing = self._ledger.get(spec.execution_id)
                if existing is None:
                    raise
                return self._replay(existing, spec)
            self._unresolved.add(spec.execution_id)
            entry = _Entry(execution_id=spec.execution_id, token=token)
            self._entries[spec.execution_id] = entry
            if spec.deadline <= now:
                self._finish(
                    spec.execution_id,
                    FAILED,
                    error=_failure_error("deadline_exceeded", "execution deadline elapsed"),
                )
                self._entries.pop(spec.execution_id)
                return self._require(spec.execution_id), True
            thread = threading.Thread(
                target=self._run,
                args=(spec.execution_id, spec.kind),
                name=f"execution-{spec.execution_id}",
                daemon=True,
            )
            entry.thread = thread
            thread.start()
        return row, True

    def cancel(self, execution_id: str) -> ExecutionRow:
        """Kills the child group and returns the record once it confirmed.

        A terminal execution is a conflict; a second cancel while the first one
        is still confirming is idempotent and answers `200`. If the child
        somehow never confirms inside the budget, the record stays
        ``cancel_requested``: reporting ``cancelled`` without a confirmed exit
        would release a slot that may still be computing.
        """

        with self._lock:
            row = self._ledger.get(execution_id)
            if row is None:
                raise ExecutionNotFound(f"execution {execution_id} is unknown")
            if row.state in TERMINAL_STATES:
                raise ExecutionFinished(f"execution {execution_id} is already {row.state}")
            entry = self._entries.get(execution_id)
            self._supervision.request_stop(execution_id)
            if entry is None:
                # Recovery never equates an absent in-memory handle with exit.
                self.reconcile()
                return self._require(execution_id)
            entry.cancel_event.set()
        entry.confirmed.wait(CANCEL_CONFIRM_SECONDS)
        return self._require(execution_id)

    def result(self, execution_id: str) -> ExecutionRow:
        """Returns the record only when its artifact may be published."""

        row = self._require(execution_id)
        if row.state == SUCCEEDED and row.compute_exit_confirmed:
            return row
        if row.state in TERMINAL_STATES:
            raise ExecutionNotSucceeded(
                f"execution {execution_id} ended {row.state} and has no publishable result"
            )
        raise ExecutionNotFinished(f"execution {execution_id} is still {row.state}")

    def shutdown(self) -> bool:
        with self._lock:
            self._closed = True
            self._closed_at = time.monotonic()
            entries = list(self._entries.values())
            for entry in entries:
                entry.cancel_event.set()
                if entry.owner_fd is not None:
                    os.close(entry.owner_fd)
                    entry.owner_fd = None
        deadline = time.monotonic() + CANCEL_CONFIRM_SECONDS + 5
        for entry in entries:
            if entry.thread is not None:
                entry.thread.join(timeout=max(0, deadline - time.monotonic()))
        # If a thread is stuck, keep the DB and instance lock until process exit.
        return all(entry.thread is None or not entry.thread.is_alive() for entry in entries)

    # -- supervisor ----------------------------------------------------------

    def _replay(self, row: ExecutionRow, spec: ExecutionSpec) -> tuple[ExecutionRow, bool]:
        same = (
            row.deadline == spec.deadline
            and row.kind == spec.kind
            and row.input_object_id == spec.input_object_id
            and row.input_sha256 == spec.input_sha256
            and row.source_version == spec.source_version
            and row.options_revision == spec.options_revision
        )
        if not same:
            raise ExecutionConflict(
                f"execution {row.execution_id} already exists with "
                f"kind={row.kind} input_object_id={row.input_object_id} "
                f"input_sha256={row.input_sha256} source_version={row.source_version} "
                f"options_revision={row.options_revision}"
            )
        return self._require(row.execution_id), False

    def _run(self, execution_id: str, kind: str) -> None:
        """Waits for a general and (for GPU kinds) a GPU slot, then executes.

        Both waits poll the cancel event, so a queued cancellation never holds a
        slot or waits for a model load. Slots release only after validated exit
        evidence; a queued cancellation writes never_started before any launch.
        """

        with self._lock:
            entry = self._entries[execution_id]
        slot_held = False
        gpu_slot_held = False
        try:
            while not entry.cancel_event.is_set():
                if self._slots.acquire(timeout=SLOT_POLL_SECONDS):
                    slot_held = True
                    break
            if entry.cancel_event.is_set():
                self._finish(execution_id, CANCELLED, error=_cancel_error(None))
                return
            if kind in GPU_KINDS:
                while not entry.cancel_event.is_set():
                    if self._gpu_slots.acquire(timeout=SLOT_POLL_SECONDS):
                        gpu_slot_held = True
                        break
                if entry.cancel_event.is_set():
                    self._finish(execution_id, CANCELLED, error=_cancel_error(None))
                    return
            self._execute(execution_id, entry)
        except Exception as exc:  # pragma: no cover - defensive
            # Log the detail with the path; never copy it into the record, which
            # the worker reads. The message stays a classification.
            logger.exception("execution %s failed in the supervisor", execution_id)
            self._finish(
                execution_id,
                FAILED,
                error=_failure_error("internal_error", f"supervisor failed ({type(exc).__name__})"),
            )
        finally:
            # Unknown evidence keeps the slots reserved; recovery also blocks admission.
            proven = self._supervision.receipt(execution_id) is not None
            if proven:
                if gpu_slot_held:
                    self._gpu_slots.release()
                if slot_held:
                    self._slots.release()
                entry.confirmed.set()
            with self._lock:
                if entry.owner_fd is not None:
                    os.close(entry.owner_fd)
                    entry.owner_fd = None
                self._entries.pop(execution_id, None)

    def _execute(self, execution_id: str, entry: _Entry) -> None:
        row = self._require(execution_id)
        if row.state in TERMINAL_STATES:
            return
        if entry.cancel_event.is_set():
            self._finish(execution_id, CANCELLED, error=_cancel_error(None))
            return

        now = _now()
        if now >= row.deadline:
            self._finish(
                execution_id,
                FAILED,
                error=_failure_error(
                    "deadline_exceeded",
                    f"deadline {row.deadline.isoformat()} passed before the execution started",
                ),
            )
            return

        try:
            staged_input = staging.input_path(self._settings.input_dir, row.input_object_id)
        except staging.StagingError as exc:
            self._finish(execution_id, FAILED, error=_failure_error("input_invalid", str(exc)))
            return
        if not staged_input.is_file():
            self._finish(
                execution_id,
                FAILED,
                error=_failure_error(
                    "input_not_found",
                    f"staged input {row.input_object_id!r} does not exist",
                ),
            )
            return

        actual = staging.sha256_file(staged_input)
        if actual != row.input_sha256:
            self._finish(
                execution_id,
                FAILED,
                error=_failure_error(
                    "input_digest_mismatch",
                    f"staged input {row.input_object_id!r} has sha256 {actual}, "
                    f"request declared {row.input_sha256}",
                ),
            )
            return

        if not self._ledger.mark_started(execution_id, _now()):
            # Cancellation can land during input hashing, before intent exists.
            # Close that queued launch with never_started evidence as well.
            self._finish(execution_id, CANCELLED, error=_cancel_error(None))
            return

        try:
            output_dir = staging.output_dir(self._settings.output_dir, execution_id)
        except staging.StagingError as exc:  # pragma: no cover - execution ids are uuids
            # StagingError messages are path-free by construction.
            self._finish(execution_id, FAILED, error=_failure_error("internal_error", str(exc)))
            return
        artifact_object_id = f"{execution_id}-{row.kind}.json"
        child_spec = {
            "execution_id": execution_id,
            "kind": row.kind,
            "input_object_id": row.input_object_id,
            "input_path": str(staged_input),
            "input_sha256": row.input_sha256,
            "source_version": row.source_version,
            "options_revision": row.options_revision,
            "output_dir": str(output_dir),
            "artifact_object_id": artifact_object_id,
            "hold_ms": self._settings.child_hold_ms,
            "ignore_sigterm": self._settings.child_ignore_sigterm,
        }

        read_fd, write_fd = os.pipe()
        try:
            with self._lock:
                if self._closed or entry.cancel_event.is_set() or self.degraded():
                    self._finish(execution_id, CANCELLED, error=_cancel_error(None))
                    return
                if not self._supervision.transition(execution_id, entry.token, "queued", "intent"):
                    raise RuntimeError("launch intent CAS failed")
                process = subprocess.Popen(
                    [
                        sys.executable,
                        "-I",
                        "-B",
                        "-m",
                        "knoted_intelligence.guardian",
                        str(self._ledger.path),
                        execution_id,
                        entry.token,
                        str(read_fd),
                        str(self._settings.cancel_grace_seconds),
                        json.dumps(
                            [
                                sys.executable,
                                "-I",
                                "-B",
                                "-c",
                                _CHILD_BOOTSTRAP,
                                json.dumps(child_spec),
                            ]
                        ),
                    ],
                    pass_fds=(read_fd,),
                    start_new_session=True,
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
                entry.process = process
                entry.owner_fd = write_fd
                write_fd = -1
        finally:
            os.close(read_fd)
            if write_fd >= 0:
                os.close(write_fd)
        sent_go = False
        while True:
            receipt = self._supervision.receipt(execution_id)
            if receipt:
                break
            with self._lock:
                if entry.cancel_event.is_set():
                    self._supervision.request_stop(execution_id)
                if not sent_go and entry.owner_fd is not None and not entry.cancel_event.is_set():
                    if self._supervision.transition(execution_id, entry.token, "ready", "go"):
                        os.write(entry.owner_fd, b"G")
                        sent_go = True
            if process.poll() is not None:
                # The guardian can commit between our first read and poll().
                # Consume that receipt through the same success/error path.
                receipt = self._supervision.receipt(execution_id)
                if receipt is not None:
                    break
                self._finish(
                    execution_id,
                    FAILED,
                    error=_failure_error("exit_unconfirmed", "supervision exit is unconfirmed"),
                )
                return
            if self._closed and time.monotonic() - self._closed_at > CANCEL_CONFIRM_SECONDS:
                return  # Persistent unknown occupancy; guardian owns further cleanup.
            time.sleep(SLOT_POLL_SECONDS)
        process.wait(timeout=5)
        reason = receipt["reason"]
        if reason == "deadline_exceeded":
            self._finish(
                execution_id,
                FAILED,
                error=_failure_error("deadline_exceeded", "execution deadline elapsed"),
            )
        elif reason in {"cancelled", "owner_eof"} or entry.cancel_event.is_set():
            self._finish(
                execution_id, CANCELLED, error=_cancel_error(_describe_exit(receipt["status"]))
            )
        elif reason != "completed" or receipt["status"] != 0:
            status = _read_child_status(output_dir)
            code = status.get("error_code") or "execution_failed"
            self._finish(
                execution_id,
                FAILED,
                error=_failure_error(
                    code, _child_failure_message(_describe_exit(receipt["status"]), output_dir)
                ),
            )
        elif _now() > row.deadline:
            self._finish(
                execution_id,
                FAILED,
                error=_failure_error("deadline_exceeded", "execution deadline elapsed"),
            )
        else:
            self._register_artifact(execution_id, output_dir, artifact_object_id)

    def _register_artifact(
        self, execution_id: str, output_dir: Path, artifact_object_id: str
    ) -> None:
        """Re-reads the artifact and registers its recomputed digest and size."""

        try:
            artifact = staging.artifact_path(output_dir, artifact_object_id)
        except staging.StagingError as exc:
            self._finish(execution_id, FAILED, error=_failure_error("artifact_invalid", str(exc)))
            return
        if not artifact.is_file():
            self._finish(
                execution_id,
                FAILED,
                error=_failure_error(
                    "artifact_missing",
                    f"child exited 0 but {artifact_object_id!r} is missing",
                ),
            )
            return
        row = self._require(execution_id)
        result = ExecutionResultDetail(
            object_id=artifact_object_id,
            sha256=staging.sha256_file(artifact),
            size=artifact.stat().st_size,
            media_type="application/json",
            metadata={
                "kind": row.kind,
                "source_version": str(row.source_version),
                "input_sha256": row.input_sha256,
                "options_revision": row.options_revision,
            },
        )
        if not self._finish(execution_id, SUCCEEDED, result=result):
            logger.warning("execution %s finished but the ledger refused the result", execution_id)
            return
        logger.info(
            "execution %s succeeded: %s (%d bytes, %s)",
            execution_id,
            result.object_id,
            result.size,
            result.sha256,
        )

    def _finish(
        self,
        execution_id: str,
        state: str,
        *,
        error: ExecutionErrorDetail | None = None,
        result: ExecutionResultDetail | None = None,
    ) -> bool:
        entry = self._entries.get(execution_id)
        if entry and self._supervision.phase(execution_id) == "queued":
            self._supervision.complete(execution_id, entry.token, "never_started", None)
        proven = self._supervision.receipt(execution_id) is not None
        changed = self._ledger.finish(
            execution_id,
            state,
            _now(),
            from_states=IN_FLIGHT_STATES,
            error=error,
            result=result,
            compute_exit_confirmed=proven,
        )
        if changed and proven:
            with self._lock:
                self._unresolved.discard(execution_id)
        return changed

    def record(self, execution_id: str) -> ExecutionRow:
        self.reconcile()
        return self._require(execution_id)

    def _require(self, execution_id: str) -> ExecutionRow:
        row = self._ledger.get(execution_id)
        if row is None:  # pragma: no cover - the row is written before we get here
            raise ExecutionNotFound(f"execution {execution_id} is unknown")
        if row.compute_exit_confirmed and self._supervision.receipt(execution_id) is None:
            with self._lock:
                self._unresolved.add(execution_id)
            raise ExecutionUnavailable("persisted execution proof failed validation")
        return row


# -- helpers -----------------------------------------------------------------


def _now() -> datetime:
    return datetime.now(UTC)


def _failure_error(code: str, message: str) -> ExecutionErrorDetail:
    return ExecutionErrorDetail(code=code, message=message, request_id=str(uuid.uuid4()))


def _cancel_error(exit_status: str | None) -> ExecutionErrorDetail:
    message = (
        f"cancelled; child {exit_status}"
        if exit_status
        else "cancelled before a child process started"
    )
    return ExecutionErrorDetail(code="cancelled", message=message, request_id=str(uuid.uuid4()))


def _child_failure_message(exit_status: str, output_dir: Path) -> str:
    """Exit status plus the path-free details the child recorded, if any."""

    status = _read_child_status(output_dir)
    reason = status.get("error_class")
    detail = status.get("error_message")
    reason = reason if isinstance(reason, str) and reason else None
    detail = detail if isinstance(detail, str) and detail else None
    if reason is None and detail is None:
        return f"child {exit_status}"
    description = reason or "unknown error"
    if detail is not None:
        # Only exceptions that expose `public_message` get here; those messages
        # are built without filesystem paths (see parsing/models.py) and start
        # with the lock summary, so this cap cannot cut the versions/revisions.
        description = f"{description}: {detail[:2000]}"
    return f"child {exit_status}: compute failed ({description})"


def _read_child_status(output_dir: Path) -> dict[str, Any]:
    try:
        payload = json.loads((output_dir / CHILD_STATUS_FILE).read_text(encoding="utf-8"))
    except (OSError, ValueError):  # pragma: no cover - the child died before writing
        return {}
    return payload if isinstance(payload, dict) else {}


def _describe_exit(returncode: int | None) -> str:
    if returncode is None:
        return "with an unknown exit status"
    if returncode < 0:
        return f"terminated by signal {-returncode}"
    return f"exited with code {returncode}"


# -- child side --------------------------------------------------------------


def child_main(spec_json: str) -> int:
    """Runs one execution inside the child process. Returns the exit code."""

    spec = json.loads(spec_json)
    output_dir = Path(spec["output_dir"])
    output_dir.mkdir(parents=True, exist_ok=True)
    status_file = output_dir / CHILD_STATUS_FILE
    identity = {
        "execution_id": spec["execution_id"],
        "pid": os.getpid(),
        "pgid": os.getpgrp(),
        "started_at": _utc_iso(),
    }

    def on_signal(signum: int, _frame: Any) -> None:
        # Record the signal, then give the supervisor's observer a moment to see
        # `cancel_requested` before this process is gone. Exiting is immediate
        # afterwards: a cancelled execution produces no artifact.
        _write_json(
            status_file,
            {
                **identity,
                "phase": "terminating",
                "signal": signum,
                "terminated_at": _utc_iso(),
            },
        )
        time.sleep(TERMINATE_NOTICE_SECONDS)
        os._exit(128 + signum)

    # Install the handler *before* announcing `running`: a supervisor that reads
    # `running` and then cancels must never hit the default SIGTERM disposition,
    # otherwise the child would die silently and the cancellation test (and an
    # operator) could not tell a graceful stop from a crash.
    if spec.get("ignore_sigterm"):
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
    else:
        signal.signal(signal.SIGTERM, on_signal)
        signal.signal(signal.SIGINT, on_signal)
    _write_json(status_file, {**identity, "phase": RUNNING})

    hold_ms = int(spec.get("hold_ms") or 0)
    if hold_ms > 0:
        time.sleep(hold_ms / 1000)

    try:
        payload = _compute(spec)
        artifact = output_dir / spec["artifact_object_id"]
        _write_json(artifact, payload)
    except Exception as exc:  # noqa: BLE001 - any compute failure ends the child
        import traceback

        # Full traceback for the server log; the status file only carries the
        # exception class so no absolute path can travel back to the worker.
        traceback.print_exc()
        _write_json(
            status_file,
            {
                **identity,
                "phase": "failed",
                "error_class": type(exc).__name__,
                "error_code": getattr(exc, "code", "execution_failed"),
                # An exception opts in to a worker-facing message by exposing
                # `public_message`; everything else stays in the server log.
                "error_message": _public_error_message(exc),
                "finished_at": _utc_iso(),
            },
        )
        return 1

    _write_json(
        status_file,
        {**identity, "phase": SUCCEEDED, "finished_at": _utc_iso()},
    )
    return 0


def _compute(spec: dict[str, Any]) -> dict[str, Any]:
    """Parse and embed handlers.

    ``parse`` runs the locked Docling/RapidOCR adapter (:mod:`parsing`).
    ``embed`` runs the locked dense BGE-M3 adapter (:mod:`embedding.bge`) on the
    staged input object, which is UTF-8 text with one text per non-blank line.
    The artifact layouts and the metadata below are the part the protocol
    depends on.
    """

    from .embedding import bge
    from .parsing import models as parsing_models
    from .parsing import parse_document

    base: dict[str, Any] = {
        "kind": spec["kind"],
        "source_version": spec["source_version"],
        "input_sha256": spec["input_sha256"],
        "options_revision": spec["options_revision"],
        "child_pid": os.getpid(),
    }
    if spec["kind"] == "parse":
        from .parsing import office
        from .parsing.pdf import parse_pdf_image

        parser = (
            parse_pdf_image
            if spec["options_revision"] == parsing_models.OPTIONS_REVISION
            else parse_document
        )
        if spec["options_revision"] == office.options_revision():
            document = office.parse_office(Path(spec["input_path"]), staged=True)
            parser_metadata = office.parser_summary()
        else:
            document = parser(
                Path(spec["input_path"]),
                options_revision=spec["options_revision"],
            )
            parser_metadata = parsing_models.parser_summary()
        payload = {
            **base,
            "text_length": len(document.full_text),
            "title": document.title,
            "source_format": document.source_format,
            "page_count": document.page_count,
            # A real parse never publishes a truncated document.
            "truncated": False,
            "parser": parser_metadata,
            # The text lives in the blocks only: `ParsedDocument.full_text` is an
            # in-process convenience, and copying it here would double the
            # artifact volume for a 100 MiB input.
            "blocks": [
                {
                    "ordinal": block.ordinal,
                    "type": block.kind,
                    "text": block.text,
                    "page": block.page,
                    "bbox": list(block.bbox) if block.bbox is not None else None,
                    "structure": block.structure,
                }
                for block in document.blocks
            ],
        }
        parsing_models.check_artifact_size(len(json.dumps(payload, sort_keys=True).encode("utf-8")))
        return payload
    if spec["kind"] == "embed":
        texts = _read_embed_texts(Path(spec["input_path"]))
        vectors = bge.embed_dense(texts)
        return {
            **base,
            "model": bge.MODEL_REPO,
            "revision": bge.locked_model_revision(),
            "dims": bge.HIDDEN_SIZE,
            "normalized": bge.NORMALIZE_EMBEDDINGS,
            "dtype": bge.OUTPUT_DTYPE,
            "device": bge.loaded_device(),
            "lock": bge.lock_summary(),
            "text_count": len(texts),
            "vectors": vectors,
        }
    raise ValueError(f"unsupported execution kind {spec['kind']!r}")


def _read_embed_texts(path: Path) -> list[str]:
    """Reads the embed input object: UTF-8 text, one text per non-blank line.

    T11 defines this provisional wire format; the batch/chunk adapter (T58)
    owns the final one. Blank lines are separators, surrounding whitespace is
    trimmed, and an empty object is a stable ``empty_input`` failure instead of
    an execution that succeeds with zero vectors.
    """

    from .embedding import bge

    size = path.stat().st_size
    if size > bge.MAX_INPUT_BYTES:
        raise bge.EmbeddingError(
            "input_too_large",
            f"embed input is {size} bytes; the limit is {bge.MAX_INPUT_BYTES} bytes",
        )
    try:
        raw = path.read_text(encoding="utf-8-sig")
    except UnicodeDecodeError as exc:
        raise bge.EmbeddingError("invalid_input", "embed input is not valid UTF-8 text") from exc
    texts = [line.strip() for line in raw.splitlines()]
    texts = [line for line in texts if line]
    if not texts:
        raise bge.EmbeddingError("empty_input", "embed input contains no non-blank lines")
    if len(texts) > bge.MAX_BATCH_SIZE:
        raise bge.EmbeddingError(
            "batch_too_large",
            f"embed input has {len(texts)} lines; the limit for one execution is "
            f"{bge.MAX_BATCH_SIZE}",
        )
    return texts


def _public_error_message(exc: BaseException) -> str:
    """The message an exception allows into an execution record, or ``""``."""

    message = getattr(exc, "public_message", None)
    return message if isinstance(message, str) and message else ""


def _write_json(path: Path, payload: dict[str, Any]) -> None:
    staging.atomic_write(path, json.dumps(payload, sort_keys=True).encode("utf-8"))


def _utc_iso() -> str:
    return _now().isoformat().replace("+00:00", "Z")
