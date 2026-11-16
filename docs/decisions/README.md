# 架构决策记录

本目录记录已确认的产品架构决策；索引条目以 ADR 自身的 Status 为准，索引本身不预先批准任何方案。

新增时使用 [template.md](template.md) 和 [ADR 指南](../development/adr-guide.md)。命名 `NNNN-short-title.md`，编号由主 Agent 在集成前按最大现有编号串行分配；模板不编号。每个正式 ADR 加入下表，替代时更新双方链接。不要用文档条数推算编号。

| 编号 | 标题 / 文件 | 日期 | 状态 | 替代关系 |
| --- | --- | --- | --- | --- |
| 0001 | 模型 artifact 采用机器级共享根并按 revision 分层（[文件](0001-machine-level-model-artifact-roots.md)） | 2026-11-15 | Accepted | 无 |
| 0002 | 通用 UI 由 mtone 提供，Knoted 直接消费 npm 包（[文件](0002-mtone-ui-ownership.md)） | 2026-11-16 | Accepted | 无；替换 v1 自有 UI 提案 |
| 0003 | 测试实例由 pcwrap 编排，NixOS 只保留外部依赖与未来生产（[文件](0003-test-instance-process-management.md)） | 2026-11-16 | Accepted | 无；替代 T21 的「测试实例由 NixOS 单元运行」边界 |

新 ADR 必须填写背景、方案、理由、正负影响、人工确认依据和关联任务。
