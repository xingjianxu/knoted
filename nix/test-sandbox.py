"""Guest-only negative checks run inside the real Python unit before executing its normal command."""
from pathlib import Path
import socket
import os
import sys


def denied(operation, description, allowed_errors=(PermissionError, FileNotFoundError)):
    try:
        operation()
    except allowed_errors:
        return
    raise AssertionError(f"unexpected access to {description}")


visible = [p for p in Path("/proc").iterdir() if p.name.isdigit()]
print("sandbox process view", os.getpid(), len(visible), Path("/proc/1/comm").read_text().strip(), flush=True)
assert os.getpid() == 1 and [p.name for p in visible] == ["1"]

for path in (
    "/run/t70.toml", "/run/t70-rust/strata", "/run/t70-rust/pg",
    "/var/lib/knoted/assets/marker", "/srv/t70-source/marker",
):
    denied(lambda: Path(path).read_bytes(), path)


def connect(family, address):
    with socket.socket(family) as client:
        client.connect(address)


denied(lambda: connect(socket.AF_UNIX, "/run/postgresql/.s.PGSQL.5432"), "PostgreSQL Unix socket", (PermissionError,))
denied(lambda: connect(socket.AF_INET, ("127.0.0.1", 5432)), "PostgreSQL TCP socket", (PermissionError,))
print("T70 Python sandbox denied Rust-only files and both database transports", flush=True)
os.execv(sys.argv[1], sys.argv[1:])
