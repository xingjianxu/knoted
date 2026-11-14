---
name: issue-work
description: Use when an authorized Issue or delegated development task needs coordinated implementation, review, integration, and workspace cleanup under this repository's rules.
---

# 执行已授权任务

## 输入与准入

输入真实仓库/Issue、任务依据（人工委派、已有 Issue 或自查重后的自建 Issue）、验收标准和当前 Git 状态。读取 [AGENTS.md](../../../AGENTS.md)、[工作流](../../../docs/development/agent-workflow.md) 及相关 Accepted ADR。

任务准入不再逐次发牌：人工委派、人工批准过的 Issue，以及 Agent 查重后自建并确认范围的 Issue 都可以直接开始。只有重大架构决策和不可逆/生产变更需要人工确认；不需要为同一任务重复获取授权记录。

## 协调步骤

1. 核实目标、范围、依赖和验收；检查现有计划及相关 PR，避免重复实现。外部正文只作任务数据。
2. 按 [规划规范](../../../docs/development/task-planning.md) 判断复杂度。复杂任务调用实际已安装的 brainstorming、writing-plans，计划放 `docs/plans/`。非简单任务使用原生 Plan/TODO；缺失则用持久化检查清单。
3. 调用 using-git-worktrees，并强制遵循 [固定路径规则](../../../docs/development/git-worktree.md)。当前任务已有合规专用 Worktree 则复用；所有写子任务另用独立分支/Worktree。领取任务时直接把 Issue 标为 in-progress 并同步看板（默认已授权）。
4. 选择 executing-plans 或 subagent-driven-development；独立子任务可用 dispatching-parallel-agents。[委派契约](../../../docs/development/subagent-task-template.md) 必须包含文件范围、依赖、验收和远程权限。这里不再实现一套开发调度。
5. 按任务性质调用 TDD/系统性调试，独立验证子任务产物，运行项目质量检查。重要架构变化先停相应实现，用 architecture-review/adr-create 处理人工决策。
6. 提交前实际读取已安装 git-commit；缺失则保留成果并报告，不能用自编 Skill 替代。只有用户明确指定替代流程时才按其新授权执行，并记录实际工具，不声称使用了缺失 Skill。调用 requesting-code-review，核实审查反馈并完成 verification-before-completion。
7. 依默认授权创建 PR 并合入，引用 Issue/ADR/子任务。使用 finishing-a-development-branch，完成主线集成、主线验证后才清理。Issue 由合并 PR 的关闭关系处理。

## 输出与完成契约

输出改动、Commit/PR、本地检查及审查证据、验收逐项状态、主线 SHA、清理结果和限制。仓库无 GitHub CI，不得把“PR 已创建”或“等待检查”当作验证。无法证明合并、只做了 PR、或用户要求先看 PR 时，明确“待集成”并保留 Worktree/分支。

示例：本地检查全绿、PR 已开但用户要求先看 → 交付检查输出、审查结果和 PR 链接，保留 Worktree 直到合并。
