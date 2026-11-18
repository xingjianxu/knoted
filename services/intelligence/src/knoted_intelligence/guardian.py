"""One exclusive subreaper/waiter for one trusted compute subtree (Linux).

The owner pipe and compute gate are separate. No compute import is possible
before GO is durable and the guardian holds the child group's kernel identity.
Numeric diagnostic PIDs are never re-used to signal after recovery.
"""

from __future__ import annotations

import ctypes
import json
import os
import select
import signal
import subprocess
import sys
import time
from datetime import UTC, datetime
from pathlib import Path

from .supervision import Supervision

PIDFD_SIGNAL_PROCESS_GROUP = 4
PR_SET_CHILD_SUBREAPER = 36
_GATE = (
    "import os,sys,json; fd=int(sys.argv[1]); go=os.read(fd,1); os.close(fd); "
    "cmd=json.loads(sys.argv[2]); (os.execv(cmd[0],cmd) if go==b'G' else sys.exit(1))"
)


def group_signal(fd, sig):
    try:
        signal.pidfd_send_signal(fd, sig, None, PIDFD_SIGNAL_PROCESS_GROUP)
    except ProcessLookupError:
        pass


def owner_event(fd):
    if select.select([fd], [], [], 0)[0]:
        data = os.read(fd, 64)
        return "owner_eof" if not data else "cancelled" if b"C" in data else "go"
    return None


def own_children():
    # These are our current direct children, held waitable (no other waiter).
    return [int(pid) for pid in Path(f"/proc/self/task/{os.getpid()}/children").read_text().split()]


def reap_tree(group_fd, leader, grace):
    """Stop spawn, then reclaim the complete adopted tree, including setsid children."""
    group_signal(group_fd, signal.SIGTERM)
    deadline = time.monotonic() + grace
    term_seen = {leader.pid}
    while True:
        sig = signal.SIGKILL if time.monotonic() >= deadline else signal.SIGTERM
        if sig == signal.SIGKILL:
            group_signal(group_fd, sig)
        for pid in own_children():
            if sig == signal.SIGTERM and pid in term_seen:
                continue
            term_seen.add(pid)
            try:
                fd = os.pidfd_open(pid)
            except ProcessLookupError:
                continue
            try:
                signal.pidfd_send_signal(fd, sig)
            except ProcessLookupError:
                pass
            finally:
                os.close(fd)
        try:
            while True:
                pid, status = os.waitpid(-1, os.WNOHANG)
                if pid == 0:
                    break
                if pid == leader.pid:
                    leader.returncode = os.waitstatus_to_exitcode(status)
        except ChildProcessError:
            return
        time.sleep(0.02)


def run(path, execution_id, token, owner_fd, grace, command):
    store = Supervision(Path(path))
    signal.signal(signal.SIGCHLD, signal.SIG_DFL)
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) != 0:
        raise OSError(ctypes.get_errno(), "subreaper unavailable")
    # Probe group flag against our own new session before any compute exists.
    probe = os.pidfd_open(os.getpid())
    try:
        signal.pidfd_send_signal(probe, 0, None, PIDFD_SIGNAL_PROCESS_GROUP)
    finally:
        os.close(probe)
    if not store.ready(execution_id, token):
        return 1
    reason = None
    while True:
        event = owner_event(owner_fd)
        row = store.row(execution_id)
        if event == "owner_eof" or event == "cancelled" or row["state"] == "cancel_requested":
            reason = event if event in {"owner_eof", "cancelled"} else "cancelled"
            break
        if datetime.now(UTC) >= datetime.fromisoformat(row["deadline"]):
            reason = "deadline_exceeded"
            break
        if event == "go":
            if store.phase(execution_id) != "go":
                raise RuntimeError("GO was not durable")
            break
        time.sleep(0.02)
    if reason:
        store.complete(execution_id, token, reason, None)
        return 0
    gate_read, gate_write = os.pipe()
    leader = None
    group_fd = None
    status = None
    try:
        leader = subprocess.Popen(
            [sys.executable, "-I", "-B", "-c", _GATE, str(gate_read), json.dumps(command)],
            pass_fds=(gate_read,),
            start_new_session=True,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        os.close(gate_read)
        gate_read = -1
        group_fd = os.pidfd_open(leader.pid)
        # The gate must be closed on guardian death; compute inherits no owner fd.
        os.write(gate_write, b"G")
        os.close(gate_write)
        gate_write = -1
        limit = time.monotonic() + min(
            900,
            max(0, (datetime.fromisoformat(row["deadline"]) - datetime.now(UTC)).total_seconds()),
        )
        while True:
            event = owner_event(owner_fd)
            row = store.row(execution_id)
            if event in {"owner_eof", "cancelled"} or row["state"] == "cancel_requested":
                reason = event if event in {"owner_eof", "cancelled"} else "cancelled"
                break
            if time.monotonic() >= limit or datetime.now(UTC) >= datetime.fromisoformat(
                row["deadline"]
            ):
                reason = "deadline_exceeded"
                break
            status = leader.poll()
            if status is not None:
                reason = "completed"
                break
            time.sleep(0.02)
    except Exception:
        reason = "guardian_error"
        # Failure before the group identity was captured is deliberately not a proof.
        if leader is not None and group_fd is None:
            raise
    finally:
        for fd in (gate_read, gate_write):
            if fd >= 0:
                os.close(fd)
        if group_fd is not None:
            try:
                reap_tree(group_fd, leader, grace)
            finally:
                os.close(group_fd)
    store.complete(execution_id, token, reason, leader.returncode if leader is not None else status)
    return 0


if __name__ == "__main__":
    raise SystemExit(
        run(
            sys.argv[1],
            sys.argv[2],
            sys.argv[3],
            int(sys.argv[4]),
            float(sys.argv[5]),
            json.loads(sys.argv[6]),
        )
    )
