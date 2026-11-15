# 模型 artifact 共享根实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: 使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 按任务执行本计划。步骤用 `- [ ]` 复选框跟踪。

**Goal:** 把 Docling artifact 与 BGE-M3 快照从实例数据根移到机器级共享根 `/data/llm/{docling,bge-m3}/<revision>/`，路径全部改由环境变量提供，代码中不再存在任何可用的默认路径。

**Architecture:** 两族模型各自一个共享根、按锁版 revision 分目录且不可变；运行时只读。代码侧把「根目录解析」从常量默认值改为环境变量解析（docling 有 `DOCLING_ARTIFACTS_PATH` 兜底，BGE 无），解析保持惰性（纯文本不需要模型）。provision 脚本进仓库，与 `LOCK.md` 的 revision 目录名由测试三处对齐。

**Tech Stack:** Python 3.12（uv 0.13.7，pytest，ruff）、bash 3.x（`curl`/`sha256sum`/`flock`）、NixOS（`/etc/nixos` 独立 host 任务）。

**Spec:** `docs/decisions/0001-machine-level-model-artifact-roots.md`（Accepted）与 `docs/rfcs/2026-11-15-docling-shared-artifacts.md`（Resolved）。执行者读两者后再动手。

## Global Constraints

- `KNOTED_INTELLIGENCE_MODELS_DIR` **必须整体移除**：任何模块都不再读它，设置它不产生任何效果。
- 代码中不得出现 `/data/knoted-dev/...` 或 `/data/llm/...` 作为路径默认值；错误消息只写变量名、规则与仓库内脚本名，不写部署绝对路径。
- 不改任何模型版本、revision、引擎选项：`models.py` 的 `OPTIONS_REVISION` / `parser_summary()` 与 `bge.py` 的 `OPTIONS_REVISION` / `MANIFEST_SCHEMA` 必须逐字符不变。
- 错误码固定：docling `artifacts_unset`，BGE `snapshot_unset`；两者都进现有的 `ParseError` / `EmbeddingError` 家族。
- 变量优先级固定：docling = `KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR` → `DOCLING_ARTIFACTS_PATH` → 失败；BGE = `KNOTED_INTELLIGENCE_BGE_M3_DIR` → 失败（无兜底）。
- revision 目录名固定：docling `heron-8f39ad3c_tf-fc0f2d45-fast_rapidocr-3.10.0`（14 个文件 / 733,054,339 B），BGE `bge-m3-5617a9f6-dense`（13 个文件 / 2,295,354,503 B）。
- 共享根权限固定：目录 `0755`、文件 `0644`、属主 host 用户、消费者只读。
- 迁移顺序固定：先复制、后切换、最后删除；**不留 symlink**。
- 测试前置：`pytest tests/test_t10.py tests/test_t11.py` 需要 `KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR` 与 `KNOTED_INTELLIGENCE_BGE_M3_DIR` 已导出（迁移完成后取值见 Task 5）。
- 本地门禁（规则 8）：`cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`（本任务不改 Rust，仍需证明未破坏）、`uv run pytest -q`、`uv run ruff check`。

## Review Focus

1. **未配置变量时纯文本解析必须仍成功**：`parse_document` 对 text 输入不能触碰模型根；任何在执行前就解析根目录的写法都会让 T09 的 `kind=parse` 用例变成 `artifacts_unset`。
2. **只设 `DOCLING_ARTIFACTS_PATH` 时必须可用**：这是非 Knoted 消费者共用的唯一入口；优先级写反等于共享契约失效。
3. **错误消息不得泄露绝对路径，且仍以锁版摘要开头**：`models_missing` / `artifacts_unset` / `snapshot_unset` 都属于会被写进执行记录 `error.message` 的文本。
4. **BGE 的运行期清单校验不能被破坏**：`bge.manifest()` 校验 `schema`/`revision`/`files`；生成脚本输出必须同形，否则嵌入直接 `manifest_mismatch`。
5. **`--verify` 遇篡改必须报错且不覆盖字节**：provision 脚本是共享根的唯一写入口，静默覆盖会污染所有消费者。

