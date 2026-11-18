# ADR 0009: OnlyOffice 取源只接受官方出站 JWT 请求头

- Date: 2026-11-18
- Status: Accepted
- Decision owner: Knoted 项目维护者
- Confirmation: 2026-11-18 当前 T51/T52 会话，维护者在「取源鉴权」中选择「采用官方请求头 (Recommended)」。用户明确要求所有 Knoted 定制仅在 Knoted，不改共享 OnlyOffice。
- Related issues: [#179](https://github.com/xingjianxu/knoted/issues/179)、[#180](https://github.com/xingjianxu/knoted/issues/180)
- Related PRs: 尚未创建
- Supersedes: 部分替代 [ADR 0008](0008-office-preview-authorization.md) 的取源随机凭据；其它决定不变
- Superseded by: 无

## 背景与问题

T51 初版将临时凭据放进 `document.url`。独立审查和实际安装的 OnlyOffice 10.0.0 程序核对证明，失败日志会输出完整 URI；只在 Knoted 脱敏不能保护共享转换器日志。原实现未部署。比较和证据见 [RFC](../rfcs/2026-11-18-office-source-header-auth.md)。

## 决策内容

- `document.url` 仅含经配置的 Knoted HTTPS origin、固定取源路径和随机预览 ID；不含任何查询或授权凭据，仅凭 URL 无法取回文件。
- Knoted 验证官方 `Authorization: Bearer` 出站 HS256 JWT，核对签名、发行/过期时间和 `payload.url` 等于该 ID 的完整规范取源 URL。不接受无头、Cookie、旧查询凭据或不同 URL/JWT 用途。签名文件由运行配置引用，不把正文写入任何配置模板或日志。
- 出站 JWT 的期限不改变 Knoted 租约。只有最多 120 秒、未撤销、且创建会话仍有效的固定副本可交付；上游缓存和已送出的响应仍不能召回。浏览器的创建/撤销 DTO、两份准入和 50 MiB 不变。
- 共享 Document Server 已支持此官方机制；不更改其代码、部署、代理、日志或配置，不加插件。签名密钥仅供已有授权的可信方。若共享服务未发出兼容的头或已有安全要求无法满足，Office 预览保持关闭并报告，不启用查询回退。

## 备选与理由

查询 bearer 即使短时/单次仍可能在转换失败时进入日志。官方出站 JWT 已由当前实例启用，把凭据从会记录的 URL 移开而不侵入其他系统。

## 影响

- 正面：URL 可作为非凭据标识显示在绑定日志中；浏览器 DTO 结构无兼容变化。
- 负面：依赖受信 Document Server 现有出站签名能力；持共享签名密钥者在信任边界内，不把密钥泄露当可容忍情形。
- 迁移与回退：端点未交付，无迁移。兼容失败关闭 Office 预览，不能回退到查询 bearer。不动共享/生产服务。

## 验证与历史

补充无头/错签/错 URL/过期/撤销和实际 HTTPS 取源失败日志检查及浏览器验收。2026-11-18：维护者确认推荐方案后 Accepted；不宣称已有修复实现或运行验收。
