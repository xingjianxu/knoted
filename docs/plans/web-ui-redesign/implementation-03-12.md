# Web 改版 03–12 实施与集成

> **For agentic workers:** Use superpowers:subagent-driven-development and superpowers:verification-before-completion. Steps use checkboxes for tracking.

**目标** 按 [任务索引](README.md) 实施 03–11，完成 12 的可复现验收，合入后更新开发服务器。01 和 02 已分别经 #251 和 #252 合入。配色采用锁定发布版 `@bixlab/mtone@0.6.0` 的默认主题。

**依赖** `f269d01` 中的工作区布局、受保护路由和各域已有 API。遵循 [ADR 0002](../../decisions/0002-mtone-ui-ownership.md)，各域保持数据契约，文件预览另受 ADR 0006、0008、0009、0010 约束。共用浏览器用例、样式和应用外壳由集成负责人管理。用户已授权本轮完整交付，合并与开发部署由集成负责人执行。

| 卡 | 负责人和范围 | 先验证的回归与验收 |
| --- | --- | --- |
| 04 | 笔记工作区，`features/notes` 与 T27–32 | 目录/历史即时可达，保存及切换失败时同一编辑器保留草稿 |
| 05、06 | 项目与事项工作区，`features/projects`、`features/tasks`、两域路由与 T38–40 | 06 负责共用清单；深链首屏与返回保留上下文；生命周期和看板状态操作 |
| 09、10 | 搜索与问答工作区，两域视图、路由与 T55/T65 | 收起筛选和资料范围仍保留草稿；问答窄屏及搜索消息深链 |
| 03、07、08、11 | 集成负责人，概览、文件、索引任务、回收站及相应路由与 T41、T42、T49、T50、T52、T56 | 到期优先；文件详情首屏和预览资源；索引空页语义；恢复目的地与焦点 |
| 12 | 集成负责人，`tests/e2e`、证据与规范 | 桌面与窄屏跨页矩阵、键盘及滚动，记录 fixture 与真实服务边界 |

- [x] 核对基线、实施范围、mtone 发行版及授权；在主仓库 `.worktrees/` 创建隔离工作区。
- [x] 建立回归断言，分域实现并独立验证。选用 mtone 公开组合，保留请求、权限、草稿屏障及游标行为。
- [x] 集成并独立审查，保存 [验收记录](evidence/03-12.md)、前后截图和实际命令输出。
- [ ] 执行 Web lint、typecheck、test、build，运行 Playwright 以及 Rust fmt、test、clippy，保存实际输出。按默认授权提交、普通合并 PR，复验主线并清理本任务工作区。
- [ ] 用 `pnpm deploy:dev` 部署已合入 SHA，读回开发服务器版本、进程健康和 HTTPS 探针；任何真实浏览器或后台服务验收缺项附原因。
