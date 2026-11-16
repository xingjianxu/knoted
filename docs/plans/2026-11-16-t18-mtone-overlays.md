# T18 mtone 弹层、菜单与通知实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按用户继续 T17–T19 的直接委派，完成 T18 真实弹层/菜单/确认/通知消费，不实现通用控件。

**Architecture:** 延续 T17 已集成 npm 0.2.0 与公开 Provider/CSS。仅在应用 UiProvider 注册公开 ToastProvider；业务流程组合留测试夹具，直接使用 Modal/ConfirmDialog/DropdownMenu/ContextMenu/Popover/useToast，不造统一包装 API。测试服务通过查询参数选择 T17/T18 夹具，原 T17 页面默认行为不变。

**Tech Stack:** React 19.3.1、TypeScript、mtone 0.2.0、Vitest、Playwright Chromium。

**Spec:** [T18](knoted-v1/tasks/T18.md)、[UI 指南](../development/ui-mtone.md)、[ADR 0002](../decisions/0002-mtone-ui-ownership.md)。上游 [T17](knoted-v1/tasks/T17.md) 已经 PR #42 普通 merge 至 `e3e1db27425a59fdc01d4dbbfc49c209f92b7500`，主线门禁/清理完成，Issue #41 自动关闭。

## Global Constraints

- 独立任务 Worktree/分支；写 Subagent 另用独立 Worktree，不写 master，不自行合并/远程写。
- 直接消费正式 npm `@bixlab/mtone@0.2.0` 公开 API/CSS；不得创建通用转发库、本地弹层或焦点管理实现，不修改 mtone/主机配置/上游 Issue/发布。
- 不安装新依赖、不修改 lock/API/数据库/Python 契约。真有升级需要先交主 Agent 串行决定。
- 无生产展示路由；测试夹具只在测试目录；复用 T17 测试服务，端口 strictPort/生命周期管理。
- 系统字体/公开主题适配，Portal 必须真实浏览器验证；不以 jsdom 或上游单测冒充焦点/样式验收。
- 不把原本通过的发布组件行为伪称实现 RED→GREEN。技术缺口先核实/查重，用 mtone-component-request 交用户，不本地补底层。
- T19 在本卡正常集成后才启动，T21 前无持续测试实例，不部署。

## Review Focus

1. Modal 的焦点陷阱、关闭回焦与 trigger 卸载：真实 Tab/Shift+Tab/Esc/close 与无页面错误。
2. 受控 open/onOpenChange 不与内部状态分叉；Popover 外部点击关闭且回焦按真实公开契约。
3. DropdownMenu/ContextMenu 键盘启用项只执行一次，disabled 项不触发；不重复同时绑定 item 和全局 handler 为同一业务事件。
4. ConfirmDialog 异步拒绝不假报成功、保持可重试，pending 不重复提交；取消零回调。
5. 成功/失败通知可关闭、StrictMode 不重复消费点击事件、Portal 字体/主题/层级可见。

## 基线和分工

- 主 Agent：`.worktrees/knoted-t18` / `feat/t18`，负责计划、独立验证、审查、PR/主线集成/清理。
- 实施 Agent：`.worktrees/agent-t18` / `feat/agent-t18`，从本计划提交创建，仅执行 Task 1。
- 基线 frozen install / pnpm test：exit 0，18 passed。日志 `/data/knoted-dev/tasks/t18/baseline-*.log`。
- 浏览器已验证环境：`PLAYWRIGHT_BROWSERS_PATH=/data/knoted-dev/tasks/t17/worker/browsers`；进程级 `LD_LIBRARY_PATH` 读取 `/data/knoted-dev/tasks/t17/worker/browser-library-path.txt`；当前任务 TMPDIR/证据放 `/data/knoted-dev/tasks/t18/`。仅复用同会话验证的缓存，不改全局配置。
- Rust 可复用同会话 `/data/knoted-dev/tasks/t17/worker/build/rust-target` 编译缓存，不改 Rust 源码。

### Task 1: 验证弹层菜单并注册应用通知 Provider

**Files:**
- Modify: `apps/web/src/lib/ui-provider.tsx`（注册 ToastProvider）；`apps/web/tests/fixtures/browser-main.tsx`（测试夹具选择），必要的 `tests/e2e/fixtures/main.tsx`；不改生产 App，不无理由修改测试启动配置。
- Create: `apps/web/tests/fixtures/t18-overlays.tsx`、`apps/web/tests/tasks/t18.test.tsx`、`tests/e2e/t18.spec.ts`、`docs/plans/knoted-v1/evidence/t18.md`。
- Docs: 本计划、T18 卡、计划索引/UI 指南；回填已完成 T17 的任务卡、证据、计划 Step8 及 AGENTS/README/架构现状里的“待主线集成”措辞，保持历史原始证据。

**Interfaces:**
- Consumes: T17 `UiProvider({children}: {children: ReactNode})`、公开 CSS、测试Vite服务4177。发布公开 Modal/ConfirmDialog 的 `open/onOpenChange/trigger`，菜单 `items/onSelect`，ToastProvider/useToast（具体类型读锁定发布包）。
- Produces: 生产与测试共用 UiProvider 内的 ToastProvider；默认 T17 测试夹具不变，`?task=t18` 渲染 T18 专用消费组合。没有业务事件总线或统一 Notification/Overlay API。

