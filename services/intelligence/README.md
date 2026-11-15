# knoted-intelligence

Internal execution service between the Rust worker and the parse/embed
backends. It speaks the contract in `api/intelligence.openapi.yaml`, listens on
`127.0.0.1:15603` by default and keeps its execution state in a dedicated SQLite
ledger (not the business database).

```bash
uv sync
uv run pytest -q
uv run knoted-intelligence            # uvicorn on KNOTED_INTELLIGENCE_HOST:PORT
```

Configuration is environment only; every default matches the deployment layout
under `/data/knoted-dev/service`:

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
