# mtone 0.6.0 AdminLayout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用 mtone 0.6.0 为所有已登录页面启用统一的响应式外壳。

**Architecture:** `_authenticated` 路由使用公开 `AdminLayout.Sider/Header/Content`，保留现有路由、验证与保存保护。AdminLayout 管理布局、断点和内层滚动；TanStack Router 保持导航重置内容滚动，应用仍管理导航和登出。

**Tech Stack:** pnpm 12.9、React 19、TanStack Router、mtone 0.6.0、Lucide、Vitest、Chromium Playwright。

**Spec:** [已确认设计](2026-11-19-mtone-admin-layout-design.md) · [Issue #242](https://github.com/xingjianxu/knoted/issues/242)

## Global Constraints

- 源码、文档及锁文件仅改在 `.worktrees/mtone-admin-layout`，从 `origin/master` 的 `06f2b8e` 创建。
- 只消费已发布 npm 精确版本及公开 API；Lucide 直接依赖匹配锁中 `1.73.0`，不升级无关依赖。仅更新 mtone 对应的发布等待期例外，不放宽全局策略。
- 登录和初始化页继续独立；不改现有导航路径、搜索默认参数、路由鉴权、退出清理与笔记阻止离页。
- 不部署、不改 API 或存储、不改上游、不直推 `master`。前端与仓库本地门禁通过后，才可创建并普通合并 PR。

## Review Focus

- 窄屏初次加载以及跨过 768px 后，窄栏图标的名称、当前页和键盘操作仍可用（Chromium）。
- 从较长页面切换到已缓存的长页面，新页面从内容顶部开始；顶栏不随内容离开视口（Chromium）。从搜索跳至已缓存的问答消息时，显式定位仍生效。
- 先显示登录或初始化时绝不渲染工作区导航（Vitest 与 Chromium 登录页）。
- 多页共用一个 `main`，包括原已有 `main` 的概览、回收站（Vitest/Chromium）。
- 未保存笔记的离页与登出失败仍受保护；登出成功仍清理状态（现有 T20/T31/T49）。

## Task 1: 工作区布局与精确锁版

**负责人：** 主 Agent。前端变更及回归测试共享布局契约，按顺序实施，不并行写入。独立审查交给只读 Subagent。

**Files:** `apps/web/package.json`、`pnpm-workspace.yaml`、`pnpm-lock.yaml`；`apps/web/src/routes/_authenticated.tsx`、`apps/web/src/lib/router.tsx`、概览及回收站若需调整 landmark；`apps/web/tests/tasks/t20.test.tsx`、`tests/e2e/admin-layout.spec.ts`、`playwright.config.ts`（真实路由测试，仅拦截合成 API 的本地应用）；`AGENTS.md`、`docs/development/ui-mtone.md`、本计划。

**Interfaces:** 继续使用 `AuthGate`、`NotesNavigationBlocked` 和现有 `Route`；外壳只公开无障碍导航名 `主导航` 与稳定的工作区滚动容器选择器。

- [x] 写消费回归测试：以真实路由挂载概览与登录；已登录外壳恰有一个 `main`，主导航存在、概览为 `aria-current=page`、顶栏包含退出；未登录无主导航。用 Playwright 检查窄屏与桌面、固定顶栏、内层滚动与切页归顶。
- [x] 在 0.5.0 和原布局运行新增测试，验证缺少工作区 AdminLayout 的断言失败（非环境错误）。基线已通过 442 个 Web 测试。
- [x] 精确更新依赖及 release-age 条目、生成最小锁差异，并用 `pnpm install --frozen-lockfile` 复验。
- [x] 在 `Shell` 用 `AdminLayout fullHeight` 嵌套右侧布局，`Sider` 配品牌/窄品牌、折叠和 768px 断点；每项提供独立图标和原标签。使用 `Header.extra` 承载退出，`Content` 承载错误、离页保护和 `Outlet`。使页面保有唯一主区并配置 Router 导航归顶；不引入额外通用组件。
- [x] 重跑新增测试和相关 T20/T31/T49；更新精确版本说明，不改历史升级记录。

## Task 2: 验证及交付

**负责人：** 主 Agent；只读 Reviewer 检查同一提交范围，重点是上方五项及锁文件范围。依赖 Task 1。

- [x] 运行并保存 `pnpm lint`、`pnpm typecheck`、`pnpm test`、`pnpm build`、Playwright 工作区与 T17–T19；`cargo fmt --check`、`cargo test --locked`、`cargo clippy --locked --all-targets -- -D warnings`；`git diff --check`。任何失败都先定位，不绕过门禁。
- [x] 核实独立审查反馈。Chromium 红绿验证覆盖缓存问答的聚焦与可见性；滚动测试确认仅去掉 Router 重置选择器时会失败。生产构建预览重跑四条布局验收。
- [ ] Conventional Commit、按模板创建 PR 并附实际本地输出，普通合并至 `master`。主线复验和安全清理的最新状态以关联 PR 的核对记录为准。
