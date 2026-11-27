# 架构现状

主线包含 Rust 2024 workspace、pnpm workspace、浏览器 OpenAPI 契约和全新数据库的 `migrations/0001—0017`。Worker 自动扫描许可来源，文件浏览、下载及 PDF、图片、文本和 Office 预览使用只读原件。笔记、项目和事项的全文命中要经 PostgreSQL 复核。CLI 提供当前格式的一致备份和隔离恢复。[ADR 0014](../decisions/0014-retire-file-index-rag.md) · [v1 历史](../plans/knoted-v1/README.md)

UI 目标已按 [ADR 0002](../decisions/0002-mtone-ui-ownership.md) 改为应用直接消费 npm `@bixlab/mtone`。**T17—T19 均已集成主线**（T17 PR #42/`e3e1db2`；T18 PR #46/`4425ab9`；T19 PR #48/`6de79a8`）：`apps/web` 使用锁定版本的公开 Provider/CSS（[当前版本](../development/ui-mtone.md#现状与目标)；T17—T19 原始验收为 0.2.0），`packages/ui` cn 占位包已移除；基础表单、弹层/菜单/通知、树/游标列表/反馈通过消费端验收；生产仅新增通知 Provider 注册与应用游标 reducer。证据见 [T17](../plans/knoted-v1/evidence/t17.md)、[T18](../plans/knoted-v1/evidence/t18.md)、[T19](../plans/knoted-v1/evidence/t19.md)。业务组合/数据适配留在 apps/web；证实上游能力缺口时存档提示词、告知用户并在上游 mtone 提 Issue，见 [UI 指南](../development/ui-mtone.md)。

| 边界 | 责任 |
|---|---|
| `crates/` | core 提供领域服务、配置、数据库、文件采集和搜索。API 提供认证的业务路由及备份恢复 CLI。Worker 执行租约任务、自动扫描、历史补存、回收和搜索发布 |
| `apps/`、`packages/` | React 前端使用 mtone 和应用业务组合，客户端由 OpenAPI 生成 |
| `api/`、`migrations/` | 浏览器契约、业务数据库结构、笔记／项目／事项搜索与维护证据 |
| `nix/` | 固定发布包、生产 systemd 模块与虚拟机验收。宿主负责生产站点、外部依赖与持久启用 |
| `pcwrap.config.json`、`process-compose.yaml`、`tools/dev/pc/` | 测试实例的运行编排（ADR 0003），提供固定产物校验、迁移及 API 与 Worker 运行。HTTPS 由宿主共享 Caddy 管理（ADR 0005） |
| `tests/pc/` | 编排配置、模板渲染、固定产物校验、端口占用与迁移失败的场景测试（`pnpm test:pc`） |
| `Cargo.toml`、`package.json` | 构建元数据与锁定依赖；技术栈迁移需单独评估 |
| `AGENTS.md` / `.agents/skills/` | 开发规则及项目管理流程，不加入运行时依赖 |
| `docs/decisions/` / `docs/rfcs/` | 已讨论的决策及重要方案 |
| `.github/` | GitHub 协作模板与仓库自动化 |

新增业务模块时补充责任、接口、依赖方向和测试入口。不要为了套用模板创建不存在的服务或数据库。本次管理体系初始化不构成产品架构决策，不产生虚构的 Accepted ADR。


测试实例的运行管理边界见 [ADR 0003](../decisions/0003-test-instance-process-management.md)：应用由 `@bixlab/pcwrap` + process-compose 在 `127.0.0.1:15600` 编排（HTTPS 15601、API 15602），数据库由宿主用户直接创建（[ADR 0004](../decisions/0004-database-owned-by-host-user.md)），ACME 由 `/etc/nixos` 管理。pcwrap 首次切换记录见 [历史证据](../plans/knoted-v1/evidence/pcwrap-dev-service.md)。独立生产实例使用 NixOS 模块、单独的数据库和数据根。[T70 主机验收](../plans/knoted-v1/evidence/t70-deployment.md)

