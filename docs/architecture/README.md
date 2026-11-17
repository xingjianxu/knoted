# 架构现状

截至 2026-11-17，T01—T35 已落地：Rust 2024 workspace（`crates/knoted-core` 配置/数据库/任务队列/认证内核/暂存与内部协议客户端、`crates/knoted-api` axum 服务与健康检查/认证/笔记/标签关联/项目/事项路由、`crates/knoted-worker` 领取-心跳-提交骨架）、pnpm workspace（`apps/web` React 外壳与笔记编辑/树/历史页面、`packages/api-client` 生成客户端；T17 已集成 mtone）、`api/openapi.yaml` 与 `api/intelligence.openapi.yaml` 契约、`migrations/0001—0010`，以及 `services/intelligence/` 的 Python 执行服务（SQLite 账本、Docling+RapidOCR 解析、BGE-M3 dense 嵌入）。笔记文档校验、CRUD 与版本 API 已实现；笔记树移动/恢复、历史快照与Worker补存、图片资产上传/鉴权读取及事务引用已实现；BlockNote 编辑、自动保存与冲突保稿、笔记树/只读历史、标签关联、项目与事项 API、终态约束与上海时区日期计算已实现；项目/事项页面、文件扫描与检索、RAG 生成留待后续任务，仍按 [实施计划](../plans/knoted-v1/README.md) 逐卡推进；本页不构成 Accepted ADR。

UI 目标已按 [ADR 0002](../decisions/0002-mtone-ui-ownership.md) 改为应用直接消费 npm `@bixlab/mtone`。**T17—T19 均已集成主线**（T17 PR #42/`e3e1db2`；T18 PR #46/`4425ab9`；T19 PR #48/`6de79a8`）：`apps/web` 当前精确消费 0.3.0 的公开 Provider/CSS（[升级记录](../plans/knoted-v1/evidence/mtone-0.3.0.md)；T17—T19 原始验收为 0.2.0），`packages/ui` cn 占位包已移除；基础表单、弹层/菜单/通知、树/游标列表/反馈通过消费端验收；生产仅新增通知 Provider 注册与应用游标 reducer。证据见 [T17](../plans/knoted-v1/evidence/t17.md)、[T18](../plans/knoted-v1/evidence/t18.md)、[T19](../plans/knoted-v1/evidence/t19.md)。业务组合/数据适配留在 apps/web；证实上游能力缺口时存档提示词、告知用户并在上游 mtone 提 Issue，见 [UI 指南](../development/ui-mtone.md)。

| 边界 | 责任 |
|---|---|
| `crates/` | Rust workspace：core（配置/数据库/`jobs`/`auth`/`files::staging`/`intelligence` 客户端、`notes` 文档校验与事务服务、`links` 标签/关联、`projects` 项目生命周期、`tasks` 单层事项/终态/日期）、api（axum 路由、`middleware` request_id/认证、`admin` CLI）、worker（runner 骨架） |
| `apps/`、`packages/` | React 前端直接消费 mtone，BlockNote 仅作笔记编辑核心，应用组合目录/草稿/历史/关联、OpenAPI 生成客户端；无本地通用 UI 包 |
| `api/`、`migrations/` | 浏览器与内部契约、数据库迁移（`0001` registry、`0002` jobs/outbox、`0003` auth、`0004` job retry、`0005` notes、`0006` history、`0007` assets、`0008` tags/links、`0009` projects、`0010` tasks） |
| `services/intelligence/` | Python（uv 管理，Python 3.12）：受控执行服务、SQLite 执行账本、离线解析/嵌入适配；不接触业务数据库与源目录 |
| `nix/` | Knoted 侧 provider 契约模块与包/模块（ADR 0003：**只给将来的生产实例**用，测试实例不再由 NixOS 运行；见 `docs/plans/knoted-v1/evidence/t12-deployment.md`） |
| `pcwrap.config.json`、`process-compose.yaml`、`tools/dev/pc/` | 测试实例的运行编排（ADR 0003）：控制端口 15600、应用配置模板、固定产物校验、API 端口检查与迁移；HTTPS 由宿主共享 Caddy 管理（ADR 0005） |
| `tests/pc/` | 编排配置、模板渲染、证书校验、端口占用与迁移失败的场景测试（`pnpm test:pc`） |
| `Cargo.toml`、`package.json` | 构建元数据与锁定依赖；技术栈迁移需单独评估 |
| `AGENTS.md` / `.agents/skills/` | 开发规则及项目管理流程，不加入运行时依赖 |
| `docs/decisions/` / `docs/rfcs/` | 已讨论的决策及重要方案 |
| `.github/` | GitHub 协作模板与仓库自动化 |

