# ADR 0008: OnlyOffice 预览使用短时固定版本授权

- Date: 2026-11-18
- Status: Accepted
- Decision owner: Knoted 项目维护者
- Confirmation: 2026-11-18 当前 T51/T52 实施会话，维护者在「预览授权契约」中选择「确认推荐方案 (Recommended)」：单 API 实例最多两个固定版本内存副本、120 秒、按实例撤销、网页只放行精确的受信 OnlyOffice 来源、不改共享服务配置。
- Related issues: [#179](https://github.com/xingjianxu/knoted/issues/179)、[#180](https://github.com/xingjianxu/knoted/issues/180)
- Related PRs: 尚未创建
- Supersedes: 无
- Superseded by: 无

## 背景与问题

[T51](../plans/knoted-v1/tasks/T51.md) 和 [T52](../plans/knoted-v1/tasks/T52.md) 在既有源验证、会话与文件详情预览链路上新增 OnlyOffice。浏览器契约最初仅预留 `url` 和 `expires_at`，不足以固定内容版本、传递签名配置、区分同时预览及跨服务取源，详见 [RFC](../rfcs/2026-11-18-t51-t52-office-preview.md)。

## 决策内容

- 保留 POST/DELETE 路径。创建绑定文件标识、选定的 `content_version`、会话；返回 `preview_id`、`expires_at`、固定 Document Server origin 和服务端签名的只读 DocEditor 配置。撤销使用同一路径并指定 `preview_id`，仅创建者会话能撤销该预览。浏览器写接口沿用会话/CSRF。
- 复用 T49 的批准根、当前稳定观察和完整摘要校验，DOCX/XLSX/PPTX 单份预览限 50 MiB，全 API 实例最多两份；取源从同一只读内存快照，源变化不是允许交付新内容的理由。未完成创建和正在交付的副本也必须有界；不持久化，不增加迁移，API 重启撤销旧授权。
- 取源 URL 由已配置的 Knoted HTTPS origin 生成；短时随机 token 只绑定该预览，最长 120 秒；取源仍核对会话有效性。接口不接受文件路径或调用方 URL。JWT 从受控运行凭据读取，权限禁写、读模式、无保存回调、不将凭据写入配置正文或日志。撤销只阻止后续取源，不能撤回已交付的副本。
- 按现有文件详情集成 DocEditor 和 mtone 控件；关闭/切换/失败销毁及撤销，禁止迟到结果渲染。浏览器策略仅放行配置的精确 HTTPS origin 作为额外 script/frame 来源，保留普通预览所需本地来源和 PDF Blob；不放宽文档网络限制，不修改共享服务或生产配置。

## 备选方案

持久快照/租约便于跨进程恢复，但多出存储权限、迁移、崩溃回收和备份复杂度。每次取源重新打开原件不满足选中版本固定，给出普通下载 URL 也不满足短时最小授权。采用有界进程内副本。

## 决策理由

当前为单 API 实例，任务要求短时、受限预览，并不要求重启恢复。复用既有安全下载的读取边界与浏览器预览生命周期，无需另一套文件身份或源根白名单。

## 影响

- 正面：新取源被限制在一个短时授权引用的版本，不触碰源写入；临时数据不会进入数据库或备份。
- 负面：API 重启需重新打开预览，不支持多 API 实例透明分担；Document Server 不可用或配置缺失时仅 Office 预览受影响。
- 迁移与回退：无 schema 迁移。停用预览配置或切回上一固定产物；旧临时授权不恢复。不更改共享 OnlyOffice、Caddy 或生产环境。

## 验证与后续任务

按 T51、T52 顺序执行契约、API 授权、网页生命周期及 HTTPS 实际 XLSX 工作表切换和 CSP 验收。本记录只确认方向，不证明实现、测试或 dev 已更新。

## 状态历史

- 2026-11-18：RFC 推荐方案在实施会话经维护者确认，记录为 Accepted；不替代已有 ADR。
