"""BAAI/bge-m3 dense embeddings from the locally provisioned snapshot.

Only the **dense** head is used: ``SentenceTransformer(<snapshot>)`` with
``normalize_embeddings=True`` yields ``float32[batch][1024]`` rows whose L2 norm
is 1.0. The snapshot also ships ``sparse_linear.pt`` and ``colbert_linear.pt``;
those heads are deliberately not loaded (T11 contract: dense only), and their
presence is not a runtime requirement.

The snapshot lives at the root named by ``KNOTED_INTELLIGENCE_BGE_M3_DIR`` (a
machine-level shared directory, see ``embedding/LOCK.md``) and is never
downloaded at runtime:
``HF_HUB_OFFLINE``/``TRANSFORMERS_OFFLINE``/``HF_HUB_DISABLE_TELEMETRY`` are set
before ``sentence_transformers`` (and therefore ``torch``) is imported, and the
snapshot path is passed explicitly. A missing, wrongly-sized or tampered file is
a :class:`EmbeddingError` with ``models_missing``, not a hub fallback: the ten
files the dense path loads are re-hashed with SHA-256 before the model loads.

GPU selection is explicit. ``KNOTED_EMBED_GPU_UUID`` must hold an
``nvidia-smi -L`` UUID (for example ``GPU-9e2b...``); it is copied into
``CUDA_VISIBLE_DEVICES`` before torch initialises. No code path defaults to
index 0/1, because on the deployment host index 0 is Strata's card. With the
variable unset the embedder runs on the CPU; an invalid UUID or a configured GPU
that torch cannot see is an error, never a silent downgrade.

Every constant here is the host-verified lock recorded in ``LOCK.md``;
``manifest()`` cross-checks the snapshot's ``MANIFEST.json`` against these
constants so a drifted snapshot fails instead of producing a mixed embedding
space. Changing a value is a new ``OPTIONS_REVISION``, not a runtime choice.
"""

from __future__ import annotations

import hashlib
import json
import logging
import math
import os
import re
import threading
from importlib.metadata import PackageNotFoundError
from importlib.metadata import version as distribution_version
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)

# -- paths -------------------------------------------------------------------

#: Environment variable naming the shared snapshot root; there is no default.
SNAPSHOT_DIR_ENV = "KNOTED_INTELLIGENCE_BGE_M3_DIR"
#: Machine-readable inventory written by the provisioning script.
MANIFEST_FILENAME = "MANIFEST.json"
MANIFEST_SCHEMA = "knoted-bge-m3-models/1"
#: Re-verification entry point shipped in the repository.
FETCH_SCRIPT = "services/intelligence/scripts/fetch-bge-m3-artifacts.sh"

# -- pinned model ------------------------------------------------------------

MODEL_REPO = "BAAI/bge-m3"
#: Hugging Face commit of the snapshot. The local copy has no ``.git``, so this
#: record (and ``MANIFEST.json``) is the only source of the revision; it must be
#: reported with every embedding.
MODEL_REVISION = "5617a9f61b028005a4858fdac845db406aefb181"
#: The tokenizer files ship in the same commit; there is no second download.
TOKENIZER_REVISION = MODEL_REVISION

# -- pinned runtime ----------------------------------------------------------

SENTENCE_TRANSFORMERS_VERSION = "6.3.0"
TRANSFORMERS_VERSION = "5.23.0"
#: Wheel pin; the resolved wheel on the deployment host is ``2.15.0+cu132``.
TORCH_VERSION = "2.15.0"
SAFETENSORS_VERSION = "0.8.0"

# -- embedding contract ------------------------------------------------------

HIDDEN_SIZE = 1024
OUTPUT_DTYPE = "float32"
NORMALIZE_EMBEDDINGS = True
MAX_SEQ_LENGTH = 8192
#: Accepted deviation of the L2 norm from 1.0 (the model yields ~1.2e-7).
NORMALIZATION_TOLERANCE = 1e-4
#: One encode call; small enough that a batch is not a memory decision.
BATCH_SIZE = 8
#: Refuse more texts than this in one execution instead of growing unbounded.
MAX_BATCH_SIZE = 64
#: Refuse an embed input object larger than this before decoding it.
MAX_INPUT_BYTES = 8 * 1024 * 1024

