# UI 开发：消费 mtone，不在 Knoted 建通用库

依据：[ADR 0002](../decisions/0002-mtone-ui-ownership.md)。适用于全部前端任务，尤其 T17—T20、T27—T29 与后续业务页面。新增或扩展组件前使用 [mtone-component-request Skill](../../.agents/skills/mtone-component-request/SKILL.md)。

## 现状与目标

T17 已精确接入 npm `@bixlab/mtone@0.2.0`，迁移 App 静态类并移除 `packages/ui`；**PR #42 已普通 merge 至主线 `e3e1db2`，独立审查/主线验证/任务清理完成**。生产入口使用应用层 `UiProvider` 配置公开 `MtoneProvider` 的 locale/density/tokens，不转发控件 API；CSS 从应用样式入口导入。基础表单组合仅在测试夹具中，不建立生产演示路由或通用包装库。证据见 [T17](../plans/knoted-v1/evidence/t17.md)。

T18 实施在同一 `UiProvider` 内注册公开 `ToastProvider`，不增加通知总线或统一弹层 API；**PR #46 已普通 merge 至主线 `4425ab9`，独立审查/主线验证/任务清理完成**。Modal/ConfirmDialog/DropdownMenu/ContextMenu/Popover 与通知组合只在测试夹具，测试服务 `?task=t18` 选择本卡，默认仍为 T17。真实 Chromium 验证焦点、异步失败重试与 Portal 字体/主题/层级，见 [T18 证据](../plans/knoted-v1/evidence/t18.md)。

T19 新增生产应用游标 reducer `apps/web/src/lib/cursor-state.ts`（无 UI），在测试夹具 `?task=t19` 直接消费公开 Tree/List/ListItem/Empty/Spinner/Alert/ConfirmDialog/Button；状态为 **in-progress / 实现就绪待主 Agent 审查集成**。Pagination 依赖 total，本卡改用 Button 组合上下页且不伪造总数；T17/T18 入口不变，见 [T19 证据](../plans/knoted-v1/evidence/t19.md)。

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

## 核查映射（不是运行验证）

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
- 任务记录：影响哪个场景、证据、上游需求路径、上游 Issue 编号/URL、恢复条件。产品卡仍未完成；Issue 用现有 `blocked` 标签并保留看板阶段，不新增标签定义。
- 上游 Issue 被关闭为不适用或转为已有实现时，同步更新本地需求记录状态与理由，不静默删除记录，也不据此直接解除 Knoted 消费验收。
- 技术等待不标 `human-required`，除非确有根规则要求的架构/权限决定。完成不依赖缺口的工作但不擅自改变串行任务策略。
- 用户返回 npm 发布版本后：核验版本实际可用、公开 API/类型/CSS → 更新精确依赖与锁文件 → 跑消费场景与本地门禁 → 按正常 PR 集成、主线验证及适用部署收尾。仅“master 已合入”“提示词已交付”不能解除全部验收。

## 验证与升级

T17—T19 测试在 `apps/web/tests/tasks/`，验证字段关联、键盘、焦点、禁用提交、空/错/加载与取消行为。浏览器实际焦点/Portal/CSS 验收使用已有 Playwright 入口。mtone 自己的内部单元测试属于上游，不复制进本仓库。T19 另验证受控 Tree 展开/选择事件分离、disabled 节点、无 total 的 next_cursor 游标往返与筛选重置。

升级必须审阅公开 API/CSS 变化并复跑受影响任务。遇到回归可回退上一已验证 npm 版本；无法满足则记录阻塞，不本地补一套通用 UI。T17 迁移与基础表单消费验证已集成；T18 已集成；T19 实现就绪但仍待审查集成；T02/T04 历史证据保留。

0.2.0 消费注意：菜单 disabled 项可由方向键聚焦以便发现，但 Enter/Space/点击均不执行；不要把 disabled 推断为一定跳过焦点。Modal 绕回时会短暂聚焦上游 inert guard，测试应等待焦点回到真实内容，不能自行补焦点管理。Toast 关闭按钮在 hover/focus 展开后暴露给可访问树。ConfirmDialog 拒绝后显示中文错误并保持打开可重试，pending 禁用确认/取消按钮；公开契约不保证 pending 时禁止 Esc 关闭，后续业务需按自身取消/请求生命周期语义处理，不将本卡测试误作不可关闭承诺。Tree 的 `value/onValueChange` 与 `expanded/onExpandedChange` 传递节点 id（非 label）；鼠标点展开箭头不触发选择，Enter/Space 才选择，disabled 节点 tabIndex -1 且不选择/展开。
