# ADR 0001: 模型 artifact 采用机器级共享根并按 revision 分层

- Date: 2026-11-15
- Status: Accepted
- Decision owner: xingjianxu（仓库 owner，本次会话的人工确认者）
- Confirmation: 2026-11-15 会话中人工逐项确认（共享根属主 = host 用户；不留 symlink；由 `/etc/nixos` 声明共享根；两棵根 `/data/llm/{docling,bge-m3}` 平行摆放），review 完 [PR #33](https://github.com/xingjianxu/knoted/pull/33) 后明确授权 Agent「你批准 ADR」。方案全文见 [RFC](../rfcs/2026-11-15-docling-shared-artifacts.md)。
- Related issues: [#32](https://github.com/xingjianxu/knoted/issues/32)
- Related PRs: [#33](https://github.com/xingjianxu/knoted/pull/33)
- Supersedes: 无
- Superseded by: 无

## 背景与问题

Docling artifact（layout-heron + TableFormer fast + RapidOCR ONNX 与字典，运行时必需 9 个文件、provision 共 14 个文件 / 733,054,339 B）与 BGE-M3 稠密嵌入快照（13 个文件 / 2,295,354,503 B）原本按实例放在 `/data/knoted-dev/service/models/`，路径以硬编码默认值存在于 `parsing/models.py:30` 与 `embedding/bge.py:51`。

观察到的三个问题：

1. **无法共用**：每个实例重复放置同一份数据；非 Knoted 的 docling 消费者没有机器级位置可依赖。本机 `/data/llm/` 已是共享模型根的先例（`/data/llm/strata`）。
2. **读权限失效**：现有树 `0750 xingjian:users`，服务账号 `knoted`（uid 988，无附加组）实测 `sudo -u knoted ls .../ocr/docling` → `Permission denied`。权重非机密，但权限模型让目标消费者不可读。
3. **默认值掩盖配置缺失**：未设置 `KNOTED_INTELLIGENCE_MODELS_DIR` 时静默退回硬编码路径，换机器或换布局不报错、不提示。

Provisioning 脚本还有一个约束：`fetch-models.sh` 对已存在但 sha256 不符的文件**报错退出、拒绝覆盖**（故意的安全设计），因此单一共享根在换 revision 时必须人工清空，期间所有消费者同时不可用。

## 决策内容

1. **两个机器级共享根，按锁版 revision 分层，目录不可变（只增不删）**：
   - `/data/llm/docling/<revision-dirname>/`，当前 `heron-8f39ad3c_tf-fc0f2d45-fast_rapidocr-3.10.0`；
   - `/data/llm/bge-m3/<revision-dirname>/`，当前 `bge-m3-5617a9f6-dense`。
   两棵根**平行摆放**，不合并为 `/data/llm/models/*`。revision 目录名由代码中的锁版常量派生，`--print-revision-id` 与 `LOCK.md` 三处由测试保证一致。
2. **路径只来自环境变量，代码中不留可用默认值**：
   - Docling：`KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR` → `DOCLING_ARTIFACTS_PATH`（Docling 上游标准变量，供直接调用 docling 的消费者共用）→ 显式失败 `artifacts_unset`；
   - BGE-M3：`KNOTED_INTELLIGENCE_BGE_M3_DIR` → 显式失败 `snapshot_unset`（无上游兜底：`SENTENCE_TRANSFORMERS_HOME` 是 HF 缓存根，不是已转换的本地快照）；
   - `KNOTED_INTELLIGENCE_MODELS_DIR` **整体移除**。设旧变量不再有任何效果。
3. **每个 revision 目录承载该 revision 的完整内容**（docling 14 个文件、BGE 13 个文件），运行时仍只校验各自必需项（docling 9 个文件的 size；BGE 10 个文件的 size 与 sha256）。
4. **读策略**：目录 `0755`、文件 `0644`，属主为 host 用户，消费者只读；共享根由 `/etc/nixos` 的 tmpfiles 声明，新 revision 目录由 provision 脚本创建。
5. **Provisioning 归属**：`services/intelligence/scripts/fetch-{docling,bge-m3}-artifacts.sh` 进仓库受审查，不依赖 venv；数据目录只放 artifact 字节与脚本生成的 `MANIFEST.json`。
6. **解析保持惰性**：纯文本解析不需要模型，未配置变量时仍成功；只有 OCR/PDF 才解析 artifact 根。
7. **迁移**：先复制、后切换、最后删除，不留 symlink。

不涵盖：不改任何模型版本、revision、引擎选项或 `OPTIONS_REVISION`；不把 Python 依赖搬进 Nix；不改执行协议、账本状态机、准入上限、离线策略；不改 `config.py` 的 state/staging 默认值。

## 备选方案

| 方案 | 收益 | 成本 / 风险 | 结论 |
| --- | --- | --- | --- |
| 保留现状（每实例一份 + 硬编码默认） | 零改动 | 700 MB 级数据重复；服务账号不可读；默认值掩盖配置 | 否 |
| 单一共享根（`/data/llm/docling` 直接作为 artifacts_path） | 路径最短、与 Docling 语义 1:1 | 整机只能一个 revision；换版必须先清空，所有消费者同时中断；两个消费者需要不同 revision 时无解 | 否 |
| 按 revision 分目录（本决策） | 多 revision 共存；升级/回退 = 改一个环境变量；写入只发生在未启用的新目录 | 路径多一层；消费者需显式知道自己用的 revision | **采纳** |
| 单根 + symlink 挂回旧路径 | 零代码改动 | 路径仍隐含写死；两棵树仍耦合在同一根；新消费者要复制 symlink 约定 | 否 |
| 先 `/data/llm/models/{docling,bge-m3}` 合并摆放 | 统一前缀 | 与既有 `/data/llm/strata` 的平级惯例不一致；多一层无收益嵌套 | 否 |
| 专用属主组 `docling-data`（`0750`） | 更紧的读控制 | 每加一个消费者要改 NixOS 组并 rebuild；权重非机密，实测这类权限已造成静默失败 | 否 |

`MANIFEST.json` 的处置另有两处差异，均已定：docling 清单运行时不被校验 → schema 升为 `knoted-docling-artifacts/2`，路径改为相对 revision 根；BGE 清单**被 `bge.manifest()` 运行时校验** → schema 保持 `knoted-bge-m3-models/1`，生成物必须同形。

## 决策理由

- **证据**：`sudo -u knoted ls` 的 `Permission denied` 证明共享根本身不可读，仅搬路径不解决问题；`fetch-models.sh` 的拒绝覆盖语义证明单一根无法无停机换 revision；`id knoted` 显示服务账号无附加组，故 `0755` 是让任意消费者可读的最小可行策略。
- **与既有约定一致**：仓库已要求「配置只来自环境变量、失败要显式」；删除默认值是补齐这条约定，而不是新增风格。
- **三处一致性可机器校验**：revision 目录名同时出现在 bash 脚本、Python 派生函数与 `LOCK.md`，由 pytest 断言相等，避免共享根里出现无法对应锁版的目录。
- 假设：非 Knoted 消费者存在且将来可能 pin 不同 revision（人工在 2026-11-15 确认）。若该假设不成立，单一根会更简单，但迁移成本相同。

## 影响

- 正面：整机各一份 733 MB + 2.29 GB 数据；服务账号可读；路径不再有隐式默认值；换 revision 与回退都退化为改一个环境变量；provision 脚本进入版本控制与评审。
- 负面：运维契约破坏性变更（`KNOTED_INTELLIGENCE_MODELS_DIR` 消失，两个新变量必填）；共享根成为多个消费者的共同依赖，provision 失败会影响全部消费者（由不可变 revision 目录 + sha256 + `flock` 缓解）；运行 T10/T11 需要先 export 变量；`docs/plans`、两份 `LOCK.md`、`/etc/nixos` 文档需同步。
- 迁移与回退：先复制（保留旧树）、切换环境变量、验证通过后删旧树；回退 = 去掉新变量回到旧树，或整体移回。旧树在删除前一直是可直接回退点。不可逆点只有「删除旧树」一步，且它发生在验证通过之后。
- 安全：共享根对所有本机账号可读，权重不含凭据；任何消费者不得向该目录写入 token 或密钥。

## 验证与后续任务

- 验收证据归实施任务：[#32](https://github.com/xingjianxu/knoted/issues/32)。要求包括：未设置变量时 OCR/PDF 得 `artifacts_unset`、嵌入得 `snapshot_unset`；纯文本解析在无变量时仍成功；revision 目录名三处一致；`--verify` 覆盖 14 / 13 个文件（BGE 含逐 tensor 摘要）；`knoted` 与非属主账号均可读；`OPTIONS_REVISION` 与锁版摘要逐字符不变。
- 待验证假设：暂无未记录的消费者依赖旧路径（若无，删除旧树即可）。
- 后续：实施前出实施计划（代码任务与 `/etc/nixos` host 任务分开），完成后由代码 PR 关闭 #32。

## 状态历史

- 2026-11-15：Proposed，Agent 起草，讨论入口 [RFC](../rfcs/2026-11-15-docling-shared-artifacts.md) 与 [PR #33](https://github.com/xingjianxu/knoted/pull/33)。
- 2026-11-15：Accepted。人工 review 完 PR 并逐项确认四项选择，明确授权 Agent 记录并批准本 ADR。