#: The value the worker should send as ``options_revision`` for embed work.
OPTIONS_REVISION = f"embed-v1+bge-m3-{MODEL_REVISION}-dense-{HIDDEN_SIZE}"

# -- GPU selection -----------------------------------------------------------

GPU_UUID_ENV = "KNOTED_EMBED_GPU_UUID"
CUDA_VISIBLE_DEVICES_ENV = "CUDA_VISIBLE_DEVICES"
_GPU_UUID_PATTERN = re.compile(r"GPU-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")

#: Applied at import and again before the lazy import of torch.
OFFLINE_ENV: dict[str, str] = {
    "HF_HUB_OFFLINE": "1",
    "HF_DATASETS_OFFLINE": "1",
    "TRANSFORMERS_OFFLINE": "1",
    "HF_HUB_DISABLE_TELEMETRY": "1",
}

#: Every file of the host snapshot (size/sha256 from ``MANIFEST.json``).
EXPECTED_FILES: tuple[tuple[str, int, str], ...] = (
    (
        "1_Pooling/config.json",
        191,
        "e54c164a07274f2eb45bb724f54a79d1efcc90c41573887cd9a29aeee0597352",
    ),
    (
        "README.md",
        15_822,
        "0b81ccf9134e5874d620a86e6905062ea999e779c34eb1a7e65eaeb7fe00e450",
    ),
    (
        "colbert_linear.pt",
        2_100_674,
        "19bfbae397c2b7524158c919d0e9b19393c5639d098f0a66932c91ed8f5f9abb",
    ),
    (
        "config.json",
        687,
        "26159e7ad065073448460117eb24b7a4572f6f4e78eadff65dc0a11c052449fa",
    ),
    (
        "config_sentence_transformers.json",
        123,
        "1eef72430e7194a1e59680e635aed81ffa083f05668dbc5bb1c56c04c0999c38",
    ),
    (
        "model.safetensors",
        2_271_064_520,
        "5632ea99fd6c2dc9c1d0837660b581ccbff68b0eca17f95d86373e16385d2f69",
    ),
    (
        "modules.json",
        349,
        "84e40c8e006c9b1d6c122e02cba9b02458120b5fb0c87b746c41e0207cf642cf",
    ),
    (
        "sentence_bert_config.json",
        54,
        "eb9b44b13c0f52a3b3685c3b1cbdea1ba8b04bea123b98f61610048940776eb1",
    ),
    (
        "sentencepiece.bpe.model",
        5_069_051,
        "cfc8146abe2a0488e9e2a0c56de7952f7c11ab059eca145a0a727afce0db2865",
    ),
    (
        "sparse_linear.pt",
        3_516,
        "45c93804d2142b8f6d7ec6914ae23a1eee9c6a1d27d83d908a20d2afb3595ad9",
    ),
    (
        "special_tokens_map.json",
        964,
        "8c785abebea9ae3257b61681b4e6fd8365ceafde980c21970d001e834cf10835",
    ),
    (
        "tokenizer.json",
        17_098_108,
        "21106b6d7dab2952c1d496fb21d5dc9db75c28ed361a05f5020bbba27810dd08",
    ),
    (
        "tokenizer_config.json",
        444,
        "a62b2b6784f990259fddef5f16388693a8043be4f69179e6a5257eeb3f9abac4",
    ),
)

#: Files the dense path actually loads. ``README.md`` and the sparse/ColBERT
#: heads are inventoried but not required and never opened.
REQUIRED_FILES: tuple[str, ...] = (
    "1_Pooling/config.json",
    "config.json",
    "config_sentence_transformers.json",
    "model.safetensors",
    "modules.json",
    "sentence_bert_config.json",
    "sentencepiece.bpe.model",
    "special_tokens_map.json",
    "tokenizer.json",
    "tokenizer_config.json",
)

_MODEL: Any = None
_LOADED_DEVICE: str | None = None
_LOADED_UUID: str | None = None
_MODEL_LOCK = threading.Lock()

os.environ.update(OFFLINE_ENV)


