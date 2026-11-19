"""Cross-check the bundle's recorded build provenance."""
from pathlib import Path
import hashlib
import json
import sys

directory = Path("share/knoted/intelligence")
metadata = json.loads((directory / "BUILD.json").read_bytes())
python = directory / "python/bin/python3.12"
digest = hashlib.sha256(python.read_bytes()).hexdigest()
lock = hashlib.sha256((directory / "uv.lock").read_bytes()).hexdigest()
if not (
    metadata["revision"] == sys.argv[1]
    and metadata["python_version"].split()[0].startswith("3.12.")
    and digest == metadata["interpreter_sha256"] == metadata["source_interpreter_sha256"]
    and lock == metadata["uv_lock_sha256"]
):
    sys.exit("invalid Python build metadata")
