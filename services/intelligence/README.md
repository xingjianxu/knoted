# knoted-intelligence

Internal execution service between the Rust worker and the parse/embed
backends. It speaks the contract in `api/intelligence.openapi.yaml`, listens on
`127.0.0.1:15603` by default and keeps its execution state in a dedicated SQLite
ledger (not the business database).

```bash
uv sync

# Model roots are machine-level and shared; there is no default path. Point at
# the locked revision directories (parsing/LOCK.md, embedding/LOCK.md):
export KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR=/data/llm/docling/heron-8f39ad3c_tf-fc0f2d45-fast_rapidocr-3.10.0
export KNOTED_INTELLIGENCE_BGE_M3_DIR=/data/llm/bge-m3/bge-m3-5617a9f6-dense

uv run pytest -q                      # t10/t11 need the two exports above
uv run knoted-intelligence            # uvicorn on KNOTED_INTELLIGENCE_HOST:PORT
```

`tests/test_t09.py` (execution protocol, plain text) runs without the model
variables; `tests/test_t10.py` (Docling) fails with `artifacts_unset` and
`tests/test_t11.py` (BGE-M3) with `snapshot_unset` until they are exported. Both
shared roots are provisioned by `scripts/fetch-{docling,bge-m3}-artifacts.sh` and
are read-only for every consumer.

Configuration is environment only; every default matches the deployment layout
under `/data/knoted-dev/service` (state, staging, logs — not the model roots):

| Variable | Default | Purpose |
| --- | --- | --- |
| `KNOTED_INTELLIGENCE_STATE_DIR` | `/data/knoted-dev/service/state/intelligence` | SQLite ledger directory (`executions.db`). |
| `KNOTED_INTELLIGENCE_STAGING_DIR` | `/data/knoted-dev/service/staging` | `input/` and `output/` staging roots. |
| `KNOTED_INTELLIGENCE_MAX_SLOTS` | `2` | Concurrent child processes. |
| `KNOTED_INTELLIGENCE_HOST` | `127.0.0.1` | Bind address. |
| `KNOTED_INTELLIGENCE_PORT` | `15603` | Bind port. |
| `KNOTED_INTELLIGENCE_CANCEL_GRACE_SECONDS` | `5` | How long a cancellation lets a child honour SIGTERM before its process group gets SIGKILL. |
| `KNOTED_INTELLIGENCE_CHILD_HOLD_MS` | `0` | Delay the child before it computes; it exists so cancellation tests observe a live process group. |
| `KNOTED_INTELLIGENCE_CHILD_IGNORE_SIGTERM` | `0` | Makes the child ignore SIGTERM; it exists so the SIGKILL escalation is tested against a genuinely unresponsive child. |
| `KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR` | unset (**required** for OCR/PDF) | Shared Docling artifact root, `<revision>` under `/data/llm/docling` (`parsing/LOCK.md`). No default path. |
| `DOCLING_ARTIFACTS_PATH` | unset | Docling's own variable, accepted as a fallback so non-Knoted consumers share one contract. |
| `KNOTED_INTELLIGENCE_BGE_M3_DIR` | unset (**required** for embeddings) | Shared BGE-M3 snapshot root, `<revision>` under `/data/llm/bge-m3` (`embedding/LOCK.md`). No fallback. |
| `KNOTED_EMBED_GPU_UUID` | unset (CPU) | `nvidia-smi -L` UUID copied to `CUDA_VISIBLE_DEVICES` before torch loads; there is no default index. |
| `KNOTED_INTELLIGENCE_GPU_SLOTS` | `1` | Slots for parse + embed work. Both kinds acquire it before a child starts, so OCR and embeddings never resident together. |
| `KNOTED_INTELLIGENCE_DOCLING_DEVICE` | `cpu` | Explicit Docling device (`cpu`, `cuda`, `cuda:N`). `auto` is refused: it would pick index 0, Strata's card. |

Execution semantics, all verified by `tests/test_t09.py`:

