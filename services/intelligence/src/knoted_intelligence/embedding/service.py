"""Read staged chunk identities and run a complete locked embedding batch."""

from __future__ import annotations

import json
from pathlib import Path
from uuid import UUID

from . import bge

OPTIONS_PREFIX = "embed-chunks-"
OPTIONS_REVISION = f"{OPTIONS_PREFIX}v1+bge-m3-{bge.MODEL_REVISION}-dense-{bge.HIDDEN_SIZE}"


def embed_chunks(path: Path, options_revision: str) -> dict:
    """Return ordered vectors for every chunk, or fail the whole execution."""
    if options_revision != OPTIONS_REVISION:
        raise bge.EmbeddingError("invalid_input", "unsupported chunk embedding revision")
    if path.stat().st_size > bge.MAX_INPUT_BYTES:
        raise bge.EmbeddingError("input_too_large", "chunk embedding manifest is too large")

    try:
        manifest = json.loads(path.read_bytes().decode("utf-8"))
        if not isinstance(manifest, dict) or set(manifest) != {"chunk_set_id", "chunks"}:
            raise ValueError("unexpected manifest fields")
        chunk_set_id = str(UUID(manifest["chunk_set_id"]))
        rows = manifest["chunks"]
        if not isinstance(rows, list) or not rows:
            raise ValueError("no chunks")
        if len(rows) > bge.MAX_BATCH_SIZE:
            raise bge.EmbeddingError("batch_too_large", "too many chunks for one execution")

        chunk_ids = []
        texts = []
        for row in rows:
            if not isinstance(row, dict) or set(row) != {"id", "text"}:
                raise ValueError("unexpected chunk fields")
            chunk_ids.append(str(UUID(row["id"])))
            text = row["text"]
            if not isinstance(text, str) or not text.strip():
                raise ValueError("chunk is empty")
            texts.append(text)
        if len(set(chunk_ids)) != len(chunk_ids):
            raise ValueError("duplicate chunk identity")
    except (ValueError, TypeError, AttributeError, RecursionError) as exc:
        # EmbeddingError inherits RuntimeError. The explicit batch error above
        # keeps its classification and never includes manifest content.
        raise bge.EmbeddingError("invalid_input", "invalid chunk embedding manifest") from exc

    vectors = []
    for start in range(0, len(texts), bge.BATCH_SIZE):
        batch = texts[start : start + bge.BATCH_SIZE]
        vectors.extend(bge.validate_embeddings(bge.embed_dense(batch), expected_count=len(batch)))
    return {
        "chunk_set_id": chunk_set_id,
        "chunk_ids": chunk_ids,
        "text_count": len(texts),
        "vectors": vectors,
    }