---

## 文件与接口

### 新增

| 路径 | 职责 |
| --- | --- |
| `services/intelligence/scripts/fetch-docling-artifacts.sh` | docling 共享根的 provision/校验入口。接口：`[<target-root>]`（默认 `/data/llm/docling`）、`--verify`（只校验）、`--print-revision-id`（只打印目录名）。写入前 `flock`，已存在且匹配则 `ok` 跳过，不匹配报错退出、不覆盖；生成 `<revision>/MANIFEST.json`（schema `knoted-docling-artifacts/2`）。 |
| `services/intelligence/scripts/fetch-bge-m3-artifacts.sh` | BGE 共享根的同一套入口（默认目标 `/data/llm/bge-m3`），`--verify` 额外用 `python3` 标准库重算 `model.safetensors` 的逐 tensor 摘要；生成同形 `MANIFEST.json`（schema 保持 `knoted-bge-m3-models/1`）。 |
| `services/intelligence/tests/test_artifact_scripts.py` | 两个脚本的 `--print-revision-id` 与 Python 派生函数、`LOCK.md` 记录值三处一致；`--verify` 对合成的坏目录报错且不改字节。 |

### 修改

| 路径 | 改动 |
| --- | --- |
| `services/intelligence/src/knoted_intelligence/parsing/models.py` | 删 `DEFAULT_MODELS_DIR`、`MODELS_DIR_ENV`、`ocr_dir()`；新增 `ARTIFACTS_DIR_ENV`、`UPSTREAM_ARTIFACTS_ENV`、`artifacts_dir()`、`revision_dirname()`；`docling_artifacts_path(root=None)`/`artifact_problems(root=None)`/`require_artifacts(root=None)` 改为 `root or artifacts_dir()`；`models_missing` 文案指向仓库内脚本；模块与常量注释去掉旧路径 |
| `services/intelligence/src/knoted_intelligence/parsing/docling_adapter.py` | 删除 `:139` 的提前解析；text 分支前不解析根；image/pdf 分支调用 `models.require_artifacts(models_dir)`；参数注释更新 |
| `services/intelligence/src/knoted_intelligence/executor.py` | `:719` 不再传 `models_dir=`（改传 `None`/省略） |
| `services/intelligence/src/knoted_intelligence/embedding/bge.py` | 删 `MODELS_DIR_ENV`、`DEFAULT_MODELS_DIR`、`MODEL_DIRNAME`、`models_dir()`；新增 `SNAPSHOT_DIR_ENV`、`snapshot_dirname()`；`model_dir()` 直读环境变量，未设置抛 `EmbeddingError("snapshot_unset", …)`；`EmbeddingError` docstring 的 code 列表补 `snapshot_unset` |
| `services/intelligence/src/knoted_intelligence/embedding/__init__.py` | 导出列表去掉 `DEFAULT_MODELS_DIR`/`models_dir`，加 `SNAPSHOT_DIR_ENV`/`snapshot_dirname` |
| `services/intelligence/tests/test_t10.py` | 6 处 `locked.default_models_dir()` 改为 `locked.artifacts_dir()`；`:316` 的 `MODELS_DIR_ENV` 改 `ARTIFACTS_DIR_ENV`；新增惰性解析、`artifacts_unset`、upstream 兜底与优先级用例 |
| `services/intelligence/tests/test_t11.py` | `:55`/`:412` 的 `bge.model_dir()` 走环境变量；新增 `snapshot_unset` 用例 |
| `services/intelligence/src/knoted_intelligence/parsing/LOCK.md` | §1 路径推导表（共享根 + revision 目录 + 变量优先级）、§4 说明 14/9、§7 安装指向仓库脚本、§8 增补证据 |
| `services/intelligence/src/knoted_intelligence/embedding/LOCK.md` | §1 路径与配置表（共享快照根 + revision 目录 + 专用变量、无兜底理由）、§2 说明目录名与清单 schema 不变 |
| `services/intelligence/README.md` | 配置表增删变量；运行节写明测试前置 export |
| `docs/plans/knoted-v1/development-and-deploy.md` | `:18`「OCR/嵌入模型」行改为两个共享根 + 变量 |

