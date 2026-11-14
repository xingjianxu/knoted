---
name: issue-triage
description: Use when an existing Issue needs validity, duplicate, impact, information completeness, or readiness assessment under the repository's default GitHub authorization.
---

# 任务分诊

## 输入与触发

输入仓库、Issue 编号或正文、相关版本和证据。读取 [规则](../../../AGENTS.md) 与 [状态映射](../../../docs/development/agent-workflow.md)。

## 步骤

1. 验证仓库后读取 Issue 正文、标签、评论与关联 PR；先识别作者提供的事实和未证实判断。必要时阅读代码、测试和 ADR，默认只读。
2. 搜索 open/closed 重复任务，比较复现、影响和解决状态。读取或搜索失败时报告查重未完成，不能断言没有重复。重复项仅建议关联/合并，不自动关闭 Issue。
3. 评估有效性、主要类型、影响范围和优先级建议 P0–P3。缺少信息时列出最少的具体问题，以及这些信息如何影响验收。
4. 给出 Agent 可执行性结论：范围是否明确、验收可验证、依赖可得、是否触及人工架构/安全边界。重大决策交 architecture-review；不自行批准。
5. 输出建议状态：needs-info、needs-triage、blocked、human-required 或 agent-ready。分诊确认范围、验收和依赖可得后，可直接写入 agent-ready 并记录判断依据；涉及重大架构或不可逆/生产变更时改为 human-required。
6. 直接更新精确对象的状态/评论，移除旧状态以保持互斥，读回核实。新增或重命名标签属仓库设置类，不能自行执行；应改用已有的标签组合。

## 输出与边界

输出有效性、重复项、类型、影响、优先级、信息缺口、验收建议、准入结论及需要人工决定的结构边界。看板 Ready 表示分诊已完成，不等于架构或生产变更已获批准。

示例：needs-triage 的小 Bug 查重、复现、验收均清楚时，可直接标 agent-ready 并开始修复；若修复会改变存储模型，则转 human-required。
