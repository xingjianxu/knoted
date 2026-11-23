---
name: architecture-review
description: Use when a proposed change affects long-term module boundaries, interfaces, storage, security policy, or an Accepted ADR and needs architecture assessment before a human decision.
---

# 架构评估

## 输入与触发

输入问题和目标、变更范围、现状证据、相关 Issue/ADR 和决策责任人。普通局部 Bug 和常规重构不触发。读取 [架构现状](../../../docs/architecture/README.md)、[ADR 指南](../../../docs/development/adr-guide.md) 及适用 AGENTS.md。

## 步骤

1. 检索已有 Proposed/Accepted/Superseded ADR，确认本次提案是否与已确认决策冲突。列明约束、未知项和需要人工确认的部分。
2. 分析至少现状与合理候选的接口、依赖、兼容性、数据、安全、性能、迁移成本和回退（若当前环境已安装设计/头脑风暴 Skill 可直接复用，不重建通用设计 Skill）。
3. 区分事实、推断、试验结果；必要研究仅在授权范围内。写草稿仍须任务 Worktree，只读研究可不建。
4. 重大/跨模块调整按 [RFC 模板](../../../docs/rfcs/template.md) 建议或起草 RFC，包含推荐方案、权衡、验收、开放问题和确认责任人。推荐不是 Accepted。
5. 输出具体可审阅方案，等待重要决策的人工确认。已有明确确认不重复请求；确认后转 adr-create。涉及旧 Accepted ADR 时建议新 ADR 及替代关系，不直接改旧决策正文。

## 输出与边界

输出变更必要性、方案比较、影响/迁移评估、RFC 或 Proposed ADR 路径及待决定事项。不得批准自己提出的架构，不以技术优势替代授权，不自行执行迁移、生产或权限策略变更。

示例：新存储方案优于 ADR 0003 → 提供新提案和迁移成本，保留 0003 的 Accepted 状态直至新决策获人工确认。