class EmbeddingError(RuntimeError):
    """A dense-embedding refusal whose message is safe to hand to the worker.

    ``code`` is stable: ``models_missing``, ``snapshot_unset``,
    ``manifest_mismatch``,
    ``invalid_gpu_uuid``, ``cuda_unavailable``, ``model_load_failed``,
    ``invalid_input``, ``empty_input``, ``empty_text``, ``batch_too_large``,
    ``input_too_large``, ``text_too_long``, ``invalid_output``,
    ``invalid_dimension``, ``output_count_mismatch``, ``non_finite_output``,
    ``not_normalized``. Every message starts with the locked combination so a
    failure reports the contract instead of inviting a different model.
    """

    def __init__(self, code: str, detail: str) -> None:
        self.code = code
        self.detail = detail
        super().__init__(f"[locked: {locked_combination_summary()}] {code}: {detail}")

    @property
    def public_message(self) -> str:
        """The message the child process may copy into an execution record."""

        return str(self)


def locked_combination_summary() -> str:
    """The model, revision and runtime that produced (or refused) an embedding."""

    return (
        f"{MODEL_REPO}@{MODEL_REVISION}, dense only (sparse/colbert heads unused), "
        f"hidden {HIDDEN_SIZE}, {OUTPUT_DTYPE}, L2-normalized, "
        f"max_seq_length {MAX_SEQ_LENGTH}, sentence-transformers "
        f"{SENTENCE_TRANSFORMERS_VERSION}, transformers {TRANSFORMERS_VERSION}, "
        f"torch {TORCH_VERSION}, safetensors {SAFETENSORS_VERSION}"
    )


def lock_summary() -> dict[str, Any]:
    """Machine-readable lock written into every embed artifact."""

    return {
        "repo": MODEL_REPO,
        "model_revision": MODEL_REVISION,
        "tokenizer_revision": TOKENIZER_REVISION,
        "hidden_size": HIDDEN_SIZE,
        "dtype": OUTPUT_DTYPE,
        "normalize_embeddings": NORMALIZE_EMBEDDINGS,
        "max_seq_length": MAX_SEQ_LENGTH,
        "heads": "dense-only",
        "runtime": {
            "sentence_transformers": SENTENCE_TRANSFORMERS_VERSION,
            "transformers": TRANSFORMERS_VERSION,
            "torch": TORCH_VERSION,
            "safetensors": SAFETENSORS_VERSION,
        },
        "gpu_uuid_env": GPU_UUID_ENV,
        "options_revision": OPTIONS_REVISION,
    }


# -- snapshot layout ---------------------------------------------------------


def model_dir() -> Path:
    """The BGE-M3 snapshot root, from the environment only.

    A missing configuration is an explicit failure: the snapshot is shared
    between consumers and lives outside any instance data root.
    """

    raw = os.environ.get(SNAPSHOT_DIR_ENV, "").strip()
    if not raw:
        raise EmbeddingError(
            "snapshot_unset",
            f"{SNAPSHOT_DIR_ENV} is not set; it names the shared BGE-M3 snapshot "
            f"root (see embedding/LOCK.md, provisioned by {FETCH_SCRIPT})",
        )
    return Path(raw).expanduser()


def snapshot_dirname() -> str:
    """The immutable directory name of this locked snapshot in the shared root."""

    return f"bge-m3-{MODEL_REVISION[:8]}-dense"


def manifest() -> dict[str, Any]:
    """Reads ``MANIFEST.json`` and refuses a snapshot that drifted from the lock."""

    path = model_dir() / MANIFEST_FILENAME
    if not path.is_file():
        raise EmbeddingError(
            "models_missing",
            f"{MANIFEST_FILENAME} is missing from the BGE-M3 snapshot; provision it with "
            f"{FETCH_SCRIPT} as described in embedding/LOCK.md",
        )
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise EmbeddingError(
            "manifest_mismatch",
            f"{MANIFEST_FILENAME} is not readable JSON ({type(exc).__name__})",
        ) from exc
    if not isinstance(payload, dict):
        raise EmbeddingError("manifest_mismatch", f"{MANIFEST_FILENAME} is not a JSON object")
    expected_contract = {
        "embedding_dim": HIDDEN_SIZE,
        "normalize_embeddings": NORMALIZE_EMBEDDINGS,
        "max_seq_length": MAX_SEQ_LENGTH,
        "dtype": OUTPUT_DTYPE,
    }
    problems = []
    if payload.get("schema") != MANIFEST_SCHEMA:
        problems.append(f"schema is {payload.get('schema')!r}, expected {MANIFEST_SCHEMA!r}")
    if payload.get("repo") != MODEL_REPO:
        problems.append(f"repo is {payload.get('repo')!r}, expected {MODEL_REPO!r}")
    if payload.get("revision") != MODEL_REVISION:
        problems.append(f"revision is {payload.get('revision')!r}, expected {MODEL_REVISION!r}")
    if payload.get("runtime_contract") != expected_contract:
        problems.append(
            f"runtime_contract is {payload.get('runtime_contract')!r}, "
            f"expected {expected_contract!r}"
        )
    if problems:
        raise EmbeddingError(
            "manifest_mismatch",
            f"{MANIFEST_FILENAME} drifted from the locked snapshot (" + "; ".join(problems) + ")",
        )
    return payload


