"""Private durable supervision evidence, and read-only operational inspection.

All receipts live in executions.db (SQLite FULL/WAL), not staging. A receipt
is not a PID observation: only the exclusive guardian writes tree_reaped after
ECHILD. The API can write never_started only before launching any guardian.
"""

from __future__ import annotations

import fcntl
import hashlib
import json
import os
import shutil
import sqlite3
import sys
import tempfile
import uuid
from contextlib import closing, contextmanager
from pathlib import Path

VERSION = 1
REQUEST_FIELDS = (
    "execution_id",
    "kind",
    "input_object_id",
    "input_sha256",
    "source_version",
    "options_revision",
    "deadline",
)
SCHEMA = """
CREATE TABLE IF NOT EXISTS supervision (
 execution_id TEXT PRIMARY KEY REFERENCES executions(execution_id),
 token TEXT NOT NULL,
 request_hash TEXT NOT NULL,
 phase TEXT NOT NULL,
 guardian_pid INTEGER,
 guardian_start TEXT,
 receipt TEXT
);
"""


def request_hash(row) -> str:
    return hashlib.sha256(
        json.dumps({key: row[key] for key in REQUEST_FIELDS}, sort_keys=True).encode()
    ).hexdigest()


def register_queued(db, row, token: str) -> None:
    """Join the caller's transaction; never commit execution separately."""
    db.execute(
        "INSERT INTO supervision(execution_id, token, request_hash, phase) VALUES(?,?,?,'queued')",
        (row["execution_id"], token, request_hash(row)),
    )


