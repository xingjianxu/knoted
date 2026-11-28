# 开发任务索引

每张任务卡对应一个 GitHub Issue。点击编号阅读交付目标、范围落点、实施步骤和验收条件，点击卡片中的 Issue 查看实时进度、阻塞与验收结论。直接前置均完成后即可领取；分支和 worktree 规则见 [AGENTS.md](../../AGENTS.md)。

历史阶段规划（已完成的 v1 首版与全站 Web UI 重设计）保留在 [docs/plans/](../plans/README.md) 作为归档参考；后续新任务均按 GitHub Issue 编号在本目录登记。

## 工作流与维护

| 任务卡 | Type | 交付目标 | 直接前置 |
|---|---|---|---|
| [#138](138.md) | Bug | 在事务回滚待执行时回收发生 panic 的测试数据库夹具 | 无 |
| [#149](149.md) | Task | 固定文件重关联与标签、关联写入的事务顺序 | 无 |
| [#153](153.md) | Task | Worker 测试正常结束或异常展开时回收临时文件 | 无 |
| [#163](163.md) | Task | 限制共享 API 客户端解析非 2xx 错误体的字节数 | 无 |
| [#270](270.md) | Task | 参考 dek 重构基于 GitHub Issue 的开发协作流程 | 无 |
| [#280](280.md) | Task | 隔离程序测试使用的目录和搜索服务 | 无 |
| [#314](314.md) | Task | 移除文件全文索引与 RAG | 无 |

## 前端与 UI

| 任务卡 | Type | 交付目标 | 直接前置 |
|---|---|---|---|
| [#169](169.md) | Bug | 恢复下载期间关闭文件预览后的键盘焦点 | 无 |
| [#170](170.md) | Bug | 为过时预览快照提供关闭重开指引 | 无 |
| [#171](171.md) | Bug | 在请求原件前拒绝图片预览白名单外的后缀 | 无 |
| [#265](265.md) | Bug | 匿名直达笔记页面时进入登录 | 无 |
| [#272](272.md) | Task | 升级并迁移前端至 `@bixlab/mtone@0.13.0` 并全面采用 mtone 组件 | 无 |
| [#274](274.md) | Bug | 升级 mtone 0.13.1 修复工作区顶栏与侧栏品牌区错位 | [xingjianxu/mtone#68](https://github.com/xingjianxu/mtone/issues/68)（已完成） |
| [#276](276.md) | Feature | 布置工作区 AdminLayout 顶栏导航和操作 | 无 |
| [#278](278.md) | Feature | 调整项目 npm 源为官方源、升级 mtone 0.13.2 并用 Splitter 重构笔记页面 | 无 |
| [#286](286.md) | Task | 升级 `@bixlab/mtone` 至 `0.15.2` 并迁移编辑器至 `@bixlab/mtone-blockeditor@0.2.0` | 无 |
| [#288](288.md) | Task | 新增 ADR 0012 记录采用 `@bixlab/mtone-blockeditor` 部分替代 ADR 0002 第 4 条 | [#286](286.md)（已完成） |
| [#290](290.md) | Task | 隐藏笔记编辑页标题输入框边框 | 无 |
| [#292](292.md) | Feature | 笔记侧栏目录采用 mtone Tree 节点操作并补充悬停操作菜单 | 无 |
| [#294](294.md) | Task | 升级 `@bixlab/mtone` 至 `0.15.3` 与 `@bixlab/mtone-blockeditor` 至 `0.2.1` | 无 |
| [#296](296.md) | Feature | 笔记编辑器填满正文区 | 无 |
| [#298](298.md) | Feature | 升级 mtone 并用 Container 布局笔记编辑区 | 无 |
| [#300](300.md) | Feature | 笔记侧栏支持拖动同级排序 | 无 |
| [#302](302.md) | Task | 优化笔记编辑页标题间距与主体内容区排版 | 无 |
| [#303](303.md) | Task | 升级 `@bixlab/mtone` 至 `0.15.5` 与 `@bixlab/mtone-blockeditor` 至 `0.2.2` | 无 |
| [#306](306.md) | Bug | 修复笔记目录拖放抖动并支持嵌套移动 | 无 |
| [#308](308.md) | Bug | 修复笔记表格无法保存 | [ADR 0013](../decisions/0013-note-table-persistence.md)（已确认） |
| [#310](310.md) | Task | 升级 mtone 0.15.6 并复核笔记侧栏拖动句柄控制 | 无 |
| [#312](312.md) | Task | 给笔记页 Splitter 加上 mtone 外框 | 无 |
| [#326](326.md) | Feature | 升级 mtone 1.0.0 与 blockeditor 1.1.0 并用 AppLayout 与 Topbar 重构笔记页面 | 无 |
| [#328](328.md) | Feature | 升级 mtone 至 1.2.0，全站采用 AppLayout 并用两栏 PageLayout 重构笔记页面 | 无 |

## 开发部署

| 任务卡 | Type | 交付目标 | 直接前置 |
|---|---|---|---|
| [#186](186.md) | Task | 由宿主持久托管开发实例的 Meilisearch | 无 |
| [#246](246.md) | Task | 手动启动开发实例时恢复已保存的搜索和 OnlyOffice 配置 | 无 |
| [#281](281.md) | Task | 部署验证通过后只保留当前运行产物 | 无 |
| [#284](284.md) | Task | 将开发服务器更新纳入 Agent 任务收尾 | 无 |
