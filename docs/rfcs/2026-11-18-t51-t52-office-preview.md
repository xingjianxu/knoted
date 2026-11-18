# RFC: T51/T52 OnlyOffice 只读预览授权

- Date: 2026-11-18
- Status: Resolved
- Owner: Knoted 项目维护者
- Related tasks / ADRs: [T51](../plans/knoted-v1/tasks/T51.md)（[#179](https://github.com/xingjianxu/knoted/issues/179)）、[T52](../plans/knoted-v1/tasks/T52.md)（[#180](https://github.com/xingjianxu/knoted/issues/180)）、[ADR 0002](../decisions/0002-mtone-ui-ownership.md)、[ADR 0005](../decisions/0005-https-owned-by-caddy.md)、[ADR 0006](../decisions/0006-file-pipeline-and-aggregate-contracts.md)
- Decision record: [ADR 0008](../decisions/0008-office-preview-authorization.md)；不授权变更共享服务或生产配置。

## 问题、目标与非目标

已确认的任务目标是从文件详情打开 DOCX、XLSX、PPTX 的原版式只读预览。T51 负责版本固定、短时文件授权和签名配置；T52 负责 DocEditor 生命周期、XLSX 工作表切换与最小 CSP。原件继续只读，不加编辑、保存回调、格式转换 API、跨重启恢复或新的文件根。不承诺从浏览器/Document Server 收回已交付的副本。

## 现状和备选方案

- 已核实 T15 `32cbb6f`、T49 `8ffb240`、T50 `2c89043` 均为基线 `a303192` 的祖先。T49 已有当前来源的批准根/FD/大小/SHA/扫描与版本栅栏读取；T50 的 Dialog 按文件快照隔离资源生命周期。两条链路都不应再实现一遍。
- OpenAPI 已登记 `/files/{fileId}/office-preview` POST/DELETE，但 `OfficePreview` 只有 `url`、`expires_at`，没有选定内容版本、实例撤销粒度及签名配置，也未登记 Document Server 的取源接口。不是调整现有已运行 API 的兼容性问题，但属于新的跨模块契约。
- 宿主已提供 `https://wlab.132140.xyz:15402`，本次只读健康检查返回 200。现有配置禁文档外联、禁止访问云元数据；当前宿主用户对默认 JWT 文件路径的 `test -r` 通过，未读取密钥；健康检查不证明 Knoted API 的实际运行进程能读取凭据，或 Document Server 能取到 Knoted 的实际源。Knoted 测试入口当前响应未见 CSP 头，不假定可在已有头上简单追加。

| 方案 | 收益 | 成本或风险 |
| --- | --- | --- |
| 单 API 进程的有界内存固定副本（推荐） | 无迁移/磁盘缓存，重启时自然清除临时授权；与当前单实例部署匹配 | 需严格限制同时创建、存留和正在交付的副本；不支持透明跨进程扩容 |
| 私有磁盘副本加持久授权/租约 | 便于扩展到多个 API 进程，并可定义可恢复的预览 | 引入新存储契约、权限、崩溃清理和迁移；两张卡都不要求重启后恢复 |
| 每次取源重新读原件 | 实现表面较少 | 原件变化后无法交付承诺的已选版本；直接给原件下载 URL 还会混淆两种授权，排除 |

## 推荐方案

1. 保留既有 POST/DELETE 路径，POST 增加 `expected_content_version`，旧选中版本不能悄悄换成新版本；返回 `preview_id`、`expires_at`、受信 Document Server origin 和服务端签名的只读 DocEditor 配置。DELETE 按 `fileId + preview_id` 撤销并核对创建者会话；同一文件的预览互不干扰。浏览器接口走 T15 会话与 CSRF。OpenAPI 先定稿，再生成客户端和实现。
2. 使用 T49 的当前批准根与稳定内容校验读入一次，只允许 DOCX/XLSX/PPTX，每份按既有预览上限 50 MiB 拒绝超限输入；普通下载不受影响。全 API 实例最多两份预览，提前预留准入，失败回收；过期、撤销或重启释放，新取源请求不得再访问。尚在交付的副本也有界，不因过期而无限叠加。
3. Document Server 的 GET 只接受已签发的高熵短时源 token，最长 120 秒，验证对应预览和仍有效的创建者会话，仅返回该内存副本；验证失败时关闭访问，不接受路径参数或任意 URL。两端以配置的固定 HTTPS origins 组装地址，不信任请求头提供的 host。JWT 只从运行凭据读取；短时授权、文件正文与 JWT 不入日志。只读签名配置禁用写入、编辑、权限扩大和保存回调。离开/撤销不会宣称已经交付的内容被收回。
4. T52 复用 T50 的文件详情入口及 mtone 控件，使用 OnlyOffice 原生 DocEditor。关闭、切换、错误、会话失效时销毁实例并撤销已取得的授权；迟到的创建回执也撤销、不渲染，网络无法交付回执时靠服务端 TTL 回收。CSP 只允许配置中唯一精确的 HTTPS origin 作为额外 script/frame 来源；保留已有本地资源及 PDF Blob 需要，不用通配符，不为接入打开任意内联脚本/eval。不改变 Document Server 既有文档外联限制；具体策略交付在 T52 中以真实浏览器验证，不默认授权调整宿主或共享服务。

## 影响与迁移

无需 PG/schema 迁移，无持久预览目录、凭据复制输出或恢复操作。未配置或不可读时 Office 预览关闭/报依赖不可用，现有文件浏览、下载和 PDF/文本/图片预览不受影响。测试实例只接入现有服务；若受信地址、凭据读取或 CSP 无法在现有授权内完成，记录对应阻塞，不关闭过滤、不更改其他应用。回退为停用 Office 预览或切回旧产物；在途临时授权不恢复。

## 验证与开放问题

- T51 真正 API 用例：过期、其他目录或不匹配身份不能取源、选中内容变化不改已签副本、旧版本拒绝、只撤当前实例/会话撤销、超过两份、失败回收、只读签名和无缓存响应。
- T52 Vitest 验证晚到结果、失败重试和单一信任来源；实际 HTTPS 浏览器验证有两个不同工作表的 XLSX 可切换、只读、精确 CSP 生效、关闭后撤销；普通预览和下载不回归。仅健康检查和模拟 DocEditor 不视为部署验收。
- 已确认采用单实例、非持久化、按预览实例撤销的 API/授权契约。OpenAPI、实施和功能验收仍待执行；共享宿主配置或非 Knoted 服务变更另行确认。

## 评审记录

2026-11-18：根据 T15/T49/T50 实现、当前 OpenAPI 和宿主只读健康检查提出草案；维护者在会话中选择推荐方案，转 ADR 0008。
