---
name: project-audit
description: Use when the user requests a scoped project audit for reproducible defects, missing tests, technical debt, dependency risks, or documentation drift; not for automatic unsolicited repair.
---

# 项目审计

## 输入与触发

输入授权的目录/模块、审计目标、时间预算、可运行命令和远程权限。读取 [AGENTS.md](../../../AGENTS.md)、架构及相关 ADR。未授权全库审计时保持给定范围。

## 步骤

1. 记录基线和范围，不扫描 `.worktrees/` 的其他任务或输出秘密。只读代码分析无需 Worktree；运行会生成文件的测试/构建需在自己独立的 Worktree。
2. 使用 `rg` 检查 TODO/FIXME、关键模块测试和文档声明；结合实际行为判断，不能把每个 TODO 自动当缺陷。
3. 在授权的安全环境运行已有测试、静态分析或依赖审计。缺少工具、网络、测试或权限时记录“未检查”，不得把零测试或未运行说成覆盖充分。依赖风险核对官方公告，不擅自安装或升级。
4. 整理每项发现的证据、影响、可信度、可复现性和优先级建议，区分确认问题与待验证风险。
5. 按 [AGENTS.md](../../../AGENTS.md) 与 [工作流详情](../../../docs/development/agent-workflow.md) 先在 open/closed Issue 与 [任务索引](../../../docs/tasks/README.md) 查重；确认无重复后使用 `gh issue create --repo xingjianxu/knoted --type Bug|Task|Feature --title '...' --body-file <正文文件>` 建单（缺陷用 `Bug`，技术债/测试/文档/维护用 `Task`，新增能力用 `Feature`），写清目标、范围、复现/证据与验收条件并读回确认。已知问题只关联，不制造重复；无远程权限时输出本地报告/草稿。
6. 向人工汇报建议和下一步，不自动修改代码或顺手修复本次审计新创建的任务。

## 输出与边界

输出审计范围、实际命令/结果、发现清单、证据、重复项、真实 Issue 链接或草稿、未覆盖区域及限制。发现凭据或漏洞时仅提供脱敏摘要，走已确认的私密渠道；渠道未确认时停在脱敏报告，不公开秘密、不自行轮换凭据或修改生产。

示例：关键路径没有测试 → 说明具体行为和风险，查重后创建 `Task` Issue 并读回确认，不能一边审计一边擅自实施修复。
