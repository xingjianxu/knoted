# RFC: 文件全文索引与 RAG 退出

- Date: 2026-11-27
- Status: Resolved
- Owner: Knoted 维护者 Xingjian Xu
- Related issues / ADRs: [#314](https://github.com/xingjianxu/knoted/issues/314)、[ADR 0001](../decisions/0001-machine-level-model-artifact-roots.md)、[ADR 0006](../decisions/0006-file-pipeline-and-aggregate-contracts.md)、[ADR 0007](../decisions/0007-execution-exit-confirmation.md)、[ADR 0011](../decisions/0011-consistent-backup-and-isolated-restore.md)
- Decision record: [ADR 0014](../decisions/0014-retire-file-index-rag.md)

## 目标

从主线移除文件正文索引和 RAG。完整的改动前代码、测试、配置及历史迁移已保存在 [archive/file-fulltext-rag](https://github.com/xingjianxu/knoted/tree/archive/file-fulltext-rag)，提交为 `3624a3e8346d4c74b5141a114c676c46479c4717`。文件浏览、原件下载及预览继续使用现有路径。

## 现状

- `file_extractions` 是文件正文发布的来源。Worker 将提取结果发布到统一全文索引，并构建 RAG 分块、词法与向量索引。下载、预览和 Office 原件授权直接读取已核实的源文件。
- `/search` 同时查找笔记、项目、事项、文件与问答消息。现有问答的生成流程会组装检索上下文，引用和回答转存位于同一功能内。
- `/files/jobs` 与 `/jobs` 同时承载扫描、采集、提取和索引任务。扫描与稳定采集还负责建立可浏览的文件来源。
- SQLx 校验已发布迁移的版本与校验和。`0020` 创建 pgvector 扩展与向量列。现有备份含数据库数据和已登记图片，恢复工具还会撤销旧授权、退休旧任务并重建检索索引。

## 退出方案

| 方法 | 运行影响 | 数据影响 |
|---|---|---|
| 增量下线 | 旧库和旧归档可继续使用，历史迁移仍需要 pgvector | 保留业务与旧功能数据 |
| 全新库切换 | 应用和部署仅包含保留能力，当前 schema 只需要 PostgreSQL | 两套实例重新初始化，现有 Knoted 数据与专属备份清空 |

维护者确认采用全新库切换。迁移只创建保留能力所需的数据结构。备份和恢复使用新清单格式，在独立端口、实例根和数据库中恢复笔记、项目、事项、账户、资产及文件元数据，重新生成保留对象的搜索索引。

## 已确认范围

| 功能 | 决定 | 影响 |
|---|---|---|
| 全站搜索 | 保留笔记、项目和事项搜索 | 保留这些对象的 Meilisearch 发布与查询，删除文件和问答命中 |
| 问答 | 下线问答页面、会话 API、引用和回答转存入口 | 旧会话随实例清理；新笔记仍可正常创建 |
| 正文提取 | 下线后台文本、PDF、Office 和 OCR 提取 | 源文件预览继续可用，运行编排只启动 API 与 Worker |
| 文件任务 | 下线文件任务页和手动扫描接口 | Worker 继续监听文件目录和定期扫描，维持文件列表及预览 |

## 切换与验证

- 在独立 PostgreSQL 实例运行全新迁移，验证当前版本备份、隔离恢复和搜索重建。验证文件自动更新、预览授权及下载字节。
- 完成代码验证后，停止旧应用进程，确认退出，再重建独立的 `knoted_dev` 和 `knoted_prod`。清理两套实例的应用存储、搜索数据和专属数据库／附件备份。只读源目录由原有外部存储继续管理，首次扫描登记当前文件。
- 用户需要重新登记 Passkey。归档分支保存旧实现和迁移，可用于另建实例。宿主全盘快照覆盖多个服务，属于共享备份系统。

## 评审记录

- 2026-11-27 维护者通过任务会话确认功能边界、开发和生产全部 Knoted 应用数据及专属数据库／附件备份清理，以及全新数据库切换。
