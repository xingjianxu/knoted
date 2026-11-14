"""Revision-locked Docling / RapidOCR artifacts, versions and parsing limits.

Everything here is derived from ``KNOTED_INTELLIGENCE_MODELS_DIR`` (default
``/data/knoted-dev/service/models``); no path points into a user home or a
per-task scratch directory. The values are the host facts verified in
``LOCK.md``: changing any of them is a new parsing option revision, not a
runtime choice.

Two kinds of constants live together on purpose:

* the **lock** — Docling/RapidOCR versions, model repo revisions, artifact file
  names, sizes and sha256, and the ``OPTIONS_REVISION`` string the worker pins;
* the **admission limits** — 100 MiB, 40M pixels, 500 PDF pages — with the
  checks that turn a violation into a :class:`ParseError`.

:class:`ParseError` is defined here (not in the adapter) because both the lock
checks and the Docling adapter raise it, and every message must carry the same
locked-combination summary.
"""

from __future__ import annotations

import os
import re
from pathlib import Path

#: Environment variable an operator uses to relocate the model tree.
MODELS_DIR_ENV = "KNOTED_INTELLIGENCE_MODELS_DIR"
#: Deployment default; the service runs on the host that owns this tree.
DEFAULT_MODELS_DIR = Path("/data/knoted-dev/service/models")
#: Model tree layout: ``<models>/ocr/docling`` is Docling's artifacts root.
OCR_DIRNAME = "ocr"
DOCLING_DIRNAME = "docling"

# -- inference device --------------------------------------------------------

#: Docling's inference device, explicitly configured. The default is ``cpu``:
#: the installed ``onnxruntime`` is the CPU wheel, and Docling's own default
#: ``auto`` picks ``cuda:0`` whenever torch can see a GPU. On the deployment
#: host index 0 is Strata's card, so an implicit choice either OOMs or steals
#: its memory. A GPU must be granted by the host (UUID at the process
#: boundary); nothing here selects a card by index default.
DOCLING_DEVICE_ENV = "KNOTED_INTELLIGENCE_DOCLING_DEVICE"
DEFAULT_DOCLING_DEVICE = "cpu"
#: Only an explicit ``cpu``/``cuda``/``cuda:N`` is accepted; ``auto`` is refused.
_DOCLING_DEVICE_PATTERN = re.compile(r"^cuda(?::\d+)?$")

# -- pinned component versions (host-verified) -------------------------------

DOCLING_VERSION = "2.150.1"
DOCLING_IBM_MODELS_VERSION = "4.0.4"
RAPIDOCR_VERSION = "3.10.0"
ONNXRUNTIME_VERSION = "1.32.0"
TORCH_VERSION = "2.15.0"
TRANSFORMERS_VERSION = "5.23.0"

# -- pinned model repositories and revisions ---------------------------------

LAYOUT_REPO = "docling-project/docling-layout-heron"
LAYOUT_REVISION = "8f39ad3c0b4c58e9c2d2c84a38465abf757272d8"
LAYOUT_ONNX_REPO = "docling-project/docling-layout-heron-onnx"
LAYOUT_ONNX_REVISION = "40bde044036bb181c130ddf6c51792187268748f"
TABLEFORMER_REPO = "docling-project/docling-models"
TABLEFORMER_REVISION = "fc0f2d45e2218ea24bce5045f58a389aed16dc23"
TABLEFORMER_MODE = "fast"
RAPIDOCR_REPO = "RapidAI/RapidOCR"
RAPIDOCR_REVISION = "v3.10.0"

#: The only accepted OCR configuration; the artifact layout below matches it.
RAPIDOCR_BACKEND = "onnxruntime"
RAPIDOCR_LANG = ("ch",)
RAPIDOCR_MODEL_SIZE = "small"

# -- artifact layout, relative to ``<models>/ocr/docling`` -------------------

