# mtone UI 边界与任务规划 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans（本次主 Agent 串行执行）或 superpowers:subagent-driven-development 按任务执行。

**Goal:** 将已有 UI 任务改为消费 mtone，并建立只向用户交付上游组件需求的流程。

**Architecture:** 应用直接消费 npm mtone；不在本仓库建立通用组件库。此计划只修改文档与 Skill，运行时迁移留给 T17—T19。

**Tech Stack:** Markdown、Agent Skills、Git/gh；验证复用已有 Rust/pnpm 工具链。

**Spec:** [已确认设计](2026-11-16-mtone-ui-design.md)。

## Global Constraints

- 人工授权：2026-11-16 会话「请实施」；[Issue #39](https://github.com/xingjianxu/knoted/issues/39)。
- 主 Agent 串行修改 `.worktrees/mtone-ui-boundary`，分支 `docs/mtone-ui-boundary`，基线 `5b2512f`；只读测试/审查可共享视图。
- 不实施 T17—T19、不修改产品源码/依赖/锁文件、不写 mtone 或主机仓库。
- 新 ADR 仅记录已经确认的 UI 归属决策；编号集成前复核。
- 产品卡 pending 与旧执行证据保留；外部依赖没有发布及消费验收不能当完成。
- 本地门禁是唯一合并依据；默认授权普通 PR merge，不直推 master。

## Review Focus

1. 已有组件不能因名称不同而被误判缺失：Skill 场景 A。
2. 不得为 total 分页修改游标 API 或伪造总数：场景 B、T19 验收。
3. 提示词必须脱离会话独立可执行，建议 API 不伪装成现存 API：场景 C。
4. 上游 master/link 不等于 npm 交付，阻塞不等于人工权限审批：场景 D。
5. 查证失败只能报未知，不能编造缺口或运行结果：场景 E。

## Task 1: 决策、指南与任务卡

**Files:** 新增 `docs/decisions/0002-mtone-ui-ownership.md`、`docs/development/ui-mtone.md`；修改 `AGENTS.md`、根 README、架构现状、ADR 索引、v1 requirements/architecture/README/acceptance、T02/T04/T17—T20/T27—T29/T31/T38—T42/T49—T50/T52/T55—T56/T65—T66/T71。

**Interfaces:** 消费已确认设计；产出统一 UI 归属、任务路径、发布阻塞与验证规则，供 Skill 引用。主 Agent 串行，无并行写。

- [x] 记录现状/调研来源和已确认决策，保留当前包存在的事实。
- [x] 重写 T17—T19 的目标、范围、步骤、测试和阻塞条件；其余卡局部补充，保留业务场景与依赖。
- [x] 更新 R25、目标架构、索引、入口与验收，追加历史迁移说明。
- [x] 运行文档结构/链接/DAG 检查与 `git diff --check`。预期：零断链、72 卡 DAG 无环、无产品文件修改。

## Task 2: mtone 需求交接 Skill

**Files:** 新增 `.agents/skills/mtone-component-request/SKILL.md`、`docs/plans/knoted-v1/mtone-requests/README.md`、本任务验证记录。

**Interfaces:** 消费 UI 指南与真实需求；输出分类结论、可复制提示词、存档和关联任务恢复条件。不输出通用组件实现或上游写操作。

- [x] 在无新增 Skill 的只读新上下文执行 A—E 场景，保存原答并逐项判定，不捏造失败。
- [x] 根据真实失败/缺失项编写最小 Skill；frontmatter 名称与目录一致，description 只描述触发条件；完整输出槽位与真实/建议事实分离。
- [x] 用新上下文复跑 A—E，检查提示词完整性、权限边界与 npm 发布门槛；必要修正后重测。
- [x] 检查 YAML frontmatter、本地发现/显式加载及链接。预期：可发现、可读取、场景符合设计；模拟结果不冒称 mtone 实测。

## Task 3: 验证、审查与集成

**Files:** 更新本计划和 `docs/plans/knoted-v1/evidence/mtone-ui-workflow.md`；临时原始输出保存在任务 scratch，交付前归档到 `/data/knoted-dev/tasks/mtone-ui-boundary/`。

**Interfaces:** 消费全部 diff 和测试结果；产出可追溯 PR、主线 SHA 和清理结果。独立 reviewer 只读，无远程写权限；主 Agent 集成。

- [x] 实际执行 `cargo fmt --check`、`cargo test --workspace --locked`、`cargo clippy --workspace --all-targets -- -D warnings`，保留输出。预期 exit 0，测试非零。
- [x] 执行 `pnpm run typecheck`、`pnpm run lint`、`pnpm run test`、`pnpm run build` 及文档检查。预期 exit 0，不改锁文件。
- [x] 发起全分支独立审查，核实并处理重要问题；验证记录区分实测、模拟与未测。审查 `5b2512f..5f4bb56` APPROVED，无 Critical/Important/Minor。
- [x] 读取有效 git-commit Skill，检查范围/敏感信息后提交；[PR #40](https://github.com/xingjianxu/knoted/pull/40) 关联 #39、ADR 与实际输出，首提交 `5f4bb56`。
- [ ] 合并前复核主线/ADR 编号，普通 merge 后主线复验；回填远程验收/关闭状态，清理本任务 Worktree/分支。此项提交时待执行，最终结果以 PR #40「合并后核对」及评论为准，不为提前勾选而伪造完成。

## 进度与裁决

- 2026-11-16：基线检查通过：Rust 85 passed（另 1 ignored doctest）、Web 13 passed；pnpm frozen install exit 0，未安装 mtone。
- Ruling: 用户已审阅方案并要求实施，不重复请求同一范围批准；采用主 Agent 串行文档编辑 + 只读场景测试/独立审查，避免共享文档合并冲突。
- Ruling: 文档修改采用结构/链接/场景验证，不伪造产品代码 RED→GREEN；Skill 的基线实测结果单独记录。