def locked_model_revision() -> str:
    """The revision, after confirming ``MANIFEST.json`` matches the constant."""

    return manifest()["revision"]


def artifact_problems(root: Path | None = None) -> list[str]:
    """Relative descriptions of files that do not match the lock.

    The ten files the dense path loads are checked for existence, size **and**
    SHA-256; a digest read of ~2.3 GiB happens once per process before the model
    loads. The three inventoried-but-unused files (README, sparse/ColBERT
    heads) are cross-checked against ``MANIFEST.json``/``LOCK.md`` instead of
    being hashed at runtime.
    """

    snapshot = root or model_dir()
    problems: list[str] = []
    locked = {relative: (size, sha256) for relative, size, sha256 in EXPECTED_FILES}
    for relative in REQUIRED_FILES:
        candidate = snapshot / relative
        if not candidate.is_file():
            problems.append(f"{relative}: missing")
            continue
        expected_size, expected_sha256 = locked[relative]
        actual = candidate.stat().st_size
        if actual != expected_size:
            problems.append(f"{relative}: expected {expected_size} bytes, found {actual} bytes")
            continue
        actual_sha256 = _sha256_file(candidate)
        if actual_sha256 != expected_sha256:
            problems.append(
                f"{relative}: sha256 {actual_sha256} does not match the lock {expected_sha256}"
            )
    return problems


