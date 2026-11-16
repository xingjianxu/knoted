# T19 mtone 树、游标列表与反馈实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成用户继续 T17–T19 委派的最后一张卡：应用游标状态与真实 mtone 树/列表/反馈/确认消费验收。

**Architecture:** 沿用 npm mtone 0.2.0、T17 CSS 和 T18 应用 Provider/测试服务；生产只新增无 UI 的应用游标 reducer，Tree/列表/反馈/确认组合和演示数据仅留测试目录。使用 List/ListItem+Button 消费 items/next_cursor，应用维护上一页游标栈，不使用依赖 total 的 Pagination，不创建通用 PagedList/Tree/ConfirmAction。

**Tech Stack:** React19.3.1、TypeScript6、mtone0.2.0、Vitest5、Playwright Chromium1254。

**Spec:** [T19](knoted-v1/tasks/T19.md)、[数据/API](knoted-v1/data-and-api.md)、[UI 指南](../development/ui-mtone.md)、[ADR0002](../decisions/0002-mtone-ui-ownership.md)。T18 已通过 PR46 普通 merge 至 `4425ab9c93ceac6a01c89ef68c6337af258dd984`，主线验证和清理完成，Issue43自动关闭、ProjectsDone。

## Global Constraints

- 所有修改独立 Worktree/分支；写 Subagent 独立工作树，不写 master、不自行远程写/集成。
- 只消费正式 npm0.2.0公开 API/CSS，不改依赖/lock、不修改 mtone源码/PR/npm/主机，不本地通用控件/转发层/包装BaseUI/patch。
- 不新增HTTP/数据库/Python契约，不实现笔记API或移动/拖拽；T31负责移动。游标是opaque字符串，不解码，不伪造total。
- 唯一生产产出是应用游标状态；数据到Tree/List props映射用测试stub展示并写约定，不创造未实现的业务DTO或通用映射库。
- 测试夹具不进入生产，无演示路由；query选择?task=t19，T17默认与T18入口不变。
- 真实键盘/Portal/CSS用Chromium验证；不以缺文件、结构失败或已存在上游行为伪造RED→GREEN。
- 缺口先穷尽公开 props/tokens/CSS变量/应用组合，资料不足不能称缺失；实施者若证实通用缺口按skill存档中文完整提示词，报告主Agent处理交接，不擅自上游写入或补组件。
- 不扩展到T20或部署，T21前无持续测试实例；既有Confirm pending可Esc关闭边界不变，本卡不承诺请求取消。

## Review Focus

1. Tree独立展开/选择事件：鼠标toggle与ArrowRight展开不误选择；键盘Enter/Space才选择，受控props回写保持一致。
2. disabled树节点不选择/展开，键盘跳过不可执行节点；Home/End/ArrowLeft/Right父子关系真实验收。
3. 游标栈不可变：三页往返、首屏previous和末页next禁用，next_cursor=null不导航，opaque字符不修改。
4. 筛选变更清空旧cursor/history，同一filterKey不误重置；列表消费真正items/next_cursor而非total/offset。
5. loading/empty/error互斥明确，危险动作确认文案包含对象，取消零回调、pending不重复、异步失败可见且不假报成功。

## 基线与分工

- 主Agent `.worktrees/knoted-t19` / `feat/t19`，负责独立验证/审查/PR/主线复验/清理。
- 实施Agent `.worktrees/agent-t19` / `feat/agent-t19`，从本计划提交创建，仅Task1。
- 基线master4425ab9，frozen install/test exit0，24tests。证据 `/data/knoted-dev/tasks/t19/`。
- 复用会话已验证 `PLAYWRIGHT_BROWSERS_PATH=/data/knoted-dev/tasks/t17/worker/browsers`，`LD_LIBRARY_PATH` 读T17 worker/browser-library-path.txt；本卡新tmp/输出用T19任务目录。Rust cache可复用T17 worker/build/rust-target，不改全局环境。

### Task 1: 交付应用游标状态与组件消费验收

