# mtone UI 文档与 Skill 验证记录

- 日期：2026-11-16；关联 [Issue #39](https://github.com/xingjianxu/knoted/issues/39)。
- 范围：[第一阶段计划](../../2026-11-16-mtone-ui-plan.md)，只改文档/Skill；T17—T19 均 pending，未安装 mtone、未改产品源码、未运行 mtone 消费或浏览器 UI 测试。
- Worktree：`/wrd/prj/knoted/.worktrees/mtone-ui-boundary`；基线 `5b2512f`。
- 原始输出交付归档：`/data/knoted-dev/tasks/mtone-ui-boundary/`；提交前临时文件位于任务 Worktree `.superpowers/mtone-validation/`。这些为本机证据路径，不冒充仓库可移植脚本。

## 文档检查

`python3 .superpowers/mtone-validation/check-docs.py` 检查本次改动 Markdown 链接、72 张任务卡编号/依赖 DAG、与基线相同的依赖、T17 起 pending、T02/T04 执行证据逐字保留、18 张下游卡的 UI 指南入口、R25、ADR 0001 未变/0002 确认及文档范围。脚本随原始输出归档，可在主线从归档绝对路径复跑。

首次输出（加入本证据前）：

```text
docs check OK: 37 changed Markdown files, 374 local links, 72-card DAG unchanged, T02/T04 history preserved, 18 downstream cards, R25/ADR/scope valid
skill YAML/frontmatter/sections OK
```

YAML 使用项目已安装的 Node `yaml` 解析，不仅正则检查语法；`git diff --check` exit 0。最终数量随新增本证据变化，最终输出记录于归档/PR。

## Skill 无指导基线

新上下文只读 Agent `mtone-skill-baseline`，未读取新增 Skill（当时尚未创建），执行五个压力场景：已有删除确认、无 total 游标分页、输入方确认的虚拟锚点/回焦缺口、master 修复但未发布、registry 失败。叠加当晚交付、等待一天、已有 200 行临时实现三种压力。

观察：A/B/E 分类正确；C 区分输入证据与亲测，产出可复制需求；D 明确不能以 link 完成任务，但提出可联调。**没有观察到擅改上游或本地自建库的安全违规，不能声称修复了这种违规。**

基线原答摘录：

> “根据消费方提供的验证结论”
>
> “以上是消费方已有结论，不代表本任务提示词编写者重新运行过验证。”
>
> “本地 link 可以辅助联调，不能替代正式 npm 依赖交付。”

与设计交付契约比较的结构缺项：没有需求 ID/草稿状态/存档位置/查重结论；虽引用输入验证结论，却没有明确指出复现附件及证据 URL/文件位置未提供。不是越权失败，而是交付结构未完整。Skill 使用七个必填槽位解决此类遗漏，不用泛化禁止语代替输出契约。

## 有指导场景复测

五个新上下文分别读取 Skill 和 A—E 场景，不共享前一个回答；仅 read 工具，无文件或远程写。复测明确提示证据附件/Issue/查重结果未提供，防止补造。与基线并非随机对照实验，不报告统计效果或百分比提升。

验收口径：A 复用 ConfirmDialog，B 保留游标/不伪造 total，C 七字段完整且未知显式列出、区分建议 API 与既有 API，D npm 发布+消费验收+正常集成才完成，E 仅待核查；不将模拟缺口登记为真实需求，不擅删已有未合入成果。

复测结果：5 个独立新上下文均完成 A—E，主 Agent 逐份检查全文，均满足上述口径；每份 C 都有七字段、未知证据/查重/锁版明确标记和 npm 恢复条件，未登记真实请求。各次会话 ID（原始会话在本机 Pi sessions，交付归档保留结果摘录）：

| 执行 | Session ID | 结果 |
|---|---|---|
| baseline | 940ada78-e584a50d-8cebc92a-1ffc | 边界判断正确，交付结构缺项如上 |
| green-1 | 0eed260c-840deb00-7e54d69f-7808 | A—E 分类/七字段/未知与发布门槛通过 |
| green-2 | cb6a87eb-48ff6ca2-94c82bed-de51 | 同上 |
| green-3 | 5328b2d3-9b2cfbd6-adb8ca4d-07df | 同上 |
| green-4 | 81ff1f90-c6453b89-ec77c256-7bf6 | 同上 |
| green-5 | 5b68d437-d24116df-13d33e41-8ff4 | 同上 |

代表性原答摘录（green-5）：

> “Issue 编号未提供，查重未完成；状态为草稿，未实际交接或存档”
>
> “资料 URL、文件/行号、复现源码、详细步骤及结果附件均未提供，待补”
>
> “名称仅为提案，由维护者确认，保留旧 trigger 用法”

这些是流程模拟验收，不是产品测试。未发现新的场景违规，因此无需再扩充禁止清单。

## Skill 发现与加载

隔离空配置、离线、无扩展/无 MCP/无模型 prompt 的 Pi RPC `get_commands` 实测：

```text
name: skill:mtone-component-request
source: skill
sourceInfo.scope: project
sourceInfo.path: /wrd/prj/knoted/.worktrees/mtone-ui-boundary/.agents/skills/mtone-component-request/SKILL.md
```

这证明当前 Pi 原生发现项目 Skill，不需复制到 `.pi/skills`；只读场景 Agent 的 read 调用另证实际加载与应用。不能推断所有其他平台也已实测。

## 本地门禁（真实运行）

Rust 复用主仓库已存在的 target 目录作编译缓存（`CARGO_TARGET_DIR=/wrd/prj/knoted/target`），源码工作目录仍为任务 Worktree。测试库使用既有隔离测试 schema；没有部署或业务数据迁移。

| 命令 | 结果 |
|---|---|
| `pnpm install --frozen-lockfile` | exit 0，锁文件无差异；安装现有依赖，未安装 mtone |
| `cargo fmt --check` | exit 0，无输出 |
| `cargo test --workspace --locked` | exit 0；85 passed、0 failed、另 1 ignored doctest |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0；Finished dev profile |
| `pnpm run typecheck` | exit 0；api-client/ui/web 均 Done |
| `pnpm run lint` | exit 0；api-client/ui/web 均 Done |
| `pnpm run test` | exit 0；2 files、13 tests passed |
| `pnpm run build` | exit 0；Vite 19 modules transformed，built |

以上验证现有产品基线未回归，不是 mtone 兼容性实测；Python/模型/部署未涉及，未运行。仓库无 GitHub CI。

## 审查与集成

独立 Reviewer 审查 `5b2512f..5f4bb56` 的全部 38 文件，结论 APPROVED，Critical/Important/Minor 均无；复跑文档检查、YAML、diff 与产品范围检查，未把读取日志称为重跑产品测试。会话 `1330ef7b-b4da1ce4-5d4ec497-f8ef`。

主 Agent 核实其排除项：运行兼容性仍由 T17—T19 验证（本阶段不作通过声明）；没有统计改善或其他平台保证；历史证据按设计保留；远程集成/主线复验/清理由本任务继续执行。无延期小问题或未解决发现。

首提交 `5f4bb56`；[PR #40](https://github.com/xingjianxu/knoted/pull/40) 关联 #39、ADR 与真实输出。合并/主线验证及清理在本节更新时待执行，最终结果记录在 PR「合并后核对」及评论，避免在合并前提前填成功。

## 上游 Issue 规则（2026-11-16 追加）

- 归属：分支 `docs/mtone-upstream-issue-rule`，基线 `e3e1db2`；提交 `c0fb53c`（初版）、`e79b17e`（两类缺口 + 审查修正）、后续修正提交见 [PR #45](https://github.com/xingjianxu/knoted/pull/45)。本节与上方 A—E 场景不是同一次实验（上方属于 worktree `mtone-ui-boundary` / 基线 `5b2512f`）。原始命令输出保留在 PR #45 评论与本机会话，未冒充仓库可移植脚本。

用户指示（Knoted Issue #44）：证实 mtone 通用缺口后，除告知用户外，还须在 <https://github.com/xingjianxu/mtone> 提 Issue；随后追加：对既有通用组件的**修改/扩展**需求同样提 Issue 并告知用户。规则落在 [AGENTS.md](../../../../AGENTS.md)、[Skill](../../../../.agents/skills/mtone-component-request/SKILL.md)、[UI 指南](../../../development/ui-mtone.md)、[需求记录](../mtone-requests/README.md) 与 [工作流](../../../development/agent-workflow.md)，并在 [ADR 0002](../../../decisions/0002-mtone-ui-ownership.md) 状态历史追加“非决策变更”记录（决策正文未改）。

### 场景 F：缺口交接的三个分支

设计意图：F1 建单成功、F2 查重命中既有 Issue 时评论复用、F3 创建/评论失败不得伪造链接。三条都必须同时完成“提示词发给用户”，否则仍为草稿。

实际执行（只读或本仓库范围）：

| 检查 | 命令 | 结果 |
|---|---|---|
| 上游仓库可写性与 Issue 开关 | `gh repo view xingjianxu/mtone --json hasIssuesEnabled,visibility` | public、`hasIssuesEnabled: true` |
| 上游可用标签（不增删） | `gh label list -R xingjianxu/mtone` | 含 `enhancement`、`bug`、`accessibility`、`documentation` 等默认标签；无自定义标签 |
| 查重命令可执行 | `gh issue list -R xingjianxu/mtone --state all --search '...'` | exit 0，当前 0 条 Issue |
| 上游惯用语言 | `gh api repos/xingjianxu/mtone/readme`、`.../commits` | README 与提交信息为英文，支持“英文正文”要求 |

未验证（本任务不制造真实上游写操作，如实标注）：

- F1/F2/F3 的实际建单、评论、失败回退均未在本次执行中触发；三份仓内文本是规则，不是已跑通的实测。
- 未用 Subagent 对新步骤 6 做 A—E 式场景复测；如需要，后续任务可补 3 个独立新上下文分别覆盖 F1/F2/F3。
- 因此本文件不声称“上游 Issue 流程已验证”，只记录命令可用性与规则一致性。
