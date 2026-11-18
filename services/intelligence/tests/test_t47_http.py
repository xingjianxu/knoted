"""Real loopback HTTP, API death, and read-only recovery evidence."""

import ctypes
import hashlib
import json
import os
import socket
import subprocess
import sys
import time
from datetime import UTC, datetime, timedelta
from pathlib import Path
from uuid import uuid4

import httpx

from knoted_intelligence.supervision import inspect


def wait_for(check, seconds=30):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        result = check()
        if result:
            return result
        time.sleep(0.02)
    raise AssertionError("owned HTTP execution did not reach expected condition")


def start(tmp_path):
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    env = os.environ | {
        "KNOTED_INTELLIGENCE_HOST": "127.0.0.1",
        "KNOTED_INTELLIGENCE_PORT": str(port),
        "KNOTED_INTELLIGENCE_STATE_DIR": str(tmp_path / "state"),
        "KNOTED_INTELLIGENCE_STAGING_DIR": str(tmp_path / "staging"),
        "KNOTED_INTELLIGENCE_CHILD_HOLD_MS": "60000",
        "KNOTED_INTELLIGENCE_CHILD_IGNORE_SIGTERM": "1",
        "KNOTED_INTELLIGENCE_CANCEL_GRACE_SECONDS": "0.1",
        "CUDA_VISIBLE_DEVICES": "",
        "KNOTED_EMBED_GPU_UUID": "",
        "KNOTED_INTELLIGENCE_DOCLING_DEVICE": "cpu",
    }
    process = subprocess.Popen(
        [sys.executable, "-I", "-B", "-c", "from knoted_intelligence.app import main;main()"],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    client = httpx.Client(base_url=f"http://127.0.0.1:{port}", timeout=20)

    def ready():
        assert process.poll() is None
        try:
            return client.get("/internal/v1/health").status_code == 200
        except httpx.TransportError:
            return False

    try:
        wait_for(ready)
    except BaseException:
        process.kill()
        process.wait()
        client.close()
        raise
    return process, client


def test_t47_real_api_sigkill_owner_eof_receipt_and_restart(tmp_path):
    # Adopt the orphan guardian so the test itself reaps every owned process.
    libc = ctypes.CDLL(None)
    assert libc.prctl(36, 1, 0, 0, 0) == 0
    server, client = start(tmp_path)
    execution_id = str(uuid4())
    guardian_pid = None
    sentinel = subprocess.Popen([sys.executable, "-I", "-B", "-c", "import time;time.sleep(60)"])
    try:
        path = tmp_path / "staging/input/source"
        path.write_bytes(b"staged")
        request = dict(
            execution_id=execution_id,
            kind="parse",
            input_object_id="source",
            input_sha256=hashlib.sha256(b"staged").hexdigest(),
            source_version=1,
            options_revision="test",
            deadline=(datetime.now(UTC) + timedelta(seconds=60)).isoformat(),
        )
        assert client.post("/internal/v1/executions", json=request).status_code == 201
        status_path = tmp_path / "staging/output" / execution_id / "child-status.json"
        wait_for(status_path.exists)
        child_pid = json.loads(status_path.read_text())["pid"]
        import sqlite3

        with sqlite3.connect(tmp_path / "state/executions.db") as db:
            guardian_pid = db.execute(
                "SELECT guardian_pid FROM supervision WHERE execution_id=?", (execution_id,)
            ).fetchone()[0]
        server.kill()
        assert server.wait(timeout=5) == -9
        wait_for(lambda: inspect(tmp_path / "state")["clean"])
        assert not Path(f"/proc/{child_pid}").exists()
        pid, status = os.waitpid(guardian_pid, 0)
        assert pid == guardian_pid and os.waitstatus_to_exitcode(status) == 0
        guardian_pid = None
        assert sentinel.poll() is None
        client.close()
        server, client = start(tmp_path)
        result = client.get(f"/internal/v1/executions/{execution_id}").json()
        assert result["state"] == "interrupted"
        assert result["compute_exit_confirmed"] is True
        assert (
            client.post("/internal/v1/executions", json=request).json()["execution_id"]
            == execution_id
        )
        assert client.get("/internal/v1/health").json()["status"] == "ok"
    finally:
        client.close()
        if server.poll() is None:
            server.terminate()
            server.wait(timeout=25)
        if guardian_pid:
            os.waitpid(guardian_pid, 0)
        sentinel.terminate()
        sentinel.wait()
        libc.prctl(36, 0, 0, 0, 0)
