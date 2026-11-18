# RFC: OnlyOffice 取源凭据改用官方出站 JWT

- Date: 2026-11-18
- Status: Resolved
- Owner: Knoted 项目维护者
- Related issues / ADRs: [T51 #179](https://github.com/xingjianxu/knoted/issues/179)、[ADR 0008](../decisions/0008-office-preview-authorization.md)
- Decision record: [ADR 0009](../decisions/0009-office-source-jwt-header.md)；不批准任何共享服务改动。

## 问题、目标与非目标

T51 首轮独立审查发现，已实现的 URL 查询凭据无法满足“不将临时授权写入日志”。主 Agent 在正在运行的 OnlyOffice Document Server 10.0.0 对应 FileConverter Nix 包中确认 `downloadFile` 错误和 `checkIpFilter` 错误都会记录完整 URI（`fileconverter/sources/converter.js:535,552`）。Knoted 的日志脱敏无法覆盖这个边界。未启用首轮实现，未生成或输出真实凭据以重现。

解决取源凭据日志风险；保留已确认的浏览器 DTO 形状、固定版本、120 秒应用授权、内存副本、两份准入、按预览实例撤销、会话检查及只读浏览。不向共享 OnlyOffice 或代理添加代码、插件、补丁、配置，也不请求为 Knoted 放宽现有安全约束。

## 方案与推荐

- 保留 URL 查询凭据：会在现有服务错误日志出现，不可交付。
- 缩短查询凭据或设置一次性：首次取源失败时仍可能记录有效凭据；不解决根因，不采用。
- **官方出站 JWT（推荐）**：URL 仅保留随机预览 ID，无查询凭据；Knoted 仅接受 `Authorization: Bearer` 中由已信任 Document Server 签发、含精确完整取源 URL 的 HS256 JWT，核对签名与时间以区分浏览器签名配置和出站请求。该 JWT 自身的默认有效期不代表 Knoted 授权仍有效：只有 120 秒内、未撤销且仍属已登录会话的同一内存租约能取源。只信任现有运行凭据，缺少或不匹配一律拒绝，没有查询回退。

[官方协议](https://api.onlyoffice.com/docs/docs-api/additional-api/signature/request/token-in-header)说明下载用出站请求头。本机启动脚本已设 `token.enable.request.outbox=true`，签名从现有运行凭据读取；安装的 10.0.0 FileConverter 实际通过 `{payload:{url},iat,exp}` 签发。核查只读程序与非秘密字段，未读取密钥。整个共享部署可原样保持；经配置获得同一签名凭据的服务是已存在的信任边界，不把已泄露的签名密钥视为可抵御威胁。源 URL 本身没有访问能力。

## 影响与验证

只调整尚未交付的 Document Server 取源 OpenAPI 契约、JWT 校验、浏览器 `document.url` 值及测试；无迁移、无客户端字段变更。拒绝无头部、错签、错 URL、过期、撤销、浏览器签名 JWT 或额外查询；保留 T51/T49/Rust 门禁。按现有 dev 与浏览器流程验证实际官方出站请求、成功取源和失败路径不传播凭据（只保留脱敏输出）；若共享服务行为不满足则保持禁用，不修改服务规避。

## 评审记录

- 2026-11-18：T51 独立审查阻止合入；主 Agent 验证当前共享构建的根因。维护者在「取源鉴权」中选择「采用官方请求头 (Recommended)」，已记录 ADR 0009；代码修复与实际运行验证尚待完成。