### 保留不动

保留不动：`REQUIRED_ARTIFACTS`（9 项）与 `bge.REQUIRED_FILES`（10 项）/`ALL_FILES`（13 项）及其 sha256；`config.py` 的 state/staging 默认值；执行协议、账本、准入上限、离线环境变量。

## 任务与依赖

| 任务 | 执行者 | 输入 / 依赖 | 输出 / 文件范围 | Worktree / 分支 | 验收 / 测试命令 | 可并行 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 docling 环境变量契约 | 实施 Agent | ADR 0001、RFC §2 | `parsing/models.py`、`docling_adapter.py`、`executor.py`、`tests/test_t10.py` | `.worktrees/model-artifact-code` / `feat/model-artifact-docling` | 见 Task 1 各步 | 与 2 并行 |
| 2 BGE 环境变量契约 | 实施 Agent | ADR 0001、RFC §2 | `embedding/bge.py`、`embedding/__init__.py`、`tests/test_t11.py` | 同 1（同分支，串行提交） | 见 Task 2 各步 | 与 1 并行（文件不重叠） |
| 3 provision 脚本 | 实施 Agent | Task 1、2 的 `revision_dirname()` | `scripts/*.sh`、`tests/test_artifact_scripts.py` | 同 1 | 见 Task 3 各步 | 依赖 1、2 |
| 4 文档同步 | 实施 Agent | Task 1–3 | 两份 `LOCK.md`、`README.md`、`development-and-deploy.md` | 同 1 | 见 Task 4 各步 | 依赖 1–3 |
| 5 host 迁移与声明 | 独立 host 任务 | Task 1–4 合并后的 SHA | `/etc/nixos` 与 `/data/llm/**` | `/etc/nixos` 自己的规则与 worktree | 见 Task 5 各步 | 依赖 4 |

任务 1、2 触碰不同文件，可由两个 Agent 并行；3、4 串行；5 必须在代码合并后进行，且不与其他应用提交混合。

---

## Task 1: docling artifact 根改为环境变量解析

**Files:** `services/intelligence/src/knoted_intelligence/parsing/models.py`、`.../parsing/docling_adapter.py`、`.../executor.py`、`services/intelligence/tests/test_t10.py`

**Interfaces（精确约定）：**

```python
# models.py
ARTIFACTS_DIR_ENV = "KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR"
UPSTREAM_ARTIFACTS_ENV = "DOCLING_ARTIFACTS_PATH"

def artifacts_dir() -> Path: ...            # 按上述优先级；都没有 → ParseError("artifacts_unset", msg)
def docling_artifacts_path(root: Path | None = None) -> Path: ...   # root or artifacts_dir()
def revision_dirname() -> str: ...          # "heron-8f39ad3c_tf-fc0f2d45-fast_rapidocr-3.10.0"
def artifact_problems(root: Path | None = None) -> list[str]: ...   # 不变签名
def require_artifacts(root: Path | None = None) -> Path: ...        # 不变签名
```

`revision_dirname()` 由既有常量派生，不新增字面量：

```python
return f"heron-{LAYOUT_REVISION[:8]}_tf-{TABLEFORMER_REVISION[:8]}-fast_rapidocr-{RAPIDOCR_VERSION}"
```

- [ ] **Step 1: 写失败测试** — 在 `tests/test_t10.py` 追加（同在同一个文件时，`locked` 即 `knoted_intelligence.parsing.models`，`ParseError`/`parse_document` 已在顶层导入）：

