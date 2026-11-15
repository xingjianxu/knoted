"""The provisioning scripts and the Python lock must agree, and --verify must not write.

The shared model roots are written only by these scripts. The revision directory
name exists in three places — the script, the Python derivation and ``LOCK.md`` —
so a drift would file the same weights under a second name. `--verify` is the
read-only path used by operators and by the host task; it must never create a
directory or overwrite a byte.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

from knoted_intelligence.embedding import bge
from knoted_intelligence.parsing import models as locked

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
DOCLING_SCRIPT = SCRIPTS / "fetch-docling-artifacts.sh"
BGE_SCRIPT = SCRIPTS / "fetch-bge-m3-artifacts.sh"


def run(
    script: Path, *args: object, env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(script), *(str(arg) for arg in args)],
        capture_output=True,
        text=True,
        check=False,
        env={**os.environ, **(env or {})},
    )


def test_docling_revision_id_matches_the_lock() -> None:
    result = run(DOCLING_SCRIPT, "--print-revision-id")

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == locked.revision_dirname()


def test_bge_revision_id_matches_the_lock() -> None:
    result = run(BGE_SCRIPT, "--print-revision-id")

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == bge.snapshot_dirname()


def test_docling_verify_reports_a_tampered_file_without_overwriting(tmp_path: Path) -> None:
    target = tmp_path / "docling"
    revision = target / locked.revision_dirname()
    victim = revision / locked.RAPIDOCR_DICT
    victim.parent.mkdir(parents=True)
    victim.write_bytes(b"not the locked dictionary")

    result = run(DOCLING_SCRIPT, target, "--verify")

    assert result.returncode != 0
    assert "MISMATCH" in result.stdout + result.stderr
    assert victim.read_bytes() == b"not the locked dictionary"


def test_docling_verify_does_not_create_the_revision_directory(tmp_path: Path) -> None:
    target = tmp_path / "absent"

    result = run(DOCLING_SCRIPT, target, "--verify")

    assert result.returncode != 0
    assert "MISSING" in result.stdout + result.stderr
    assert not target.exists()


def test_bge_verify_fails_on_an_incomplete_snapshot(tmp_path: Path) -> None:
    target = tmp_path / "bge"
    (target / bge.snapshot_dirname()).mkdir(parents=True)

    result = run(BGE_SCRIPT, target, "--verify")

    assert result.returncode != 0
    assert "MISSING" in result.stdout + result.stderr


def test_bge_verify_does_not_create_the_revision_directory(tmp_path: Path) -> None:
    target = tmp_path / "absent"

    result = run(BGE_SCRIPT, target, "--verify")

    assert result.returncode != 0
    assert not target.exists()


def test_lock_documents_name_the_same_revision_directory() -> None:
    """The third place the name lives is the lock document itself."""

    parsing_lock = (
        Path(__file__).resolve().parents[1] / "src/knoted_intelligence/parsing/LOCK.md"
    ).read_text(encoding="utf-8")
    embedding_lock = (
        Path(__file__).resolve().parents[1] / "src/knoted_intelligence/embedding/LOCK.md"
    ).read_text(encoding="utf-8")

    assert locked.revision_dirname() in parsing_lock
    assert bge.snapshot_dirname() in embedding_lock


#: Nothing listens here, so a provisioning copy fails fast instead of downloading.
NO_NETWORK = {"HF_ENDPOINT": "http://127.0.0.1:9", "MODELSCOPE_ENDPOINT": "http://127.0.0.1:9"}


def test_docling_provisioning_refuses_to_overwrite_a_mismatched_file(tmp_path: Path) -> None:
    """A drifted file in an immutable revision directory is reported, never replaced."""

    target = tmp_path / "docling"
    revision = target / locked.revision_dirname()
    victim = revision / locked.RAPIDOCR_DICT
    victim.parent.mkdir(parents=True)
    victim.write_bytes(b"tampered dictionary")

    result = run(DOCLING_SCRIPT, target, env=NO_NETWORK)

    assert result.returncode != 0
    assert f"MISMATCH {locked.RAPIDOCR_DICT}" in result.stdout + result.stderr
    assert victim.read_bytes() == b"tampered dictionary"


def test_bge_provisioning_refuses_to_overwrite_a_mismatched_file(tmp_path: Path) -> None:
    target = tmp_path / "bge"
    revision = target / bge.snapshot_dirname()
    revision.mkdir(parents=True)
    victim = revision / "config.json"
    victim.write_bytes(b"tampered config")

    result = run(BGE_SCRIPT, target, env=NO_NETWORK)

    assert result.returncode != 0
    assert "MISMATCH config.json" in result.stdout + result.stderr
    assert victim.read_bytes() == b"tampered config"
