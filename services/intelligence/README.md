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

Execution semantics follow [ADR 0007](../../docs/decisions/0007-execution-exit-confirmation.md):

- Replaying the same execution ID requires the complete original request, including
  its original deadline. A different request returns 409. An expired first POST
  persists a failed, never-started execution; a delayed duplicate cannot revive it.
- The API holds an exclusive, non-inherited `state/service.lock` before recovering
  SQLite. A lightweight guardian owns one compute subtree. Durable intent/READY/GO
  registration precedes compute imports. API owner-pipe EOF triggers cleanup.
- The guardian uses a kernel pidfd identity, subreaper adoption and a single waiter.
  It terminates the complete descendant tree, including children that call setsid,
  and writes a versioned receipt only after ECHILD. A leader exit alone is insufficient.
- `compute_exit_confirmed` is true only for a terminal row with validated private
  evidence. Missing legacy evidence, a dead guardian or corrupt receipt blocks
  new admission and reports degraded. Restart marks unfinished rows interrupted;
  that state alone never releases resources. No stale PID is used for recovery signals.
- Rust persists each complete request in the current job lease before POST and
  settles all previous attempts before a replacement. Resource proof is independent
  of the original cancelled-only helper. A 404, timeout or cancel 409 is not proof.
- Object IDs remain local staging names. Input digest checks, admission limits and
  the 900-second execution deadline apply before publication; partial results are
  never published. Rust verifies manifest bytes and the current lease/source fence.

Cancellation waits up to 15 seconds for confirmation. Worker shutdown stops claims,
settles persisted attempts with a 60-second outer bound and retains unknown work.
The API closes owner pipes and waits at most 20 seconds; pc shutdown budgets are
75 seconds for Worker and 30 for intelligence (SIGTERM). Guardians start new
sessions, independently of the API process group. Uninterruptible descendants or
receipt-write failure remain unknown and block a later deployment.

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

## 固定 Python 产物与停机检查

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

`process-compose.yaml` 启动固定产物的 intelligence，Worker 在其 health=ok 后启动。
`service.sh` 用清理后的环境显式传入两个只读模型根、CPU 和空 GPU 变量；不传开发
测试钩子。父进程、guardian、compute 均使用相同产物解释器及 `-I -B`。

停写并停止本实例服务后，使用同一产物的私有检查命令：

```bash
TMPDIR=/外置临时目录 <产物>/share/knoted/intelligence/python/bin/python3.12 \
  -I -B -m knoted_intelligence.supervision <intelligence-state目录>
```

命令只读检查 `executions.db` 中 executions/supervision 的关联、版本、请求摘要、
所有权与完整退出凭证；不调用 `Ledger.open()`，不执行恢复写入。为避免 SQLite 在
冷备旁创建 WAL/SHM，它先核对源文件身份并复制 DB/WAL/SHM 到外置 TMPDIR，再读取
该临时快照。输出仅包含安全聚合和 execution/guardian 诊断标识，不输出随机 token、
请求正文或凭据。exit 0 才表示全部计算已确认退出；缺库、legacy 缺表、孤立监督记录、
损坏或 unknown 均非 0。新格式空库允许通过，缺库不是首次启用证明。

支持 SQLite `backup()` 生成的独立 `executions.db` 副本，不依赖原绝对路径或活服务。
原运行目录还有 `service.lock`；receipt 保存在 DB，不另建凭证文件。备份前必须关闭
所有写者并确认退出；检查命令不能替代服务停机或 guardian 证明。实时诊断如确需查询，
只读取 `execution_id, phase, guardian_pid, guardian_start`；`go` 是持久授权阶段，
观察真实 compute 还需 `staging/output/<execution_id>/child-status.json`。

仓库端到端测试使用随机 loopback、独立 PostgreSQL schema 和 TMPDIR；Rust 真实 HTTP
测试解释器可由 `KNOTED_TEST_PYTHON` 指定，否则使用 `UV_PROJECT_ENVIRONMENT/bin/python`
或本服务 `.venv/bin/python`。完整测试仍需上文两个模型根，显式 CPU。