```python
def test_t10_6_artifacts_dir_requires_an_explicit_variable(monkeypatch):
    monkeypatch.delenv(locked.ARTIFACTS_DIR_ENV, raising=False)
    monkeypatch.delenv(locked.UPSTREAM_ARTIFACTS_ENV, raising=False)
    with pytest.raises(locked.ParseError) as excinfo:
        locked.artifacts_dir()
    assert excinfo.value.code == "artifacts_unset"
    message = excinfo.value.public_message
    assert str(locked.ARTIFACTS_DIR_ENV) in message
    assert "/data/" not in message          # 不泄露部署绝对路径

def test_t10_6b_upstream_variable_is_honoured_and_precedence_holds(monkeypatch, tmp_path):
    monkeypatch.setenv(locked.UPSTREAM_ARTIFACTS_ENV, str(tmp_path))
    monkeypatch.delenv(locked.ARTIFACTS_DIR_ENV, raising=False)
    assert locked.artifacts_dir() == tmp_path
    prefer = tmp_path / "preferred"
    monkeypatch.setenv(locked.ARTIFACTS_DIR_ENV, str(prefer))
    assert locked.artifacts_dir() == prefer

def test_t10_6c_revision_dirname_matches_the_lock():
    assert locked.revision_dirname() == "heron-8f39ad3c_tf-fc0f2d45-fast_rapidocr-3.10.0"
```

- [ ] **Step 2: 运行并确认失败** — `cd services/intelligence && uv run pytest tests/test_t10.py -k "t10_6" -q`，预期 `AttributeError: module ... has no attribute 'ARTIFACTS_DIR_ENV'`。
- [ ] **Step 3: 实现 models.py** — 按上面的 Interfaces 增删；`models_missing` 与 `artifacts_unset` 的消息以锁版摘要开头，正文只写变量名与 `services/intelligence/scripts/fetch-docling-artifacts.sh`；删除 `DEFAULT_MODELS_DIR`/`MODELS_DIR_ENV`/`ocr_dir()` 与相关注释里的旧路径。
- [ ] **Step 4: 运行测试** — `uv run pytest tests/test_t10.py -k "t10_6" -q`，预期 3 passed。
- [ ] **Step 5: 写惰性解析的失败测试** — 追加：

```python
def test_t10_seven_text_parse_needs_no_artifacts(tmp_path, monkeypatch):
    monkeypatch.delenv(locked.ARTIFACTS_DIR_ENV, raising=False)
    monkeypatch.delenv(locked.UPSTREAM_ARTIFACTS_ENV, raising=False)
    source = tmp_path / "note.txt"
    source.write_text("Knoted T09 input\nsecond line\n", encoding="utf-8")
    document = parse_document(source, options_revision="parse-v1")
    assert "second line" in document.full_text

def test_t10_seven_b_image_parse_without_artifacts_is_explicit(tmp_path, monkeypatch):
    monkeypatch.delenv(locked.ARTIFACTS_DIR_ENV, raising=False)
    monkeypatch.delenv(locked.UPSTREAM_ARTIFACTS_ENV, raising=False)
    image = tmp_path / "tiny.png"
    image.write_bytes(png_header(width=8, height=8))   # 复用本文件 :459 的 helper
    with pytest.raises(ParseError) as excinfo:
        parse_document(image, options_revision="parse-v1")
    assert excinfo.value.code == "artifacts_unset"
```

复用该文件已有的 `png_header()`（`test_t10.py:459`）与顶层导入的 `parse_document`/`ParseError`，不新增依赖。

- [ ] **Step 6: 运行并确认失败** — 同 Step 2 的选择器换成 `-k "t10_7"`，预期 `t10_7` 抛 `artifacts_unset`（证明现在解析过早）。
- [ ] **Step 7: 改为惰性解析** — `docling_adapter.parse_document` 删除提前的 `root = ...`，在 `if source_format == "text":` 之后、`models.require_artifacts(...)` 处传 `models_dir`；`executor.py:719` 删除 `models_dir=` 实参。另外同步：`test_t10.py:53-58` 的本地 `require_artifacts()` helper 与模块 docstring 里的 `KNOTED_INTELLIGENCE_MODELS_DIR` 字样。
- [ ] **Step 8: 运行受影响测试** — `uv run pytest tests/test_t09.py tests/test_t10.py -q`（含真实 artifact 用例，需已 export `KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR`），预期全绿。
- [ ] **Step 9: 提交** — 读 `git-commit` Skill 后 `git commit -m "refactor(parsing): 从环境变量解析 docling artifact 根"`。

