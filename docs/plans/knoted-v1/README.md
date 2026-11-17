# Knoted v1 文档与实施任务入口

> 产品实施采用实际可用的 executing-plans 或 subagent-driven-development。当前 T01—T26 已完成、审查并集成到主线（T17 为 PR #42 / `e3e1db2`；T18 为 PR #46 / `4425ab9`；T19 为 PR #48 / `6de79a8`；T22 为 PR #58 / `8d4a472`）；T27—T35 也已完成并通过主线与适用实例验收；T36—T72 仍待领取。

## 阅读顺序

1. [需求与确认记录](requirements.md)
2. [架构与目录](architecture.md)
3. [数据与API](data-and-api.md)
4. [索引与RAG](indexing-and-rag.md)
5. [开发与部署](development-and-deploy.md)
6. [验收](acceptance.md)
7. 前端任务先读 [UI 开发指南](../../development/ui-mtone.md) 与 [ADR 0002](../../decisions/0002-mtone-ui-ownership.md)：通用组件消费 mtone；证实通用缺口时存档提示词、告知用户并在上游 mtone 提 Issue；T17—T19 均已集成主线（T19 应用游标 reducer 已落地，下游消费需在加载期禁用翻页）。
8. 按依赖领取下表单张任务卡；[文档交付记录](delivery.md)与产品状态分开。

## 约束与范围

