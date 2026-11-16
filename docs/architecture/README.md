# 架构现状

截至 2026-11-15，T01—T16 已落地：Rust 2024 workspace（`crates/knoted-core` 配置/数据库/任务队列/认证内核/暂存与内部协议客户端、`crates/knoted-api` axum 服务与健康检查/认证路由、`crates/knoted-worker` 领取-心跳-提交骨架）、pnpm workspace（`apps/web` React 外壳、`packages/api-client` 生成客户端；T17 分支接入 mtone，待集成）、`api/openapi.yaml` 与 `api/intelligence.openapi.yaml` 契约、`migrations/0001—0004`，以及 `services/intelligence/` 的 Python 执行服务（SQLite 账本、Docling+RapidOCR 解析、BGE-M3 dense 嵌入）。笔记、项目/事项、文件扫描与检索、RAG 生成尚未实现，仍按 [实施计划](../plans/knoted-v1/README.md) 逐卡推进；本页不构成 Accepted ADR。

UI 目标已按 [ADR 0002](../decisions/0002-mtone-ui-ownership.md) 改为应用直接消费 npm `@bixlab/mtone`。**T17 分支实现就绪、待主线集成**：`apps/web` 精确消费 0.2.0 的公开 Provider/CSS，`packages/ui` cn 占位包已移除；基础表单通过消费端验收，见 [T17 证据](../plans/knoted-v1/evidence/t17.md)。业务组合/数据适配留在 apps/web，上游能力缺口交用户处理，见 [UI 指南](../development/ui-mtone.md)。

| 边界 | 责任 |
|---|---|
| `crates/` | Rust workspace：core（配置/数据库/`jobs`/`auth`/`files::staging`/`intelligence` 客户端）、api（axum 路由、`middleware` request_id/认证、`admin` CLI）、worker（runner 骨架） |
| `apps/`、`packages/` | React 前端直接消费 mtone（T17 分支实现就绪、待集成）、OpenAPI 生成客户端；无本地通用 UI 包 |
| `api/`、`migrations/` | 浏览器与内部契约、数据库迁移（`0001` registry、`0002` jobs/outbox、`0003` auth、`0004` job retry） |
| `services/intelligence/` | Python（uv 管理，Python 3.12）：受控执行服务、SQLite 执行账本、离线解析/嵌入适配；不接触业务数据库与源目录 |
| `nix/` | Knoted 侧 provider 契约模块（当前作为部署契约与 preflight 参考，见 `docs/plans/knoted-v1/evidence/t12-deployment.md`） |
| `Cargo.toml`、`package.json` | 构建元数据与锁定依赖；技术栈迁移需单独评估 |
| `AGENTS.md` / `.agents/skills/` | 开发规则及项目管理流程，不加入运行时依赖 |
| `docs/decisions/` / `docs/rfcs/` | 已讨论的决策及重要方案 |
| `.github/` | GitHub 协作模板与仓库自动化 |

新增业务模块时补充责任、接口、依赖方向和测试入口。不要为了套用模板创建不存在的服务或数据库。本次管理体系初始化不构成产品架构决策，不产生虚构的 Accepted ADR。
