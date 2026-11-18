# RFC: T52 浏览器预览不向共享 OnlyOffice 发送 Knoted 会话

- Date: 2026-11-18
- Status: Resolved
- Owner: Knoted 项目维护者
- Related issues / ADRs: [T52 #180](https://github.com/xingjianxu/knoted/issues/180)、[ADR 0008](../decisions/0008-office-preview-authorization.md)、[ADR 0009](../decisions/0009-office-source-jwt-header.md)
- Decision record: [ADR 0010](../decisions/0010-office-preview-credentialless-host.md)；不批准任何共享服务或宿主改动。

## 问题与证据

T52 独立审查发现，Knoted dev（`https://wlab.132140.xyz:15601`）与共享 OnlyOffice（`https://wlab.132140.xyz:15402`）虽不同源但同 hostname，Cookie 不区分端口。顶层直接加载官方 `api.js`，以及其创建的查看器 iframe，可以携带 Knoted 的 Cookie；HttpOnly 不会阻止发送。当前 T52 未合入、未部署，未用有效的 Knoted 审查会话执行这个不安全的页面路径。官方脚本对该调用源未提供 ACAO，且只处理脚本也不足以隔离嵌套 iframe。T51 的真实转换验收使用单独的无 Knoted Cookie 浏览器上下文，不受此页面问题影响。

Chromium 156 合成测试对同主机双端口发出脚本、内层框架请求：普通框架两次都带测试 Cookie；外层 Knoted `credentialless` 框架两次都没有带 Cookie。测试只使用回环两个临时监听和随机 canary，不输出 Cookie 值；具体证据 `/data/knoted-dev/tasks/t52/evidence/credentialless-probe.json`。这证明当前浏览器隔离机制，不证明 OnlyOffice SDK 已经在隔离页内正常。

## 方案与推荐

1. **Knoted `credentialless` 预览页（推荐）**：应用页面不加载 OnlyOffice 脚本，使用同源的 Knoted 静态预览宿主页放在 `credentialless` iframe 内。只在该页使用标准 DocEditor 原样加载官方脚本与嵌套查看器。脚本和框架 CSP 仅放行配置的精确 HTTPS origin；父页只需原本本地模块、同源宿主页及 PDF `blob:`。浏览器不支持 `credentialless` 时不发预览配置或加载 SDK，继续提供原件下载。
2. 强制使用不同 hostname 的受信 OnlyOffice 地址：隔离 Cookie 且支持更广；当前没有一个已经验证、可使用的不同 hostname 的同一共享服务入口。在不改共享服务/宿主的前提下，当前 dev Office 预览保持禁用；可以等维护者提供现成批准的服务地址。
3. 仅使用 HTTPS、SameSite、端口限制或给脚本设置 `crossorigin`：HTTPS/CSP 只控制传输与来源不控制端口 Cookie；仅限制脚本不会阻止查看器 iframe 的 Cookie，现有官方脚本也不提供所需 CORS，不采用。

推荐方案中，鉴权创建、撤销及会话仍在 Knoted 父页，宿主页不调用认证 API、不获取会话 Cookie。两页使用同源、逐打开绑定的消息通道，校验来源、目标窗口、生命周期与消息结构；签名配置不经 URL、共享存储或日志传递。清理时销毁或卸载宿主页并在父页撤销；启动超时同样销毁、撤销、提示重试，晚到不渲染。隔离用于避免自动附带认证凭据，不把部署信任的 SDK 宣称为非可信代码沙箱。

不更改共享 OnlyOffice、Caddy、宿主、任何其它消费者、Knoted Cookie 规则、T51 契约或原件下载。Unsupported browser fallback is fail-closed (不退回顶层加载)。

## 验证与决策

修复 IPv6 无法表达的 CSP 源和 silent startup 无重试，测试发布产生的精确 CSP。使用合成 Cookie 浏览器断言实际 SDK、真实 viewer 网络请求不带 Knoted Cookie，并完成认证页面的真实 XLSX 工作表切换、DOCX/PPTX 只读、精确 CSP 和原生 PDF。真实服务不兼容时只保持禁用，不增改共享服务或借端口区分 Cookie。

维护者已确认采用 Knoted 侧 `credentialless` 宿主页，并接受不支持浏览器关闭 Office 预览、保留原件下载。

## 评审记录

- 2026-11-18：T52 首轮独立审查阻止合入。主 Agent 已通过 Chromium 循证同主机端口 Cookie 行为和合成隔离能力，维护者在「浏览器隔离」中选择「在 Knoted 隔离 (Recommended)」，记录 ADR 0010。
