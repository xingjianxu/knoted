# 任务拆分与规划

Knoted 以 GitHub Issue 为唯一长期任务入口，以 [docs/tasks/](../tasks/README.md) 为按 Issue 编号组织的任务卡索引；[docs/plans/](../plans/README.md) 保留已完成的 v1 首版 / 全站 Web 重设计归档，以及确需跨卡协作时的架构/设计总览。

## 任务卡粒度与拆分

| 场景 | 记录方式 |
|---|---|
| 单点任务或明确交付（功能、缺陷、维护） | 创建带原生 Type（`Feature`/`Bug`/`Task`）的 GitHub Issue，按返回编号写 `docs/tasks/<编号>.md` 并在 `docs/tasks/README.md` 登记 |
| 超出单次 Agent 上下文或含多个独立验收目标 | 拆分为多个可独立评审的 GitHub Issue，分别取号建卡，用 GitHub 原生 `blocked-by` 维护直接前置 |
| 跨模块大改动或涉及长期架构取舍 | 先走 [architecture-review](../../.agents/skills/architecture-review/SKILL.md) / RFC，经人工确认后由 [adr-create](../../.agents/skills/adr-create/SKILL.md) 落 ADR，再拆分为按 Issue 编号的任务卡 |

## 卡片必备内容

每张 `docs/tasks/<编号>.md` 须让下一位开发者或 Agent 能直接接手：

- 标题 `# #<编号> <Issue 标题>`、GitHub Issue 链接与原生 Type（`Feature`/`Bug`/`Task`）。
- 分支名 `issue-<编号>-<slug>`、Worktree 路径 `.worktrees/<编号>-<slug>/` 与直接前置（同步写入 GitHub `blocked-by`）。
- 交付目标、范围与代码/文档落点、相关架构或 ADR 引用。
- 实施步骤与可复现的验收条件（包含正常路径与失败路径）。

实时进度、阻塞、范围变更与合并后的验证摘要记在对应 GitHub Issue 评论中；改变验收标准的结论同步回任务卡与索引。

## 并行与子任务边界

- 只有文件范围和接口相互独立的写任务才可并行；共享核心文件、锁文件、OpenAPI 契约或数据库迁移的任务由主 Agent 串行集成。
- 委派子任务使用 [委派模板](subagent-task-template.md)；子任务报告完成不等于整体完成，由主 Agent 独立核对提交、测试与验收条件后再集成。