class InstanceLock:
    def __init__(self, state_dir: Path):
        self.path = state_dir / "service.lock"
        self.fd = None

    def acquire(self):
        self.fd = os.open(self.path, os.O_CREAT | os.O_RDWR | os.O_CLOEXEC, 0o600)
        try:
            fcntl.flock(self.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            os.close(self.fd)
            self.fd = None
            raise RuntimeError("intelligence state directory is already owned") from exc

    def close(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None


class Supervision:
    def __init__(self, path: Path):
        self.path = Path(path)

    @contextmanager
    def connect(self):
        db = sqlite3.connect(self.path, timeout=5)
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA synchronous=FULL")
        try:
            yield db
            db.commit()
        except BaseException:
            db.rollback()
            raise
        finally:
            db.close()

    def reserve(self, execution_id: str) -> str:
        token = str(uuid.uuid4())
        with self.connect() as db:
            db.executescript(SCHEMA)
            row = db.execute(
                "SELECT * FROM executions WHERE execution_id=?", (execution_id,)
            ).fetchone()
            register_queued(db, row, token)
        return token

    def phase(self, execution_id: str):
        with self.connect() as db:
            row = db.execute(
                "SELECT phase FROM supervision WHERE execution_id=?", (execution_id,)
            ).fetchone()
        return row[0] if row else None

    def transition(self, execution_id: str, token: str, before: str, after: str) -> bool:
        allowed = {("queued", "intent"), ("intent", "ready"), ("ready", "go")}
        if (before, after) not in allowed:
            raise ValueError("invalid launch transition")
        with self.connect() as db:
            return (
                db.execute(
                    "UPDATE supervision SET phase=? WHERE execution_id=? AND token=? AND phase=?",
                    (after, execution_id, token, before),
                ).rowcount
                == 1
            )

    def ready(self, execution_id: str, token: str) -> bool:
        # Diagnostic only; never used to signal a process after restart.
        start = Path("/proc/self/stat").read_text().rsplit(")", 1)[1].split()[19]
        with self.connect() as db:
            return (
                db.execute(
                    "UPDATE supervision SET phase='ready', guardian_pid=?, guardian_start=? "
                    "WHERE execution_id=? AND token=? AND phase='intent'",
                    (os.getpid(), start, execution_id, token),
                ).rowcount
                == 1
            )

    def complete(self, execution_id: str, token: str, reason: str, status: int | None) -> bool:
        with self.connect() as db:
            row = db.execute(
                "SELECT * FROM supervision WHERE execution_id=? AND token=?", (execution_id, token)
            ).fetchone()
            if row is None or row["phase"] == "exited":
                return False
            receipt = json.dumps(
                dict(
                    version=VERSION,
                    execution_id=execution_id,
                    token=token,
                    request_hash=row["request_hash"],
                    tree_reaped=True,
                    reason=reason,
                    status=status,
                ),
                sort_keys=True,
            )
            return (
                db.execute(
                    "UPDATE supervision SET phase='exited', receipt=? WHERE execution_id=? "
                    "AND token=? AND phase=?",
                    (receipt, execution_id, token, row["phase"]),
                ).rowcount
                == 1
            )

    def receipt(self, execution_id: str):
        with self.connect() as db:
            return validated_receipt(db, execution_id)

    def request_stop(self, execution_id: str):
        with self.connect() as db:
            db.execute(
                "UPDATE executions SET state='cancel_requested' WHERE execution_id=? "
                "AND state IN ('queued','running')",
                (execution_id,),
            )

    def row(self, execution_id: str):
        with self.connect() as db:
            return db.execute(
                "SELECT * FROM executions WHERE execution_id=?", (execution_id,)
            ).fetchone()


def validated_receipt(db, execution_id):
    try:
        launch = db.execute(
            "SELECT * FROM supervision WHERE execution_id=?", (execution_id,)
        ).fetchone()
        row = db.execute(
            "SELECT * FROM executions WHERE execution_id=?", (execution_id,)
        ).fetchone()
        if launch is None or row is None or launch["phase"] != "exited":
            return None
        receipt = json.loads(launch["receipt"])
        if (
            type(receipt.get("version")) is not int
            or receipt.get("version") != VERSION
            or receipt.get("tree_reaped") is not True
            or receipt.get("execution_id") != execution_id
            or receipt.get("token") != launch["token"]
            or receipt.get("request_hash") != launch["request_hash"]
            or launch["request_hash"] != request_hash(row)
            or set(receipt)
            != {
                "version",
                "execution_id",
                "token",
                "request_hash",
                "tree_reaped",
                "reason",
                "status",
            }
            or (receipt.get("status") is not None and type(receipt["status"]) is not int)
            or receipt.get("reason")
            not in {
                "never_started",
                "completed",
                "cancelled",
                "owner_eof",
                "deadline_exceeded",
                "guardian_error",
            }
        ):
            return None
        return receipt
    except (sqlite3.Error, ValueError, TypeError, AttributeError):
        return None


def _inspect_snapshot(state_dir: Path):
    path = state_dir / "executions.db"
    if not path.is_file():
        return {"clean": False, "status": "absent", "unconfirmed": None}
    try:
        with closing(sqlite3.connect(f"file:{path}?mode=ro", uri=True)) as db:
            db.row_factory = sqlite3.Row
            healthy = db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
            rows = db.execute("SELECT execution_id FROM executions").fetchall()
            unconfirmed = sum(validated_receipt(db, row[0]) is None for row in rows)
            orphans = db.execute(
                "SELECT count(*) FROM supervision s LEFT JOIN executions e "
                "ON e.execution_id=s.execution_id WHERE e.execution_id IS NULL"
            ).fetchone()[0]
            guardians = [
                dict(row)
                for row in db.execute(
                    "SELECT execution_id, guardian_pid, guardian_start, phase FROM supervision"
                )
            ]
            return {
                "clean": healthy and unconfirmed == 0 and orphans == 0,
                "status": "ok" if healthy else "corrupt",
                "unconfirmed": unconfirmed,
                "orphan_receipts": orphans,
                "executions": len(rows),
                "guardians": guardians,
            }
    except sqlite3.Error:
        return {"clean": False, "status": "legacy_or_corrupt", "unconfirmed": None}


def inspect(state_dir: Path):
    """Stopped-service inspection. Source DB/WAL/SHM and lock are never written.

    Copies a stable cold view to private TMPDIR before SQLite can create any
    sidecars. Works on native backup() copies, which may retain WAL mode.
    """
    lock_fd = None
    try:
        lock_path = state_dir / "service.lock"
        if lock_path.exists():
            lock_fd = os.open(lock_path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
            try:
                fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                return {"clean": False, "status": "service_running", "unconfirmed": None}
        names = ("executions.db", "executions.db-wal", "executions.db-shm")

        def snapshot():
            result = {}
            for name in names:
                path = state_dir / name
                if path.is_symlink():
                    raise ValueError("state links are not accepted")
                if path.exists():
                    st = path.stat()
                    if not path.is_file():
                        raise ValueError("state is not regular")
                    result[name] = (
                        st.st_dev,
                        st.st_ino,
                        st.st_size,
                        st.st_mtime_ns,
                        st.st_ctime_ns,
                    )
            return result

        before = snapshot()
        if "executions.db" not in before:
            return {"clean": False, "status": "absent", "unconfirmed": None}
        with tempfile.TemporaryDirectory(prefix="knoted-proof-") as temporary:
            target = Path(temporary)
            for name in before:
                shutil.copyfile(state_dir / name, target / name)
            if snapshot() != before:
                return {"clean": False, "status": "state_changed", "unconfirmed": None}
            return _inspect_snapshot(target)
    except (OSError, ValueError):
        return {"clean": False, "status": "unreadable", "unconfirmed": None}
    finally:
        if lock_fd is not None:
            os.close(lock_fd)


if __name__ == "__main__":
    report = inspect(Path(sys.argv[1]))
    print(json.dumps(report, sort_keys=True))
    raise SystemExit(0 if report["clean"] else 1)
