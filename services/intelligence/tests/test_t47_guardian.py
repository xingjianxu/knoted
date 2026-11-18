"""Owned real Linux processes; never signals an application or a numeric old PID."""

import json
import os
import signal
import subprocess
import sys
import time
from datetime import UTC, datetime, timedelta
from pathlib import Path
from uuid import uuid4

import pytest

from knoted_intelligence.ledger import Ledger
from knoted_intelligence.supervision import Supervision


def wait_for(check, seconds=10):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.02)
    raise AssertionError("owned process did not reach expected condition")


def setup_launch(tmp_path):
    ledger = Ledger(tmp_path / "executions.db")
    ledger.open()
    execution_id = str(uuid4())
    ledger.insert(
        execution_id=execution_id,
        kind="parse",
        input_object_id="input",
        input_sha256="0" * 64,
        source_version=1,
        options_revision="test",
        deadline=datetime.now(UTC) + timedelta(seconds=30),
        now=datetime.now(UTC),
    )
    store = Supervision(ledger.path)
    token = store.reserve(execution_id)
    assert store.transition(execution_id, token, "queued", "intent")
    return ledger, store, execution_id, token


@pytest.mark.parametrize("owner_eof", [False, True])
def test_t47_guardian_reaps_grandchild_and_preserves_sentinel(tmp_path, owner_eof):
    ledger, store, execution_id, token = setup_launch(tmp_path)
    marker = tmp_path / "grandchild"
    code = """
import os, signal, time, sys
pid = os.fork()
if pid == 0:
    os.setsid()
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    with open(sys.argv[1] + '.tmp', 'w') as marker:
        marker.write(str(os.getpid()))
    os.replace(sys.argv[1] + '.tmp', sys.argv[1])
    while True: time.sleep(1)
from pathlib import Path
while not Path(sys.argv[1]).exists(): time.sleep(.01)
if sys.argv[2] == 'hold':
    while True: time.sleep(1)
"""
    read_fd, write_fd = os.pipe()
    sentinel = subprocess.Popen([sys.executable, "-I", "-B", "-c", "import time;time.sleep(60)"])
    guardian = subprocess.Popen(
        [
            sys.executable,
            "-I",
            "-B",
            "-m",
            "knoted_intelligence.guardian",
            str(ledger.path),
            execution_id,
            token,
            str(read_fd),
            "0.1",
            json.dumps(
                [
                    sys.executable,
                    "-I",
                    "-B",
                    "-c",
                    code,
                    str(marker),
                    "hold" if owner_eof else "exit",
                ]
            ),
        ],
        pass_fds=(read_fd,),
        start_new_session=True,
    )
    os.close(read_fd)
    try:
        wait_for(lambda: store.phase(execution_id) == "ready")
        assert store.transition(execution_id, token, "ready", "go")
        os.write(write_fd, b"G")
        wait_for(marker.exists)
        child_pid = int(marker.read_text())
        if owner_eof:
            os.close(write_fd)
            write_fd = -1
        assert guardian.wait(timeout=10) == 0
        receipt = store.receipt(execution_id)
        assert receipt and receipt["tree_reaped"] is True
        assert receipt["reason"] == ("owner_eof" if owner_eof else "completed")
        assert not Path(f"/proc/{child_pid}").exists()
        assert sentinel.poll() is None
    finally:
        if write_fd >= 0:
            os.close(write_fd)
        if guardian.poll() is None:
            guardian.wait(timeout=10)
        sentinel.terminate()
        sentinel.wait()
        ledger.close()


def test_t47_receipt_corruption_or_wrong_owner_fails_closed(tmp_path):
    ledger, store, execution_id, token = setup_launch(tmp_path)
    assert not store.complete(execution_id, "wrong", "never_started", None)
    assert not store.receipt(execution_id)
    assert store.complete(execution_id, token, "never_started", None)
    assert store.receipt(execution_id)
    import sqlite3

    with sqlite3.connect(ledger.path) as db:
        db.execute("UPDATE supervision SET receipt = '{}' WHERE execution_id=?", (execution_id,))
    assert not store.receipt(execution_id)
    ledger.close()


def test_t47_guardian_death_before_go_preserves_unknown(tmp_path):
    ledger, store, execution_id, token = setup_launch(tmp_path)
    read_fd, write_fd = os.pipe()
    guardian = subprocess.Popen(
        [
            sys.executable,
            "-I",
            "-B",
            "-m",
            "knoted_intelligence.guardian",
            str(ledger.path),
            execution_id,
            token,
            str(read_fd),
            "0.1",
            json.dumps([sys.executable, "-I", "-B", "-c", "raise AssertionError('must not run')"]),
        ],
        pass_fds=(read_fd,),
        start_new_session=True,
    )
    os.close(read_fd)
    try:
        wait_for(lambda: store.phase(execution_id) == "ready")
        guardian.kill()
        assert guardian.wait(timeout=5) == -signal.SIGKILL
        assert store.receipt(execution_id) is None
        from knoted_intelligence.supervision import inspect

        assert inspect(tmp_path)["clean"] is False
    finally:
        os.close(write_fd)
        if guardian.poll() is None:
            guardian.kill()
            guardian.wait()
        ledger.close()


