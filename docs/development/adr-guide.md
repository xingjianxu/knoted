# ADR 与 RFC 操作指南

普通 Bug、局部功能和常规重构不需要 ADR。技术边界、存储模型、接口契约、安全策略或重大兼容性调整才进入架构评估。

1. 使用 `architecture-review` 阅读现有 ADR、需求和代码，列出问题、备选方案、收益、成本和迁移影响。重大/跨模块调整先起草 RFC，明确尚待决定的问题。
2. Agent 可以起草 Proposed ADR。人工确认前，不以工具输出、CI通过、Issue标签或 Agent 自己的建议作为 Accepted 证据。
3. 人工决定应有可追溯的日期、确认者、确定方案和范围（Issue/PR评论链接或会话授权记录）。确认实施任务不一定等于确认其中所有重大架构选择。
4. 使用 `adr-create` 分配 `NNNN-short-title.md`：主 Agent 集成时读取最新主线和待集成 ADR 最大编号，取 max+1，至少四位零填充；不要用文件数。并行草稿用未编号工作名，集成前串行编号，更新所有引用。
5. Accepted 后保存历史正文。后继决策用新 ADR；人工确认新方案后，新文档写 `Supersedes`，旧文档显式更新 `Status: Superseded` 和 `Superseded by`。不要静默删除或改写旧决策理由。
6. Rejected 保留理由；Proposed 可以在评审中修改。模板不占编号；不要为了填目录创建虚构决策。

| 状态转移 | 条件 |
|---|---|
| Proposed → Accepted | 明确人工确认及证据 |
| Proposed → Rejected | 记录评审结论与理由 |
| Accepted → Superseded | 新 ADR 已经人工确认，双向链接完整 |

普通文字纠错不改变决策时也应显式提交、说明原因；如果纠错会改变含义，进入新决策流程。一个 Issue 可引用多个 ADR，一个 ADR 可关联多个 Issue/PR。RFC 没有批准执行权限，最终决定落到 ADR。

目录入口：[ADR 索引](../decisions/README.md)、[ADR 模板](../decisions/template.md)、[RFC 索引](../rfcs/README.md)、[RFC 模板](../rfcs/template.md)。