- [x] **Step 1: 阅读实际公开接口。** 核实 Modal、ConfirmDialog、DropdownMenu、ContextMenu、Popover、ToastProvider/useToast 的发布 types/实现及已有需求记录，记录本卡能力分类。测试未知行为先最小实验，不猜 props。
- [x] **Step 2: 先建立消费测试。** Vitest 断言应用 Provider 下真实通知成功/失败/关闭、受控弹层/菜单事件、取消与失败重试，先以缺少 ToastProvider 的具体应用行为失败或明确记录原本通过的消费特征。无测试文件/缺包不算行为 RED。
- [x] **Step 3: 最小生产接入。** 仅将公开 ToastProvider 注册进 UiProvider，测试与生产共享同一 Provider；不增加无消费者的事件映射/总线，不重复实现上游焦点逻辑。
- [x] **Step 4: 真实浏览器验收。** `t18_1` Modal 焦点陷阱/Portal样式，`t18_2` Esc/按钮/外部点击/回焦及触发点卸载无抛错，`t18_3` 两种菜单键盘启用/disabled/单次动作，`t18_4` 确认取消/异步失败/重试/pending 单次，`t18_5` 通知成功/失败/关闭/不重复；检查系统字体、主题、实际可见/层级和页面错误。测试内可注入可控 Promise，不模拟 mtone 组件。
- [x] **Step 5: 回归和门禁。** `(cd apps/web && pnpm exec vitest run tests/tasks/t18.test.tsx)`，全量 `pnpm exec playwright test`（T17+T18），`pnpm typecheck/lint/test/build`（分别），E2E独立eslint/tsc，`cargo fmt --check`、`cargo test --locked`、`cargo clippy --locked --all-targets -- -D warnings`；逐条输出/退出码留存。异常先 systematic-debugging，真实缺口停止受影响实现。
- [x] **Step 6: 回填/提交。** T18 状态实现就绪待集成，不提前写完成。T17 实际已完成事实：PR42/master e3e1db2，主线前端18/Chromium4/Rust85通过、1既有ignored，所有其他门禁exit0，两个任务Worktree/分支已清理、主线干净；可回填其生命周期状态。提交前实际读取 git-commit，检查diff/敏感信息，自审，不派发Subagent。完整报告 `/data/knoted-dev/tasks/t18/worker-report.md`。
- [ ] **Step 7: 主 Agent 审查/集成。** 独立读diff/复验并请求task/final review，处理重要反馈；创建PR普通merge、主线再验证、安全清理后才完成T18与领取T19。

## 进度

- 设计/范围沿用已批准任务卡，本计划只具体化公开包消费；无新增需人工确认的架构或生产边界。
- T17 移除源码包而非 ignored 安装缓存的修复是下游回归必须保留的约束；本轮默认 T17 夹具与 4 个 Chromium 回归保持通过。
- Task 1 实施就绪：生产仅 ToastProvider 注册；`?task=t18` 使用 StrictMode + 同一 UiProvider，业务组合全部在夹具；无依赖/lock/生产演示页变更。
- RED：应用通知缺失 ToastProvider 真实报错，1 failed / 3 published-characterization passed；注册后通知路径通过。初次 GREEN 因 Toast 关闭按钮仅 focus/hover 时暴露给 AT 失败，改用真实 focus 后通过。
- Chromium 初测 2 passed / 5 failed：误用可访问树查找 modal 背景、误判 disabled 菜单项必须跳过焦点；第二轮 5 passed / 2 failed：瞬时 inert focus guard、误判 pending ConfirmDialog 禁止 Esc。独立最小探针证实实际公开行为后修正测试契约，无上游源码修改。最终 T18 7 passed / 全量 11 passed。
- Ruling: 允许 disabled 菜单项聚焦但绝不执行，焦点陷阱等待 inert guard 的异步重定向 — 实测契约与可访问性一致，卡片不要求跳过焦点 — 若下游要求跳过需另验公开组合，不能借本卡宣称覆盖。
- Ruling: ConfirmDialog 验证 pending 确认/取消按钮禁用与单次请求，不要求 pending 禁 Esc — 0.2.0 的 Esc 可关闭，卡片只要求不重复提交；生产没有本卡确认业务 — 后续业务必须明确取消/请求生命周期，若要求不可关闭需按公开受控 API 验证，不能把本卡误作保证。
- 最终门禁：frozen、T18 Vitest 6、全量前端 24、Chromium T17+T18 11、E2E ESLint/TS、Rust fmt/test85（1 既有 ignored）/clippy、生产产物 Chromium smoke 均 exit 0。首次 typecheck/build exit 1 为测试数组非 tuple 与 RTL 不支持 Playwright 的 `exact`；修正测试类型后 typecheck/lint/test/build 重跑 exit 0，未重跑不受影响的 Rust。
- 完整证据 [T18](knoted-v1/evidence/t18.md)，日志/命令/退出码 `/data/knoted-dev/tasks/t18/worker/`，报告 `/data/knoted-dev/tasks/t18/worker-report.md`。实施 Agent 自审不等于主 Agent 独立审查；Step 7 保持未完成。
