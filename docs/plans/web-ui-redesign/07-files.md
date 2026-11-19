# 07 文件浏览、详情与预览

实施 Issue [#257](https://github.com/xingjianxu/knoted/issues/257) · [验收与限制](evidence/03-12.md)

目标是让使用者从文件清单进入详情后，直接打开原件或预览，同时能区分原件属性、Knoted 内维护的备注与标签、来源重关联。前置为 01-foundation 已验收并锁定的最新正式版 mtone（本次审计的仓库版本为 0.6.0）。设计放在现有 AdminLayout.Content 内。文件写入与预览边界见 [ADR 0006](../../decisions/0006-file-pipeline-and-aggregate-contracts.md)、[ADR 0008](../../decisions/0008-office-preview-authorization.md)、[ADR 0009](../../decisions/0009-office-source-jwt-header.md)、[ADR 0010](../../decisions/0010-office-preview-credentialless-host.md)。

## 页面审计与排布

| 页面或入口 | 现状与影响 | 改后阅读和操作顺序 |
| --- | --- | --- |
| `/files` | 每行把路径与状态拼作描述，末尾重复「打开」。一页最多 50 项，只有上一页和下一页。 | 页首「文件」和「索引任务」链接。用 List/ListItem 显示可直达的文件名、可换行的相对路径及有文字的 Status，底部游标翻页。桌面未选择文件也展示完整清单。 |
| `/files/$fileId` | 父路由先绘制整页清单，文件详情接在翻页后。名称后先展示 ID 和哈希，预览和下载在编辑表单后。 | 宽屏改为左侧紧凑清单、右侧所选文件。详情首行「文件名、状态、预览文件、下载原文件」，第二行路径；随后备注与标签编辑区、来源属性、手动重关联。窄屏详情首行提供「返回文件列表」，路由决定当前可见面板。 |
| 文件详情内预览 | 所有格式共用 Dialog，普通格式根据已验证字节选择阅读器，Office 有独立宿主页。 | 名称作为弹层标题，正文是最大可用阅读区，关闭和下载留在可到达的页脚。保留「当前格式无法显示」等可操作反馈。使用单个预览弹层。 |

```text
文件                                            索引任务
文件名、路径、状态       │  文件名／状态         预览文件  下载原文件
所选项与键盘焦点        │  相对路径
…                     │  备注、标签         保存变更  丢弃草稿
上一页    下一页        │  来源属性          手动重关联
```

当内容区不足以同时容纳清单与编辑字段时改为单面板；保留父路由的游标状态。文件名和路径在两种布局均可换行，SHA-256 和 UUID 可选中复制。来源属性用 PropertyList/PropertyItem 显示 ID、大小、SHA-256、实体版本／内容版本、最后出现时间；宽度不足时值另起一行。标签用 FormField + MultiSelect，载入更多与失败重试靠近该字段。重关联用低频展开区，读取目标后 ConfirmDialog 同时展示来源与目标的路径和 ID。「已丢失」时将恢复入口提至只读说明旁；仍能查阅属性与原有备注、标签。

## 预览面与交互边界

| 预览 | 主体 | 失败、关闭与切换的处理 |
| --- | --- | --- |
| PDF | 浏览器原生 PDF iframe，保留文档名、下载退路和宽高自适应。 | 仅展示已验证签名与版本的 Blob。关闭或切换后撤销 URL；实际渲染须在支持原生 PDF 的浏览器验收。 |
| PNG／JPEG／WebP | mtone Image，按容器等比完整显示，alt 为文件名，关闭额外图片弹层。 | 继续检查实际格式、静态容器及 4000 万像素上限，解码失败立即释放 URL 并给出下载入口。 |
| 严格文本 | 使用可选择文字的 pre，保留换行，长行在窄屏内折行。HTML 等按源码展示。 | 保留 UTF-8、带 BOM 的 UTF-16 与严格解码错误反馈；普通预览流累计上限 50 MiB。 |
| DOCX／XLSX／PPTX | 同一 Dialog 嵌入已授权的只读 Office 宿主页，留出文档滚动、工作表或幻灯片切换空间。 | 保持固定内容版本和逐实例撤销。浏览器缺少 credentialless 或服务未配置时显示原因和下载；启动失败／超时按已有入口显式重试。关闭、切文件或迟到的授权回执均按所属实例清理。 |
| 不支持、丢失、版本变化、会话失效 | 可下载的格式错误显示当前原因；missing 状态禁用原件操作。 | 下载仍走鉴权原件读取和版本校验，401 清除私有内容；仅在原文件入口仍挂载且可用时回焦。 |

## 文件范围与执行步骤

主要改动位于 `apps/web/src/features/files/{FilesPage,FileDetail}.tsx`、`preview/{FilePreview,OfficePreview}.tsx`（preview 位于同一 files 目录）以及 `apps/web/src/routes/_authenticated.files.tsx`。以 `features/files/queries.ts`、`preview/{inspect,read,images,office-preview-host}.ts` 和 `apps/web/office-preview.html` 为回归依据。保持大小写归一化、UUID 直达与 `/files/jobs` 独立路由。

- [ ] 以长清单最后一项、超长路径、missing 来源建立路由级测试。断言宽屏显示当前文件与原件操作，窄屏打开详情后可直接操作、返回原清单，Tab 可定位所选文件。加载、读取失败与空清单各有独立反馈；加载时锁定翻页。
- [ ] 重排清单与详情，再迁移属性和表单。保留仅提交已更改字段及 expected_version、未知标签 ID、冲突草稿副本、刷新与确认采用当前版本两个动作。保存成功但列表刷新失败时区分两者；有草稿时禁用重关联。
- [ ] 调整预览可视高度与页脚位置。保持鉴权读取、ETag、显式打开、abort、URL 撤销及逐预览 Office 授权；验证 A→B→A、Esc、失败重试与回焦。窄屏软键盘、原生查看器工具条不得盖住关闭或下载。
- [ ] 在仓库根运行下列检查，按 01 的共享门禁留存结果。浏览器用 390、768、1440 像素分别截图清单、详情和打开的弹层，提供一份真实 PDF、静态图、长文本及 Office 工作表切换的脱敏验收记录。说明哪些环境缺少原生查看器或隔离能力，对应记录降级路径。

```sh
pnpm --filter @knoted/web exec vitest run tests/tasks/t49.test.tsx tests/tasks/t50.test.tsx tests/tasks/t52.test.tsx
pnpm --filter @knoted/web run typecheck
pnpm --filter @knoted/web run lint
pnpm --filter @knoted/web run build
```