def _sha256_file(path: Path) -> str:
    """Streams a file through SHA-256 without loading it into memory."""

    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(8 * 1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def require_model_dir(root: Path | None = None) -> Path:
    """Returns the snapshot path, raising if it is absent or incomplete."""

    snapshot = root or model_dir()
    if not snapshot.is_dir():
        raise EmbeddingError(
            "models_missing",
            "the BGE-M3 snapshot is not installed; provision it with "
            f"{snapshot}/{FETCH_SCRIPT} as described in embedding/LOCK.md",
        )
    manifest()
    problems = artifact_problems(snapshot)
    if problems:
        raise EmbeddingError(
            "models_missing",
            "the locked BGE-M3 snapshot is incomplete ("
            + "; ".join(problems)
            + f"); re-run {snapshot}/{FETCH_SCRIPT} --check as described in "
            "embedding/LOCK.md",
        )
    return snapshot


# -- device selection --------------------------------------------------------


def configured_device() -> str:
    """``cuda`` when ``KNOTED_EMBED_GPU_UUID`` is set, ``cpu`` otherwise.

    Copying the UUID into ``CUDA_VISIBLE_DEVICES`` here (before torch is
    imported) is what makes the explicit card choice stick. There is no default
    index: an unset variable means CPU, and a malformed one is an error.
    """

    raw = os.environ.get(GPU_UUID_ENV, "").strip()
    if not raw:
        return "cpu"
    if not _GPU_UUID_PATTERN.fullmatch(raw):
        raise EmbeddingError(
            "invalid_gpu_uuid",
            f"{GPU_UUID_ENV} must be an nvidia-smi -L GPU UUID "
            f"(GPU-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx) to copy into "
            f"{CUDA_VISIBLE_DEVICES_ENV}, got {raw!r}; "
            "Knoted never selects a card by index",
        )
    os.environ[CUDA_VISIBLE_DEVICES_ENV] = raw
    return "cuda"


def loaded_device() -> str:
    """The device the loaded model runs on (configured device before loading)."""

    return _LOADED_DEVICE or configured_device()


def _get_model() -> Any:
    """Loads and caches the snapshot; offline flags and UUID land before torch."""

    global _MODEL, _LOADED_DEVICE, _LOADED_UUID
    with _MODEL_LOCK:
        if _MODEL is not None:
            return _MODEL
        os.environ.update(OFFLINE_ENV)
        device = configured_device()
        snapshot = require_model_dir()
        try:
            import torch
            from sentence_transformers import SentenceTransformer
        except Exception as exc:  # noqa: BLE001 - import failure is a domain failure
            logger.exception("the embedding runtime could not be imported")
            raise EmbeddingError(
                "model_load_failed",
                f"the embedding runtime could not be imported ({type(exc).__name__})",
            ) from exc
        if device == "cuda" and not torch.cuda.is_available():
            raise EmbeddingError(
                "cuda_unavailable",
                f"{GPU_UUID_ENV} selects a GPU but torch cannot see a CUDA device; "
                "check the driver and LD_LIBRARY_PATH (/run/opengl-driver/lib on NixOS)",
            )
        try:
            model = SentenceTransformer(str(snapshot), device=device)
        except Exception as exc:  # noqa: BLE001 - any load failure is a domain failure
            logger.exception("the BGE-M3 snapshot could not be loaded")
            raise EmbeddingError(
                "model_load_failed",
                f"the locked snapshot could not be loaded ({type(exc).__name__}); "
                f"verify it with {snapshot}/{FETCH_SCRIPT} --check",
            ) from exc
        model.max_seq_length = MAX_SEQ_LENGTH
        loaded_dims = _embedding_dimension(model)
        if loaded_dims != HIDDEN_SIZE:
            raise EmbeddingError(
                "invalid_dimension",
                f"the loaded model reports {loaded_dims} dimensions, the lock requires "
                f"{HIDDEN_SIZE}",
            )
        _MODEL = model
        _LOADED_DEVICE = device
        _LOADED_UUID = os.environ.get(GPU_UUID_ENV, "").strip() or None
        logger.info(
            "loaded %s@%s on %s (uuid=%s, max_seq_length=%d)",
            MODEL_REPO,
            MODEL_REVISION,
            device,
            _LOADED_UUID,
            MAX_SEQ_LENGTH,
        )
        return model


def loaded_contract() -> dict[str, Any]:
    """What is actually loaded: revision, shape, tokenizer and runtime versions."""

    model = _get_model()
    versions: dict[str, str] = {}
    for distribution in ("sentence-transformers", "transformers", "torch", "safetensors"):
        try:
            versions[distribution.replace("-", "_")] = distribution_version(distribution)
        except PackageNotFoundError:  # pragma: no cover - the model could not load
            versions[distribution.replace("-", "_")] = "unknown"
    return {
        "repo": MODEL_REPO,
        "revision": locked_model_revision(),
        "tokenizer_revision": TOKENIZER_REVISION,
        "hidden_size": _embedding_dimension(model),
        "max_seq_length": model.max_seq_length,
        "dtype": OUTPUT_DTYPE,
        "normalize_embeddings": NORMALIZE_EMBEDDINGS,
        "device": loaded_device(),
        "device_uuid": _LOADED_UUID,
        "versions": versions,
    }


def _embedding_dimension(model: Any) -> int:
    """Reads the loaded embedding width (the accessor was renamed in 6.x)."""

    accessor = getattr(model, "get_embedding_dimension", None)
    if accessor is None:  # pragma: no cover - older sentence-transformers
        accessor = model.get_sentence_embedding_dimension
    return int(accessor())


# -- embedding ---------------------------------------------------------------


def embed_dense(texts: list[str]) -> list[list[float]]:
    """Embeds ``texts`` into L2-normalized ``float32[n][1024]`` rows.

    An empty batch returns ``[]`` without loading the model. A blank text, more
    than :data:`MAX_BATCH_SIZE` texts, or a text over :data:`MAX_SEQ_LENGTH`
    tokens is refused explicitly: nothing is silently truncated, because a
    truncated embedding would represent different content under the same
    revision. The model output is validated (shape, finiteness, normalization)
    before it is returned.
    """

    if isinstance(texts, str):
        raise EmbeddingError("invalid_input", "embed_dense takes a list of strings, not a string")
    items = list(texts)
    if not items:
        return []
    for index, text in enumerate(items):
        if not isinstance(text, str):
            raise EmbeddingError("invalid_input", f"text {index} is not a string")
        if not text.strip():
            raise EmbeddingError("empty_text", f"text {index} is empty or whitespace only")
    if len(items) > MAX_BATCH_SIZE:
        raise EmbeddingError(
            "batch_too_large",
            f"the batch has {len(items)} texts; the limit for one execution is {MAX_BATCH_SIZE}",
        )

    model = _get_model()
    _check_text_lengths(model, items)
    matrix = model.encode(
        items,
        batch_size=BATCH_SIZE,
        convert_to_numpy=True,
        normalize_embeddings=NORMALIZE_EMBEDDINGS,
        show_progress_bar=False,
    )
    return validate_embeddings(matrix, expected_count=len(items))


def _check_text_lengths(model: Any, texts: list[str]) -> None:
    """Refuses inputs longer than the locked window before any encoding."""

    encoded = model.tokenizer(
        texts,
        add_special_tokens=True,
        truncation=False,
        padding=False,
    )
    token_ids = encoded["input_ids"]
    for index, ids in enumerate(token_ids):
        if len(ids) > MAX_SEQ_LENGTH:
            raise EmbeddingError(
                "text_too_long",
                f"text {index} is {len(ids)} tokens; the locked maximum is {MAX_SEQ_LENGTH} tokens",
            )


def validate_embeddings(matrix: Any, *, expected_count: int) -> list[list[float]]:
    """Validates a ``[n][1024]`` float32 L2-normalized matrix and returns lists.

    This is the single acceptance gate for model output. NaN, Inf, a wrong row
    count, a wrong width, a non-float dtype, a non-number element or a row that
    is not normalized is an :class:`EmbeddingError`; no such row is ever handed
    to the worker.
    """

    dtype = getattr(matrix, "dtype", None)
    if dtype is not None:
        name = getattr(dtype, "name", str(dtype).rsplit(".", 1)[-1])
        if name != OUTPUT_DTYPE:
            raise EmbeddingError(
                "invalid_output",
                f"model output dtype is {name}, the lock requires {OUTPUT_DTYPE}",
            )
    converter = getattr(matrix, "tolist", None)
    rows = converter() if callable(converter) else matrix
    if not isinstance(rows, (list, tuple)):
        raise EmbeddingError(
            "invalid_dimension",
            f"model output is {type(rows).__name__}, expected a [n][{HIDDEN_SIZE}] matrix",
        )
    if rows and not isinstance(rows[0], (list, tuple)):
        raise EmbeddingError(
            "invalid_dimension",
            f"model output is not two-dimensional; expected [n][{HIDDEN_SIZE}]",
        )
    if len(rows) != expected_count:
        raise EmbeddingError(
            "output_count_mismatch",
            f"model returned {len(rows)} rows for {expected_count} texts",
        )
    validated: list[list[float]] = []
    for row_index, row in enumerate(rows):
        if isinstance(row, (str, bytes)) or not isinstance(row, (list, tuple)):
            raise EmbeddingError(
                "invalid_dimension",
                f"row {row_index} is not a sequence; expected {HIDDEN_SIZE} values",
            )
        if len(row) != HIDDEN_SIZE:
            raise EmbeddingError(
                "invalid_dimension",
                f"row {row_index} has {len(row)} values; the lock requires {HIDDEN_SIZE}",
            )
        values: list[float] = []
        for column, value in enumerate(row):
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                raise EmbeddingError(
                    "invalid_output",
                    f"row {row_index} column {column} is {type(value).__name__}, expected a number",
                )
            number = float(value)
            if not math.isfinite(number):
                raise EmbeddingError(
                    "non_finite_output",
                    f"row {row_index} column {column} is not finite ({number!r})",
                )
            values.append(number)
        norm = math.sqrt(math.fsum(value * value for value in values))
        if abs(norm - 1.0) > NORMALIZATION_TOLERANCE:
            raise EmbeddingError(
                "not_normalized",
                f"row {row_index} has L2 norm {norm:.6f}; the lock requires "
                f"1.0 ± {NORMALIZATION_TOLERANCE}",
            )
        validated.append(values)
    return validated


def _apply_environment() -> None:
    """Applies the offline switches and the explicit GPU choice at import.

    Setting ``CUDA_VISIBLE_DEVICES`` here (before anything imports torch) also
    keeps the parse/OCR side of this service off Strata's card: the Python
    process is configured for exactly one UUID, not for "whatever index 0 is".
    A malformed UUID fails here rather than letting a later CUDA call pick a
    default device.
    """

    os.environ.update(OFFLINE_ENV)
    if os.environ.get(GPU_UUID_ENV, "").strip():
        configured_device()


_apply_environment()
