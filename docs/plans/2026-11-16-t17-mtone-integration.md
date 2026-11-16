# T17 mtone 接入实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按用户「继续 T17–T19」直接委派先完成 T17，并在集成后依次领取 T18、T19。

**Architecture:** 遵从 ADR 0002，应用直接消费正式 npm mtone，不建立通用库/转发层。生产仅接入样式及必要 Provider、迁移占位 App；交互组合留在测试专用服务中。T18/T19 使用本卡建立的测试服务，不创建生产演示路由。

**Tech Stack:** React 19.3.1、TypeScript 6、Vite 8、Vitest 5、Playwright 1.65、pnpm 12.9、正式 npm @bixlab/mtone 0.2.0（唯一非预发布最高版本；安装时核实公开 API）。

**Spec:** [T17](knoted-v1/tasks/T17.md)、[T18](knoted-v1/tasks/T18.md)、[T19](knoted-v1/tasks/T19.md)、[ADR 0002](../decisions/0002-mtone-ui-ownership.md)、[UI 指南](../development/ui-mtone.md)。既有需求/方案已明确，本次不更改设计与跨模块契约。

## Global Constraints

- 所有修改在主仓库根 `.worktrees/` 下独立分支；写 Subagent 独立 Worktree；不直推 master。
- 不修改 mtone、不创建上游 Issue/PR、不发布、不私有导入/patch/包装 Base UI，不创建 @knoted/ui 转发层。（2026-11-16 后续规则变更：证实通用缺口后必须在上游 mtone 提 Issue；本行仅记录 T17 执行期的约束，现行规则见 [UI 指南](../development/ui-mtone.md)。）
- 正式 npm 精确锁版；不得 link/Git/未发布 tarball；不改 HTTP/数据库/Python 契约。
- Tailwind 仅布局/公开主题适配；系统字体且无远程字体请求。
- 不扩大到业务路由、编辑器或后续产品功能；T17 集成后再领取 T18，T18 集成后再领取 T19。
- 真实通用缺口遵循 mtone-component-request，保留成果并记录技术阻塞。
- 不编造 RED→GREEN、零测试、浏览器/部署结果；T21 前没有测试实例。

## Review Focus

1. 原 cn 使用 tailwind-merge，不假定 mtone cn 等价；验证冲突类合并，当前 App 静态类无需 cn。
2. Provider/CSS 与测试夹具使用同一生产入口，不能只给测试额外主题而漏应用。
3. loading/disabled 真实阻止重复提交，恢复后可再次操作。
4. 字段 label、description、error 与 input/textarea 对应，可键盘访问。
5. 无远程字体检查覆盖发布 CSS、构建 CSS/HTML 和浏览器请求，不只检查源码。

## 基线与执行边界

- 主线 `5aeb659`，主工作区干净；T17–T19 尚无现有实施 Issue/PR；#39/#40 仅历史文档阶段。
- 主 Agent 集成 Worktree `.worktrees/knoted-t17` / `feat/t17`。
- 实施 Agent 独立 Worktree `.worktrees/agent-t17` / `feat/agent-t17`（从本计划提交创建）。主 Agent 不与实施者并发写同一工作树。
- 只执行下方 Task 1。本卡完成后为 T18/T19 按实际公开 API 记录续做计划，不把三卡合为一次交付。
- 证据目录 `/data/knoted-dev/tasks/t17/`。基线 `pnpm install --frozen-lockfile`、`pnpm test` 成功，13 个测试。

### Task 1: 接入 npm mtone 并完成基础表单消费验收

**Files:**
- Modify: `apps/web/package.json`, `apps/web/src/App.tsx`, `apps/web/src/main.tsx`, `apps/web/src/styles.css`, `apps/web/tests/tasks/t02.test.tsx`, `eslint.config.mjs`, `pnpm-lock.yaml`, `playwright.config.ts`。
- Create: `apps/web/src/lib/ui-provider.tsx`（仅在公开 Provider 确有需要时）, `apps/web/tests/tasks/t17.test.tsx`, `tests/e2e/t17.spec.ts`, `tests/e2e/fixtures/index.html`, `tests/e2e/fixtures/main.tsx`, `tests/e2e/fixtures/t17.tsx`, `tests/e2e/fixtures/vite.config.ts`；可按测试服务需求增加测试专用配置/共享组合文件，不建立生产组件库。
- Remove: `packages/ui/`；只清理实际引用，不无理由改通配 workspace。
- Docs: T17 卡、计划索引、AGENTS/README/架构现状/UI 指南迁移措辞；保留 T02/T04 历史证据；新增 `docs/plans/knoted-v1/evidence/t17.md`。