**Files:**
- Create: `apps/web/src/lib/cursor-state.ts`、`apps/web/tests/fixtures/t19-tree-list.tsx`、`apps/web/tests/tasks/t19.test.tsx`、`tests/e2e/t19.spec.ts`、`docs/plans/knoted-v1/evidence/t19.md`。
- Modify: `apps/web/tests/fixtures/browser-main.tsx`（增加T19选择，保留T17/T18）；本计划、T19卡、索引/UI指南；必要的README/AGENTS/架构当前状态说明。
- 回填上游T18卡/证据/计划Step7的实际完成事实（见下）；不改历史证据/ADR正文/依赖/生产App/main/provider。

**Interfaces:**
- Consumes: `UiProvider`、共享CSS和测试Vite服务4177；发布Tree受控 `value/onValueChange`、`expanded/onExpandedChange` 与TreeItem；公开List/ListItem/Empty/Skeleton或Spinner/Alert/ConfirmDialog/Button。
- Produces in `cursor-state.ts`:
  - `CursorState = { readonly filterKey: string; readonly cursor: string | null; readonly previous: readonly (string | null)[] }`。
  - `CursorAction = {type: "next"; nextCursor: string | null} | {type: "previous"} | {type: "filter"; filterKey: string}`。
  - `createCursorState(filterKey?: string): CursorState` 默认空filter、cursor=null、history=[]。
  - `cursorReducer(state: CursorState, action: CursorAction): CursorState`：next非null压入当前cursor；next=null与首屏previous无操作；previous弹栈；不同filter重建初始状态，同filter不丢栈；不突变输入。
  - 夹具`?task=t19`使用此reducer，无total三页stub数据（真items/next_cursor形状）验证上下页；stub title映射到公开label/children等，业务DTO不进入mtone。

- [ ] **Step 1: 核实公开能力与分类。** 读取锁定types/Tree等实现和需求索引，确认公开组合能满足；不因无同名PagedList开单。保留T18 pendingEsc等已验证边界。
- [ ] **Step 2: 游标TDD。** 先写 reducer 行为测试：初始/三页next及previous、null next/首屏previous、opaque值不变、筛选变更/相同filter、Object.freeze输入不突变。模块缺失单独记录，建立可运行的最小占位后观察行为RED，再最小实现到GREEN；不把加载错误当行为RED。
- [ ] **Step 3: 建立真实组件消费测试。** t19_1受控Tree展开不误选择/键盘/disabled；t19_2空/错/加载互斥明确；t19_3对象文案/取消/确认单次/pending/失败可见；t19_4无total列表上下页/筛选reset，测试不耦合未实现API；原本通过的上游特征如实标注。
- [ ] **Step 4: 最小应用与夹具。** 仅新增cursor-state生产逻辑，测试stub映射实际公开props、useReducer管理游标，选择器?task=t19复用原服务。列表/反馈与Confirm都直接消费mtone，不新控件wrapper、不显示虚构总数。
- [ ] **Step 5: 浏览器与回归门禁。** `(cd apps/web && pnpm exec vitest run tests/tasks/t19.test.tsx)`；全量Playwright（T17/T18/T19），真实Tree键盘/禁用、列表button状态/往返/筛选、反馈可见与确认cancel/fail/单次、主题/CSS/页面错误；分别pnpm typecheck/lint/test/build，E2Eeslint/tsc，cargo fmt/test --locked/clippy --locked --all-targets -- -D warnings。逐命令输出/exit/非零数留存，迭代用定向测试，最终一次全量。
- [ ] **Step 6: 文档和本地提交。** T18事实：PR46/master4425ab9，主线前端24/Chromium11/Rust85+1既有ignored及全部门禁exit0，task/final/并发主线冲突增量审查均通过，两个T18Worktree/分支已清理、Issue43自动关闭/ProjectsDone。T19仅标实现就绪待集成；提交前读有效git-commit，自审范围/敏感信息。报告 `/data/knoted-dev/tasks/t19/worker-report.md`，主体精简，原始日志独立文件不要逐条整份复制。
- [ ] **Step 7: 主Agent交付。** 独立diff/复验、task和whole-branch review，重要发现修复并复核；普通PR合并、主线再验证、安全清理；最后在合规独立文档任务分支回填T19最终生命周期与索引（若本卡已合并后才能取得事实），不在主线直接写文档。

## 进度与决定

- 沿用既有设计，无新增跨模块/存储/生产边界。游标reducer作为应用适配，不是公共UI分页库。
- 实施者仅本地写入和提交，所有远程/上游交接由主Agent核查；本卡未发现缺口时不发布上游需求。
