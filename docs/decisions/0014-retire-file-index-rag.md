# ADR 0014: 退出文件全文索引、RAG 与后台正文提取

- Date: 2026-11-27
- Status: Accepted
- Decision owner: Knoted 维护者 Xingjian Xu
- Confirmation: 2026-11-27 任务会话中，维护者确认保留笔记、项目和事项搜索，下线问答、后台正文提取、文件任务页及手动扫描。随后明确回复「清空两套 Knoted 的全部应用数据」和「删除两套实例的已有数据备份」，选择全新数据库切换。
- Related issues: [#314](https://github.com/xingjianxu/knoted/issues/314)
- Related PRs: 见关联 Issue
- Supersedes: [ADR 0006](0006-file-pipeline-and-aggregate-contracts.md) 的后台提取与概览索引状态、[ADR 0007](0007-execution-exit-confirmation.md) 的计算执行服务、[ADR 0011](0011-consistent-backup-and-isolated-restore.md) 的退出核验与恢复依赖
- Superseded by: 无

## 背景与决定

笔记、项目和事项的全文搜索继续由 Meilisearch 提供，并由 PostgreSQL 复核来源。文件自动扫描、采集、浏览、下载和固定版本预览走原件路径。浏览器去掉文件任务和手扫接口，Worker 停止提交文件正文索引、RAG、聊天和文档提取。运行包和编排由 API 与 Worker 组成。全新迁移链去除 `pgvector`、向量、提取、聊天和引用数据结构。

数据库维护锁、已准入写入的完成检查、Meilisearch 提交回执、同一快照的图片与数据库归档继续有效。当前归档格式 2 的隔离恢复另占 API 和 Meilisearch 端口，维持 RP ID，由独立 Worker 重建笔记、项目和事项索引。历史格式及旧迁移保存在 [归档分支](https://github.com/xingjianxu/knoted/tree/archive/file-fulltext-rag)。

## 备选与影响

增量下线可留存旧数据，仍需依赖历史迁移并维护旧归档兼容。维护者选择清空开发和生产的 Knoted 应用数据库、应用存储、搜索数据，以及两套实例的专属数据库与附件备份。旧账户、Passkey 和业务记录随实例清理。只读文件源由独立服务管理，新的文件列表由自动扫描重建。

切换前先验收当前产物与全新 schema，停止旧写进程并确认退出后重建数据库。删除旧实例数据和备份为不可逆步骤，由上述会话授权。宿主共享快照与跨服务资源由宿主管理。完整代码回退点为 `archive/file-fulltext-rag` 的 `3624a3e8346d4c74b5141a114c676c46479c4717`。

## 验证

隔离演练验证普通 PostgreSQL 迁移、当前格式备份与恢复、旧 API 路径不可用、保留全文搜索、自动扫描和原件预览。部署后分别核对开发、生产实际进程版本与健康。[任务卡](../tasks/314.md) · [退出方案](../rfcs/2026-11-27-retire-file-index-rag.md)

## 状态历史

- 2026-11-27 维护者确认功能范围、无兼容的全新数据库方案和两套实例数据清理，记录为 Accepted。