LAYOUT_DIR = "docling-project--docling-layout-heron"
LAYOUT_CONFIG = f"{LAYOUT_DIR}/config.json"
LAYOUT_WEIGHTS = f"{LAYOUT_DIR}/model.safetensors"
LAYOUT_PREPROCESSOR = f"{LAYOUT_DIR}/preprocessor_config.json"
LAYOUT_ONNX_DIR = "docling-project--docling-layout-heron-onnx"
TABLEFORMER_FAST_DIR = "docling-project--docling-models/model_artifacts/tableformer/fast"
TABLEFORMER_FAST_WEIGHTS = f"{TABLEFORMER_FAST_DIR}/tableformer_fast.safetensors"
TABLEFORMER_FAST_CONFIG = f"{TABLEFORMER_FAST_DIR}/tm_config.json"
RAPIDOCR_DIR = "RapidOcr"
RAPIDOCR_DET = f"{RAPIDOCR_DIR}/PP-OCRv6_det_small.onnx"
RAPIDOCR_CLS = f"{RAPIDOCR_DIR}/ch_ppocr_mobile_v2.0_cls_mobile.onnx"
RAPIDOCR_REC = f"{RAPIDOCR_DIR}/PP-OCRv6_rec_small.onnx"
RAPIDOCR_DICT = f"{RAPIDOCR_DIR}/ppocrv6_dict.txt"

#: Files the parser needs, with the size and sha256 recorded by the host task.
#: Sizes are checked on every start (cheap); sha256 is verified by
#: ``/data/knoted-dev/service/models/ocr/fetch-models.sh`` and documented in
#: ``LOCK.md`` rather than re-hashed for every execution.
REQUIRED_ARTIFACTS: tuple[tuple[str, int, str], ...] = (
    (
        LAYOUT_CONFIG,
        3_268,
        "fdea30805ce2f5666b147fca941dcdd27ad468e27d6ed21902207d3da056a97d",
    ),
    (
        LAYOUT_WEIGHTS,
        171_658_996,
        "00333a43451945aaf89db8ca9c0a17e75d1537c17db60fdb91aa95f4c7929e0c",
    ),
    (
        LAYOUT_PREPROCESSOR,
        444,
        "cd38cd59999e7a95d68e487fbe5132df3d4e5c32a0836add57e6126ba0c4eaf1",
    ),
    (
        TABLEFORMER_FAST_WEIGHTS,
        145_453_276,
        "3119563aab5a7c96fda4d621119b63fd8806272b86c30936d15507616422f718",
    ),
    (
        TABLEFORMER_FAST_CONFIG,
        7_060,
        "dca6762508dddfae6d57d6cb4ef822c6000119dff0f3b6489db7413118c2622a",
    ),
    (
        RAPIDOCR_DET,
        9_929_594,
        "090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f",
    ),
    (
        RAPIDOCR_CLS,
        585_532,
        "e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c",
    ),
    (
        RAPIDOCR_REC,
        21_234_383,
        "6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884",
    ),
    (
        RAPIDOCR_DICT,
        74_947,
        "b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d",
    ),
)

# -- admission limits (card T10, indexing-and-rag "解析和GPU") ----------------

MAX_INPUT_BYTES = 100 * 1024 * 1024
MAX_IMAGE_PIXELS = 40_000_000
MAX_PDF_PAGES = 500
#: Upper bound for the parse artifact. The artifact carries one copy of the text
#: (the blocks, not a second `full_text`); 100 MiB of pathologically short lines
#: can still inflate the JSON, so the bound is enforced, not just documented.
MAX_ARTIFACT_BYTES = 128 * 1024 * 1024
#: Docling's page raster scale. Locked here because the PDF pixel check uses it.
DOCLING_IMAGES_SCALE = 1.0

