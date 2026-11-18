"""T47 review regressions: exact lifecycle interleavings and bounded read work."""

import hashlib
import sqlite3
import subprocess
import sys
import time
from datetime import UTC, datetime, timedelta
from uuid import uuid4

import pytest

from knoted_intelligence.config import Settings
from knoted_intelligence.executor import ExecutionSpec, Scheduler
from knoted_intelligence.ledger import Ledger


def setup(tmp_path):
    settings = Settings(
        state_dir=tmp_path / "state", staging_dir=tmp_path / "staging", child_hold_ms=200
    )
    settings.prepare()
    ledger = Ledger(settings.ledger_path)
    ledger.open()
    return settings, ledger, Scheduler(ledger, settings)


def spec(expired=False):
    return ExecutionSpec(
        str(uuid4()),
        "parse",
        "source",
        hashlib.sha256(b"hello").hexdigest(),
        1,
        "test",
        datetime.now(UTC) + timedelta(seconds=-1 if expired else 60),
    )


def test_t47_receipt_committed_between_read_and_exit_poll_keeps_success(tmp_path, monkeypatch):
    settings, ledger, scheduler = setup(tmp_path)
    (settings.input_dir / "source").write_bytes(b"hello")
    request = spec()
    receipt = scheduler._supervision.receipt
    interleaved = []

    def stale_read(execution_id):
        value = receipt(execution_id)
        entry = scheduler._entries.get(execution_id)
        if (
            value is None
            and not interleaved
            and entry is not None
            and entry.process is not None
            and scheduler._supervision.phase(execution_id) == "go"
        ):
            interleaved.append(True)
            # Preserve this real pre-commit read, but allow the real guardian to
            # commit its receipt and exit before returning to the API poll().
            assert entry.process.wait(timeout=45) == 0
            assert receipt(execution_id)["reason"] == "completed"
        return value

    monkeypatch.setattr(scheduler._supervision, "receipt", stale_read)
    try:
        scheduler.submit(request)
        deadline = time.monotonic() + 50
        while time.monotonic() < deadline:
            row = ledger.get(request.execution_id)
            if row.state in {"succeeded", "failed"}:
                break
            time.sleep(0.02)
        assert interleaved
        assert row.state == "succeeded" and row.compute_exit_confirmed, row
        assert row.error_code is None and row.result_object_id is not None
        replay, created = scheduler.submit(request)
        assert not created and replay.state == "succeeded"
    finally:
        scheduler.shutdown()
        ledger.close()


def test_t47_runtime_reads_do_not_rescan_confirmed_history(tmp_path, monkeypatch):
    settings, ledger, scheduler = setup(tmp_path)
    statements = []
    connect = sqlite3.connect

    def traced(*args, **kwargs):
        db = connect(*args, **kwargs)
        db.set_trace_callback(statements.append)
        return db

    monkeypatch.setattr(sqlite3, "connect", traced)
    ledger._connection_or_raise().set_trace_callback(statements.append)
    try:
        first = spec(expired=True)
        scheduler.submit(first)

        def read_work():
            statements.clear()
            assert scheduler.record(first.execution_id).compute_exit_confirmed
            assert not scheduler.degraded()
            return list(statements)

        small = read_work()
        for _ in range(80):
            scheduler.submit(spec(expired=True))
        large = read_work()
        assert len(large) <= len(small) + 2, (len(small), len(large))
        assert not any(s.lstrip().upper().startswith("UPDATE") for s in large)
        assert not any(s.strip().upper() == "SELECT * FROM EXECUTIONS" for s in large)
        # Recovery still validates the complete history, including a previously
        # confirmed row which was not the one being requested above.
        with connect(settings.ledger_path) as db:
            db.execute(
                "UPDATE supervision SET receipt='{}' WHERE execution_id != ?", (first.execution_id,)
            )
        restarted = Scheduler(ledger, settings)
        restarted.recover()
        assert restarted.degraded()
    finally:
        scheduler.shutdown()
        ledger.close()


@pytest.mark.parametrize("window", ["before_supervision", "after_insert"])
def test_t47_initial_registration_crash_is_atomic(tmp_path, window):
    # Exit the actual owner process at deterministic SQLite/ledger boundaries;
    # no guardian can yet have been launched in either window.
    code = r"""
import os, sqlite3, sys
from datetime import UTC, datetime, timedelta
from knoted_intelligence.config import Settings
from knoted_intelligence.ledger import Ledger
from knoted_intelligence.executor import Scheduler, ExecutionSpec
from pathlib import Path
settings=Settings(state_dir=Path(sys.argv[1]), staging_dir=Path(sys.argv[2]))
settings.prepare()
original_connect=sqlite3.connect
def connect(*args, **kwargs):
    db=original_connect(*args, **kwargs)
    def trace(sql):
        if sys.argv[3] == 'before_supervision' and 'INSERT INTO supervision' in sql:
            os._exit(73)
    db.set_trace_callback(trace)
    return db
sqlite3.connect=connect
ledger=Ledger(settings.ledger_path);ledger.open()
original_insert=ledger.insert
def insert(**kwargs):
    result=original_insert(**kwargs)
    if sys.argv[3] == 'after_insert':os._exit(73)
    return result
ledger.insert=insert
Scheduler(ledger,settings).submit(ExecutionSpec('11111111-1111-4111-8111-111111111111',
    'parse','source','0'*64,1,'test',datetime.now(UTC)+timedelta(seconds=60)))
raise AssertionError('controlled crash did not run')
"""
    result = subprocess.run(
        [
            sys.executable,
            "-I",
            "-B",
            "-c",
            code,
            str(tmp_path / "state"),
            str(tmp_path / "staging"),
            window,
        ],
        timeout=15,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 73, result.stderr
    settings, ledger, scheduler = setup(tmp_path)
    try:
        with sqlite3.connect(settings.ledger_path) as db:
            executions = db.execute("SELECT count(*) FROM executions").fetchone()[0]
            launches = db.execute("SELECT count(*) FROM supervision").fetchone()[0]
        assert (executions, launches) == ((0, 0) if window == "before_supervision" else (1, 1))
        scheduler.recover()
        assert not scheduler.degraded()
        if executions:
            row = ledger.get("11111111-1111-4111-8111-111111111111")
            assert row.state == "interrupted" and row.compute_exit_confirmed
    finally:
        scheduler.shutdown()
        ledger.close()
