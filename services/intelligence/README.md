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

## 固定 Python 产物（T47 Phase A）

`tools/dev/build-intelligence.sh <完整提交SHA> <新输出目录>` 从 `git archive`
读取已提交源码；`build-artifact.sh` 调用同一 helper 将 Python 纳入完整产物。
构建需要已安装的 Python 3.12（`uv python find --no-python-downloads`，也可通过
`KNOTED_BUILD_PYTHON` 指定真实解释器），不会下载解释器或安装模型。
设置 `KNOTED_BUILD_ROOT` 将构建目录、uv 缓存与临时目录放到任务目录。
依赖用 `uv export --locked --no-dev --no-editable --no-emit-project` 导出并带哈希安装；
不使用开发 venv。解释器、标准库、依赖和源码全部在 `share/knoted/intelligence/`
中，解引用文件链接后进入完整 `SHA256SUMS`；`BUILD.json` 记录解释器版本、实际
解释器 SHA256、源码 REVISION 与 uv.lock SHA256。重复输出目录拒绝覆盖。

运行入口 `bin/knoted-intelligence` 使用产物内解释器 `-I -B`，要求显式设置外部
`KNOTED_INTELLIGENCE_STATE_DIR`、`KNOTED_INTELLIGENCE_STAGING_DIR`、`XDG_CACHE_HOME`
与 `TMPDIR`。模型仍使用 ADR 0001 两个只读共享根，Docling 显式 CPU。
产物搬移后无需重写入口；它仍依赖宿主 loader/glibc 等 ELF 运行库，不能视作跨机器
独立系统闭包。`prepare` 要求 Python 必需文件存在，并对全部 bin/share 文件重算
完整清单，拒绝多文件、少文件、篡改和链接。

本阶段仅交付打包与 `parsing.pdf.parse_pdf_image` 的 PDF/JPEG/PNG/WebP 准入。
未将该入口接入执行账本或 Rust Worker，未在 process-compose 启用 intelligence；
超时、取消和计算进程退出证明需后续已批准协议与真实双端验证。不得据此启用旧
生命周期服务并声称 T47 完成。