新增业务模块时补充责任、接口、依赖方向和测试入口。不要为了套用模板创建不存在的服务或数据库。本次管理体系初始化不构成产品架构决策，不产生虚构的 Accepted ADR。


测试实例的运行管理边界见 [ADR 0003](../decisions/0003-test-instance-process-management.md)：应用由 `@bixlab/pcwrap` + process-compose 在 `127.0.0.1:15600` 编排（HTTPS 15601、API 15602），数据库由宿主用户直接创建（[ADR 0004](../decisions/0004-database-owned-by-host-user.md)），Strata/模型共享根/ACME 仍由 `/etc/nixos` 管理，生产实例将来才由 NixOS 运行。真实切换已随 PR #64 完成（运行 `4a546b2`，[证据](../plans/knoted-v1/evidence/pcwrap-dev-service.md)）。

T23 的 `notes::service::{create,get,list,update,delete}` 消费 `AppContext` 与命名输入，复用 T22 文档校验；SQL 在 `notes::repository`，service 负责事务，HTTP 返回 T03 的 Note/NotePage、ETag 与统一错误。创建/软删除使用同一 notes 树事务锁维护父可见性与同级 position；T24 结构写和 T32 笔记链接写遵守相同树锁顺序。内容更新以 ID/version/live CAS，实体注册、语义版本和 outbox 同事务。`note.changed` 当前只保留未发布事件，索引消费属后续卡；测试入口 `crates/knoted-api/tests/t23.rs`。详见 [T23证据](../plans/knoted-v1/evidence/t23.md)。

HTTPS 管理归属已由 [ADR 0005](../decisions/0005-https-owned-by-caddy.md) 调整：宿主共享 Caddy 提供 15601、静态 SPA 和 API/health 代理；pcwrap 仅准备应用配置、迁移与 API，不再包含 knoted-https 或专用 reload。

T24–T26 已完成页面树移动/恢复、历史快照/补存及图片资产引用。当前/回收站/快照图片引用在同一调用方事务维护，引用事务锁先于树和笔记行锁；未来T67回收须遵守同一锁顺序并读取zero_ref_at。资产以UUID路径私有发布，由ready登记与会话鉴权控制读取。实施、限制及运行证据见[T26](../plans/knoted-v1/evidence/t26-assets.md)。

T27—T31 的编辑器只在每次挂载时读取正文，应用以 `noteId:resetEpoch` 为 key；新挂载从保留会话的当前草稿初始化，普通自动保存与 Query 刷新不重建实例。草稿由 Notes 工作区拥有，Query 只保存服务器状态；后台更新只提示，显式加载/历史恢复才整体替换草稿和 CAS。移动与恢复使用完整请求屏障，导航先保存/补快照，退出先安全离开再撤会话；已初始化笔记遇401时保留可复制草稿并暂停后续写入。目录读完全部游标页后按真实父子关系组树；历史是真正只读的独立编辑器。

T32 的分类型标签/关联表复用 registry 稳定身份，再以实际业务表核实可见性；归档可读、写受限，删除或文件失踪后保留目标 UUID。T33—T35 项目/事项关系写先取现有 `33001/projects OID` 事务锁稳定发现集合，再将项目与事项行按全局 UUID 顺序锁定；CAS、级联版本和 outbox 同事务。父事项只有一层子项；父换项目同步直属子，存活归档或历史错配子拒绝，已删除子仅迁关系/版本而不复活。父 done 要求全部存活子 done/cancelled，创建、重开与换父共同验证完整最终状态。DateValue 原样保存，使用 IANA Asia/Shanghai 解释本地边界并拒绝歧义/不存在时刻；日期型截止取下一本地日午夜，逾期计算注入时钟，不增加存储派生字段。实现与集成证据见 [T27—T35](../plans/knoted-v1/evidence/t27-t35-integration.md)。