## Task 2: BGE-M3 快照根改为环境变量解析

**Files:** `.../embedding/bge.py`、`.../embedding/__init__.py`、`services/intelligence/tests/test_t11.py`

**Interfaces：**

```python
# bge.py
SNAPSHOT_DIR_ENV = "KNOTED_INTELLIGENCE_BGE_M3_DIR"

def model_dir() -> Path: ...        # 直读环境变量；未设置 → EmbeddingError("snapshot_unset", msg)
def snapshot_dirname() -> str: ...  # "bge-m3-5617a9f6-dense"
```

- [ ] **Step 1: 写失败测试** — 在 `tests/test_t11.py` 追加：

```python
def test_t11_9_snapshot_dir_requires_the_variable(monkeypatch):
    monkeypatch.delenv(bge.SNAPSHOT_DIR_ENV, raising=False)
    with pytest.raises(bge.EmbeddingError) as excinfo:
        bge.model_dir()
    assert excinfo.value.code == "snapshot_unset"
    assert "/data/" not in excinfo.value.public_message

def test_t11_9b_snapshot_dirname_matches_the_lock():
    assert bge.snapshot_dirname() == "bge-m3-5617a9f6-dense"
```

- [ ] **Step 2: 运行并确认失败** — `uv run pytest tests/test_t11.py -k "t11_9" -q`，预期 `AttributeError`。
- [ ] **Step 3: 实现** — 删 `MODELS_DIR_ENV`/`DEFAULT_MODELS_DIR`/`MODEL_DIRNAME`/`models_dir()`；`model_dir()` 直读 `SNAPSHOT_DIR_ENV`；`EmbeddingError` docstring 的 code 列表补 `snapshot_unset`；`embedding/__init__.py` 的导入与 `__all__` 同步。
- [ ] **Step 4: 运行受影响测试** — `uv run pytest tests/test_t11.py -q`（需已 export `KNOTED_INTELLIGENCE_BGE_M3_DIR`），预期全绿，尤其 `manifest`/`artifact_problems` 相关用例不变。
- [ ] **Step 5: 全量回归** — `uv run pytest -q` 与 `uv run ruff check`，预期全绿、无 lint 错误。
- [ ] **Step 6: 提交** — `git commit -m "refactor(embedding): 从环境变量解析 BGE-M3 快照根"`。

## Task 3: provision 脚本进仓库

**Files:** `services/intelligence/scripts/fetch-docling-artifacts.sh`、`services/intelligence/scripts/fetch-bge-m3-artifacts.sh`、`services/intelligence/tests/test_artifact_scripts.py`

**Interfaces：**

```
fetch-docling-artifacts.sh [<target-root>] [--verify] [--print-revision-id]
fetch-bge-m3-artifacts.sh  [<target-root>] [--verify] [--print-revision-id]
```

- 默认 target：docling `/data/llm/docling`，BGE `/data/llm/bge-m3`；实际写入 `<target-root>/<revision-dirname>/`。
- `--print-revision-id` 只输出目录名（无换行以外任何东西），不触网、不建目录。
- `--verify` 只校验，不创建、不修改任何字节。
- 默认模式：`install -d -m 0755 <revision dir>`、`flock` 串行、逐文件 size+sha256 校验；已存在且匹配打印 `ok` 跳过；不匹配打印 `MISMATCH` 并以非零退出，**不覆盖**。
- 移植来源：现 host 脚本 `/data/knoted-dev/service/models/ocr/fetch-models.sh`（14 个文件表）与 `/data/knoted-dev/service/models/bge-m3/fetch-model.sh`（13 个文件表 + 逐 tensor 摘要）。

- [ ] **Step 1: 写失败测试** — 新建 `tests/test_artifact_scripts.py`：

