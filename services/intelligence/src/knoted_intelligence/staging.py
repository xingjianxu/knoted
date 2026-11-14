"""Controlled staging paths and content digests.

Object ids are the only handle a caller gets on a staged file. They are
therefore validated twice: against the contract pattern
``^(?!\\.\\.?$)[A-Za-z0-9._-]+$`` and against the resolved path, which must stay
inside the staging root. A path separator, a ``..`` segment or a URL scheme is
rejected before any file system call, so ``../x``, ``a/b`` and ``http://evil``
can never address anything outside ``staging/input``.
"""

from __future__ import annotations

import hashlib
import os
import re
import uuid
from pathlib import Path

#: Contract pattern for a controlled object id (``api/intelligence.openapi.yaml``).
OBJECT_ID_PATTERN = re.compile(r"^(?!\.\.?$)[A-Za-z0-9._-]+$")

#: Characters or sequences that must never appear, even if the pattern allowed them.
FORBIDDEN_IN_OBJECT_ID = ("/", "\\", "..", "://")

#: Read size when hashing, so a large artifact never has to fit in memory.
_HASH_CHUNK_BYTES = 1 << 20

#: Temporary file suffix used by the atomic write; tests assert none is left behind.
TEMP_SUFFIX = ".part"


class StagingError(ValueError):
    """A staging path or object id is not acceptable."""


def ensure_object_id(value: str, *, field: str = "object_id") -> str:
    """Returns ``value`` if it is a controlled object id, else raises.

    ``field`` only improves the message; the rule is the same everywhere an
    object id enters the service.
    """

    if not isinstance(value, str) or not value:
        raise StagingError(f"{field} must be a non-empty string")
    if OBJECT_ID_PATTERN.fullmatch(value) is None:
        raise StagingError(
            f"{field} {value!r} must match {OBJECT_ID_PATTERN.pattern} "
            "(letters, digits, dot, dash and underscore only)"
        )
    for forbidden in FORBIDDEN_IN_OBJECT_ID:
        if forbidden in value:
            raise StagingError(f"{field} {value!r} must not contain {forbidden!r}")
    return value


def input_path(input_root: Path, object_id: str) -> Path:
    """Resolves an input object id inside ``input_root``."""

    return _contained(input_root, object_id, field="input_object_id")


def output_dir(output_root: Path, execution_id: str) -> Path:
    """Resolves the per-execution output directory inside ``output_root``."""

    return _contained(output_root, execution_id, field="execution_id")


def artifact_path(execution_id_dir: Path, object_id: str) -> Path:
    """Resolves one artifact inside an execution output directory."""

    return _contained(execution_id_dir, object_id, field="result_object_id")


def _contained(root: Path, name: str, *, field: str) -> Path:
    ensure_object_id(name, field=field)
    resolved_root = root.resolve()
    candidate = (resolved_root / name).resolve()
    if candidate == resolved_root or not candidate.is_relative_to(resolved_root):
        raise StagingError(f"{field} {name!r} resolves outside {resolved_root}")
    return candidate


def sha256_bytes(payload: bytes) -> str:
    """Lowercase hex SHA-256 of ``payload``."""

    return hashlib.sha256(payload).hexdigest()


def sha256_file(path: Path) -> str:
    """Lowercase hex SHA-256 of the file at ``path``."""

    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(_HASH_CHUNK_BYTES), b""):
            digest.update(block)
    return digest.hexdigest()


def atomic_write(path: Path, payload: bytes) -> None:
    """Writes ``payload`` to ``path`` so readers see either the old or new file.

    The temporary file lives in the destination directory (same file system, so
    ``os.replace`` is an atomic rename) and is fsynced before the rename.
    """

    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{uuid.uuid4().hex}{TEMP_SUFFIX}")
    try:
        with temporary.open("wb") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        if temporary.exists():  # pragma: no cover - only on a failed write
            temporary.unlink()
