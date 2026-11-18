# 架构现状

截至 2026-11-18，主线已包含 Rust 2024 workspace、pnpm workspace、浏览器/内部 OpenAPI 契约和 `migrations/0001—0015`。笔记、项目与事项领域及页面、看板/时间线、工作概览、统一回收站，以及只读文件扫描、稳定采集、保守身份识别/手动重关联、Rust 文本解析与成功提取发布已通过主线和 dev 验收。PDF/PNG/JPEG/WebP 已按 [ADR0007](../decisions/0007-execution-exit-confirmation.md) 接入真实提取链。当前 dev 使用同一固定 SHA 的 API/Worker/intelligence 联合产物，Python 通过 SQLite supervision/guardian 确认计算退出；主线、产物与活动计算重启验收已完成。Office DOCX/XLSX/PPTX 也已完成真实提取链的主线/dev 验收；文件浏览、元数据编辑和原件下载也已验收；预览、索引与 RAG 继续按 [任务状态](../plans/knoted-v1/README.md) 推进。索引尚未开放，本页不构成 Accepted ADR。

UI 目标已按 [ADR 0002](../decisions/0002-mtone-ui-ownership.md) 改为应用直接消费 npm `@bixlab/mtone`。**T17—T19 均已集成主线**（T17 PR #42/`e3e1db2`；T18 PR #46/`4425ab9`；T19 PR #48/`6de79a8`）：`apps/web` 当前精确消费 0.4.0 的公开 Provider/CSS（[升级记录](../plans/knoted-v1/evidence/mtone-0.4.0.md)；T17—T19 原始验收为 0.2.0），`packages/ui` cn 占位包已移除；基础表单、弹层/菜单/通知、树/游标列表/反馈通过消费端验收；生产仅新增通知 Provider 注册与应用游标 reducer。证据见 [T17](../plans/knoted-v1/evidence/t17.md)、[T18](../plans/knoted-v1/evidence/t18.md)、[T19](../plans/knoted-v1/evidence/t19.md)。业务组合/数据适配留在 apps/web；证实上游能力缺口时存档提示词、告知用户并在上游 mtone 提 Issue，见 [UI 指南](../development/ui-mtone.md)。

| 边界 | 责任 |
|---|---|
| `crates/` | Rust workspace：core（配置/数据库/`jobs`/`auth`/`intelligence` 客户端、`notes`、`links`、`projects`、`tasks`、`dashboard`/`trash` 领域服务及 `files` 的 FD 安全读取/扫描/采集/身份/文本/PDF图片/Office/提取发布）、api（axum 业务路由、认证与 request_id、`admin` CLI）、worker（单 Dispatcher/租约 runner、历史补存、事项回收、文件扫描、文本与受监督 Python 提取） |
| `apps/`、`packages/` | React 前端直接消费 mtone，BlockNote 仅作笔记编辑核心，应用组合目录/草稿/历史/关联、项目事项/看板时间线/概览回收站、OpenAPI 生成客户端；无本地通用 UI 包 |
| `api/`、`migrations/` | 浏览器与内部契约、数据库迁移（`0001` registry、`0002` jobs/outbox、`0003` auth、`0004` job retry、`0005` notes、`0006` history、`0007` assets、`0008` tags/links、`0009` projects、`0010` tasks、`0011` activities、`0012` 文件根/扫描/观察/来源、`0013` 闭合采集、`0014` 移动资格、`0015` 成功提取） |
| `services/intelligence/` | Python（uv 管理，Python 3.12）：受控执行服务、SQLite execution/supervision/退出凭证、guardian 全后代回收、固定解释器与离线解析/嵌入适配；不接触业务数据库与源目录 |
| `nix/` | Knoted 侧 provider 契约模块与包/模块（ADR 0003：**只给将来的生产实例**用，测试实例不再由 NixOS 运行；见 `docs/plans/knoted-v1/evidence/t12-deployment.md`） |
| `pcwrap.config.json`、`process-compose.yaml`、`tools/dev/pc/` | 测试实例的运行编排（ADR 0003）：控制端口 15600、应用配置模板、固定产物校验、API/intelligence 端口检查与迁移、三运行进程及依赖健康；HTTPS 由宿主共享 Caddy 管理（ADR 0005） |
| `tests/pc/` | 编排配置、模板渲染、固定产物校验、端口占用与迁移失败的场景测试（`pnpm test:pc`） |
| `Cargo.toml`、`package.json` | 构建元数据与锁定依赖；技术栈迁移需单独评估 |
| `AGENTS.md` / `.agents/skills/` | 开发规则及项目管理流程，不加入运行时依赖 |
| `docs/decisions/` / `docs/rfcs/` | 已讨论的决策及重要方案 |
| `.github/` | GitHub 协作模板与仓库自动化 |

新增业务模块时补充责任、接口、依赖方向和测试入口。不要为了套用模板创建不存在的服务或数据库。本次管理体系初始化不构成产品架构决策，不产生虚构的 Accepted ADR。