T23 的 `notes::service::{create,get,list,update,delete}` 消费 `AppContext` 与命名输入，复用 T22 文档校验；SQL 在 `notes::repository`，service 负责事务，HTTP 返回 T03 的 Note/NotePage、ETag 与统一错误。创建/软删除使用同一 notes 树事务锁维护父可见性与同级 position；T24 结构写和 T32 笔记链接写遵守相同树锁顺序。内容更新以 ID/version/live CAS，实体注册、语义版本和 outbox 同事务。`note.changed` 由搜索 Worker 消费。测试入口 `crates/knoted-api/tests/t23.rs`。详见 [T23证据](../plans/knoted-v1/evidence/t23.md)。

HTTPS 管理归属已由 [ADR 0005](../decisions/0005-https-owned-by-caddy.md) 调整：宿主共享 Caddy 提供 15601、静态 SPA 和 API/health 代理；pcwrap 准备应用配置、迁移并运行 API 和 Worker。

T24–T26 已完成页面树移动/恢复、历史快照/补存及图片资产引用。当前/回收站/快照图片引用在同一调用方事务维护，引用事务锁先于树和笔记行锁。T67 回收遵循同一锁顺序并依据 `zero_ref_at` 保留历史图片。[T67 回收证据](../plans/knoted-v1/evidence/t67-reclamation.md)资产以UUID路径私有发布，由ready登记与会话鉴权控制读取。实施、限制及运行证据见[T26](../plans/knoted-v1/evidence/t26-assets.md)。

T27—T31 的编辑器只在每次挂载时读取正文，应用以 `noteId:resetEpoch` 为 key；新挂载从保留会话的当前草稿初始化，普通自动保存与 Query 刷新不重建实例。草稿由 Notes 工作区拥有，Query 只保存服务器状态；后台更新只提示，显式加载/历史恢复才整体替换草稿和 CAS。移动与恢复使用完整请求屏障，导航先保存/补快照，退出先安全离开再撤会话；已初始化笔记遇401时保留可复制草稿并暂停后续写入。目录读完全部游标页后按真实父子关系组树；历史是真正只读的独立编辑器。

T32 的分类型标签/关联表复用 registry 稳定身份，再以实际业务表核实可见性；归档可读、写受限，删除或文件失踪后保留目标 UUID。T33—T35 项目/事项关系写先取现有 `33001/projects OID` 事务锁稳定发现集合，再将项目与事项行按全局 UUID 顺序锁定；CAS、级联版本和 outbox 同事务。父事项只有一层子项；父换项目同步直属子，存活归档或历史错配子拒绝，已删除子仅迁关系/版本而不复活。父 done 要求全部存活子 done/cancelled，创建、重开与换父共同验证完整最终状态。DateValue 原样保存，使用 IANA Asia/Shanghai 解释本地边界并拒绝歧义/不存在时刻；日期型截止取下一本地日午夜，逾期计算注入时钟，不增加存储派生字段。实现与集成证据见 [T27—T35](../plans/knoted-v1/evidence/t27-t35-integration.md)。

T37 的 `tasks::lifecycle` 复用关系锁、CAS 和 `task.changed` outbox；归档单项且不级联、仍可读，父/项目只读封闭正文、跟进、标签和 source-link 写入口。恢复保留当前父/项目并重验终态约束，父已删时先恢复父。Worker 启动及每 30 秒独立执行 task-only 清理：满 30 天叶子先删、尚存子行阻止父清理，时间线和 tag 绑定同事务清除，保留 registry/失效 links/shared tags/outbox；不引入迁移或虚构搜索投影。证据见 [T37](../plans/knoted-v1/evidence/t37-lifecycle.md)。

编辑输入使用公开onBeforeChange的候选事务tr.doc，在块ID分配前检查实际链接标记，复用原URI校验；不依赖getChanges将临时节点转换为Block。无法保存的URI整次拒绝并反馈，原文/选区保留，后续有效编辑继续进入草稿。实现与真实剪贴板/正常建块验收见[T27恢复证据](../plans/knoted-v1/evidence/t27-clipboard-recovery.md)。