```python
def test_revision_ids_agree_across_bash_python_and_lock():
    for script, expected in (
        ("fetch-docling-artifacts.sh", locked.revision_dirname()),
        ("fetch-bge-m3-artifacts.sh", bge.snapshot_dirname()),
    ):
        result = subprocess.run(
            ["bash", str(SCRIPTS / script), "--print-revision-id"],
            capture_output=True, text=True, check=True,
        )
        assert result.stdout.strip() == expected

def test_verify_reports_a_tampered_file(tmp_path):
    target = tmp_path / "docling"
    revision = target / locked.revision_dirname()
    revision.mkdir(parents=True)
    (revision / locked.RAPIDOCR_DICT).write_bytes(b"x")
    result = subprocess.run(
        ["bash", str(SCRIPTS / "fetch-docling-artifacts.sh"), str(target), "--verify"],
        capture_output=True, text=True,
    )
    assert result.returncode != 0
    assert "MISMATCH" in result.stdout + result.stderr
    assert (revision / locked.RAPIDOCR_DICT).read_bytes() == b"x"   # 未覆盖
```

- [ ] **Step 2: 运行并确认失败** — `uv run pytest tests/test_artifact_scripts.py -q`，预期脚本不存在。
- [ ] **Step 3: 移植两个脚本** — 保留原有的固定 revision、逐文件 sha256/size、`HF_ENDPOINT`/`MODELSCOPE_ENDPOINT` 镜像、`ok`/`MISMATCH` 语义；新增 `--verify`、`--print-revision-id`、`flock`、revision 目录与生成 `MANIFEST.json`（docling schema `knoted-docling-artifacts/2`，路径相对 revision 根；BGE 保持 `knoted-bge-m3-models/1` 同形字段）。
- [ ] **Step 4: 运行测试** — `uv run pytest tests/test_artifact_scripts.py -q`，预期全绿。
- [ ] **Step 5: 真实目录只读校验** — `bash services/intelligence/scripts/fetch-docling-artifacts.sh /data/knoted-dev/service/models/ocr --verify` 的等价形式不可直接套用（旧目录多一层 `docling/`），改为：先把旧 `docling/` 复制到临时目录 `<task tmp>/docling-rehearsal/<revision>/`，再用 `--verify` 校验，预期全部 `ok`。记录输出到 `docs/plans/knoted-v1/evidence/` 之外的任务证据目录。
- [ ] **Step 6: 提交** — `git commit -m "feat(scripts): 增加按 revision 目录 provision 的 artifact 脚本"`。

## Task 4: 文档与路径引用清零

**Files:** `parsing/LOCK.md`、`embedding/LOCK.md`、`services/intelligence/README.md`、`docs/plans/knoted-v1/development-and-deploy.md`

- [ ] **Step 1: 结构检查（预期失败）** — `grep -rn "/data/knoted-dev/service/models" services/intelligence/src services/intelligence/README.md docs/plans/knoted-v1/development-and-deploy.md`，预期仍命中。
- [ ] **Step 2: 改写两份 `LOCK.md`** — 路径推导表换成「共享根 + revision 目录 + 变量优先级」；docling §4 注明「provision 14 个 / 运行时必需 9 个」；BGE §1 注明专用变量与无兜底理由、§2 注明目录名与清单 schema 未变。
- [ ] **Step 3: 改写 `README.md`** — 配置表删除 `KNOTED_INTELLIGENCE_MODELS_DIR`、新增两个变量与 `DOCLING_ARTIFACTS_PATH` 兜底说明；「运行」节写明测试前置：

```bash
export KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR=/data/llm/docling/heron-8f39ad3c_tf-fc0f2d45-fast_rapidocr-3.10.0
export KNOTED_INTELLIGENCE_BGE_M3_DIR=/data/llm/bge-m3/bge-m3-5617a9f6-dense
```

- [ ] **Step 4: 改写 `development-and-deploy.md:18`** — 「OCR/嵌入模型」行改为两个共享根 + 变量名。
- [ ] **Step 5: 结构检查（预期通过）** — 重跑 Step 1 的 grep，预期**无输出**；再跑 `grep -rn "KNOTED_INTELLIGENCE_MODELS_DIR" services/intelligence docs` 只允许命中历史 RFC/ADR/计划。
- [ ] **Step 6: 提交** — `git commit -m "docs(intelligence): 同步共享根的路径推导与测试前置"`。

