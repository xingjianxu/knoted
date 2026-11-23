# UI 开发：消费 mtone，不在 Knoted 建通用库

依据：[ADR 0002](../decisions/0002-mtone-ui-ownership.md)。适用于全部前端任务，尤其 T17—T20、T27—T29 与后续业务页面。新增或扩展组件前使用 [mtone-component-request Skill](../../.agents/skills/mtone-component-request/SKILL.md)。

## 布局与界面文案

**避免不必要的次级标签、副标题和说明，让布局本身说明内容与操作的关系。** 优先用明确的主标题、分组、对齐和留白表达结构；能从布局、标题或按钮名看懂的内容，不再用文案重复一遍。

- 例如，页面标题为「项目」，列表旁已有「新建项目」按钮，不必再加「项目列表」副标题或「在这里查看和管理项目」的说明。
- 保留理解或操作所必需的信息，例如字段标签、不能从界面看出的输入要求、错误提示和危险操作的后果。简化可见文案时，仍须保留控件的无障碍名称及必要的关联描述，不能只靠位置或颜色传达含义。

设计与验收时逐项检查：去掉这句文案后，用户（包括使用辅助技术的用户）还能理解内容并正确完成操作吗？能就省略，不能就保留提供新信息的最短表述。

## 现状与目标

当前精确依赖为 **`@bixlab/mtone@0.13.0`**（`apps/web/package.json` / `pnpm-lock.yaml`）。下文保留 T17—T19 的原始接入与各版本升级记录。

T17 已精确接入 npm `@bixlab/mtone@0.2.0`，迁移 App 静态类并移除 `packages/ui`；**PR #42 已普通 merge 至主线 `e3e1db2`，独立审查/主线验证/任务清理完成**。T17 接入时，生产入口使用应用层 `UiProvider` 配置公开 `MtoneProvider`，不转发控件 API；CSS 从应用样式入口导入。基础表单组合仅在测试夹具中，不建立生产演示路由或通用包装库。证据见 [T17](../plans/knoted-v1/evidence/t17.md)。

T18 实施在同一 `UiProvider` 内注册公开 `ToastProvider`，不增加通知总线或统一弹层 API；**PR #46 已普通 merge 至主线 `4425ab9`，独立审查/主线验证/任务清理完成**。Modal/ConfirmDialog/DropdownMenu/ContextMenu/Popover 与通知组合只在测试夹具，测试服务 `?task=t18` 选择本卡，默认仍为 T17。真实 Chromium 验证焦点、异步失败重试与 Portal 字体/主题/层级，见 [T18 证据](../plans/knoted-v1/evidence/t18.md)。

T19 新增生产应用游标 reducer `apps/web/src/lib/cursor-state.ts`（无 UI），在测试夹具 `?task=t19` 直接消费公开 Tree/List/ListItem/Empty/Spinner/Alert/ConfirmDialog/Button；**PR #48 已普通 merge 至主线 `6de79a8`，独立审查/主线验证/任务清理完成**。Pagination 依赖 total，本卡改用 Button 组合上下页且不伪造总数；T17/T18 入口不变，见 [T19 证据](../plans/knoted-v1/evidence/t19.md)。

## 责任判定

| 需求 | 处理 |
|---|---|
| mtone 已有公开组件满足 | 直接消费，测试业务接入，不重复申请同名组件 |
| 现有组件组合即可满足 | 在 `apps/web/src/features/` 或应用适配层组合；不重新包装一套通用 API，也不开上游 Issue |
| Knoted 数据、权限、游标、时间转换、编辑命令 | 留在 Knoted；多页面使用也不自动成为通用组件 |
| 缺所需通用能力，或既有通用组件需修改/扩展（行为、API、默认值、无障碍、样式）；能用公开 props/tokens/主题/CSS 变量或应用层组合满足的不算缺口 | 存档提示词 + 把提示词发给用户 + 在上游 mtone 创建或补充 Issue，记录外部依赖 |
| 版本/类型/资料不可取得 | 标记待核查，不宣称缺失，不凭名字猜 API |