T38—T42 已交付项目/事项页面、看板与跟进时间线、默认概览和统一回收站。概览消费注入时钟与上海日期规则，回收站读取历史快照；CAS/状态门槛失败保留草稿，成功响应更新查询缓存。实现与真实 HTTPS 证据分别见 [T39](../plans/knoted-v1/evidence/t39-task-page.md)、[T40](../plans/knoted-v1/evidence/t40-board-timeline.md)、[T41](../plans/knoted-v1/evidence/t41-dashboard.md)、[T42](../plans/knoted-v1/evidence/t42-trash.md)。

T43—T45 的 `files` 通过根身份与扫描代次管理只读来源；`safe_fs` 逐段 FD/NOFOLLOW，通知与周期扫描由单 Worker Dispatcher 消费。扫描中断或根不确定时暂停，不批量宣告失踪；只有稳定且完整校验的闭合对象才能更新来源。`FileSource.version` 用于元数据 CAS/ETag，`content_version` 仅随正文摘要变化；同摘要不证明身份，移动需要强证明，否则保守建立新来源，手动 relink 原子迁移元数据/关系并保留历史身份。契约见 [ADR 0006](../decisions/0006-file-pipeline-and-aggregate-contracts.md)，运行与边界见 [T43](../plans/knoted-v1/evidence/t43-scanner.md)、[T44](../plans/knoted-v1/evidence/t44-capture.md)、[T45](../plans/knoted-v1/evidence/t45-identity.md)。

T49 的 `files::browse/download` 提供 UUID 游标列表、missing 详情、备注标签可选字段更新和联合CAS，以及当前批准根FD的完整有界原件读取。下载在100MiB/SHA/捕获观察/最终root→source栅栏后响应attachment、元数据ETag与private/no-store/nosniff；不读历史staging，不提供Range/304。Web保留字段草稿与base、仅提交实际变化字段，401卸载/取消，普通Blob下载独立于T50的50MiB。真实API、中文文件名Chromium下载/重关联/撤销会话和八格式原件校验见 [T49证据](../plans/knoted-v1/evidence/t49-files.md)。下载完整缓冲，不是性能或文件系统外部写入冻结承诺。

T50 由 FileDetail 捕获打开时的 FileSource，交给受控 FilePreview；流式逐块限制50MiB，文本转义/严格编码、静态图片解码前40M像素检查、PDF鉴权Blob与原生iframe。每次身份拥有独立abort/URL，关闭、切换、会话失效撤销；普通原件下载使用同一快照及ETag，不受预览50MiB限制。实际Chrome158三图片解码、两页PDF、HTML/UTF16、52,428,801字节下载与401清理见 [T50证据](../plans/knoted-v1/evidence/t50-preview.md)。预览不等于文档净化或跨浏览器兼容承诺。

同一强身份的新采集与旧闭合对象的完整 SHA 和大小一致时，复用现有对象 ID。手动重关联在打开旧根前核对当前启用配置和路径。实现见 [文件链路修复](../plans/knoted-v1/evidence/t39-t50-final-fixes.md)。

搜索 Worker 消费笔记、项目和事项的领域事件。Meilisearch 完成提交才发布，读取时在 PostgreSQL 复核当前内容版本、标签、项目关系和删除状态。索引服务恢复后，运维可用 `knoted-api admin retry-search --job <UUID> --config <PATH>` 重投耗尽重试的搜索任务。[退出决策](../decisions/0014-retire-file-index-rag.md)

T68 把一致数据库快照和登记图片发布成带摘要的私有归档。T69 验证归档后恢复到新实例，撤销旧会话、保留 Passkey 和业务历史并重新采集及建立派生索引。生产生命周期与依赖降级在隔离 NixOS 验收，高风险业务与故障证据逐项登记。[ADR 0011](../decisions/0011-consistent-backup-and-isolated-restore.md) · [T69 恢复](../plans/knoted-v1/evidence/t69-isolated-restore.md) · [T71 矩阵](../plans/knoted-v1/evidence/t71-acceptance-matrix.md)
