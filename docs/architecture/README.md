# 架构现状

截至 2026-11-16，T01—T23 已落地：Rust 2024 workspace（`crates/knoted-core` 配置/数据库/任务队列/认证内核/暂存与内部协议客户端、`crates/knoted-api` axum 服务与健康检查/认证/笔记路由、`crates/knoted-worker` 领取-心跳-提交骨架）、pnpm workspace（`apps/web` React 外壳、`packages/api-client` 生成客户端；T17 已集成 mtone）、`api/openapi.yaml` 与 `api/intelligence.openapi.yaml` 契约、`migrations/0001—0005`，以及 `services/intelligence/` 的 Python 执行服务（SQLite 账本、Docling+RapidOCR 解析、BGE-M3 dense 嵌入）。笔记文档校验、CRUD 与版本 API 已实现；笔记树移动/恢复、快照、编辑器及项目/事项、文件扫描与检索、RAG 生成留待后续任务，仍按 [实施计划](../plans/knoted-v1/README.md) 逐卡推进；本页不构成 Accepted ADR。

UI 目标已按 [ADR 0002](../decisions/0002-mtone-ui-ownership.md) 改为应用直接消费 npm `@bixlab/mtone`。**T17—T19 均已集成主线**（T17 PR #42/`e3e1db2`；T18 PR #46/`4425ab9`；T19 PR #48/`6de79a8`）：`apps/web` 当前精确消费 0.3.0 的公开 Provider/CSS（[升级记录](../plans/knoted-v1/evidence/mtone-0.3.0.md)；T17—T19 原始验收为 0.2.0），`packages/ui` cn 占位包已移除；基础表单、弹层/菜单/通知、树/游标列表/反馈通过消费端验收；生产仅新增通知 Provider 注册与应用游标 reducer。证据见 [T17](../plans/knoted-v1/evidence/t17.md)、[T18](../plans/knoted-v1/evidence/t18.md)、[T19](../plans/knoted-v1/evidence/t19.md)。业务组合/数据适配留在 apps/web；证实上游能力缺口时存档提示词、告知用户并在上游 mtone 提 Issue，见 [UI 指南](../development/ui-mtone.md)。

| 边界 | 责任 |
|---|---|
| `crates/` | Rust workspace：core（配置/数据库/`jobs`/`auth`/`files::staging`/`intelligence` 客户端、`notes` 文档校验与事务服务）、api（axum 路由、`middleware` request_id/认证、`admin` CLI）、worker（runner 骨架） |
| `apps/`、`packages/` | React 前端直接消费 mtone（T17—T19 已集成）、OpenAPI 生成客户端；无本地通用 UI 包 |
| `api/`、`migrations/` | 浏览器与内部契约、数据库迁移（`0001` registry、`0002` jobs/outbox、`0003` auth、`0004` job retry、`0005` notes） |
| `services/intelligence/` | Python（uv 管理，Python 3.12）：受控执行服务、SQLite 执行账本、离线解析/嵌入适配；不接触业务数据库与源目录 |
| `nix/` | Knoted 侧 provider 契约模块与包/模块（ADR 0003：**只给将来的生产实例**用，测试实例不再由 NixOS 运行；见 `docs/plans/knoted-v1/evidence/t12-deployment.md`） |
| `pcwrap.config.json`、`process-compose.yaml`、`tools/dev/pc/` | 测试实例的运行编排（ADR 0003）：控制端口 15600、配置/Caddyfile 模板、证书校验、固定产物校验、迁移与 reload 入口 |
| `tests/pc/` | 编排配置、模板渲染、证书校验、端口占用与迁移失败的场景测试（`pnpm test:pc`） |
| `Cargo.toml`、`package.json` | 构建元数据与锁定依赖；技术栈迁移需单独评估 |
| `AGENTS.md` / `.agents/skills/` | 开发规则及项目管理流程，不加入运行时依赖 |
| `docs/decisions/` / `docs/rfcs/` | 已讨论的决策及重要方案 |
| `.github/` | GitHub 协作模板与仓库自动化 |

新增业务模块时补充责任、接口、依赖方向和测试入口。不要为了套用模板创建不存在的服务或数据库。本次管理体系初始化不构成产品架构决策，不产生虚构的 Accepted ADR。


测试实例的运行管理边界见 [ADR 0003](../decisions/0003-test-instance-process-management.md)：应用由 `@bixlab/pcwrap` + process-compose 在 `127.0.0.1:15600` 编排（HTTPS 15601、API 15602），数据库由宿主用户直接创建（[ADR 0004](../decisions/0004-database-owned-by-host-user.md)），Strata/模型共享根/ACME 仍由 `/etc/nixos` 管理，生产实例将来才由 NixOS 运行。真实切换已随 PR #64 完成（运行 `4a546b2`，[证据](../plans/knoted-v1/evidence/pcwrap-dev-service.md)）。

T23 的 `notes::service::{create,get,list,update,delete}` 消费 `AppContext` 与命名输入，复用 T22 文档校验；SQL 在 `notes::repository`，service 负责事务，HTTP 返回 T03 的 Note/NotePage、ETag 与统一错误。创建/软删除使用同一 notes 树事务锁维护父可见性与同级 position；未来 T24 结构写必须遵守相同锁顺序。内容更新以 ID/version/live CAS，实体注册、语义版本和 outbox 同事务。`note.changed` 当前只保留未发布事件，索引消费属后续卡；测试入口 `crates/knoted-api/tests/t23.rs`。详见 [T23证据](../plans/knoted-v1/evidence/t23.md)。
