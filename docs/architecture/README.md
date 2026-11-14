# 架构现状

截至 2026-11-14，T01—T05 已落地基础骨架：Rust 2024 workspace（`crates/knoted-core` 配置与数据库、`crates/knoted-api`、`crates/knoted-worker`）、pnpm workspace（`apps/web` React 外壳、`packages/ui`、`packages/api-client` 生成客户端）、`api/openapi.yaml` 与 `api/intelligence.openapi.yaml` 契约、`migrations/` 迁移。业务模块、认证、笔记、文件与 RAG 尚未实现，仍按 [实施计划](../plans/knoted-v1/README.md) 逐卡推进；本页不构成 Accepted ADR。

| 边界 | 责任 |
|---|---|
| `crates/` | Rust workspace：core（配置/数据库）、api、worker 进程入口 |
| `apps/`、`packages/` | React 前端、UI 基础包与 OpenAPI 生成客户端 |
| `api/`、`migrations/` | 浏览器与内部契约、数据库迁移 |
| `Cargo.toml`、`package.json` | 构建元数据与锁定依赖；技术栈迁移需单独评估 |
| `AGENTS.md` / `.agents/skills/` | 开发规则及项目管理流程，不加入运行时依赖 |
| `docs/decisions/` / `docs/rfcs/` | 已讨论的决策及重要方案 |
| `.github/` | GitHub 协作模板与仓库自动化 |

新增业务模块时补充责任、接口、依赖方向和测试入口。不要为了套用模板创建不存在的服务或数据库。本次管理体系初始化不构成产品架构决策，不产生虚构的 Accepted ADR。
