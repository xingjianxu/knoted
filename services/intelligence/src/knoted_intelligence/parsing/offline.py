"""Applies the offline Hugging Face / Transformers flags before docling imports.

Docling only reaches for a model hub when ``artifacts_path`` is unset, but the
flags are the second guard: with them a missing artifact is a hard failure
instead of a silent download at parse time. Importing this module applies them,
and :mod:`knoted_intelligence.parsing.docling_adapter` re-applies them before
every conversion.
"""

from __future__ import annotations

import os

#: Hub/telemetry switches that make any accidental network use fail loudly.
OFFLINE_ENV: dict[str, str] = {
    "HF_HUB_OFFLINE": "1",
    "HF_DATASETS_OFFLINE": "1",
    "TRANSFORMERS_OFFLINE": "1",
    "HF_HUB_DISABLE_TELEMETRY": "1",
}


def enforce_offline() -> None:
    """Sets the offline switches; idempotent, so callers may re-apply them."""

    os.environ.update(OFFLINE_ENV)


enforce_offline()