- a repeated `execution_id` with the same `(input_sha256, source_version,
  options_revision, kind, input_object_id)` returns the stored record with
  `200` and never restarts the work; any different parameter is a `409`;
- every execution runs in its own child process group (`setsid`), so cancel
  sends `SIGTERM` to the whole group, escalates to `SIGKILL` after a grace
  period and only records `cancelled` once `wait()` confirmed the child exited
  (`error.message` carries the exit status);
- a restart marks `queued`/`running`/`cancel_requested` rows as `interrupted`, a
  retryable terminal state; the in-memory queue is gone, so a client must create
  a new `execution_id` for all three;
- `object_id` values are restricted to `^(?!\.\.?$)[A-Za-z0-9._-]+$`, 1—128
  bytes (the same limit as `MAX_OBJECT_ID_LEN` in
  `crates/knoted-core/src/files/staging.rs`), may not contain `..` and may not
  contain a path separator or a URL scheme; every resolved path — symlinks
  included — stays inside the staging input/output root; error messages name
  the field and the rule but never a filesystem path;
- a `deadline` that already passed and an `input_sha256` that does not match
  the staged input both end `failed` without ever starting a child.

A cancel request may spend the whole SIGTERM grace period plus the SIGKILL
budget before it answers, so the Rust client allows 30 seconds for it and treats
a timeout as *unconfirmed*, never as cancelled.

The compute handlers run the revision-locked backends: `parse` uses Docling and
RapidOCR (`src/knoted_intelligence/parsing/LOCK.md`), `embed` uses the local
dense BGE-M3 snapshot (`src/knoted_intelligence/embedding/LOCK.md`) and reads
the staged input as UTF-8 text with one text per non-blank line. Both reject
invalid output with a stable error code instead of publishing a partial result.
Parse and embed share `KNOTED_INTELLIGENCE_GPU_SLOTS` (default 1), so the two
model families are serialised on the one Python card.

Office parsing is available as the **pure library** `parsing.office.parse_office`
(T48 stage A); execution dispatch and Rust/Worker integration await T47/stage B.
It reads transitional DOCX/XLSX/PPTX OOXML with bounded ZIP/XML admission and
never launches Office, calculates formulas, loads external resources, or extracts
embedded objects. It reuses `ParsedDocument`/`ParsedBlock`; its options revision
binds the extraction policy and internal budgets.

DOCX extracts body paragraphs/tables in original order (including merged and
nested cells); unsupported revisions, fields, content controls and text boxes
fail explicitly. XLSX extracts visible sheets/rows/columns using actual sparse
cells, original names and A1 coordinates. Formulas use stored caches only, omit
missing caches and retain error caches; external/structured-reference formulas
are conservatively refused. Numeric/date values retain their OOXML stored text.
PPTX extracts slide-owned text boxes/groups/tables in XML z-order, including
hidden slides with their original 1-based ordinal. Charts/SmartArt and embedded
objects are refused. Headers/footers, comments/notes, inherited master/layout
text, images/OCR, formatting and presentation animations are outside this body
text policy. DOCX/XLSX have no invented page numbers; all Office bbox values are
null. A successful result means complete extraction within this declared subset,
not a full Office rendering.

Admission includes the existing 100 MiB input ceiling plus internal limits:
4096 parts, 32 MiB per expanded part, 128 MiB total expansion, 200:1 compression,
500,000 XML nodes, depth 64, 16 MiB XML/output text, 100,000 actual cells and
100,000 blocks. These conservative implementation choices may reject valid
large documents; refusals never return truncated results. `corrupt_input`,
`unsupported_format`, `input_too_large`, `office_budget_exceeded` and
`office_io_error` remain distinct safe parser codes. The execution supervisor,
not the pure function, must enforce the existing 900-second deadline in stage B.
See [T48 evidence](../../docs/plans/knoted-v1/evidence/t48-office.md) for exact
boundaries, budget rationale and verification status.