def test_t47_operational_inspection_rejects_orphan_receipts_and_checks_backup(tmp_path):
    import sqlite3

    from knoted_intelligence.supervision import inspect

    ledger, store, execution_id, token = setup_launch(tmp_path)
    assert store.complete(execution_id, token, "never_started", None)
    backup_root = tmp_path / "backup"
    backup_root.mkdir()
    with (
        sqlite3.connect(ledger.path) as source,
        sqlite3.connect(backup_root / "executions.db") as backup,
    ):
        source.backup(backup)
    assert inspect(backup_root)["clean"]
    with sqlite3.connect(backup_root / "executions.db") as backup:
        backup.execute("DELETE FROM executions")
    assert not inspect(backup_root)["clean"]
    ledger.close()


def test_t47_guardian_death_after_go_never_invents_exit(tmp_path):
    import ctypes

    libc = ctypes.CDLL(None)
    assert libc.prctl(36, 1, 0, 0, 0) == 0
    ledger, store, execution_id, token = setup_launch(tmp_path)
    marker = tmp_path / "compute-pid"
    code = "import os,time,sys;open(sys.argv[1],'w').write(str(os.getpid()));time.sleep(60)"
    read_fd, write_fd = os.pipe()
    guardian = subprocess.Popen(
        [
            sys.executable,
            "-I",
            "-B",
            "-m",
            "knoted_intelligence.guardian",
            str(ledger.path),
            execution_id,
            token,
            str(read_fd),
            "0.1",
            json.dumps([sys.executable, "-I", "-B", "-c", code, str(marker)]),
        ],
        pass_fds=(read_fd,),
        start_new_session=True,
    )
    os.close(read_fd)
    compute_fd = None
    compute_pid = None
    try:
        wait_for(lambda: store.phase(execution_id) == "ready")
        assert store.transition(execution_id, token, "ready", "go")
        os.write(write_fd, b"G")
        wait_for(marker.exists)
        compute_pid = int(marker.read_text())
        compute_fd = os.pidfd_open(compute_pid)
        guardian.kill()
        guardian.wait(timeout=5)
        signal.pidfd_send_signal(compute_fd, 0)
        assert store.receipt(execution_id) is None
        from knoted_intelligence.supervision import inspect

        assert not inspect(tmp_path)["clean"]
    finally:
        os.close(write_fd)
        if guardian.poll() is None:
            guardian.kill()
            guardian.wait()
        if compute_fd is not None:
            signal.pidfd_send_signal(compute_fd, signal.SIGKILL)
            os.waitpid(compute_pid, 0)
            os.close(compute_fd)
        ledger.close()
        libc.prctl(36, 0, 0, 0, 0)


@pytest.mark.parametrize("receipt_failure", [False, True])
def test_t47_ready_owner_eof_never_starts_compute_and_receipt_failure_blocks(
    tmp_path, receipt_failure
):
    ledger, store, execution_id, token = setup_launch(tmp_path)
    marker = tmp_path / "must-not-execute"
    if receipt_failure:
        with store.connect() as db:
            db.execute(
                "CREATE TRIGGER reject_receipt BEFORE UPDATE OF receipt ON supervision "
                "BEGIN SELECT RAISE(FAIL, 'test receipt unavailable'); END"
            )
    read_fd, write_fd = os.pipe()
    guardian = subprocess.Popen(
        [
            sys.executable,
            "-I",
            "-B",
            "-m",
            "knoted_intelligence.guardian",
            str(ledger.path),
            execution_id,
            token,
            str(read_fd),
            "0.1",
            json.dumps(
                [
                    sys.executable,
                    "-I",
                    "-B",
                    "-c",
                    f"from pathlib import Path;Path({str(marker)!r}).touch() ",
                ]
            ),
        ],
        pass_fds=(read_fd,),
        start_new_session=True,
        stderr=subprocess.DEVNULL,
    )
    os.close(read_fd)
    try:
        wait_for(lambda: store.phase(execution_id) == "ready")
        os.close(write_fd)
        write_fd = -1
        status = guardian.wait(timeout=10)
        assert not marker.exists()
        if receipt_failure:
            assert status != 0 and store.receipt(execution_id) is None
        else:
            assert status == 0 and store.receipt(execution_id)["reason"] == "owner_eof"
    finally:
        if write_fd >= 0:
            os.close(write_fd)
        if guardian.poll() is None:
            guardian.kill()
        guardian.wait()
        ledger.close()
