# 02 · 账号入口与安全页验收

关联 [#250](https://github.com/xingjianxu/knoted/issues/250)；基线 `58b65f5`（左列），实现后（右列）。[1440×900 对照](02-desktop.jpg) · [390×844 对照](02-mobile.jpg)。从上到下为登录、首次注册、恢复和安全页。使用本地 Chromium 与 API/浏览器认证合成响应；没有部署、真实账号或实际 Passkey 仪式。

| 检查 | 实测 |
| --- | --- |
| 公开页 | 登录主按钮为首个键盘操作；待认证时同一按钮不可重复启动，模拟浏览器取消后焦点和重试保留。注册以密码字段接收令牌；恢复在填写前解释旧会话和凭据撤销。模拟恢复失败保留可修正输入。 |
| 注册与退出 | 安全页进入后不展示新增表单；从「添加 Passkey」进入时聚焦名称字段，取消回到触发按钮，重新验证链接返回安全页。切换路由时中止未完成的浏览器注册，迟到结果不提交。 |
| 列表与反馈 | 凭据与会话独立请求、游标、加载及重试；凭据加载失败时仍可翻会话页，取数期间上一页和下一页不可重复触发。现有最后一枚凭据保护和服务端冲突说明保持。 |
| 危险操作 | 用长中文名在 320px 下键盘打开删除确认；确认、取消都在视口内，Escape 回具体凭据且未执行删除。服务端拒绝留在对应对话框，仅在 `recent_verification_required` 时提供重新验证；`csrf_failed` 提示刷新，且失败仅有一处 assertive alert。撤销当前测试会话清除私有查询并跳回登录。 |
| 宽度 | 1440、768、390、320 CSS px 下，四个账号页面的主体无横向溢出。相同长名夹具在 320px 安全页的 `clientWidth/scrollWidth` 从 `268/627` 变为 `268/268`。实际 Chromium 标签页 200% 缩放（640px → 320 CSS px、DPR=2）下，登录和安全页也无横向溢出。 |

`pnpm --filter @knoted/web exec vitest run tests/tasks/t20.test.tsx` 验证 API 请求与生命周期；`pnpm exec playwright test tests/e2e/admin-layout.spec.ts tests/e2e/account-layout.spec.ts` 验证真实浏览器布局与焦点。使用 `@bixlab/mtone@0.6.0` 公开的 Button、Alert、FormField、Input、List/ListItem、Empty、Spinner 和 ConfirmDialog；没有变更依赖、接口或 `safeReturnTo` 白名单。