本文档交付的原授权是将会话计划落成Markdown，不执行T01—T72。此后用户单独授权实施 T01—T05（已完成并集成，见下方“实施进展”）；T06—T72 仍按任务卡串行逐张领取（串行领取不等于逐次授权），重大架构实施前按仓库ADR/RFC规则确认，不把本计划误作Accepted ADR。Issue 与远程操作按根 AGENTS.md 规则 6 与「远程操作授权」执行：查重后可直接创建并领取任务 Issue，push 非默认分支、Issue、Labels、Projects、PR 创建与普通 merge 默认授权、不再逐次许可；仓库级设置与规则 12 的边界仍需人工确认，任务卡“远程权限”行与此一致。T01—T05 全部成果此后经分支 `feat/t01-t05-skeleton` 推送，以 PR [#4](https://github.com/xingjianxu/knoted/pull/4) 普通 merge 进入远程 master `795d624`。计划编写时点的基线master `6c00202c1a389a4e0e0ae797744c2159654c761d`，当前主线 SHA 见「实施进展」。

测试实例的运行管理自 2026-11-16 起按 [ADR 0003](../../decisions/0003-test-instance-process-management.md) 执行，并已随 PR [#64](https://github.com/xingjianxu/knoted/pull/64) 完成真实切换（运行 `4a546b2`，[证据](evidence/pcwrap-dev-service.md)）：应用由本仓库 `@bixlab/pcwrap` + process-compose 编排（控制端口 15600），数据库由宿主用户直接创建（[ADR 0004](../../decisions/0004-database-owned-by-host-user.md)），Strata、模型共享根与 ACME 仍由 `/etc/nixos` 管理，生产实例将来才用 NixOS。因此「改变运行行为的任务必须更新测试实例」改为 `pnpm pc` 流程，不再产生 host pin 更新；`app/host` 拆分只保留给外部依赖与生产接入（T70）。

默认串行按编号推进；直接依赖列用于定位输入，不等于自动授权并行。共享锁文件、API、迁移、主机配置由主Agent串行集成。T10/T11/T12/T21/T70有独立app/host子任务，必须分别领取。除这些明确跨仓库交接外，一张卡一个Agent、一个Worktree、一组可独立验收的结果。

每任务接口和校验值以契约为准。与未出现的工具版本相关的可运行性结果必须由前置任务实际验证并锁定；不靠大模型猜版本。配置值允许按明确参数覆盖，改变业务语义须更新计划记录。

## 任务索引

| 任务 | 成果 | 直接依赖 | 状态 |
|---|---|---|---|
| [T01](tasks/T01.md) | Rust workspace与配置 | — | done（已集成 cd2cb6a） |
| [T02](tasks/T02.md) | React与工具链骨架 | — | done（已集成 7b62fdc） |
| [T03](tasks/T03.md) | 公共OpenAPI契约 | T01, T02 | done（已集成 7e3c54b） |
| [T04](tasks/T04.md) | 生成客户端与查询入口 | T03 | done（已集成 892178e） |
| [T05](tasks/T05.md) | 数据库迁移与测试隔离 | T01, T03 | done（已集成 79a7d06） |
| [T06](tasks/T06.md) | API健康和日志 | T05 | done（PR [#19](https://github.com/xingjianxu/knoted/pull/19)，集成 `ec29695`） |
| [T07](tasks/T07.md) | 任务领取与租约 | T05 | done（PR [#20](https://github.com/xingjianxu/knoted/pull/20)，集成 `b4cc721`） |
| [T08](tasks/T08.md) | 任务重试和取消 | T07 | done（PR [#22](https://github.com/xingjianxu/knoted/pull/22)，集成 `9bd5bcd`） |
| [T09](tasks/T09.md) | Python任务协议与暂存对象 | T03, T08 | done（PR [#25](https://github.com/xingjianxu/knoted/pull/25)，集成 `40ce7c6`） |
| [T10](tasks/T10.md) | Docling与中文OCR验证 | T09 | done（app PR [#27](https://github.com/xingjianxu/knoted/pull/27) 集成 `9dac968`；host `/etc/nixos` `b5ca207`） |
| [T11](tasks/T11.md) | BGE-M3可运行性验证 | T09 | done（app PR [#30](https://github.com/xingjianxu/knoted/pull/30) 集成 `9056642`；host `/etc/nixos` `3cc5e31`） |
| [T12](tasks/T12.md) | Strata IQ3_S可运行性验证 | T03 | done（app PR [#21](https://github.com/xingjianxu/knoted/pull/21)、验收回填 PR [#24](https://github.com/xingjianxu/knoted/pull/24) 集成 `392cb60`；host `/etc/nixos` `ae00563`） |
| [T13](tasks/T13.md) | 唯一账号与首次注册 | T05, T06 | done（PR [#23](https://github.com/xingjianxu/knoted/pull/23)，集成 `11cb981`） |
| [T14](tasks/T14.md) | Passkey注册与登录 | T13 | done（PR [#28](https://github.com/xingjianxu/knoted/pull/28)，集成 `e0fa445`） |
| [T15](tasks/T15.md) | 会话与请求保护 | T14 | done（PR [#26](https://github.com/xingjianxu/knoted/pull/26)，集成 `32cbb6f`） |
| [T16](tasks/T16.md) | 凭据管理与恢复 | T15 | done（PR [#29](https://github.com/xingjianxu/knoted/pull/29)，集成 `6ad602e`） |
| [T17](tasks/T17.md) | mtone接入与基础表单验收 | T02 | done（PR [#42](https://github.com/xingjianxu/knoted/pull/42)，主线 `e3e1db2`，验证/清理完成） |
| [T18](tasks/T18.md) | mtone弹层菜单与通知集成 | T17 | done（PR [#46](https://github.com/xingjianxu/knoted/pull/46)，集成 `4425ab9`） |
| [T19](tasks/T19.md) | mtone树列表与反馈集成 | T18 | done（PR [#48](https://github.com/xingjianxu/knoted/pull/48)，集成 `6de79a8`） |
| [T20](tasks/T20.md) | 路由登录与应用外壳 | T04, T16, T19 | done（PR [#52](https://github.com/xingjianxu/knoted/pull/52)，主线 `e8d2c4f`，验证/清理完成） |
| [T21](tasks/T21.md) | 最小HTTPS测试部署 | T06, T20 | done（PR [#53](https://github.com/xingjianxu/knoted/pull/53)，主线 `d623582`，HTTPS/虚拟Passkey验收通过） |
| [T22](tasks/T22.md) | 笔记JSON与提取规则 | T03, T05 | done（PR [#58](https://github.com/xingjianxu/knoted/pull/58)，主线 `8d4a472`，验证/清理完成） |
| [T23](tasks/T23.md) | 笔记CRUD与版本 | T15, T22 | done（PR [#61](https://github.com/xingjianxu/knoted/pull/61)，主线 `b0409b6`，dev `7dc601a`，验收见[证据](evidence/t23.md)） |
| [T24](tasks/T24.md) | 页面树与生命周期 | T23 | done（PR [#76](https://github.com/xingjianxu/knoted/pull/76)，主线 `465f4ee`，dev `623e4c4`；[证据](evidence/t24-note-tree.md)） |
| [T25](tasks/T25.md) | 历史快照与恢复 | T08, T23 | done（PR [#79](https://github.com/xingjianxu/knoted/pull/79)，主线 `337090e`，dev `8429aed`；[证据](evidence/t25-history.md)） |
| [T26](tasks/T26.md) | 图片资产与引用 | T15, T25 | done（PR [#82](https://github.com/xingjianxu/knoted/pull/82)，主线 `ac8cd1f`，dev `5809cfc`；[证据](evidence/t26-assets.md)） |
| [T27](tasks/T27.md) | BlockNote编辑核心与工具栏 | T17, T18, T22, T23 | completed |
| [T28](tasks/T28.md) | 斜杠菜单与块菜单 | T27, T19 | completed |
| [T29](tasks/T29.md) | 图片块和代码块 | T26, T28 | completed |
| [T30](tasks/T30.md) | 自动保存与冲突草稿 | T25, T29 | completed |
| [T31](tasks/T31.md) | 笔记树和历史页面 | T24, T30 | completed |
| [T32](tasks/T32.md) | 标签和资料关联 | T05, T24 | completed |
| [T33](tasks/T33.md) | 项目API与归档 | T32 | completed |
| [T34](tasks/T34.md) | 事项与单层子任务 | T33 | completed |
| [T35](tasks/T35.md) | 状态优先级与日期 | T34 | completed |
| [T36](tasks/T36.md) | 跟进时间线与历史 | T35 | pending |
| [T37](tasks/T37.md) | 事项归档和回收站 | T36, T08 | pending |
| [T38](tasks/T38.md) | 项目页面 | T33, T20 | pending |
| [T39](tasks/T39.md) | 事项页面和编辑 | T35, T37, T38 | pending |
| [T40](tasks/T40.md) | 看板和时间线 | T36, T39 | pending |
| [T41](tasks/T41.md) | 工作概览与到期 | T31, T39 | pending |
| [T42](tasks/T42.md) | 统一回收站页面 | T24, T37, T41 | pending |
| [T43](tasks/T43.md) | 目录扫描与排除 | T08, T32 | pending |
| [T44](tasks/T44.md) | 稳定副本与版本 | T43, T09 | pending |
| [T45](tasks/T45.md) | 移动识别和重关联 | T44 | pending |
| [T46](tasks/T46.md) | 文本解析 | T44 | pending |
| [T47](tasks/T47.md) | PDF与图片解析 | T10, T44, T46 | pending |
| [T48](tasks/T48.md) | Office解析 | T47 | pending |
| [T49](tasks/T49.md) | 文件浏览备注下载 | T45, T48, T32 | pending |
| [T50](tasks/T50.md) | PDF图片文本预览 | T49 | pending |
| [T51](tasks/T51.md) | OnlyOffice授权后端 | T49, T15 | pending |
| [T52](tasks/T52.md) | OnlyOffice网页接入 | T51, T50 | pending |
| [T53](tasks/T53.md) | 全文索引发布 | T07, T48, T36 | pending |
| [T54](tasks/T54.md) | 统一搜索接口 | T53, T37 | pending |
| [T55](tasks/T55.md) | 搜索界面 | T54, T19 | pending |
| [T56](tasks/T56.md) | 索引任务状态与重试 | T08, T54 | pending |
| [T57](tasks/T57.md) | 分块和定位 | T48, T22, T36, T53 | pending |
| [T58](tasks/T58.md) | 嵌入批处理适配 | T11, T57 | pending |
| [T59](tasks/T59.md) | pgvector存储与检索 | T58 | pending |
| [T60](tasks/T60.md) | 混合检索和范围 | T59, T54 | pending |
| [T61](tasks/T61.md) | 模型服务适配器 | T12, T60 | pending |
| [T62](tasks/T62.md) | 会话消息与生成队列 | T61, T15, T08 | pending |
| [T63](tasks/T63.md) | 上下文预算与依赖 | T62 | pending |
| [T64](tasks/T64.md) | 引用快照与验证 | T63 | pending |
| [T65](tasks/T65.md) | 聊天界面 | T64, T18, T55 | pending |
| [T66](tasks/T66.md) | 回答转存笔记 | T65, T23, T32 | pending |
| [T67](tasks/T67.md) | 图片和临时对象回收 | T26, T37, T64 | pending |
| [T68](tasks/T68.md) | 一致性备份 | T67 | pending |
| [T69](tasks/T69.md) | 隔离恢复演练 | T68, T53, T59 | pending |
| [T70](tasks/T70.md) | 完整Nix模块与主机接入 | T21, T69 | pending |
| [T71](tasks/T71.md) | 功能与故障恢复集成验收 | T70 | pending |
| [T72](tasks/T72.md) | 最终测试部署和交接 | T71 | pending |

## 实施进展（T06—T16）

基线 master `1147172`。每个任务在 `.worktrees/knoted-t0N` 的独立分支（`feat/t0N`、`docs/t12b`、`feat/t12b`）实施，经独立审查（多数任务两轮以上修复循环）后以 PR 普通 merge 进入远程 master；本地主线 `git merge --ff-only` 同步，分支在工作区核验后清理。跨仓库 host 交付在 `/etc/nixos` 独立提交，不混入本仓库。

| 任务 | 实施范围 | PR / 集成 | 主线复跑证据 |
|---|---|---|---|
| T06 | axum 服务骨架、`AppContext`、request_id、`/health/live|ready`、安全日志 | [#19](https://github.com/xingjianxu/knoted/pull/19) → `ec29695` | `cargo test --workspace --locked` 23 passed；fmt/clippy 0；PG 失联 ready=503/live=200；日志零正文与 Cookie |
| T07 | `jobs`/`outbox` 迁移、幂等入队、`SKIP LOCKED` 领取、90s 租约/20s 心跳、worker 骨架 | [#20](https://github.com/xingjianxu/knoted/pull/20) → `b4cc721` | `t07_` 15 passed（真实 PG）；证明 `SKIP LOCKED` 的持锁用例可证伪；租约 CHECK 与 `cancel_requested` 自洽 |
| T08 | 任务状态机、5/30/120s 退避（总执行 ≤4）、取消确认、人工重试 `retry_of` | [#22](https://github.com/xingjianxu/knoted/pull/22) → `9bd5bcd` | `t08_` 6 passed；交错竞态（取消被旧 token 覆盖）先 RED 后 GREEN；全量 44 passed |
| T12 | 本地 provider 契约、IQ3_S 固定安装与证据；host `/etc/nixos` `ae00563` | [#21](https://github.com/xingjianxu/knoted/pull/21)、[#24](https://github.com/xingjianxu/knoted/pull/24) → `392cb60` | `t12_1/2/3` 真实通过：仅回环 15604、别名 `qwen3.8-flash-next-iq3_s`、流式 + 断连取消、三件套 sha256 与 HF revision 一致 |
| T13 | `0003_t13_auth` 迁移、唯一 owner、一次性 bootstrap/recovery token、admin CLI | [#23](https://github.com/xingjianxu/knoted/pull/23) → `11cb981` | `t13_` 6 passed；并发签发/消费取证；token 零明文落库与日志；ceremony 绑定 CHECK |
| T09 | Python 内部执行协议 + SQLite 账本 + 受控暂存；Rust 客户端与 staging | [#25](https://github.com/xingjianxu/knoted/pull/25) → `40ce7c6` | Python 24 passed + Rust 8 passed（双端真实执行）；重启 queued/running → interrupted；进程组取消耗时确认 |
| T15 | 会话、Cookie、闲置 7d/绝对 30d、CSRF/Origin、多会话管理、事务内原语 | [#26](https://github.com/xingjianxu/knoted/pull/26) → `32cbb6f` | `t15_` 11 passed；`max_connections=1` 连接池回归；事务回滚探针；全量 69 passed |
| T10 | Docling + RapidOCR 离线解析适配、锁版报告、中英 fixture；host `b5ca207` | [#27](https://github.com/xingjianxu/knoted/pull/27) → `9dac968` | Python 31 passed（含 T09 24）；离线零连接零缓存；PDF 页像素炸弹与产物上限；host 断网解析通过 |
| T14 | webauthn-rs 注册/登录、ceremony 5 分钟单次消费、counter 更新 | [#28](https://github.com/xingjianxu/knoted/pull/28) → `e0fa445` | `t14_` 8 passed；重放/并发 finish/合法签名+evil origin 均 422/409；全量 77 passed |
| T16 | 凭据分页与管理、最后凭据保护、recovery 单事务替换与旧会话撤销 | [#29](https://github.com/xingjianxu/knoted/pull/29) → `6ad602e` | `t16_` 8 passed；恢复失败回滚后原凭据可用；Origin/CSRF 负向；全量 85 passed |
| T11 | BGE-M3 dense 嵌入（1024 维归一化）、GPU UUID、OCR/嵌入串行；host `3cc5e31` | [#30](https://github.com/xingjianxu/knoted/pull/30) → `9056642` | Python 40 passed（CPU 与 GPU 嵌入两种环境）；运行期模型摘要校验；语义样本实测 gap 0.4197 |

主线整体验证（T16 合并后 `6ad602e`）：`cargo fmt --check`、`cargo test --workspace --locked`（85 passed）、`cargo clippy --workspace --all-targets -- -D warnings` 全部 exit 0；Python 侧 `uv run --group dev pytest`（T09/T10/T11 共 40 passed，CPU 与 GPU 嵌入两种环境各跑一次）与 `ruff check/format --check` exit 0。逐卡证据与限制写在各任务卡「执行证据」及 `/data/knoted-dev/tasks/knoted-t06-t16/{reports,reviews,evidence}/`；过程记录见 `/data/knoted-dev/tasks/knoted-t06-t16/ledger.md`。

T17 已由 PR #42 普通 merge 至主线 `e3e1db2`，见 [证据](evidence/t17.md)；主线前端 18 / Chromium 4 / Rust 85 通过（1 既有 ignored），其他门禁 exit 0，Issue #41 自动关闭、Projects Done、两个任务 Worktree/已合并本地分支已安全清理。T18 已由 PR #46 普通 merge 至主线 `4425ab9`，见 [证据](evidence/t18.md)；主线前端 24 / Chromium 11 / Rust 85（1 ignored）及全部门禁 exit 0，Issue #43 自动关闭、Projects Done、任务 Worktree/分支已安全清理。T19 已由 PR #48 普通 merge 至主线 `6de79a8`，见 [证据](evidence/t19.md)；主线前端 36 / Chromium 15（T17 4 + T18 7 + T19 4）/ Rust 85（1 既有 ignored）及 frozen install、typecheck、lint、build、fmt、clippy 均 exit 0，Issue #47 自动关闭、Projects Done、任务 Worktree/已合并本地分支已安全清理。三个 review Minor（夹具未知游标回落、游标重复派发压栈、卡内远程权限措辞）已分诊为可延后并在证据中记录消费约定。后续更新：T20/T21已完成（见下方进展），T22已由PR #58集成主线 `8d4a472` 并完成验证/实现工作区清理（[证据](evidence/t22.md)），T23已由PR #61集成主线 `b0409b6` 并部署dev `7dc601a`（[证据](evidence/t23.md)），T24已由PR #76集成主线 `465f4ee` 并部署dev `623e4c4`，T25已由PR #79集成主线 `337090e` 并部署dev `8429aed`，T26已由PR #82集成主线 `ac8cd1f` 并部署dev `5809cfc`，T27—T35 已完成，见[本轮集成证据](evidence/t27-t35-integration.md)与[计划](../2026-11-17-t27-t35.md)；HTTPS测试实例已建立。T06—T16原交付仍为库/契约/Python服务。仍存在的跨任务限制：无真实硬件 Passkey 人工验证；Docling Office 格式由 T46—T48 承接；Strata 与 Knoted 的 provider 模块**已裁决统一并生效**（2026-11-15：host 独占部署 + app 侧纯客户端契约，`nixos-rebuild switch` 已激活，见 T12 证据 §11.3）。

## UI 规划调整（2026-11-16）

用户确认通用 UI 改为直接消费 npm `@bixlab/mtone`，不在 Knoted 自建通用库。重写 T17—T19 但保留编号与依赖；同步下游 UI 卡、R25 和验收规则。规划调整交付时仍有 cn 占位包；现已由 T17 接入精确 npm 0.2.0 并移除占位包，基础表单消费验收、PR/主线验证及清理完成；T18 注册应用通知 Provider 并在夹具验收弹层菜单，已 PR 合并主线；T19 新增应用游标 reducer 并在夹具验收树/列表/反馈/确认，PR #48 已合并主线并完成验证及清理。见 [变更计划](../2026-11-16-mtone-ui-plan.md)、[上游需求记录](mtone-requests/README.md)。

外部能力缺口按消费任务记录技术依赖及 npm 发布/消费验收恢复条件；提示词交付或上游 master 合入不等于完成，不自动将全部下游任务标为 blocked，也不因此授权并行。T02/T04 历史记录保留。

## 阶段交付

A基础T01—T12；B认证/UI T13—T21；C笔记T22—T32；D事务T33—T42；E文件/全文T43—T56；F RAG T57—T66；G运维T67—T72。部署T21后按可部署任务逐次更新，不把只有PR或测试通过称为生命周期完成。

## 实施进展（T01—T05）

基线 master `715e1c2`。五个任务各自在 `.worktrees/knoted-t0N` 的 `feat/t0N` 分支实施，经独立审查、修复循环后合并到本地 master；实施阶段未推送、未创建 PR。39 个提交随后整体经 `feat/t01-t05-skeleton` 分支推送，由 PR [#4](https://github.com/xingjianxu/knoted/pull/4) 普通 merge 进入远程 master `795d624`；本地 master 已快进同步，该分支已删。

| 任务 | 任务分支提交范围 | 主线集成 | 主线复跑证据 |
|---|---|---|---|
| T01 | `8a32d2a`..`cd2cb6a` | 快进至 `cd2cb6a` | `cargo test --workspace t01_` 7 passed；fmt/clippy 0；二进制缺库配置 exit 1、凭据零泄漏 |
| T02 | `d4d0ced`..`f01a8fa` | merge `7b62fdc` | `pnpm install --frozen-lockfile`、typecheck、lint、build 0；vitest t02 5 passed |
| T03 | `1dfd5b8`..`5d344bb` | merge `7e3c54b` | `pnpm api:check` 0（79 operations / 16 examples / 8 invalid rejected） |
| T04 | `a5d7231`..`b3691e3` | merge `892178e` | `pnpm api:generate` 无差异；web vitest 13 passed |
| T05 | `7805c6d`..`e9a08a4` | merge `79a7d06` | `cargo test --workspace t05_` 8 passed（真实 PG 18）；测试后无残留 schema |

主线整体验证：`cargo fmt --check`、`cargo test --workspace --locked`（15 passed）、`cargo clippy --workspace --all-targets -- -D warnings`、`pnpm install --frozen-lockfile`、`pnpm api:check`、`pnpm run typecheck`、`pnpm run lint`、`pnpm run test`（13 passed）、`pnpm run build` 全部 exit 0。逐卡证据与仍存在的限制写在各任务卡“执行证据”及 `/data/knoted-dev/tasks/t0N/report.md`；过程记录见 `/data/knoted-dev/tasks/knoted-v1-impl/progress.md`。

尚未执行：远程推送、PR、CI、测试实例部署（T21 前无测试实例，T01—T05 均为库/契约/前端骨架，无需部署）。

## T20—T21 实施进展（2026-11-16）

T20 PR [#52](https://github.com/xingjianxu/knoted/pull/52)普通merge至`e8d2c4f`，前端54 passed（T20 18）/Rust85 passed（1既有ignored），全部前端与Rust门禁通过；主线独立复跑及Chromium15例均通过，Issue #50 Closed/Projects Done，工作区与本地分支已清理。T21最终固定app提交`f35a69a4ec0686ab08ae448bd4bf33d96ecce942`构建封装并定向部署成功，真实HTTPS与虚拟Passkey验收通过；会话HTTP补修PR #55已集成，Rust测试增至92；PR #53已集成至`d623582`，主线全门禁通过；host33450ee已本地集成并清理，最后文档分支随收尾PR清理。证据见[t20](evidence/t20.md)、[t21](evidence/t21-deployment.md)。
