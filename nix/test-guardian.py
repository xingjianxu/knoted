"""Run inside the T70 VM with a held synthetic child and the real service units."""
import argparse
from datetime import UTC, datetime, timedelta
import hashlib
import json
from http.client import RemoteDisconnected
from pathlib import Path
import sqlite3
import subprocess
import time
from urllib.error import URLError
from urllib.request import Request, urlopen
from uuid import uuid4


def wait_for(probe, seconds=60):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            result = probe()
            if result:
                return result
        except (FileNotFoundError, URLError, TimeoutError, ConnectionResetError, RemoteDisconnected):
            pass
        time.sleep(0.1)
    raise TimeoutError(f"timed out waiting for {probe}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("port", type=int)
    args = parser.parse_args()
    if not Path("/run/knoted-t70-isolated-vm").is_file():
        parser.error("missing isolated VM acceptance marker")
    unit = "knoted-intelligence.service"
    url = f"http://127.0.0.1:{args.port}/internal/v1"
    for action in ("stop", "kill"):
        execution = str(uuid4())
        object_id = str(uuid4())
        source = args.root / "staging/input" / object_id
        source.write_bytes(b"staged")
        request = {
            "execution_id": execution,
            "kind": "parse",
            "input_object_id": object_id,
            "input_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
            "source_version": 1,
            "options_revision": "t70-vm",
            "deadline": (datetime.now(UTC) + timedelta(minutes=2)).isoformat(),
        }
        with urlopen(Request(f"{url}/executions", data=json.dumps(request).encode(),
                             headers={"Content-Type": "application/json"}), timeout=20) as response:
            assert response.status == 201
        status = args.root / "staging/output" / execution / "child-status.json"
        child = wait_for(lambda: json.loads(status.read_text())["pid"])
        with sqlite3.connect(args.root / "python/executions.db") as db:
            guardian = db.execute(
                "SELECT guardian_pid FROM supervision WHERE execution_id=?", (execution,)
            ).fetchone()[0]
        # Ledger PIDs are in the private service namespace. Map them within
        # this cgroup before asserting kernel disappearance from the VM host.
        cgroup = Path("/sys/fs/cgroup/system.slice") / unit / "cgroup.procs"
        pids = {}
        for pid in cgroup.read_text().split():
            status_file = Path(f"/proc/{pid}/status")
            labels = dict(line.split(":", 1) for line in status_file.read_text().splitlines() if ":" in line)
            pids[int(labels["NSpid"].split()[-1])] = pid
        child, guardian = pids[child], pids[guardian]
        main_pid = subprocess.check_output(["systemctl", "show", "-p", "MainPID", "--value", unit], text=True).strip()
        assert Path(f"/proc/{main_pid}/exe").resolve().name == "python3.12"
        assert pids[1] != main_pid
        if action == "stop":
            subprocess.run(["systemctl", "stop", unit], check=True)
            subprocess.run(["systemctl", "start", unit], check=True)
        else:
            subprocess.run(["systemctl", "kill", "--kill-whom=main", "--signal=KILL", unit], check=True)
        wait_for(lambda: not Path(f"/proc/{child}").exists() and not Path(f"/proc/{guardian}").exists())

        def confirmed():
            with urlopen(f"{url}/executions/{execution}", timeout=5) as response:
                result = json.load(response)
                return result if result.get("compute_exit_confirmed") is True else None

        result = wait_for(confirmed)
        print(action, execution, result["state"], "compute_exit_confirmed=true", flush=True)

    # Simulate losing the last proof on the isolated guest's private ledger.
    subprocess.run(["systemctl", "stop", unit], check=True)
    with sqlite3.connect(args.root / "python/executions.db") as db:
        changed = db.execute("UPDATE supervision SET receipt=NULL WHERE execution_id=?", (execution,))
        assert changed.rowcount == 1
    subprocess.run(["systemctl", "start", unit], check=True)

    def degraded():
        with urlopen(f"{url}/health", timeout=5) as response:
            return json.load(response)["status"] == "degraded"

    wait_for(degraded)
    print("missing receipt => degraded", flush=True)


if __name__ == "__main__":
    main()