**Interfaces:**
- Consumes: 正式包 `@bixlab/mtone@0.2.0` 的根导出及 `@bixlab/mtone/styles.css`，实际类型为准。
- Produces: 应用 CSS/必要 Provider 接入；App 不再依赖 @knoted/ui；测试专用 Vite 服务与 baseURL/webServer 供 T18/T19 扩展。生产不渲染表单演示。

- [x] **Step 1: 核实版本与浏览器。** 读取 npm 元数据、发布类型/README/CSS，记录 exports、React peers、Provider/tokens、cn 语义。探测 Playwright 可执行文件并真实启动，环境缺少运行库按项目已有 Nix 方法解决，不修改全局环境、不掩盖失败。
- [x] **Step 2: 建立失败测试。** `t17_1` 验证真实 Button/Input/FormField/Textarea/Badge label 与键盘；`t17_2` 验证 loading/disabled 不提交、恢复后提交；`t17_3` 验证 description/error 关联；`t17_4` 验证删除占位包与 CSS/无远程字体。测试共享消费组合置于测试目录；行为测试先失败，结构/解析失败如实分别记录。
- [x] **Step 3: 运行 RED。** `(cd apps/web && pnpm exec vitest run tests/tasks/t02.test.tsx tests/tasks/t17.test.tsx)`；记录失败断言及原因，不能仅缺包就宣称行为 RED。
- [x] **Step 4: 最小接入。** 精确安装 0.2.0，按真实公开接口接入 CSS/必要 Provider。移除 App 的 cn 依赖（只有静态类，无合并需要），测试原 cn 语义与迁移理由；删除占位包，更新实际 ESLint 等引用。系统字体及应用公开主题配置覆盖 Portal 使用。
- [x] **Step 5: GREEN 与真实浏览器。** 运行上述 Vitest，`pnpm exec playwright test tests/e2e/t17.spec.ts`；浏览器断言真实 CSS、生效字体、Tab、disabled/loading、表单关联并观察远程请求；测试服务有生命周期管理且不添加生产演示路由。
- [x] **Step 6: 全量本地门禁。** `pnpm typecheck`、`pnpm lint`、`pnpm test`、`pnpm build`；`cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`。保存每条命令输出/退出码/非零测试数量，不重复运行昂贵全量命令直至最终状态。
- [x] **Step 7: 回填并提交。** 证据文档区分实现就绪、审查、集成，未合并不写完成。实际读取有效 git-commit，检查范围/敏感信息，提交中文摘要 Conventional Commit。报告实现、TDD、各命令输出、风险及 SHA，禁止自行派生 Subagent、PR/主线合并。
- [ ] **Step 8: 主 Agent 独立审查与集成。** 读取全量差异、独立复跑受影响门禁、发起 Reviewer 审查；核实反馈，普通 merge 进入集成 Worktree。创建关联 Issue 的 PR、记录输出，门禁通过后普通合并；更新干净主线并验证，才清理两 Worktree 与已合并分支。接着领取 T18。

## 决策与进度

- Ruling: 复用任务卡的已批准目标与 ADR，而不重开设计问答；本次直接委派已明确继续 T17–T19，无新架构决定。
- Ruling: 仅当公开包要求 Provider 时添加应用层 Provider，避免凭假设创建统一包装 API；测试与生产共享实际接入。
- 实施提交 `225fbcb`，旧安装残留断言修复 `87f57c7`；首轮独立 task review Approved。主 Agent 独立复验前端 18、Chromium 4、Rust 85 通过，1 既有 ignored doctest，typecheck/lint/build/fmt/clippy 及产物扫描/生产 smoke 均 exit 0。
- 旧目录存在不等于源码包存在，修复保留 ignored 残留证明 RED→GREEN；没有靠清理缓存掩盖失败。
- 证据见 [T17](knoted-v1/evidence/t17.md) 和 `/data/knoted-dev/tasks/t17/controller/`。最终审查/PR/主线验证尚待本轮收尾，不提前勾选 Step 8。
