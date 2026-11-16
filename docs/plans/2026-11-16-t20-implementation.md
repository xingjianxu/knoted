# T20 路由认证实施计划

> 执行使用 superpowers:executing-plans，单实施 Agent，主 Agent 独立审查与集成。

**Goal:** 交付文件路由、AuthGate、Passkey 登录/首次注册/恢复、凭据与会话管理及禁用的业务入口。

**Architecture:** 复用 T20 任务卡与既有 API，TanStack Router 文件路由，TanStack Query 是会话缓存唯一所有者，createQueryClient 是唯一装配入口。mtone 0.2.0 提供公开布局、表单和反馈组件；应用保留认证、URL、缓存及游标状态。

**Tech Stack:** React 19，TanStack Router/Query，WebAuthn，mtone，React Hook Form/Zod。

**Spec:** [T20](knoted-v1/tasks/T20.md)、[架构](knoted-v1/architecture.md)、[API](knoted-v1/data-and-api.md)、ADR 0002。

## 约束与分工

实施 Agent 仅写 `.worktrees/knoted-t20` 的 feat/t20；主 Agent 负责审查、PR、主线验证、清理。依赖 T04/T16/T19 已在主线。保持 API/storage/ADR 不变；不实现后续业务，不修改 mtone。基线 `6ee4380`，前端基线 36 passed。

## Review Focus

- URL redirect 必须是允许的应用内页，外部 URL、协议相对 URL、认证循环和错误参数不能绕过登录。
- 401 撤销会话时清理缓存并关闭受保护页面；网络失败不当作未登录。
- WebAuthn 取消/不支持不会提交 finish；二进制参数和响应正确编码。
- 退出失败不得伪装撤销成功；退出成功清空查询与 mutation 缓存。
- 最后凭据保护由后端强制；列表使用真实游标且加载中禁用翻页。

## Task 1: 认证路由与外壳

文件：apps/web/src/routes、features/auth、lib/router.tsx，App.tsx/main.tsx，必要依赖与 vite 生成注册。接口：createAppRouter({ history?, bundle? }) 供启动和内存路由测试；上下文为 QueryClientBundle。

- [x] 添加 t20_1 重定向、t20_2 登录返回原页、t20_3 退出清理缓存及恶意 redirect/401/网络失败测试，运行观察 RED。
- [x] 用共享 queryKeys.auth.session 定义/AuthGate、/login、/bootstrap、/recovery、/settings/security 与根外壳实现 GREEN。
- [x] 组合公开 Layout/Sidebar/NavGroup/NavItem，概览与未实现入口明确禁用。

## Task 2: 凭据与会话

文件：features/auth/security-page.tsx、webauthn.ts，t20.test.tsx。消费公开 /auth/passkeys、/auth/sessions 及 ceremony API。

- [x] 先写 WebAuthn 转换/取消、删除失败、撤销当前会话、列表下一页用例并观察 RED。
- [x] 实现注册/恢复/新增凭据，安全 token 表单（不入 URL/storage），凭据/会话游标列表和撤销。
- [x] 更新 T02 两条允许替换的占位外壳测试，保留其余守卫。

## Task 3: 验证与交付

- [x] frozen install、api:check、typecheck、lint、test、build；任务 RED/GREEN 及输出放 /data/knoted-dev/tasks/t20/evidence。
- [ ] 更新卡内证据，读取有效 git-commit Skill，安全检查并提交。
- [ ] 主 Agent 独立审查/最终门禁与集成，T21 承接测试实例部署，实施 Agent 不自行 merge。

风险：浏览器硬件 Passkey 需真实环境验收，自动化使用虚拟认证器或最小浏览器边界替身；无接口/依赖能力时记录阻塞。