#: The value the worker sends as ``options_revision`` for parse executions.
#: It names every component and revision that can change the output, so a
#: result produced under a different lock is never reused.
OPTIONS_REVISION = (
    "parse-v1"
    f"+docling-{DOCLING_VERSION}"
    f"+layout-{LAYOUT_REVISION}"
    f"+tableformer-{TABLEFORMER_REVISION}-{TABLEFORMER_MODE}"
    f"+rapidocr-{RAPIDOCR_VERSION}-{RAPIDOCR_REVISION}"
    f"+ocr-{RAPIDOCR_BACKEND}-{RAPIDOCR_MODEL_SIZE}-{RAPIDOCR_LANG[0]}"
    f"+onnxruntime-{ONNXRUNTIME_VERSION}"
)


class ParseError(RuntimeError):
    """A parsing refusal whose message is safe to hand back to the worker.

    ``code`` is stable (``corrupt_input``, ``unsupported_format``,
    ``input_too_large``, ``image_too_large``, ``too_many_pages``,
    ``models_missing``); ``detail`` never contains a filesystem path. Every
    instance appends :func:`locked_combination_summary` so a failure reports the
    lock instead of inviting a different component selection.
    """

    def __init__(self, code: str, detail: str) -> None:
        self.code = code
        self.detail = detail
        # The lock summary leads so no downstream truncation (records, logs,
        # terminals) can ever drop the versions and revisions.
        super().__init__(f"[locked: {locked_combination_summary()}] {code}: {detail}")

    @property
    def public_message(self) -> str:
        """The message the child process may copy into an execution record."""

        return str(self)


def locked_combination_summary() -> str:
    """Version + revision of every component that produced (or refused) a parse."""

    return (
        f"docling {DOCLING_VERSION}, docling-ibm-models {DOCLING_IBM_MODELS_VERSION}, "
        f"layout {LAYOUT_REPO}@{LAYOUT_REVISION}, "
        f"tableformer {TABLEFORMER_REPO}@{TABLEFORMER_REVISION} ({TABLEFORMER_MODE}), "
        f"rapidocr {RAPIDOCR_VERSION} "
        f"({RAPIDOCR_REPO}@{RAPIDOCR_REVISION}; {RAPIDOCR_BACKEND}/"
        f"{RAPIDOCR_MODEL_SIZE}/{'/'.join(RAPIDOCR_LANG)}), "
        f"onnxruntime {ONNXRUNTIME_VERSION}, torch {TORCH_VERSION}, "
        f"transformers {TRANSFORMERS_VERSION}"
    )


def parser_summary() -> dict[str, str]:
    """Machine-readable lock written into every parse artifact."""

    return {
        "docling": DOCLING_VERSION,
        "docling_ibm_models": DOCLING_IBM_MODELS_VERSION,
        "layout": f"{LAYOUT_REPO}@{LAYOUT_REVISION}",
        "tableformer": f"{TABLEFORMER_REPO}@{TABLEFORMER_REVISION}/{TABLEFORMER_MODE}",
        "rapidocr": RAPIDOCR_VERSION,
        "rapidocr_models": f"{RAPIDOCR_REPO}@{RAPIDOCR_REVISION}",
        "ocr": f"{RAPIDOCR_BACKEND}/{RAPIDOCR_MODEL_SIZE}/{'/'.join(RAPIDOCR_LANG)}",
        "onnxruntime": ONNXRUNTIME_VERSION,
        "docling_device": docling_device(),
        "torch": TORCH_VERSION,
        "transformers": TRANSFORMERS_VERSION,
        "options_revision": OPTIONS_REVISION,
    }


