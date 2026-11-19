# 工作区与账号 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** 完成任务卡 01、02，为后续页面提供可复用的工作区布局和四个可独立操作的账号页面。

**Architecture:** 保留现有 TanStack 路由层、AuthGate 和 mtone AdminLayout。工作区只提供应用级排版约定与返回入口；账号组件拥有自己的业务状态、错误和仪式生命周期。

**Tech Stack:** React、TanStack Router/Query、Tailwind、mtone、Vitest、Playwright。

**Spec:** [总方案](README.md)、[01](01-foundation.md)、[02](02-account.md)。实施 Issue [#249](https://github.com/xingjianxu/knoted/issues/249)、[#250](https://github.com/xingjianxu/knoted/issues/250)。

## Global Constraints

- 直接使用已发布并精确锁定的 mtone；核对 latest、peers、CSS 与公开类型。不新增通用控件层或 API/存储契约。
- 分两张卡、两个主仓库 `.worktrees/` 下的独立分支；01 提供版心与文字约定后才做 02。主 Agent 实施与验证、独立 reviewer 审查，PR 关联各自 Issue。
- 不重做卡 03 之后的业务 UI；不改变路由、认证白名单、隐私缓存清理或离开笔记保护。

## Review Focus

- 320px 图标栏下当前项、顶栏按钮和详情返回均可用；没打开详情就没有返回入口。
- 已访问的长页面走前进/后退恢复滚动，来自搜索的消息目标仍可见，不让归顶逻辑覆盖目标。
- 根错误重试需可见忙态，重复失败不丢焦点，可再次发起；404 能真正回首页。
- 账号页在离开或重复操作时不得并发启动仪式；迟到的结果不提交。
- 凭据查询故障与会话分页互不干扰；保留最后一枚凭据保护、确认取消回焦及当前会话撤销后清缓存。

### 01 · 工作区与系统反馈（#249）

**Files:** `apps/web/src/routes/{_authenticated,__root}.tsx`、`apps/web/src/styles.css`、`apps/web/src/lib/{router,ui-provider}.tsx`，`tests/e2e/admin-layout.spec.ts`。

**Produces:** `.workspace-frame`（约 70rem）、`.workspace-frame--wide`（全宽）、`.reading-measure`（68ch）、`.page-title`、`.section-title` 和 `.page-link`；`[data-workspace-content]` 仍是唯一外壳滚动点。公开入口可复用排版类，不嵌套外壳。

- [x] 先添加浏览器断言：事项与项目详情返回到正确清单；直接访问 404 提供真实首页链接；根加载失败时重试可键盘操作、等待和再试；长页前进后退恢复位置。新增行为未实现时断言应失败。
- [x] 实现受路由匹配控制的顶栏返回、作用域排版、滚动恢复及系统反馈；从已有 tokens 出发，不另造主题。笔记、问答和共用看板的事项保持可用全宽；其余路由在统一版心内。
- [x] 合成数据测试 1440/768/390/320px、键盘及缩放，保存宽窄对照；运行卡 01 指定 Vitest、Playwright 和合并门禁。独立审查、提交和 PR。

### 02 · 公开入口和账号安全（#250）

**Consumes:** 01 排版类与原有 URL/API/`safeReturnTo`、`cursorReducer`；不编辑 01 公共文件。变更 `apps/web/src/features/auth/{auth-page,security-page}.tsx`、`apps/web/tests/tasks/t20.test.tsx`，在 `tests/e2e/` 增加账号场景。

- [x] 先测试公开页只提供离开当前流程的链接；恢复后果可读；注册表单仅通过按钮展开；故障重试只影响被重试的列表；离开安全页时中止未完成注册。
- [x] 重排三个公开页与安全页：单一主操作、行内错误及忙态，宽屏安全页约 720px，窄屏行操作换行；按记录展示 ConfirmDialog 内错误并保留既有 API 语义。
- [x] 用合成账号对照四个入口的桌面/手机状态，验证键盘、取消及列表独立。跑 T20、工作区和账号浏览器测试；经独立审查，区分验证超时与 CSRF，并为错误播报加回归断言。

证据：[01](evidence/01-foundation.md)（已通过 PR [#251](https://github.com/xingjianxu/knoted/pull/251) 合入 `db14025`）、[02](evidence/02-account.md)（451 Vitest / 30 Chromium，通过 lint、typecheck、build）。

### 集成

- [x] 01、02 已经 PR [#251](https://github.com/xingjianxu/knoted/pull/251) 和 [#252](https://github.com/xingjianxu/knoted/pull/252) 普通合并至 `master`。02 合并提交 `f269d01` 是剩余任务的基线。
