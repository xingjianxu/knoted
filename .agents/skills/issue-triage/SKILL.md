---
name: issue-triage
description: Use when an existing Issue needs validity, duplicate, impact, information completeness, or agent readiness assessment before a human decides whether to approve implementation.
---

# 任务分诊

## 输入与触发

输入仓库、Issue 编号或正文、相关版本和证据、授权范围。读取 [规则](../../../AGENTS.md) 与 [状态映射](../../../docs/development/agent-workflow.md)。

## 步骤

1. 验证仓库后读取 Issue 正文、标签、评论与关联 PR；先识别作者提供的事实和未证实判断。必要时阅读代码、测试和 ADR，默认只读。
2. 搜索 open/closed 重复任务，比较复现、影响和解决状态。读取或搜索失败时报告查重未完成，不能断言没有重复。重复项仅建议关联/合并，不自动关闭 Issue。
3. 评估有效性、主要类型、影响范围和优先级建议 P0–P3。缺少信息时列出最少的具体问题，以及这些信息如何影响验收。
4. 给出 Agent 可执行性结论：范围是否明确、验收可验证、依赖可得、是否触及人工架构/安全边界。重大决策交 architecture-review；不自行批准。
5. 输出建议状态：needs-info、needs-triage、blocked 或 human-required。只有人工已经明确批准，且会话授权允许远程修改时，才能**代为记录** agent-ready；必须附批准证据，不能根据自己的低风险判断授权自己。
6. 已获写授权时才更新精确对象的状态/评论，移除旧状态以保持互斥，读回核实；否则提供可审阅分诊报告。不得擅自改变优先级政策或仓库标签。

## 输出与边界

输出有效性、重复项、类型、影响、优先级建议、信息缺口、验收建议、准入结论及人工决定点。技术建议不是实施授权；看板 Ready 不是审批证据。

示例：needs-triage 的小 Bug 即使容易修复，也先提交“建议批准及依据”，保留原状态等待人工决定。