基础控件、可访问性、设计变量优先由 mtone 提供。应用可用 Tailwind 做布局、用 mtone 公开 tokens 定制，不另建基础控件样式体系；禁止直接包装 Base UI、复制上游源码、私有路径导入、修改 node_modules 或用 patch 绕过缺口。这里不禁止 mtone 自身的 Base UI 传递依赖。

BlockNote 仍只保留编辑核心，关闭默认 UI；工具栏和菜单用 mtone 组合，命令、选区和文档模型在 Knoted。不把完整编辑器、后端或 Knoted DTO 搬进 mtone。

## T17 接入与版本锁定

实施时重新核查发布版本，而非无条件安装调研时的 0.2.0：

1. 读取 `apps/web/package.json` 与 `pnpm-lock.yaml`；记录未安装或精确已解析版本。区分 npm 发布包、网站文档、master 源码。
2. 检查该发布版本的 exports、types、peerDependencies、CSS 与组件契约。发布包是消费依据，不猜 props 或深层导入。
3. 在任务 Worktree 精确锁版并更新 lock，使用 `@bixlab/mtone/styles.css`、公开 Provider；验证 React 兼容、CSS 导入顺序、tokens、Portal 样式、字体及构建。
4. 迁移 App/main/styles 与 cn 使用；原 cn 含 tailwind-merge，不能假设同名 mtone 工具等价。删除占位包后同步 ESLint、workspace/脚本中实际存在的引用与 T02 源码扫描测试。workspace 当前使用通配路径，无需为删除目录强改通配规则。
5. 在 apps/web 验证真实消费行为，不以 mtone 上游测试代替。保留无远程字体验收，包含实际发布 CSS/构建产物。

T17 实测：旧 `cn('p-2', 'p-4')` 返回 `p-4`，mtone 的 `cn` 返回 `p-2 p-4`，不具备冲突合并语义。App 原有类全为静态，因此直接改用字符串，而非替换成同名工具。应用 CSS 将预编译样式放在 `mtone` 层（base 后、utilities 前），仅覆盖公开样式范围的系统字体；主题 tokens 通过公开 context 传播到 body-level MtoneScope。真实弹层交互与焦点生命周期的 T18 分支验收见下文及 T18 证据，主线集成状态单独跟踪。

0.2.0 在实施时距发布不足 pnpm 默认最小等待时间；`pnpm-workspace.yaml` 仅为已明确选择、检查过的精确版本添加 minimumReleaseAgeExclude，其他依赖策略保持不变。不改 workspace 通配范围。

只使用 npm 已发布版本作为正式依赖；不以浮动 latest、Git、file/link、未发布 tarball 交付。不修改 API 或业务语义来迁就组件。

## 0.13.0 升级与全站组件迁移记录