def docling_device() -> str:
    """The explicitly configured Docling device (``cpu`` unless overridden).

    ``auto`` is refused on purpose: with ``LD_LIBRARY_PATH`` set torch can see
    every card, and Docling would then pick index 0 — Strata's card. A GPU is
    only used when an operator names it (``KNOTED_INTELLIGENCE_DOCLING_DEVICE``
    = ``cuda`` or ``cuda:N``, where ``N`` is the index **inside** the process's
    ``CUDA_VISIBLE_DEVICES``).
    """

    raw = os.environ.get(DOCLING_DEVICE_ENV, "").strip()
    if not raw:
        return DEFAULT_DOCLING_DEVICE
    if raw == "cpu" or _DOCLING_DEVICE_PATTERN.fullmatch(raw):
        return raw
    raise ParseError(
        "invalid_device",
        f"{DOCLING_DEVICE_ENV} must be 'cpu', 'cuda' or 'cuda:N' with an explicit "
        f"device, got {raw!r}; automatic selection is refused because it picks "
        f"card index 0",
    )


def default_models_dir() -> Path:
    """The configured model tree, from ``KNOTED_INTELLIGENCE_MODELS_DIR``."""

    raw = os.environ.get(MODELS_DIR_ENV)
    if raw is not None and raw.strip():
        return Path(raw).expanduser()
    return DEFAULT_MODELS_DIR


def ocr_dir(root: Path | None = None) -> Path:
    return (root or default_models_dir()) / OCR_DIRNAME


def docling_artifacts_path(root: Path | None = None) -> Path:
    """Docling's ``artifacts_path`` (``<models>/ocr/docling``)."""

    return ocr_dir(root) / DOCLING_DIRNAME


def artifact_problems(root: Path | None = None) -> list[str]:
    """Relative descriptions of missing or wrongly-sized locked artifacts."""

    artifacts = docling_artifacts_path(root)
    problems: list[str] = []
    for relative, expected_size, _sha256 in REQUIRED_ARTIFACTS:
        candidate = artifacts / relative
        if not candidate.is_file():
            problems.append(f"{relative}: missing")
            continue
        actual = candidate.stat().st_size
        if actual != expected_size:
            problems.append(f"{relative}: expected {expected_size} bytes, found {actual} bytes")
    return problems


def require_artifacts(root: Path | None = None) -> Path:
    """Returns the artifacts root, raising :class:`ParseError` if it is incomplete."""

    problems = artifact_problems(root)
    if problems:
        raise ParseError(
            "models_missing",
            "the locked OCR artifacts are incomplete ("
            + "; ".join(problems)
            + "); provision them with .../models/ocr/fetch-models.sh as described in "
            "parsing/LOCK.md",
        )
    return docling_artifacts_path(root)


def check_input_size(size_bytes: int) -> None:
    """Refuses an input over 100 MiB before anything is read."""

    if size_bytes > MAX_INPUT_BYTES:
        raise ParseError(
            "input_too_large",
            f"input is {size_bytes} bytes; the limit is 100 MiB ({MAX_INPUT_BYTES} bytes)",
        )


def check_image_pixels(width: int, height: int, *, page: int | None = None) -> None:
    """Refuses an image over 40M pixels from its header, before decoding.

    ``page`` only labels a PDF page in the message; the limit is the same for
    a standalone image and for every rasterised PDF page.
    """

    pixels = width * height
    if pixels > MAX_IMAGE_PIXELS:
        subject = f"pdf page {page}" if page is not None else "image"
        raise ParseError(
            "image_too_large",
            f"{subject} is {width}x{height} = {pixels} pixels; the limit is "
            f"{MAX_IMAGE_PIXELS} pixels (40M)",
        )


def check_page_count(pages: int) -> None:
    """Refuses a PDF over 500 pages."""

    if pages > MAX_PDF_PAGES:
        raise ParseError(
            "too_many_pages",
            f"document has {pages} pages; the limit is {MAX_PDF_PAGES} pages",
        )


def check_artifact_size(size_bytes: int) -> None:
    """Refuses a parse artifact over 128 MiB with a stable error code."""

    if size_bytes > MAX_ARTIFACT_BYTES:
        raise ParseError(
            "output_too_large",
            f"parse artifact would be {size_bytes} bytes; the limit is "
            f"{MAX_ARTIFACT_BYTES // (1024 * 1024)} MiB ({MAX_ARTIFACT_BYTES} bytes)",
        )
