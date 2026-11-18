# ADR 0010: OnlyOffice 网页使用不携带凭据的 Knoted 宿主页

- Date: 2026-11-18
- Status: Accepted
- Decision owner: Knoted 项目维护者
- Confirmation: 2026-11-18 当前 T51/T52 会话，维护者在「浏览器隔离」中选择「在 Knoted 隔离 (Recommended)」，接受不支持 `credentialless` 的浏览器关闭 Office 预览、保留普通下载。
- Related issues: [#180](https://github.com/xingjianxu/knoted/issues/180)
- Related PRs: [#183](https://github.com/xingjianxu/knoted/pull/183)
- Supersedes: 无；细化 [ADR 0008](0008-office-preview-authorization.md) 网页边界
- Superseded by: 无

## 背景与问题

当前 Knoted 与共享 OnlyOffice 为同 hostname 不同 HTTPS 端口，浏览器 Cookie 不按端口隔离。顶层加载官方脚本或其框架会发送 Knoted Cookie；HTTPS 和 HttpOnly 不改变此行为。合成 Chromium 样本对比和备选见 [RFC](../rfcs/2026-11-18-office-preview-browser-isolation.md)。未启用首轮页面。

## 决策内容

- Knoted 父页创建独立的、固定同源宿主页 `credentialless` iframe，再在其中通过固定的可信 HTTPS origin 加载官方 `api.js`，使用标准 DocsAPI。父页不加载该外部脚本；宿主页只通过逐预览绑定且验证来源/目标窗口的通道接收签名配置，不调用会话 API，不经 URL 或 storage 传配置。隔离针对浏览器自动发送凭据，不宣称可信服务脚本可视为不可信代码。
- 浏览器不支持隔离时，不加载 SDK、不领取 Office 授权，提供说明与普通原件下载，不回退到直接加载。父页 CSP 保留自己、宿主页及 PDF Blob；宿主页脚本/框架 CSP 只增加配置的精确来源。不接受 CSP 无法准确表达的 Origin，不用通配符或任意 inline/eval。
- 预览创建/撤销、实例状态和 120 秒租约仍沿用 T51。关闭/切换/错误/超时销毁宿主页、撤销该实例；迟到回执不得复活。失败后可重试，只要求已证实的浏览器行为。
- Knoted 自行实现，不改共享 OnlyOffice、Caddy、宿主、凭据 Cookie 策略或其它消费者。不加共享插件、补丁或路由；真实协议不兼容时保持未启用并记录阻塞。

## 备选、理由与影响

不同 hostname 可以使用既有浏览器网络边界，但当前没有无需更改共享配置的已验证地址。仅配置跨端口 CSP、HTTPS 或脚本 crossorigin 不覆盖查看器框架请求。选择只改 Knoted 的显式无凭据宿主，减少父页网络权限；代价是多一静态入口、信道和生命周期，以及不支持浏览器降级为下载。无迁移；回退时关闭 Office 配置或回到前一主线固定产物，不回退到直接加载。

## 验证与历史

以真实 Chromium 监测 SDK/子帧的请求凭据并验证 XLSX 工作表切换、只读、撤销/失败重试、精确 CSP 及 PDF 回归。2026-11-18：维护者确认，记录 Accepted；合成 probe 不是产品部署或功能通过证据。
