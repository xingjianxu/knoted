# 重大方案讨论

使用 [template.md](template.md) 讨论跨模块、长期约束或高迁移成本的方案。一般任务不写 RFC。

流程：先读相关 Accepted ADR → 提出问题与多个方案 → 评估兼容、安全和迁移 → 人工讨论与确认 → 用 ADR 记录最终决定 → 通过批准的 Issue 实施。RFC 本身不授予执行权限。

文件命名 `YYYY-MM-DD-short-title.md`，状态使用 Draft、In Review、Resolved、Withdrawn；Resolved 必须链接最终 ADR 或说明未采纳原因。重大方案尚无结论时不能被 Agent 当作 Accepted ADR。

新增时在此索引列出标题、文件链接、负责人、状态和 ADR。

| 标题 | 文件 | 负责人 | 状态 | ADR |
| --- | --- | --- | --- | --- |
| Docling 与 BGE-M3 模型数据改为机器级共享目录并按 revision 分层 | [2026-11-15-docling-shared-artifacts.md](2026-11-15-docling-shared-artifacts.md) | 待指定 | Resolved | [ADR 0001](../decisions/0001-machine-level-model-artifact-roots.md) |
| T41—T50 聚合与文件持久化契约补齐 | [2026-11-17-t39-t50-contracts.md](2026-11-17-t39-t50-contracts.md) | 用户决策，主 Agent 整理 | Resolved | [ADR 0006](../decisions/0006-file-pipeline-and-aggregate-contracts.md) |
| T47 计算进程退出确认与崩溃恢复 | [2026-11-18-t47-execution-exit-confirmation.md](2026-11-18-t47-execution-exit-confirmation.md) | 用户决策，主 Agent 整理 | Resolved | [ADR 0007](../decisions/0007-execution-exit-confirmation.md) |
