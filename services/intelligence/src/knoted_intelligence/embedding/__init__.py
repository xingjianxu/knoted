"""Dense BGE-M3 embeddings for the internal embed execution.

Only the dense head is implemented (T11); sparse and ColBERT are out of scope.
The revision, file digests and runtime versions are locked in ``bge.py`` and
``LOCK.md``.
"""

from .bge import (
    GPU_UUID_ENV,
    HIDDEN_SIZE,
    MAX_BATCH_SIZE,
    MAX_INPUT_BYTES,
    MAX_SEQ_LENGTH,
    MODEL_REPO,
    MODEL_REVISION,
    NORMALIZATION_TOLERANCE,
    OPTIONS_REVISION,
    OUTPUT_DTYPE,
    SNAPSHOT_DIR_ENV,
    TOKENIZER_REVISION,
    EmbeddingError,
    configured_device,
    embed_dense,
    loaded_contract,
    loaded_device,
    lock_summary,
    locked_combination_summary,
    locked_model_revision,
    manifest,
    model_dir,
    require_model_dir,
    snapshot_dirname,
    validate_embeddings,
)

__all__ = [
    "SNAPSHOT_DIR_ENV",
    "GPU_UUID_ENV",
    "HIDDEN_SIZE",
    "MAX_BATCH_SIZE",
    "MAX_INPUT_BYTES",
    "MAX_SEQ_LENGTH",
    "MODEL_REPO",
    "MODEL_REVISION",
    "NORMALIZATION_TOLERANCE",
    "OPTIONS_REVISION",
    "OUTPUT_DTYPE",
    "TOKENIZER_REVISION",
    "EmbeddingError",
    "configured_device",
    "embed_dense",
    "loaded_contract",
    "loaded_device",
    "lock_summary",
    "locked_combination_summary",
    "locked_model_revision",
    "manifest",
    "model_dir",
    "snapshot_dirname",
    "require_model_dir",
    "validate_embeddings",
]
