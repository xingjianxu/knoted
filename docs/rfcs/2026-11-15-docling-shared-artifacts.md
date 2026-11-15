# RFC: Docling 与 BGE-M3 模型数据改为机器级共享目录并按 revision 分层

- Date: 2026-11-15
- Status: Resolved
- Owner: 待指定（本文由当前会话 Agent 起草）
- Related issues / ADRs: [#32](https://github.com/xingjianxu/knoted/issues/32)、[PR #33](https://github.com/xingjianxu/knoted/pull/33)、[ADR 0001](../decisions/0001-machine-level-model-artifact-roots.md)
- Decision record: 已决定 → [ADR 0001：模型 artifact 采用机器级共享根](../decisions/0001-machine-level-model-artifact-roots.md)（Accepted，2026-11-15）

## 问题、目标与非目标

### 现状

Docling 解析所需的 artifact 是 layout-heron（transformers engine）、TableFormer v2.3.0（fast）、RapidOCR PP-OCRv6（det/cls/rec + 字典）。**运行时必需 9 个文件**（`models.REQUIRED_ARTIFACTS`）；**provision 脚本实际下载 14 个**，额外 5 个是未启用的 layout-onnx 变体（3）与 TableFormer accurate 变体（2），`MANIFEST.json` 与 `parsing/LOCK.md` §4 都按 14 记录。实测磁盘占用 733,054,339 B（733 MB / 699 MiB）。它们按**实例**存放：

```
/data/knoted-dev/service/models/            # KNOTED_INTELLIGENCE_MODELS_DIR (两族共用)
├── ocr/
│   ├── docling/                            # Docling artifacts_path
│   │   ├── docling-project--docling-layout-heron/
│   │   ├── docling-project--docling-layout-heron-onnx/   (已下载未启用)
│   │   ├── docling-project--docling-models/              (TableFormer fast + accurate)
│   │   └── RapidOcr/
│   ├── MANIFEST.json
│   └── fetch-models.sh
└── bge-m3/                                 # BGE-M3 快照, 13 个文件
    ├── (config/tokenizer/权重 10 个必需 + README/sparse_linear/colbert_linear)
    ├── MANIFEST.json                        (schema: knoted-bge-m3-models/1)
    └── fetch-model.sh
```

同一棵 `models/` 根同时承载 `<root>/ocr/docling`（解析）与 `<root>/bge-m3`（嵌入）——这正是“两个模型族耦合在一个根上”的来源。

代码中的路径是**硬编码默认值**加环境变量覆盖，共两处：

| 位置 | 内容 |
| --- | --- |
| `services/intelligence/src/knoted_intelligence/parsing/models.py:30` | `DEFAULT_MODELS_DIR = Path("/data/knoted-dev/service/models")` |
| `services/intelligence/src/knoted_intelligence/embedding/bge.py:51` | 同值（BGE-M3 用） |

消费链：`executor.py:719` → `parse_document(..., models_dir=parsing_models.default_models_dir())` → `docling_adapter` → `models.require_artifacts()` → 显式 `artifacts_path` 与 `DOCLING_ARTIFACTS_PATH` 回写。

同一根下还有 BGE-M3 稠密嵌入快照（`kind=embed`）：`BAAI/bge-m3@5617a9f61b028005a4858fdac845db406aefb181`，**13 个文件 / 2,295,354,503 B**，位于 `<models>/bge-m3`，`SentenceTransformer(<dir>)` 直接加载。它与 docling 的差别在于：运行时逐文件校验**存在性 + size + sha256**（10 个必需文件，每个进程加载前一次），并且 `bge.manifest()` 会交叉校验快照内 `MANIFEST.json` 的 `schema` / `revision` / `files`，不一致即 `manifest_mismatch`。也就是说这个清单是**运行时契约**，不像 docling 的清单只是来源证据。

### 三个实测到的问题

1. **不能共用。** 每个实例都要重复放置同一份 733 MB（14 个文件）的 docling 数据与 2.29 GB（13 个文件）的 BGE-M3 快照；非 Knoted 的消费者没有可依赖的机器级位置。本机已有 `/data/llm/` 作为共享模型根的先例（`/data/llm/strata`，82 GB，含 `bin/config/models/logs/state`），且 `/data/llm/docling` 已创建但空置。
2. **读权限已实际失效。** 现有树是 `0750 xingjian:users`，服务账号 `knoted`（uid 988, gid 982, 无附加组）不是属主也不在 `users` 组：

   ```text
   $ sudo -u knoted ls /data/knoted-dev/service/models/ocr/docling
   ls: cannot access '...': Permission denied
   ```

   权重不是机密，但当前权限模型让它对目标消费者不可读。
3. **默认值掩盖配置缺失。** 未设置 `KNOTED_INTELLIGENCE_MODELS_DIR` 时静默退回硬编码路径（`parsing/models.py:30`、`embedding/bge.py:51` 两处同值）。换机器或换布局时不报错、不提示，与仓库「配置只来自环境变量、失败要显式」的约定不一致。

### 目标

- Docling artifact 根由环境变量提供，代码中**不存在可用的默认路径**；未配置时以稳定错误码显式失败。
- `/data/llm/docling` 成为机器级共享根：按锁版 revision 分目录、目录不可变、对任意消费者可读，Knoted 只是消费者之一。
- 非 Knoted 的消费者无需了解 Knoted 命名空间即可定位同一份数据。
- **两个模型族都按同一套规则外置**：Docling artifact（`/data/llm/docling/<revision>`）与 BGE-M3 稠密嵌入快照（`/data/llm/bge-m3/<revision>`）均不再位于实例数据根，`KNOTED_INTELLIGENCE_MODELS_DIR` 彻底移除。
- 解析行为、`OPTIONS_REVISION`、锁版清单内容逐字符不变。

### 非目标

- 不改 Docling / docling-ibm-models / RapidOCR / TableFormer 的版本、revision、引擎或选项，不产生新的 `OPTIONS_REVISION`。
- 不把 Python 依赖搬进 Nix（`uv` + `uv.lock` 保持现状，理由见既有讨论：nixpkgs 的 docling 2.145.0 与锁定 2.150.1 等 5 个组件全部不一致）。
- 不改执行协议、账本状态机、准入上限、离线策略。
- 不改 BGE-M3（`BAAI/bge-m3@5617a9f61b028005a4858fdac845db406aefb181`）的模型 revision、tokenizer、dense 路径或依赖版本，不产生新的嵌入 `OPTIONS_REVISION`。BGE-M3 **在本次范围内**（2026-11-15 人工确认与 docling 一起迁），但不改它的文件集合、校验规则与加载方式。
- 不新建任何 systemd 单元或服务账号（服务本体属 T21/T70）。

## 现状和备选方案

三个维度各有候选，逐维比较。D1 的布局直接决定其余两维的自由度。

### 维度 1：共享根的布局

**方案 A：单一根目录，环境变量指向根**

```
/data/llm/docling/
├── docling-project--docling-layout-heron/
├── docling-project--docling-models/
├── RapidOcr/
└── MANIFEST.json
```

- 优点：与 Docling `artifacts_path` 语义 1:1 对应，实现最简单，路径最短。
- 缺点：**整机只能存在一个 revision**。`fetch-models.sh` 对已存在但 sha256 不符的文件**报错退出、拒绝覆盖**（故意的安全设计），因此换 revision 必须人工清空旧文件；清空期间**所有**消费者同时不可用，没有并行验证窗口。两个消费者需要不同 revision 时无解。
- 未知项：无。

**方案 B：按 revision 分目录，环境变量指向具体 revision（推荐）**

```
/data/llm/docling/
├── heron-8f39ad3c_tf-fc0f2d45-fast_rapidocr-3.10.0/    ← 不可变，14 文件 / 733 MB
│   ├── docling-project--docling-layout-heron/
│   ├── docling-project--docling-models/
│   ├── RapidOcr/
│   └── MANIFEST.json
└── <未来的新 revision>/                                 ← 共存，不动旧的
```

- 优点：多 revision 共存；每个消费者各自 pin 一个目录；升级 = provision 新目录 + 改环境变量，回退 = 把环境变量改回去；共享根的写操作只发生在**新**目录，不触碰在用目录。
- 缺点：路径多一层；provision 脚本要接受目标 revision 目录参数；消费者要显式知道自己在用哪个 revision（这其实是优点，但确实是配置项增加）。
- 未知项：revision 目录名的生成规则需要固定（见推荐方案的「命名」）。

**方案 C：保留单根，用 symlink 把共享目录挂进旧路径**

```
/data/knoted-dev/service/models/ocr/docling -> /data/llm/docling/<revision>
```

- 优点：零代码改动，最小风险。
- 缺点：路径仍是**隐含**写死的；两棵模型树（docling 与 bge-m3）仍然耦合在同一个 `models/` 根上，换布局时要维持两套 symlink；每个新消费者都要自己造 symlink 并知道这个约定。不满足「路径不写死」。

### 维度 2：环境变量契约

**A1：专用变量 + upstream 兜底（推荐）**

解析顺序 `KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR` → `DOCLING_ARTIFACTS_PATH` → 显式失败。

- 非 Knoted 消费者用 Docling 上游自己的变量名（`DOCLING_ARTIFACTS_PATH`，docling 库本身也认它），无需了解 Knoted 命名空间；Knoted 保留命名空间内的明确名字，便于在同一台机器上区分自己的配置与上游默认。
- 代价：两个名字都要在文档与测试里覆盖。

**A2：只用 `KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR`**

边界最清楚，但直接调 docling 的其他服务只能靠文档对齐，机器级契约多一个隐含约定。

**A3：只认 `DOCLING_ARTIFACTS_PATH`**

机器上只有一个变量名，与库行为一致。缺点：与既有 `KNOTED_INTELLIGENCE_*` 风格不一致，且现在代码是「显式传参 + 回写环境」两条腿，统一后少了一层 Knoted 命名空间。

### 维度 3：读策略

**D1：全体可读（推荐）** 目录 `0755`、文件 `0644`，属主可写，消费者只读。权重非机密，完整性由 sha256 保证；新增消费者不需要改 NixOS 组或 rebuild。

**D2：专用组 `0750`** 更紧，但每增加一个消费者都要改 `/etc/nixos` 的组与成员并 rebuild。本次实测已证明这类权限会静默失败（`knoted` 读不了 `0750 xingjian:users`），而数据本身不敏感。

### 已选组合

| 维度 | 选择 | 抉择人 |
| --- | --- | --- |
| 布局 | 方案 B（按 revision 分目录） | 人工，2026-11-15 会话 |
| 环境变量 | A1（专用变量 + upstream 兜底） | 人工，2026-11-15 会话 |
| 读策略 | D1（全体可读 0755/0644） | 人工，2026-11-15 会话 |
| 范围 | Docling artifact 与 BGE-M3 快照一起外置，两套共享根用同一套规则 | 人工，2026-11-15 会话 |
| 共享根属主 | host 用户（`xingjian`）拥有并执行 provision；不引入专用属主 | 人工，2026-11-15 会话 |
| 旧路径兼容 | 不留 symlink，直接切换 | 人工，2026-11-15 会话 |
| 共享根声明 | 由 `/etc/nixos` 的 tmpfiles 声明两个共享根与 revision 目录的权限 | 人工，2026-11-15 会话 |
| 预处理路径形状 | 两棵根平行：`/data/llm/docling` 与 `/data/llm/bge-m3`，不合并成 `/data/llm/models/*` | 人工，2026-11-15 会话 |

以下为 Agent 的默认判断，可被推翻：provision 脚本与 `MANIFEST.json` 模板**移入仓库**受审查（数据目录只放字节与生成物）；迁移采用**先复制、后切换、最后删除**，不留 symlink（当前无运行中的 Knoted 服务，只有 `knoted-api-key.service`，不存在需要兼容的常驻进程）；未配置环境变量时**fail-fast**，不静默退回旧路径。

## 推荐方案

### 1. 目录契约（机器级）

```text
/data/llm/docling/                           0755, 属主为 host 用户, 消费者只读
└── <revision-dirname>/                      0755, 内容不可变(只增不删)
    ├── docling-project--docling-layout-heron/
    ├── docling-project--docling-layout-heron-onnx/   (未启用变体, 随 provision 一起存在)
    ├── docling-project--docling-models/              (tableformer/fast + accurate)
    ├── RapidOcr/
    └── MANIFEST.json                        生成的清单: 逐文件 size + sha256 + 上游 revision
                                             (tree 合计 14 个文件 / 733 MB)
```

**revision 目录命名**（由锁版常量派生，不允许手写）：

```text
heron-<8 位 layout revision>_tf-<8 位 tableformer revision>-fast_rapidocr-<rapidocr 版本>
→ heron-8f39ad3c_tf-fc0f2d45-fast_rapidocr-3.10.0
```

派生函数放在 `parsing/models.py`，并由测试断言它与 `OPTIONS_REVISION` 的成分一致，避免目录名与锁版字符串各自漂移。

**文件集合**：revision 目录承载 provision 脚本的**全部 14 个文件**，不是运行时必需的 9 个。理由：脚本、`MANIFEST.json`（`artifact_count: 14`）与 `parsing/LOCK.md` §4 三处都把 14 当作该 revision 的完整内容，只搬 9 个就等于删除 `LOCK.md` 记录为在场的 34 MB 文件；将来启用 layout-onnx 或 accurate 变体时也不必重新 provision。运行时仍然只校验 9 个（`REQUIRED_ARTIFACTS` 不变）。

**BGE-M3 快照根**（同一套规则、独立共享根）：

```text
/data/llm/bge-m3/                            0755, 消费者只读
└── bge-m3-5617a9f6-dense/                   13 文件 / 2,295,354,503 B, 不可变
    ├── config.json / config_sentence_transformers.json / sentence_bert_config.json
    ├── modules.json / 1_Pooling/config.json
    ├── model.safetensors / tokenizer.json / tokenizer_config.json
    ├── special_tokens_map.json / sentencepiece.bpe.model
    ├── README.md / sparse_linear.pt / colbert_linear.pt    (计入清单, 运行时不读)
    └── MANIFEST.json                         生成的清单 (schema 不变)
```

- revision 目录名：`bge-m3-<8 位模型 revision>-dense` → `bge-m3-5617a9f6-dense`，由 `bge.revision_dirname()` 派生，`fetch-bge-m3-artifacts.sh --print-revision-id` 输出相同值。
- 快照目录**直接**就是 `SentenceTransformer(<dir>)` 的加载路径（与现状一致），不再拼 `bge-m3/`；因此“变量一改、布局不变”对两个模型族都成立。
- 快照清单 `MANIFEST.json` 的 **schema 保持 `knoted-bge-m3-models/1` 不变**：与 docling 的清单不同，它**在运行时被校验**（`bge.manifest()` 核对 schema、`revision`、`files`），升 schema 就必须同步改校验代码，而本次不需要改校验语义。生成脚本必须输出与现有字段同形的内容（`schema`/`repo`/`revision`/`files`/`file_count`/`total_bytes` 必备）。

**不可变性**：目录内容只增不删。同一 revision 重复 provision 是幂等 no-op；文件 sha256 不符时按现有语义报错退出，**不覆盖**。provision 用 `flock` 串行化，避免两个消费者同时写同一新目录。

### 2. 环境变量契约（两个模型族）

```python
# parsing/models.py
ARTIFACTS_DIR_ENV = "KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR"
UPSTREAM_ARTIFACTS_ENV = "DOCLING_ARTIFACTS_PATH"   # docling 自己的变量, 作为共享契约

def artifacts_dir() -> Path:          # 取代 default_models_dir()/docling_artifacts_path()
    for name in (ARTIFACTS_DIR_ENV, UPSTREAM_ARTIFACTS_ENV):
        raw = os.environ.get(name, "").strip()
        if raw:
            return Path(raw).expanduser()
    raise ParseError("artifacts_unset", "...")   # 消息只写变量名与规则, 不写绝对路径

# embedding/bge.py
SNAPSHOT_DIR_ENV = "KNOTED_INTELLIGENCE_BGE_M3_DIR"

def model_dir() -> Path:              # 取代 models_dir()/MODEL_DIRNAME 拼接
    raw = os.environ.get(SNAPSHOT_DIR_ENV, "").strip()
    if not raw:
        raise EmbeddingError("snapshot_unset", "...")
    return Path(raw).expanduser()
```

- **docling**：新错误码 `artifacts_unset`，归入现有 `ParseError` 家族；`public_message` 规则不变（锁版摘要开头、不泄露部署绝对路径）。
- **BGE-M3**：新错误码 `snapshot_unset`，归入 `EmbeddingError` 家族；**没有 upstream 兜底**——sentence-transformers 只提供 `SENTENCE_TRANSFORMERS_HOME` 这类 HF 缓存根，语义是缓存目录而不是已经转换好的本地快照，拿它做兜底会引入与锁定 revision 不一致的风险。因此 BGE 只认专用变量，文档里写清楚原因。
- `models.py` 中删除 `DEFAULT_MODELS_DIR`、`MODELS_DIR_ENV`、`ocr_dir()`；`docling_artifacts_path(root)` 保留为**显式根入口**（`root` 为空时才走惰性环境变量解析），不再拼接 `ocr/docling` 两段。`REQUIRED_ARTIFACTS` 的相对路径**不变**（它们本来就相对 artifacts 根）。
- `bge.py` 中删除 `MODELS_DIR` 路径默认值、`models_dir()`、`MODEL_DIRNAME` 拼接；`model_dir()` 改为直接读 `KNOTED_INTELLIGENCE_BGE_M3_DIR`。`REQUIRED_FILES`、`ALL_FILES`（13 项）、`MANIFEST_SCHEMA` 与逐文件 sha256 校验**全部不变**。
- 代码注释与错误消息中出现的 `/data/knoted-dev/...` 字面量一并删除（`models.py` 的模块注释、`REQUIRED_ARTIFACTS` 注释、`require_artifacts` 的 `models_missing` 正文目前都写着旧的 fetch 脚本绝对路径，改为写「仓库内 provision 脚本 + LOCK.md 的路径推导规则」）。
- `docling_adapter` 继续显式传 `artifacts_path` 并回写 `DOCLING_ARTIFACTS_PATH`（两条腿都保留：显式传参保证行为，回写环境保证库内部任何读取路径一致）。
- `executor.py:719` 不再传 `parsing_models.default_models_dir()`，改传 `None`，由适配器在使用处解析；不再存在「默认值」这条分支。
- **解析必须保持惰性**：`docling_adapter.parse_document` 目前在 `:139` 于 `_detect_format` 之前就解析根目录，纯文本走 `:140-143` 的短路分支、完全不需要模型（注释明写「also works for the T09 fixture」）。改为：纯文本分支之前**不**解析 artifact 根，只有确定 `source_format` 为 `image`/`pdf` 后才调用 `models.require_artifacts()` 并解析环境变量。这样未配置新变量时纯文本解析仍然成功（T09 的 `kind=parse` 用例不需要新变量），OCR/PDF 解析则显式以 `artifacts_unset` 失败。
- `parse_document(..., models_dir=...)` 这个显式参数**保留**（它是已文档化的公开参数，测试用它注入根目录）；`models_dir=None` 时走「惰性环境变量解析」，绝不回退到任何常量路径。
- 回归测试新增一条：未设置两个环境变量时，纯文本 `kind=parse` 经执行协议成功；同一环境下图片输入得到 `artifacts_unset`。

### 3. 读策略

- `/data/llm/docling` 与 `/data/llm/bge-m3` 两个共享根及其各 revision 目录 `0755`，artifact 文件 `0644`，属主为 host 用户（`xingjian`），由属主执行 provision。
- 消费者（`knoted` 及其他服务账号）只有读权限；共享根内不出现任何凭据。
- 两个共享根与 revision 目录的权限由 `/etc/nixos` 声明式保证（见 §6），不依赖手工 `mkdir` 的偶然正确性。

### 4. provisioning 归属

- **两套脚本，同一个契约**，都放在 `services/intelligence/scripts/`：
  - `fetch-docling-artifacts.sh`（由 host 的 `fetch-models.sh` 演进）：目标根 `/data/llm/docling`，revision 目录名 `heron-…`。
  - `fetch-bge-m3-artifacts.sh`（由 host 的 `fetch-model.sh` 演进）：目标根 `/data/llm/bge-m3`，revision 目录名 `bge-m3-…-dense`。BGE 脚本需要 `python3` 标准库重算 `model.safetensors` 的逐 tensor 摘要（现有实现已如此），因此它依赖 `python3`，但**不依赖 venv、不依赖本项目包**。
- 两个脚本都支持 `--verify`（逐文件 size + sha256 校验、不修改任何字节）与 `--print-revision-id`。
- **两个脚本都不依赖 venv 或本项目包**：provision 是纯数据操作，可能在 `uv sync` 之前或在只装了基础工具的机器上执行。docling 脚本只需 `bash` + `curl` + `sha256sum`；BGE 脚本额外需 `python3` 标准库（现有 `fetch-model.sh` 已如此，用于逐 tensor 摘要）。revision 目录名由脚本自己从内嵌的 revision/版本常量派生，无需手写，也无需 import 包。
- **单一来源用测试保证**：两个脚本各自的 `--print-revision-id` 输出，必须 == 对应 Python 派生函数（`models.revision_dirname()` / `bge.revision_dirname()`）== 对应 `LOCK.md` 里记录的目录名。三处漂移会在 pytest 里直接失败。
- 数据目录内只保留脚本生成的 `MANIFEST.json` 与 artifact 字节；脚本本身受版本控制与评审。
- 保留现有语义：固定 revision 下载、逐文件 sha256 + size 校验、已匹配即 `ok` 跳过、不匹配报错不覆盖、docling 脚本支持 `HF_ENDPOINT` / `MODELSCOPE_ENDPOINT` 镜像。

### 5. 迁移步骤（host 任务，独立于代码提交）

顺序基线：**先复制、后切换、最后删除**。733 MB 在 22 TB 的 `/data` 上复制成本可忽略，因此不需要「代码先或 host 先」的取舍窗口：copy 阶段旧树原样保留，旧代码（读 `<models>/ocr/docling`）继续可用；新代码上线并验证通过后才删旧树。

1. `install -d -m 0755 /data/llm/docling/heron-8f39ad3c_tf-fc0f2d45-fast_rapidocr-3.10.0`。
2. **复制**（`cp -a`，非移动）`docling/` 下的完整子树到新 revision 目录根：`docling-project--docling-layout-heron/`、`docling-project--docling-layout-heron-onnx/`、`docling-project--docling-models/`（含 `fast/` 与 `accurate/`）、`RapidOcr/`，共 14 个文件；目录层级去掉旧的 `docling/` 一层。
3. 用 `fetch-docling-artifacts.sh --verify` 重算全部 14 个文件的 size + sha256：与脚本内嵌表、生成后的 `MANIFEST.json` 双向比对；再对 `LOCK.md` §4 的 9 行必需项单独比对。
4. BGE-M3 同样 **复制** 快照的 13 个文件到 `/data/llm/bge-m3/bge-m3-5617a9f6-dense`（2,295,354,503 B），再用 `fetch-bge-m3-artifacts.sh --verify` 校验（含 `model.safetensors` 的逐 tensor 摘要），并确认生成的 `MANIFEST.json` 仍为 `knoted-bge-m3-models/1`、`revision` 为 `5617a9f61…`。
5. 在启动进程处设置 `KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR` 与 `KNOTED_INTELLIGENCE_BGE_M3_DIR`（当前无 user unit，先写入运维/启动说明、`services/intelligence/README.md` 与 `/etc/nixos` 的选项描述；T21/T70 建单元时落成 `Environment=`）。
6. 新代码上线并验证通过后，删除旧 `models/ocr/` 与 `models/bge-m3/` 子树（连同 `fetch-models.sh`、`fetch-model.sh`、旧 `MANIFEST.json`），确认实例数据根只剩下 state/staging/assets/logs。
7. `sudo -u knoted` 与另一名非属主账号各读一遍**两棵树**的全部 artifact。

**回退**：新代码回退到旧版本时，去掉环境变量即可回到旧树（旧树在步骤 5 之前一直在）；host 侧回退 = 把新目录整体移回，或直接用仍存在的旧树。

### 6. `/etc/nixos` 侧（独立 host 任务，不与其他应用提交混合）

- `features/server/knoted.nix` 的 `dirs` 移除 `/models`、`/models/ocr`、`/models/bge-m3`（两棵模型树都不再位于实例数据根），并删除默认值里对应的条目与描述。
- `AGENTS.md` 中 `knoted.*` 那一行的「模型权重由 Knoted 的 host 任务预拉到 dataRoot/models/」描述要改写为「Docling artifact 在 `/data/llm/docling/<revision>`、BGE-M3 快照在 `/data/llm/bge-m3/<revision>`」。
- **新增 tmpfiles 声明**（人工已确认）：两个共享根与当前锁版 revision 目录由模块声明 `0755`、属主 `cfg.user`，位置可以是新增的 module option（例：`artifactRoots = [ "/data/llm/docling" "/data/llm/bge-m3" ]`）而不写死常量。未启用的新 revision 目录不在声明内，由 provision 脚本自己 `install -d -m 0755`。

## 影响与迁移

### 兼容性

- **`OPTIONS_REVISION` 不变**，锁版摘要字符串不变，`parser_summary()` 输出不变 → 下游「revision 不同则重解析」的判断逻辑无需改动。
- `KNOTED_INTELLIGENCE_MODELS_DIR` **彻底移除**（两个模型族都外置），替换为 `KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR`（+ `DOCLING_ARTIFACTS_PATH` 兜底）与 `KNOTED_INTELLIGENCE_BGE_M3_DIR`。这是对运维契约的**破坏性变更**：设旧变量不再有任何效果，不会静默生效或部分生效；需在 `README.md` 配置表与两份 `LOCK.md` 的「路径推导」节显式标注。
- BGE 侧其余不变：`REQUIRED_FILES`（运行时校验的10 个）、`ALL_FILES`（13 个）、逐文件 sha256 校验、`MANIFEST_SCHEMA = knoted-bge-m3-models/1`、`OPTIONS_REVISION = embed-v1+bge-m3-…-dense-1024` 全部保持原值；`bge.manifest()` 继续交叉校验 schema/revision/files，因此生成的清单必须与旧字段同形。
- **开发/测试前置条件**：`tests/test_t10.py` 现有 6 处经 `locked.default_models_dir()` 解析根目录（`:57,73,157,252,270,371`），`:316` 还把 `MODELS_DIR_ENV` 指向空目录来制造 `models_missing`。该函数删除后，这些用例改为调用环境变量驱动的 `artifacts_dir()`（或显式传根），因此 **`uv run pytest tests/test_t10.py` 需要先 export 新变量**；`services/intelligence/README.md` 必须把它写成明确前置条件并给出本机取值。T09（纯文本解析）按「惰性解析」设计**不需要**该变量：合并门禁对模型数据的依赖只落在本来就依赖真实 artifact 的 T10/T11 用例上。
- **`MANIFEST.json` 从手工文件变为生成物**：schema 从 `knoted-ocr-models/1` 升为 `knoted-docling-artifacts/2`；artifact 路径改为相对 revision 目录（去掉旧的 `docling/` 前缀）；删掉旧 `env` 块里的绝对路径 `DOCLING_ARTIFACTS_PATH`（只记录变量名与派生规则，不记录部署路径）。运行时**不**校验该文件（与 `bge.py` 的 manifest 交叉校验不同），因此它只是来源证据，重写不影响解析路径。

### 另需改动的文件（完整清单）

| 文件 | 改动 |
| --- | --- |
| `parsing/models.py` | 删 `DEFAULT_MODELS_DIR`/`MODELS_DIR_ENV`/`ocr_dir`；新增 `ARTIFACTS_DIR_ENV`、`UPSTREAM_ARTIFACTS_ENV`、`artifacts_dir()`、`revision_dirname()`；新错误码 `artifacts_unset`；清掉注释里的旧绝对路径 |
| `parsing/docling_adapter.py` | 根目录解析改为惰性；`models_dir=None` 时由使用处调用 `artifacts_dir()` |
| `executor.py:719` | 不再传默认根目录（改传 `None`） |
| `parsing/LOCK.md` | §1 路径推导表重写（共享根 + revision 目录 + 变量优先级）；§4 说明 14/9 的区别；§7 安装/修复指向仓库内脚本；§8 增补本轮证据 |
| `embedding/bge.py` | 删路径默认值、`models_dir()`、`MODEL_DIRNAME` 拼接；`model_dir()` 直读 `KNOTED_INTELLIGENCE_BGE_M3_DIR`；新错误码 `snapshot_unset`；`REQUIRED_FILES`/`ALL_FILES`/`MANIFEST_SCHEMA`/`OPTIONS_REVISION` 与校验逻辑不动 |
| `embedding/LOCK.md` | §1 路径与配置表重写（共享快照根 + revision 目录 + 专用变量、无 upstream 兜底的理由）；§2 说明 revision 目录名与清单 schema 不变 |
| `services/intelligence/README.md` | 配置表：新增变量行、删旧默认路径描述；运行节写明测试前置 export |
| `services/intelligence/scripts/fetch-docling-artifacts.sh` | 新增（由 host 脚本演进）：`--print-revision-id`、`--verify`、按 revision 目录 provision、生成 `MANIFEST.json`（schema 升 `knoted-docling-artifacts/2`） |
| `services/intelligence/scripts/fetch-bge-m3-artifacts.sh` | 新增（由 host `fetch-model.sh` 演进）：`--print-revision-id`、`--verify`（含逐 tensor 摘要）、按 revision 目录 provision、生成同形 `MANIFEST.json`；依赖 `python3` 标准库，不依赖 venv |
| `tests/test_t10.py` | 6 处根目录解析改写；`:316` 的 `MODELS_DIR_ENV` 改为 `ARTIFACTS_DIR_ENV`；新增惰性解析、`artifacts_unset`、revision 目录名一致性用例 |
| `tests/test_t11.py` | `:55`、`:412` 的 `bge.model_dir()` 改为环境变量驱动；`snapshot_unset` 用例；BGE revision 目录名一致性用例 |
| `docs/plans/knoted-v1/development-and-deploy.md:18` | 「OCR/嵌入模型」行改为共享根 + 变量说明 |
| `/etc/nixos`（host 仓库） | `knoted.nix` 的 `dirs`、`AGENTS.md` 的 `knoted.*` 描述、共享根 tmpfiles（若采纳开放问题 4） |

不改：`config.py:23,25` 的 state/staging 默认值（同族问题但属实例本地状态，不是共享 artifact）；`crates/`、`api/`、`apps/`、`packages/` 全无涉及。

### 数据与安全

- 733 MB + 2.29 GB 由「每实例一份」变为「整机各一份」，`/data` 现用 541 GB / 22 TB，容量无压力。
- 共享根对所有本机账号可读：权重不含凭据，完整性由 sha256 保证；**任何消费者不得把 token、密钥写入该目录**。顺带发现 `/data/llm/download.sh` 含明文 HuggingFace token（位于仓库之外，不在本次改动范围），建议轮换并改为从环境变量读取；本方案的任何提交、文档、Issue、PR、日志都不得包含该值。
- 读权限从「服务账号不可读」变为「可读」，是本次修复的实际缺陷之一。

### 运维与性能

- 解析性能不变：仍然从本地文件系统读同一批文件；多消费者只读共享还带来 page cache 复用（不承诺量化收益，本次不做性能测试）。
- 新增运维面：provision 变成「按 revision 装到共享根」，需要一次 `flock` 串行化与一份目录命名规则说明。
- 回退成本低：单变量切换。

### 阶段划分

1. **RFC 评审与 ADR**（本文）：人工确认「已选组合」四行（布局/变量/权限/范围），Resolved 后记 Accepted ADR。
2. **代码任务**：`parsing/models.py` / `docling_adapter.py` / `embedding/bge.py` / `executor.py` / 两份 `LOCK.md` / README / 测试，TDD；`cargo` 侧不受影响，Python 侧跑 `pytest`、`ruff`。建议拆两个可独立合并的提交（docling、BGE），但同一 PR 交付以免旧变量被提前删除。
3. **host 任务**：搬迁 + 权限 + `/etc/nixos` 描述更新，与代码任务分开提交。
4. **收尾**：旧树清理与旧路径引用清零的验证。

## 验证与开放问题

### 试验与测试（实施阶段）

- `test_t10` / `test_t11` 全绿；新增用例覆盖：未设置变量时 OCR/PDF 得到 `artifacts_unset`、嵌入得到 `snapshot_unset`；只设 `DOCLING_ARTIFACTS_PATH` 时 docling 可用；两个 docling 变量都设时专用变量优先；错误消息不含绝对路径。
- revision 目录名三处一致性（两个模型族各一组）：`bash scripts/fetch-docling-artifacts.sh --print-revision-id` / `fetch-bge-m3-artifacts.sh --print-revision-id` == 对应 Python 派生函数 == 两份 `LOCK.md` 记录的目录名，且成分与各自的 `OPTIONS_REVISION` 一致。
- provision 脚本：`--verify` 对 14（docling）/ 13（BGE）个文件逐个 size + sha256，BGE 额外校验逐 tensor 摘要；重复执行幂等 no-op；篡改一个字节后报错且**不覆盖**；并发靠 `flock` 串行。
- 迁移验证：两棵树搬迁前后逐文件 sha256 + size 比对生成的 `MANIFEST.json`，再对 `LOCK.md` §4/§2 的必需项单独比对；`sudo -u knoted` 与非属主账号各读一遍两棵树的全部 artifact。
- 结构检查：`grep -rn "/data/knoted-dev/service/models" services docs` 在改动后只剩历史记录类文档。

### 已决定的事项（原先的开放问题）

1. **共享根属主** → host 用户（`xingjian`）。provision 以属主身份执行；不引入 `docling-data` 类专用属主（D1 下消费者只需读）。
2. **旧路径是否留 symlink** → **不留**，直接切换；旧树在验证通过后删除。依赖旧路径的未记录消费者不存在（当前只有 `knoted-api-key.service`，无 Python 常驻单元）。
3. **`/etc/nixos` 的 tmpfiles** → **声明**两个共享根与当前 revision 目录（`0755`，属主 host 用户）；新 revision 目录由 provision 脚本创建。
4. **两棵根的路径形状** → 平行：`/data/llm/docling` 与 `/data/llm/bge-m3`，不合并成 `/data/llm/models/*`。
5. **ADR** → 已写入并标记 Accepted：`docs/decisions/0001-machine-level-model-artifact-roots.md`，确认依据记录在 ADR 与本文「评审记录」。

## 评审记录

### 2026-11-15 需求对齐（人工 + Agent）

- 人工提出：docling 数据文件迁到 `/data/llm/docling`，所有使用 docling 的服务共用；路径不写死，改由环境变量读入；确认存在非 Knoted 消费者，故它必须是机器级共享 artifact。
- 人工在选项中选择：按 revision 分目录（方案 B）、专用变量 + upstream 兜底（A1）、全体可读 0755/0644（D1）。
- 人工后续确认：**BGE-M3 与 docling 一起迁**到共享根（同一套规则），因此 `KNOTED_INTELLIGENCE_MODELS_DIR` 整体移除而不只是收窄；并要求先提交 RFC 供 review，ADR 在实施前另写 Proposed。
- Agent 补充的实测证据：现有树 `0750 xingjian:users` 导致 `knoted` 不可读；当前无运行中的 Knoted 服务（仅 `knoted-api-key.service`）；`fetch-models.sh` 拒绝覆盖已存在且 sha256 不符的文件；nixpkgs 的 docling 与锁定版本不一致（作为「不搬 Nix」的既有理由）。
- 关联 Issue：[#32](https://github.com/xingjianxu/knoted/issues/32)。

### 2026-11-15 Builder check（独立 subagent，5 个问题）

独立 reviewer 以实现者视角提出问题，Agent 逐项核实后全部当场收敛（核实方式：直读 `docling_adapter.py:139-143`、`tests/test_t09.py`、host `fetch-models.sh` 的 14 个 `fetch` 调用、`MANIFEST.json` 字段、`du -sb` 实测 733,054,339 B）。

| # | 问题 | 结论 | 归类 |
| --- | --- | --- | --- |
| 1 | 根目录解析必须保持惰性吗？否则 T09 纯文本解析会因 `artifacts_unset` 失败 | 必须惰性；已写入 §2 并加回归用例 | 由项目现状决定（代码注释与 T09 设计已经回答） |
| 2 | pytest / 开发环境的根目录从哪来、代码与 host 搬迁的先后 | 先复制、后切换、最后删除（733 MB 可忽略）；测试前 export 写入 README；T09 不需要该变量 | Agent 定（成本对比明确） |
| 3 | revision 目录装 9 个还是 14 个文件 | 14 个（脚本、`MANIFEST`、`LOCK.md` §4 三处一致），运行时仍只校验 9 个；原句歧义已修 | Agent 定（项目已有权威记录） |
| 4 | bash 脚本怎么拿到 revision 目录名而不手写 | 脚本自带 `--print-revision-id`，不依赖 venv；测试断言 bash / Python / `LOCK.md` 三处一致 | Agent 定（保持了「provision 不依赖 Python」） |
| 5 | `MANIFEST.json` 谁写、什么 schema、迁移拿什么当判据 | 脚本生成；schema 升为 `knoted-docling-artifacts/2`；路径改为相对 revision 根；迁移用「14 个文件的生成清单 + `LOCK.md` §4 的 9 行」双判据 | Agent 定（该文件运行时不被校验，不改变解析行为） |

### 2026-11-15 PR review 与 ADR 确认（人工）

- 人工 review 完 PR #33 并逐项确认：共享根属主 = host 用户；不留 symlink；由 `/etc/nixos` 声明共享根；两棵根 `/data/llm/{docling,bge-m3}` 平行摆放。
- ADR 批准方式：人工明确表示「你批准 ADR」，即授权 Agent 将本文决定写入并标记为 Accepted。ADR 的 `Confirmation` 字段记录了该会话指令与本次逐项确认。
- 因此本文状态由 Draft 置为 Resolved，决定记入 [ADR 0001](../decisions/0001-machine-level-model-artifact-roots.md)。
