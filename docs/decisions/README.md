# 架构决策记录

本目录记录已确认的产品架构决策；索引条目以 ADR 自身的 Status 为准，索引本身不预先批准任何方案。

新增时使用 [template.md](template.md) 和 [ADR 指南](../development/adr-guide.md)。命名 `NNNN-short-title.md`，编号由主 Agent 在集成前按最大现有编号串行分配；模板不编号。每个正式 ADR 加入下表，替代时更新双方链接。不要用文档条数推算编号。

| 编号 | 标题 / 文件 | 日期 | 状态 | 替代关系 |
| --- | --- | --- | --- | --- |
| 0001 | 模型 artifact 采用机器级共享根并按 revision 分层（[文件](0001-machine-level-model-artifact-roots.md)） | 2026-11-15 | Accepted | 无 |
| 0002 | 通用 UI 由 mtone 提供，Knoted 直接消费 npm 包（[文件](0002-mtone-ui-ownership.md)） | 2026-11-16 | Accepted | 无；替换 v1 自有 UI 提案；第 4 条（BlockNote 编辑器边界）由 0012 部分替代 |
| 0003 | 测试实例由 pcwrap 编排，NixOS 只保留外部依赖与未来生产（[文件](0003-test-instance-process-management.md)） | 2026-11-16 | Accepted | 无；替代 T21 的「测试实例由 NixOS 单元运行」边界；数据库 provisioning 部分由 0004 取代；HTTPS 部分由 0005 取代 |
| 0004 | 开发/测试与生产数据库由宿主用户直接创建（[文件](0004-database-owned-by-host-user.md)） | 2026-11-16 | Accepted | 部分替代 0003（PostgreSQL provisioning 归属） |
| 0005 | 测试 HTTPS 由宿主共享 Caddy 管理（[文件](0005-https-owned-by-caddy.md)） | 2026-11-16 | Accepted | 部分替代 0003（HTTPS / 续期 reload 归属） |
| 0006 | 文件处理链与概览回收站契约（[文件](0006-file-pipeline-and-aggregate-contracts.md)） | 2026-11-17 | Accepted | 部分由 0014 替代 |
| 0007 | 计算退出证明与持久监督（[文件](0007-execution-exit-confirmation.md)） | 2026-11-18 | Superseded | 由 0014 替代 |
| 0008 | OnlyOffice 预览使用短时固定版本授权（[文件](0008-office-preview-authorization.md)） | 2026-11-18 | Accepted | 取源凭据部分由 0009 替代 |
| 0009 | OnlyOffice 取源只接受官方出站 JWT 请求头（[文件](0009-office-source-jwt-header.md)） | 2026-11-18 | Accepted | 部分替代 0008 的取源随机凭据 |
| 0010 | OnlyOffice 网页使用不携带凭据的 Knoted 宿主页（[文件](0010-office-preview-credentialless-host.md)） | 2026-11-18 | Accepted | 无；细化 0008 网页边界 |
| 0011 | 使用数据库维护锁完成一致性备份和隔离恢复（[文件](0011-consistent-backup-and-isolated-restore.md)） | 2026-11-19 | Accepted | 部分由 0014 替代 |
| 0012 | 笔记编辑器直接消费 npm `@bixlab/mtone-blockeditor`，替代直接依赖 BlockNote（[文件](0012-mtone-blockeditor-adoption.md)） | 2026-11-25 | Accepted | 部分替代 0002 第 4 条；普通表格的持久化限制由 0013 替代 |
| 0013 | 笔记保存普通表格（[文件](0013-note-table-persistence.md)） | 2026-11-26 | Accepted | 部分替代 0012 的普通表格持久化限制 |
| 0014 | 退出文件全文索引、RAG 与后台正文提取（[文件](0014-retire-file-index-rag.md)） | 2026-11-27 | Accepted | 替代 0007，部分替代 0006 和 0011 |

新 ADR 必须填写背景、方案、理由、正负影响、人工确认依据和关联任务。