测试实例的运行管理边界见 [ADR 0003](../decisions/0003-test-instance-process-management.md)：应用由 `@bixlab/pcwrap` + process-compose 在 `127.0.0.1:15600` 编排（HTTPS 15601、API 15602），数据库由宿主用户直接创建（[ADR 0004](../decisions/0004-database-owned-by-host-user.md)），Strata/模型共享根/ACME 仍由 `/etc/nixos` 管理，生产实例将来才由 NixOS 运行。真实切换已随 PR #64 完成（运行 `4a546b2`，[证据](../plans/knoted-v1/evidence/pcwrap-dev-service.md)）。

T23 的 `notes::service::{create,get,list,update,delete}` 消费 `AppContext` 与命名输入，复用 T22 文档校验；SQL 在 `notes::repository`，service 负责事务，HTTP 返回 T03 的 Note/NotePage、ETag 与统一错误。创建/软删除使用同一 notes 树事务锁维护父可见性与同级 position；T24 结构写和 T32 笔记链接写遵守相同树锁顺序。内容更新以 ID/version/live CAS，实体注册、语义版本和 outbox 同事务。`note.changed` 当前只保留未发布事件，索引消费属后续卡；测试入口 `crates/knoted-api/tests/t23.rs`。详见 [T23证据](../plans/knoted-v1/evidence/t23.md)。

HTTPS 管理归属已由 [ADR 0005](../decisions/0005-https-owned-by-caddy.md) 调整：宿主共享 Caddy 提供 15601、静态 SPA 和 API/health 代理；pcwrap 准备应用配置、迁移并运行 API/Worker/intelligence，不再包含 knoted-https 或专用 reload。

T24–T26 已完成页面树移动/恢复、历史快照/补存及图片资产引用。当前/回收站/快照图片引用在同一调用方事务维护，引用事务锁先于树和笔记行锁；未来T67回收须遵守同一锁顺序并读取zero_ref_at。资产以UUID路径私有发布，由ready登记与会话鉴权控制读取。实施、限制及运行证据见[T26](../plans/knoted-v1/evidence/t26-assets.md)。

T27—T31 的编辑器只在每次挂载时读取正文，应用以 `noteId:resetEpoch` 为 key；新挂载从保留会话的当前草稿初始化，普通自动保存与 Query 刷新不重建实例。草稿由 Notes 工作区拥有，Query 只保存服务器状态；后台更新只提示，显式加载/历史恢复才整体替换草稿和 CAS。移动与恢复使用完整请求屏障，导航先保存/补快照，退出先安全离开再撤会话；已初始化笔记遇401时保留可复制草稿并暂停后续写入。目录读完全部游标页后按真实父子关系组树；历史是真正只读的独立编辑器。

T32 的分类型标签/关联表复用 registry 稳定身份，再以实际业务表核实可见性；归档可读、写受限，删除或文件失踪后保留目标 UUID。T33—T35 项目/事项关系写先取现有 `33001/projects OID` 事务锁稳定发现集合，再将项目与事项行按全局 UUID 顺序锁定；CAS、级联版本和 outbox 同事务。父事项只有一层子项；父换项目同步直属子，存活归档或历史错配子拒绝，已删除子仅迁关系/版本而不复活。父 done 要求全部存活子 done/cancelled，创建、重开与换父共同验证完整最终状态。DateValue 原样保存，使用 IANA Asia/Shanghai 解释本地边界并拒绝歧义/不存在时刻；日期型截止取下一本地日午夜，逾期计算注入时钟，不增加存储派生字段。实现与集成证据见 [T27—T35](../plans/knoted-v1/evidence/t27-t35-integration.md)。

T37 的 `tasks::lifecycle` 复用关系锁、CAS 和 `task.changed` outbox；归档单项且不级联、仍可读，父/项目只读封闭正文、跟进、标签和 source-link 写入口。恢复保留当前父/项目并重验终态约束，父已删时先恢复父。Worker 启动及每 30 秒独立执行 task-only 清理：满 30 天叶子先删、尚存子行阻止父清理，时间线和 tag 绑定同事务清除，保留 registry/失效 links/shared tags/outbox；不引入迁移或虚构搜索投影。证据见 [T37](../plans/knoted-v1/evidence/t37-lifecycle.md)。

编辑输入使用公开onBeforeChange的候选事务tr.doc，在块ID分配前检查实际链接标记，复用原URI校验；不依赖getChanges将临时节点转换为Block。无法保存的URI整次拒绝并反馈，原文/选区保留，后续有效编辑继续进入草稿。实现与真实剪贴板/正常建块验收见[T27恢复证据](../plans/knoted-v1/evidence/t27-clipboard-recovery.md)。


