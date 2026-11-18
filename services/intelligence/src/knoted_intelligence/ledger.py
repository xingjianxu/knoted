"""Durable execution ledger (SQLite, WAL).

The ledger is the only durable state of this service: which execution was
accepted, what it was asked to do, how it ended and which artifact it produced.
It is **not** the business database — PostgreSQL stays the source of truth for
jobs — and it never stores the input or output content itself, only the object
ids and digests the worker verifies.

Every transition is a conditional ``UPDATE ... WHERE state = ...`` so a late
worker cannot overwrite a cancellation that arrived while it was computing.
Recovery runs only after the API acquires its exclusive instance lock. Old
nonterminal rows become interrupted, but resource release requires a validated
guardian receipt; opening SQLite alone never asserts that a process exited.
"""

from __future__ import annotations

import json
import logging
import sqlite3
import threading
from collections.abc import Iterable
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

logger = logging.getLogger(__name__)

#: States in which the service still owns the execution.
IN_FLIGHT_STATES: tuple[str, ...] = ("queued", "running", "cancel_requested")
#: States a restart must turn into ``interrupted``.
#:
#: ``queued`` is included on purpose: the in-memory queue does not survive the
#: process, and the worker creates a new ``execution_id`` to retry, so a queued
#: row left behind is dead rather than pending.
RESTART_STATES: tuple[str, ...] = ("queued", "running", "cancel_requested")
#: States no transition may leave.
TERMINAL_STATES: tuple[str, ...] = ("succeeded", "failed", "cancelled", "interrupted")

_SCHEMA = """
CREATE TABLE IF NOT EXISTS executions (
    execution_id       TEXT PRIMARY KEY,
    kind               TEXT NOT NULL,
    input_object_id    TEXT NOT NULL,
    input_sha256       TEXT NOT NULL,
    source_version     INTEGER NOT NULL,
    options_revision   TEXT NOT NULL,
    deadline           TEXT NOT NULL,
    state              TEXT NOT NULL,
    error_code         TEXT,
    error_message      TEXT,
    error_request_id   TEXT,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL,
    started_at         TEXT,
    finished_at        TEXT,
    result_object_id   TEXT,
    result_sha256      TEXT,
    result_size        INTEGER,
    result_media_type  TEXT,
    result_metadata    TEXT
);
CREATE INDEX IF NOT EXISTS executions_state_idx ON executions (state);
"""


def to_storage(moment: datetime) -> str:
    """Serialises an instant as an RFC 3339 UTC string with a ``Z`` suffix."""

    return moment.astimezone(UTC).isoformat().replace("+00:00", "Z")


def from_storage(value: str | None) -> datetime | None:
    """Parses a value written by :func:`to_storage`."""

    if value is None:
        return None
    return datetime.fromisoformat(value)


@dataclass(frozen=True, slots=True)
class ExecutionErrorDetail:
    """Diagnostic stored for a non-success terminal state."""

    code: str
    message: str
    request_id: str


@dataclass(frozen=True, slots=True)
class ExecutionResultDetail:
    """Artifact registered for a succeeded execution."""

    object_id: str
    sha256: str
    size: int
    media_type: str
    metadata: dict[str, str]


@dataclass(frozen=True, slots=True)
class ExecutionRow:
    """One ledger row, already parsed into Python values."""

    execution_id: str
    kind: str
    input_object_id: str
    input_sha256: str
    source_version: int
    options_revision: str
    deadline: datetime
    state: str
    created_at: datetime
    updated_at: datetime
    started_at: datetime | None
    finished_at: datetime | None
    error_code: str | None
    error_message: str | None
    error_request_id: str | None
    result_object_id: str | None
    result_sha256: str | None
    result_size: int | None
    result_media_type: str | None
    result_metadata: dict[str, str] | None
    compute_exit_confirmed: bool


class LedgerConflict(Exception):
    """The ``execution_id`` was inserted concurrently."""