## Task 5: host 迁移与声明式权限（独立 host 任务）

前置：Task 1–4 的 PR 已合并，记录合并 SHA。

- [ ] **Step 1: 复制而非移动** — 以 host 用户执行两个脚本指向真实共享根，把旧树内容复制进 revision 目录（docling 去掉旧的一层 `docling/`）；`/data` 有多余空间，旧树保持可用。
- [ ] **Step 2: 校验** — `--verify` 对两棵树全绿；docling 额外核对 `LOCK.md` §4 的 9 行必需项；BGE 核对 `MANIFEST.json` 的 `revision` 与 `file_count`。
- [ ] **Step 3: 权限与可读性** — `stat -c '%A %U:%G' /data/llm/docling /data/llm/bge-m3` 应为 `drwxr-xr-x xingjian:...`；`sudo -u knoted find /data/llm -maxdepth 1 -type d -readable` 与逐文件可读检查通过。
- [ ] **Step 4: 设置变量** — 在服务的启动入口（T21/T70 建 unit 时落成 `Environment=`）设置两个变量；当前无单元，先把取值写进运维说明与 `/etc/nixos` 的选项描述。
- [ ] **Step 5: `/etc/nixos` 改动** — 按该仓库 AGENTS.md 建独立 worktree：`knoted.nix` 的 `dirs` 移除 `/models*`，新增 option 声明两个共享根（`0755`，属主 `cfg.user`）与当前 revision 目录；`AGENTS.md` 的 `knoted.*` 行同步描述。`nix eval` 该选项，确认只影响 Knoted。
- [ ] **Step 6: 删除旧树** — 新代码上线并验证通过后删除 `/data/knoted-dev/service/models/{ocr,bge-m3}`，确认实例数据根只剩 `state/staging/assets/logs`。
- [ ] **Step 7: 证据** — 把每一步命令与输出写入任务证据文件，并回填 Issue #32。

---

## 风险与人工决策

| 风险 | 触发信号 | 应对 | 决策负责人 |
| --- | --- | --- | --- |
| 未记录的消费者仍读旧路径 | 删旧树后出现 `models_missing` | 删树前 grep 全仓 + 询问；旧树保留到验证通过 | 人工 |
| 共享根被误写 | 文件 sha256 与锁版不符 | `--verify` 报 `MISMATCH` 且不覆盖；revision 目录不可变 | Agent |
| 解析过早解析根目录 | T09 用例失败 `artifacts_unset` | Review Focus 1；惰性解析测试 | Agent |
| BGE 清单 schema 被改 | 嵌入报 `manifest_mismatch` | 生成同形字段；Task 2 Step 4 断言 | Agent |
| host 仓库与 app 提交混合 | PR 同时含 `/etc/nixos` 改动 | Task 5 独立任务、独立 PR | 主 Agent |
| 两个 Agent 并行改同一文件 | `git status` 出现预期外文件 | 任务 1/2 文件不重叠；主 Agent 串行集成 | 主 Agent |

## 执行检查清单

- [ ] 需求、方案与验收明确（ADR 0001 + RFC 已合并）
- [ ] Worktree 路径与忽略规则通过
- [ ] 分任务实现与验证完成
- [ ] 子任务已审查并集成
- [ ] 整体质量检查与独立审查通过
- [ ] 已读取 git-commit，差异审核并提交
- [ ] 已按授权完成 PR/主线集成
- [ ] 主线验证通过
- [ ] 本任务 Worktree 和已合并分支安全清理

## 当前状态及证据

- 当前主要步骤：计划已落档，尚未开始实施。
- 已运行命令、结果、SHA/PR：ADR 与 RFC 已合并到 master，合并提交 `1e3d19c`（PR #35）、`dc2fe59`（PR #33）。
- 阻塞原因、保留路径和恢复条件：无阻塞；实施需先建 `.worktrees/model-artifact-code`。
- 计划变更：初始版本，2026-11-15。