T38—T42 已交付项目/事项页面、看板与跟进时间线、默认概览和统一回收站。概览消费注入时钟与上海日期规则，回收站读取历史快照；CAS/状态门槛失败保留草稿，成功响应更新查询缓存。实现与真实 HTTPS 证据分别见 [T39](../plans/knoted-v1/evidence/t39-task-page.md)、[T40](../plans/knoted-v1/evidence/t40-board-timeline.md)、[T41](../plans/knoted-v1/evidence/t41-dashboard.md)、[T42](../plans/knoted-v1/evidence/t42-trash.md)。

T43—T45 的 `files` 通过根身份与扫描代次管理只读来源；`safe_fs` 逐段 FD/NOFOLLOW，通知与周期扫描由单 Worker Dispatcher 消费。扫描中断或根不确定时暂停，不批量宣告失踪；只有稳定且完整校验的闭合对象才能更新来源。`FileSource.version` 用于元数据 CAS/ETag，`content_version` 仅随正文摘要变化；同摘要不证明身份，移动需要强证明，否则保守建立新来源，手动 relink 原子迁移元数据/关系并保留历史身份。契约见 [ADR 0006](../decisions/0006-file-pipeline-and-aggregate-contracts.md)，运行与边界见 [T43](../plans/knoted-v1/evidence/t43-scanner.md)、[T44](../plans/knoted-v1/evidence/t44-capture.md)、[T45](../plans/knoted-v1/evidence/t45-identity.md)。

T46 的 `files::text/extraction` 读取已校验闭合输入，严格 UTF-8/UTF-16 BOM，保留原文与结构位置，拒绝非法编码/NUL。T46 当时仅对明确文本白名单开放；T47 已接续 PDF/图片路径。成功在租约、当前根/来源/观察栅栏下写入 `file_extractions` 与 outbox，唯一身份为来源/内容版本/解析器/选项，元数据修改不重算。确定失败的同语义工作不因新扫描重新获得预算；取消的旧观察仍可由新捕获补排。Rust 文本记录 execution_id 为 NULL，未伪造 Python 执行 ID。测试入口为 core/worker 的 `t46_` 与 Web 概览回归，实际命令/主线/dev/清理见 [T46](../plans/knoted-v1/evidence/t46-text.md)。当前 outbox 尚无全文/向量发布消费者，不将解析成功等同检索可用。

T47 的 `files::pdf` 与 Worker 在持久 lease attempt 登记后调用内部 intelligence，使用原请求/原 deadline 收束旧尝试，真实 execution_id 随成功写入。业务终态与资源退出证明分离；guardian 持久 GO、pidfd/subreaper、回收到 ECHILD 再签发 receipt，无法证明则拒绝新计算。Python 首次 execution/supervision 同事务，运行期只调和未解决记录；发布事务最后以 PG clock_timestamp 检查截止，过期回滚成功/outbox。pcwrap 用同一完整 SHA 的 Rust/Web/Python 产物运行 API 15602 与 loopback intelligence 15603，Worker 等健康；模型只读、显式 CPU、state/cache/tmp/staging 外置。首次部署/搬移清单/真实解析及活动计算重启验收见 [T47](../plans/knoted-v1/evidence/t47-parse.md) 与 [ADR0007](../decisions/0007-execution-exit-confirmation.md)。Office 已由 `files::office` 按持久格式身份及共有受界 artifact 读取接入相同发布链，见 [T48](../plans/knoted-v1/evidence/t48-office.md)。XLSX 保留工作表/A1/缓存公式，PPTX 保留原页序，DOCX/XLSX 不虚构页码；仅受界 ZIP32 OOXML 正文子集，不执行 Office/外联/公式。文件浏览、备注标签和当前原件下载已通过 T49 主线/dev 验收；PDF/静态图片/严格文本预览已完成 T50 主线/dev 验收，检索/RAG未启用。

T49 的 `files::browse/download` 提供 UUID 游标列表、missing 详情、备注标签可选字段更新和联合CAS，以及当前批准根FD的完整有界原件读取。下载在100MiB/SHA/捕获观察/最终root→source栅栏后响应attachment、元数据ETag与private/no-store/nosniff；不读历史staging，不提供Range/304。Web保留字段草稿与base、仅提交实际变化字段，401卸载/取消，普通Blob下载独立于T50的50MiB。真实API、中文文件名Chromium下载/重关联/撤销会话和八格式原件校验见 [T49证据](../plans/knoted-v1/evidence/t49-files.md)。下载完整缓冲，不是性能或文件系统外部写入冻结承诺。

T50 由 FileDetail 捕获打开时的 FileSource，交给受控 FilePreview；流式逐块限制50MiB，文本转义/严格编码、静态图片解码前40M像素检查、PDF鉴权Blob与原生iframe。每次身份拥有独立abort/URL，关闭、切换、会话失效撤销；普通原件下载使用同一快照及ETag，不受预览50MiB限制。实际Chrome158三图片解码、两页PDF、HTML/UTF16、52,428,801字节下载与401清理见 [T50证据](../plans/knoted-v1/evidence/t50-preview.md)。预览不等于文档净化或跨浏览器兼容承诺。