class Ledger:
    """Thread-safe access to the execution ledger.

    One connection guarded by a re-entrant lock: the statements are short and
    the service is a single process, so a pool would add machinery without
    buying anything. SQLite still serialises writes for any other reader, and
    WAL keeps reads from blocking.
    """

    def __init__(self, path: Path) -> None:
        self._path = path
        self._lock = threading.RLock()
        self._connection: sqlite3.Connection | None = None

    @property
    def path(self) -> Path:
        return self._path

    def open(self) -> None:
        """Opens the database, applies the schema and recovers stale rows."""

        self._path.parent.mkdir(parents=True, exist_ok=True)
        with self._lock:
            if self._connection is not None:
                return
            connection = sqlite3.connect(
                self._path,
                check_same_thread=False,
                isolation_level=None,  # autocommit; single statements are atomic
                timeout=5.0,
            )
            connection.row_factory = sqlite3.Row
            connection.execute("PRAGMA journal_mode=WAL")
            connection.execute("PRAGMA synchronous=FULL")
            connection.execute("PRAGMA busy_timeout=5000")
            connection.executescript(_SCHEMA)
            columns = {row[1] for row in connection.execute("PRAGMA table_info(executions)")}
            if "compute_exit_confirmed" not in columns:
                connection.execute(
                    "ALTER TABLE executions ADD COLUMN "
                    "compute_exit_confirmed INTEGER NOT NULL DEFAULT 0"
                )
            from .supervision import SCHEMA

            connection.executescript(SCHEMA)
            self._connection = connection

    def close(self) -> None:
        with self._lock:
            if self._connection is not None:
                self._connection.close()
                self._connection = None

    # -- reads ---------------------------------------------------------------

    def get(self, execution_id: str) -> ExecutionRow | None:
        with self._lock:
            row = (
                self._connection_or_raise()
                .execute("SELECT * FROM executions WHERE execution_id = ?", (execution_id,))
                .fetchone()
            )
        return _row(row) if row is not None else None

    def active_count(self) -> int:
        return self._count_states(IN_FLIGHT_STATES)

    def stuck_cancel_count(self) -> int:
        """Executions whose child has not confirmed the cancellation yet."""

        return self._count_states(("cancel_requested",))

    def _count_states(self, states: Iterable[str]) -> int:
        placeholders = ", ".join("?" for _ in states)
        with self._lock:
            row = (
                self._connection_or_raise()
                .execute(
                    f"SELECT COUNT(*) AS total FROM executions WHERE state IN ({placeholders})",
                    tuple(states),
                )
                .fetchone()
            )
        return int(row["total"])

    # -- writes --------------------------------------------------------------

    def insert(
        self,
        *,
        execution_id: str,
        kind: str,
        input_object_id: str,
        input_sha256: str,
        source_version: int,
        options_revision: str,
        deadline: datetime,
        now: datetime,
    ) -> ExecutionRow:
        """Inserts a ``queued`` row; raises :class:`LedgerConflict` if it exists."""

        with self._lock:
            try:
                self._connection_or_raise().execute(
                    """
                    INSERT INTO executions (
                        execution_id, kind, input_object_id, input_sha256,
                        source_version, options_revision, deadline, state,
                        created_at, updated_at
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?)
                    """,
                    (
                        execution_id,
                        kind,
                        input_object_id,
                        input_sha256,
                        source_version,
                        options_revision,
                        to_storage(deadline),
                        to_storage(now),
                        to_storage(now),
                    ),
                )
            except sqlite3.IntegrityError as exc:
                raise LedgerConflict(execution_id) from exc
            row = (
                self._connection_or_raise()
                .execute("SELECT * FROM executions WHERE execution_id = ?", (execution_id,))
                .fetchone()
            )
        if row is None:  # pragma: no cover - the insert above just wrote it
            raise RuntimeError(f"execution {execution_id} disappeared after insert")
        return _row(row)

    def mark_started(self, execution_id: str, now: datetime) -> bool:
        """Moves ``queued`` to ``running``; ``False`` if something else moved it."""

        with self._lock:
            cursor = self._connection_or_raise().execute(
                """
                UPDATE executions
                   SET state = 'running', started_at = ?, updated_at = ?
                 WHERE execution_id = ? AND state = 'queued'
                """,
                (to_storage(now), to_storage(now), execution_id),
            )
            return cursor.rowcount == 1

    def mark_cancel_requested(self, execution_id: str, now: datetime) -> bool:
        with self._lock:
            cursor = self._connection_or_raise().execute(
                """
                UPDATE executions
                   SET state = 'cancel_requested', updated_at = ?
                 WHERE execution_id = ? AND state = 'running'
                """,
                (to_storage(now), execution_id),
            )
            return cursor.rowcount == 1

    def finish(
        self,
        execution_id: str,
        state: str,
        now: datetime,
        *,
        from_states: Iterable[str] = IN_FLIGHT_STATES,
        error: ExecutionErrorDetail | None = None,
        result: ExecutionResultDetail | None = None,
        compute_exit_confirmed: bool = False,
    ) -> bool:
        """Writes a terminal state, but only if the row is still in ``from_states``."""

        states = tuple(from_states)
        if state not in TERMINAL_STATES:
            raise ValueError(f"{state!r} is not a terminal state")
        placeholders = ", ".join("?" for _ in states)
        metadata = json.dumps(result.metadata, sort_keys=True) if result is not None else None
        with self._lock:
            cursor = self._connection_or_raise().execute(
                f"""
                UPDATE executions
                   SET state = ?, updated_at = ?, finished_at = ?,
                       error_code = ?, error_message = ?, error_request_id = ?,
                       result_object_id = ?, result_sha256 = ?, result_size = ?,
                       result_media_type = ?, result_metadata = ?, compute_exit_confirmed = ?
                 WHERE execution_id = ? AND state IN ({placeholders})
                """,
                (
                    state,
                    to_storage(now),
                    to_storage(now),
                    error.code if error else None,
                    error.message if error else None,
                    error.request_id if error else None,
                    result.object_id if result else None,
                    result.sha256 if result else None,
                    result.size if result else None,
                    result.media_type if result else None,
                    metadata,
                    int(compute_exit_confirmed),
                    execution_id,
                    *states,
                ),
            )
            return cursor.rowcount == 1

    def all_rows(self) -> list[ExecutionRow]:
        with self._lock:
            return [
                _row(row) for row in self._connection_or_raise().execute("SELECT * FROM executions")
            ]

    def confirm_exit(self, execution_id: str) -> None:
        from .supervision import validated_receipt

        with self._lock:
            db = self._connection_or_raise()
            if validated_receipt(db, execution_id):
                db.execute(
                    "UPDATE executions SET compute_exit_confirmed=1 WHERE execution_id=? "
                    "AND state IN ('succeeded','failed','cancelled','interrupted')",
                    (execution_id,),
                )

    def interrupt_unfinished(self, now: datetime) -> int:
        """Turns rows a dead process left in flight into ``interrupted``."""

        placeholders = ", ".join("?" for _ in RESTART_STATES)
        with self._lock:
            cursor = self._connection_or_raise().execute(
                f"""
                UPDATE executions
                   SET state = 'interrupted', updated_at = ?, finished_at = ?,
                       error_code = 'interrupted',
                       error_message = ?,
                       error_request_id = ?
                 WHERE state IN ({placeholders})
                """,
                (
                    to_storage(now),
                    to_storage(now),
                    "the intelligence service restarted while the execution was in flight",
                    "00000000-0000-4000-8000-000000000000",
                    *RESTART_STATES,
                ),
            )
            return cursor.rowcount

    def _connection_or_raise(self) -> sqlite3.Connection:
        if self._connection is None:
            raise RuntimeError("ledger is not open")
        return self._connection


def _row(row: sqlite3.Row) -> ExecutionRow:
    metadata = json.loads(row["result_metadata"]) if row["result_metadata"] else None
    return ExecutionRow(
        execution_id=row["execution_id"],
        kind=row["kind"],
        input_object_id=row["input_object_id"],
        input_sha256=row["input_sha256"],
        source_version=row["source_version"],
        options_revision=row["options_revision"],
        deadline=from_storage(row["deadline"]),
        state=row["state"],
        created_at=from_storage(row["created_at"]),
        updated_at=from_storage(row["updated_at"]),
        started_at=from_storage(row["started_at"]),
        finished_at=from_storage(row["finished_at"]),
        error_code=row["error_code"],
        error_message=row["error_message"],
        error_request_id=row["error_request_id"],
        result_object_id=row["result_object_id"],
        result_sha256=row["result_sha256"],
        result_size=row["result_size"],
        result_media_type=row["result_media_type"],
        result_metadata=metadata,
        compute_exit_confirmed=bool(row["compute_exit_confirmed"]),
    )