从 `0.8.0` 升级至 **`@bixlab/mtone@0.13.0`**，同步更新 `pnpm-workspace.yaml` 精确等待期例外与 `pnpm-lock.yaml`（新增上游 `Typography.Code` 语法高亮所需的 `@shikijs/*` 等传递依赖，无其他无关依赖漂移）。公开包入口、CSS 导出路径与 React peers 不变。[npm 元数据](https://registry.npmjs.org/@bixlab%2fmtone/0.13.0) · [发布说明](https://github.com/xingjianxu/mtone/releases/tag/v0.13.0) · [迁移指南](https://github.com/xingjianxu/mtone/blob/v0.13.0/docs/MIGRATION.md)

- **0.8.0 → 0.13.0 破坏性 API 迁移**：全仓将已移除导出迁移至新版公开 API——`Badge` → `Tag`、`FilterBar` → `Toolbar`、`FormField` → `Form.Item`、`Textarea` → `Input.TextArea`、`SearchInput` → `Input.Search`、`MultiSelect` → `Select mode="multiple"`、`DropdownMenu` 与带 `trigger` 的 `Menu` → `Popover.Menu`、`Segmented` → `RadioGroup variant="segmented"`。默认单行按钮基准高度由 34px 调整为 32px，已同步更新 T17 浏览器样式断言。
- **全站 mtone 组件全面采用**：
  - 页面外壳与分节：凭据与会话、工作概览、项目、事项、索引任务、搜索页面采用 `PageLayout`；笔记/项目关联、子事项、跟进时间线、文件详情与索引任务阶段采用 `SectionGroup`；根路由错误/404 页采用 `Result`。
  - 表单与容器：全站原生 `<form>` 统一迁移为 `Form`，原生 `<fieldset>`/`<legend>` 迁移为 `Fieldset`，手写边框面板统一替换为 `Card` / `CardGroup`，手写分割线替换为 `Divider`。
  - 工具栏、选择器、排版与对话：笔记操作栏、编辑器格式工具栏及回收站/事项/索引任务/搜索筛选栏采用 `Toolbar`（`background="white"` 保持标准控件边框）；笔记移动与回答转存笔记的父笔记选择迁移至可搜索 `TreeSelect`（仍由应用过滤当前笔记及后代，后端继续做完整性校验）；代码片段与文本预览采用 `Typography.Code`；问答消息行采用 `AiMessage`（保留外层列表、定位高亮与业务引用/转存流程）。
- 跟踪记录见 [Issue #272](https://github.com/xingjianxu/knoted/issues/272) 与 [任务卡 #272](../tasks/272.md)。

## 0.8.0 升级记录

`MtoneProvider` 不再支持 `density`；应用移除 `density="compact"`，保留中文 locale 与默认浅色主题。普通单行控件和侧栏导航项采用新版默认 34px 高度，不补写旧版紧凑模式样式。公开包入口和 `styles.css` 导出路径不变，直接依赖和 React peers 也不变。仅更新 mtone 的锁定版本、完整性摘要与发布等待期例外。[npm 元数据](https://registry.npmjs.org/@bixlab%2fmtone/0.8.0) · [发布说明](https://github.com/xingjianxu/mtone/releases/tag/v0.8.0) · [迁移指南](https://github.com/xingjianxu/mtone/blob/v0.8.0/docs/MIGRATION.md)

## 0.7.0 升级记录

npm 发布包保留公开根入口、`styles.css`、React peers 和直接依赖，仅更新 mtone 锁定版本、完整性摘要与该版本的发布等待期例外。新版的深色/跟随系统主题、TagSelect 和列表排序手柄不在本次启用范围，应用沿用默认浅色主题。虚拟列表吸顶分组的包装层有变，应用未使用该模式，也不依赖相关 CSS 选择器。 [npm 元数据](https://registry.npmjs.org/@bixlab%2fmtone/0.7.0) · [发布说明](https://github.com/xingjianxu/mtone/releases/tag/v0.7.0) · [迁移指南](https://github.com/xingjianxu/mtone/blob/v0.7.0/docs/MIGRATION.md)

## 0.6.0 升级与工作区布局

`_authenticated` 直接组合发布包的 `AdminLayout.Sider/Header/Content`。路由激活态、退出与笔记保存保护仍由应用负责；未登录的登录和初始化页不使用工作区外壳。侧栏在 768px 以下自动收为带可访问名称的图标导航，主区独立滚动；TanStack Router 在切页时重置主区滚动位置。

保留公开包根入口、CSS 与 React peers。`lucide-react@1.73.0` 原为 mtone 的传递依赖，现同时作为 Web 的精确直接依赖，用于工作区导航图标，未引入新的解析版本。发布等待期只替换 mtone 的精确例外。 [npm 元数据](https://registry.npmjs.org/@bixlab%2fmtone/0.6.0) · [发布包组件参考](https://unpkg.com/@bixlab/mtone@0.6.0/llms.txt) · [实施计划](../plans/2026-11-19-mtone-admin-layout-plan.md)

## 0.5.0 升级与兼容约定

正式 npm 发布包的根入口、CSS 导出路径、直接依赖和 React peers 与 0.4.0 相同。锁文件仅升级 mtone，并把该包的精确 release-age 例外同步到 0.5.0。 [npm 元数据](https://registry.npmjs.org/@bixlab%2fmtone/0.5.0) · [发布说明](https://github.com/xingjianxu/mtone/releases/tag/v0.5.0)

- Input/Textarea 的原生事件、ref 与标签关联通过消费测试。现有应用样式没有依赖新版调整的 DOM 结构。现有 List 使用子节点模式，编辑器 DropdownMenu 的操作项仍受支持。 [迁移指南](https://github.com/xingjianxu/mtone/blob/v0.5.0/docs/MIGRATION.md)
- 更新 Chromium 断言以检查新版可见焦点轮廓，以及包含当前可见弹层的容器。键盘提交、弹层关闭后的焦点恢复、禁用菜单项、树与游标行为均通过原有消费测试。实际构建页面用合成 API 数据对照 390、768、1440 像素宽度，搜索、事项详情和问答均无横向溢出。搜索框保留放大镜、清空入口和提交按钮位置，清空草稿与 Enter 提交通过浏览器对照。
- 新版把实心按钮的焦点轮廓移到深色填充内侧，原主题色与填充仅有 2.53:1 的对比度。通过 `--mt-focus` 给实心按钮使用浅色主题值，Chromium 核验键盘焦点在常态和悬停时均达到 3:1。 [WCAG 非文本对比度说明](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html#relationship-with-focus-visible)
- 若后续优化窄屏体验，优先考虑折叠常驻导航与低频搜索筛选项。390 像素下侧栏在两个版本均占 119 像素，搜索结果排在整组筛选字段之后，首条结果链接在样例中位于页面约 844 像素处。
- 较深的笔记目录可评估使用新版 TreeSelect 展示父笔记层级，帮助区分同名笔记。迁移时仍由应用校验根目录、当前笔记及其后代的可选规则，后端继续执行完整性校验。 [发布包组件参考](https://unpkg.com/@bixlab/mtone@0.5.0/llms.txt)

升级与回归记录见 [Issue #240](https://github.com/xingjianxu/knoted/issues/240)。

## 0.4.0 升级记录

已亲查正式 npm 发布包及其 README、llms.txt、公开声明与 CSS；exports、直接依赖与 React/ReactDOM peers 相对 0.3.0 不变，应用继续使用 React 19.3.1、公开根导出与 `@bixlab/mtone/styles.css`。lock 只替换 mtone 版本与 integrity，不升级其他依赖。

- **Tree**：`TreeItem.id` 扩展为 `string | number`，原 `items/value/expanded` 及回调仍可用。笔记树在应用内以递归 `NoteTreeItem` 保留字符串 UUID 类型，不将通用数字键传播到笔记目录完整性校验，不做类型强转或 ID 转换。生产笔记目录及 T19 继续消费原公开接口，无需采用多选、勾选、拖放或虚拟滚动。
- **其他扩展**：Tabs、Avatar、Divider 等公开类型有扩展，并新增多种组件；本任务不顺带迁移或接入未使用能力，不能以发布清单代替业务验收。
- **安装策略**：仅将 mtone 的精确 release-age 例外替换为 `@bixlab/mtone@0.4.0`，保留原 `react-resizable-panels@4.24.0` 例外，不全局放宽等待期。包管理器重解析的无关 Babel runtime 变化已排除，frozen install 通过。
- **消费复验**：现有 199 项 Vitest、T17—T19 共 15 项真实 Chromium 测试、前端 lint/typecheck/build 均通过。继续沿用 0.3.0 的 Dialog 外部关闭及 FormField 有效性语义适配；Provider、主题、系统字体、Portal 和编辑器无需改动。

发布依据：[npm 0.4.0 元数据](https://registry.npmjs.org/@bixlab%2fmtone/0.4.0)、[README](https://unpkg.com/@bixlab/mtone@0.4.0/README.md)、[类型](https://unpkg.com/@bixlab/mtone@0.4.0/dist/index.d.ts)、[llms.txt](https://unpkg.com/@bixlab/mtone@0.4.0/llms.txt)。完整检查与限制见 [0.4.0 证据](../plans/knoted-v1/evidence/mtone-0.4.0.md)。未发现需交接的新通用能力缺口。

## 0.3.0 升级记录（历史，兼容约定仍沿用）

已核查正式发布包的 README、exports、类型声明、实现和 CSS；React/ReactDOM peers 仍为 `^19.2.0`，本应用继续使用 `19.3.1`。公开根导出与 `@bixlab/mtone/styles.css` 导入不变；不因新增能力重构现有表单、外壳或列表。

- **Dialog**：新增推荐名称 `Dialog/DialogProps`，`Modal/ModalProps` 为已弃用兼容别名。`modal` 默认为 `true`，阻止点击外部关闭；需要保留原外部点击关闭场景时，显式使用 `<Dialog modal={false}>`。该属性不关闭焦点陷阱、遮罩或滚动锁，也不改变 Esc/关闭按钮。T18 夹具已迁移公开 Dialog 并复验关闭回焦，不在应用补焦点管理。
- **Form/FormField**：新增托管字段及规则 API；不传 FormField `name` 时仍是手动模式，现有 React Hook Form/业务状态无需迁移。手动字段清错后可能省略 `aria-invalid`（与 `"false"` 语义相同）；消费测试验证有效性、错误关联与描述恢复，不依赖旧的序列化形式。已有 `Form.onSubmit` 的原生提交消费保持不变。
- **发布参考**：0.3.0 新增 `@bixlab/mtone/llms.txt` 导出。本仓库 pnpm 工作区从 `apps/web/node_modules/@bixlab/mtone/llms.txt` 读取同版本参考，并结合包内 `dist/index.d.ts` 查完整签名；该文本不是 JS 模块，不加入浏览器入口。在线站点跟随 master，不能代替当前发布包，也不能仅凭示例认定消费场景已通过。
- **依赖与安装**：0.3.0 新增 `@rc-component/table` 与 `react-resizable-panels` 等传递依赖，不是 Knoted 直接包装底层组件的入口。`pnpm-workspace.yaml` 仅保留精确的 `@bixlab/mtone@0.3.0`、其要求的 `react-resizable-panels@4.24.0` 两项已核查版本等待期例外；不放宽其他包/版本策略，移除旧 0.2.0 例外。
- **样式与主题**：沿用公开 CSS 的 `mtone` layer、应用系统字体和 Provider tokens；T17—T19 复验真实字体、Portal、键盘、树选择/展开、无 total 游标及通知/确认行为。

发布依据：[npm 0.3.0 元数据](https://registry.npmjs.org/@bixlab%2fmtone/0.3.0)、[README](https://unpkg.com/@bixlab/mtone@0.3.0/README.md)、[类型](https://unpkg.com/@bixlab/mtone@0.3.0/dist/index.d.ts)、[llms.txt](https://unpkg.com/@bixlab/mtone@0.3.0/llms.txt)。新增组件未在本任务消费，不能以发布清单冒称已完成其业务验收。

## 历史核查映射（0.2.0，不是当前运行验证）

2026-11-16 核查 `@bixlab/mtone@0.2.0`：

| 旧计划意图 | 实际导出或应用责任 |
|---|---|
| Button/Input/FormField/TextArea/Badge | Button/Input/FormField/**Textarea**/Badge |
| Dialog/Menu/Popover/Toast | **Modal/DropdownMenu**、ContextMenu、Popover、ToastProvider/useToast |
| Tree/EmptyState/ConfirmAction | Tree/**Empty/ConfirmDialog**；业务确认语义留在应用 |
| PagedList | List 或 Table 展示 + 应用游标控制；不是现成同名导出 |
| 应用外壳 | Layout、Sidebar、NavItem、NavGroup；路由绑定留在应用 |

来源：[npm 元数据](https://registry.npmjs.org/@bixlab%2fmtone/0.2.0)、[发布 README](https://unpkg.com/@bixlab/mtone@0.2.0/README.md)、[发布类型](https://unpkg.com/@bixlab/mtone@0.2.0/dist/index.d.ts)。该版本 ESM、React/ReactDOM peers `^19.2.0`，CSS 预编译；这些事实不证明当前应用运行通过。

重点复验：Pagination 的 total 与本项目游标不匹配时，先用现有 Button 组合上下页，不能伪造总数；DatePicker 单日期不代表覆盖精确时间；树移动可由业务命令驱动，不自动等同于需要通用拖拽；编辑器虚拟锚点、中文输入法及回焦需消费场景验证。以上均是检查项，不是已确认上游缺陷。

## 缺口交接与恢复

使用 Skill 先查证、查重并分类。只有确认通用能力不足或既有通用组件需变更时才输出可复制提示词，存档于[mtone 需求目录](../plans/knoted-v1/mtone-requests/README.md)。交接含三个动作，缺一不可：提示词存档、把提示词直接发给用户（不仅给路径）、在 <https://github.com/xingjianxu/mtone> 创建或补充上游 Issue（先查重；命中既有同能力 Issue 时在其下评论并注明“复用”；英文正文、按性质仅用上游既有标签——新能力/扩展 `enhancement`、缺陷 `bug`、无障碍 `accessibility`；不粘 Knoted 内部链接与私有数据、回写编号与 URL）。能由现有组件组合满足的业务需求不算缺口，不开单。Issue 创建或评论失败时如实报告并列为待补项，不得伪造链接。不得修改 mtone 源码、代发上游 PR、发布 npm 包，也不在其仓库增删标签或改仓库设置。

- 查重：搜索该目录及关联 Knoted Issue；同一能力补充已有需求与消费任务，不开重复需求。
- 任务记录：影响哪个场景、证据、上游需求路径、上游 Issue（`xingjianxu/mtone#<编号>`）与恢复条件直接写回关联 Knoted Issue 评论及对应任务卡，不新增标签定义。
- 上游 Issue 被关闭为不适用或转为已有实现时，同步更新本地需求记录状态与理由，不静默删除记录，也不据此直接解除 Knoted 消费验收。
- 完成不依赖该缺口的其他工作但不擅自扩大任务范围；仅在确有根规则要求的重大架构或生产/不可逆决策时才停下来请求人工确认。
- 用户返回 npm 发布版本后：核验版本实际可用、公开 API/类型/CSS → 更新精确依赖与锁文件 → 跑消费场景与本地门禁 → 按正常 PR 集成、主线验证及适用部署收尾。仅“master 已合入”“提示词已交付”不能解除全部验收。

## 验证与升级

T17—T19 测试在 `apps/web/tests/tasks/`，验证字段关联、键盘、焦点、禁用提交、空/错/加载与取消行为。浏览器实际焦点/Portal/CSS 验收使用已有 Playwright 入口。mtone 自己的内部单元测试属于上游，不复制进本仓库。T19 另验证受控 Tree 展开/选择事件分离、disabled 节点、无 total 的 next_cursor 游标往返与筛选重置。

升级必须审阅公开 API/CSS 变化并复跑受影响任务。遇到回归可回退上一已验证 npm 版本；无法满足则记录阻塞，不本地补一套通用 UI。T17 迁移与基础表单消费验证已集成；T18 弹层/通知与 T19 树/游标/反馈已集成主线；T02/T04 历史证据保留。

0.2.0 消费注意：菜单 disabled 项可由方向键聚焦以便发现，但 Enter/Space/点击均不执行；不要把 disabled 推断为一定跳过焦点。Modal 绕回时会短暂聚焦上游 inert guard，测试应等待焦点回到真实内容，不能自行补焦点管理。Toast 关闭按钮在 hover/focus 展开后暴露给可访问树。ConfirmDialog 拒绝后显示中文错误并保持打开可重试，pending 禁用确认/取消按钮；公开契约不保证 pending 时禁止 Esc 关闭，后续业务需按自身取消/请求生命周期语义处理，不将本卡测试误作不可关闭承诺。Tree 的 `value/onValueChange` 与 `expanded/onExpandedChange` 传递节点 id（非 label）；鼠标点展开箭头不触发选择，Enter/Space 才选择，disabled 节点 tabIndex -1 且不选择/展开。
