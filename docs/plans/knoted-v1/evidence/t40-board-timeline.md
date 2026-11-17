# T40 看板与跟进时间线实施证据

- 日期：2026-11-17；Issue：[#124](https://github.com/xingjianxu/knoted/issues/124)。
- 基线：`bbdef3674d7b5cae1afd75e6da03a223669e6390`（T39 PR #123），Web 基线 223 项测试。
- Worktree：`/wrd/prj/knoted/.worktrees/knoted-t40`；分支：`feat/t40`。
- 范围：TaskBoard、ActivityTimeline、TasksPage 业务接线、T40 测试与本卡证据。依赖/锁、OpenAPI、后端、README 主状态均未改。
- 当前阶段：本卡实施和前端门禁完成；独立审查、PR/主线集成、dev 更新与真实 HTTPS 浏览器验收交主 Agent，尚未完整交付。

## UI 与业务行为

亲查正式安装 `@bixlab/mtone@0.4.0` 的 `dist/index.d.ts` 和 `llms.txt`：List 接收 HTMLAttributes、ListItem 接收 LiHTMLAttributes；Timeline 接收 items/title/content。固定四列和状态 API、原生拖放、Alt+左右方向键、CAS 与回滚属于 Knoted 业务组合；不实现通用拖拽库，不需要上游组件变更。符合 ADR 0002；ADR 0006 无需新增或替代决策。

- 四列固定为 `todo/in_progress/done/cancelled`，与列表共享项目/归档过滤及游标页。看板写入期间禁止切视图、筛选及翻页。
- 状态 PATCH 使用服务端版本；拖放/键盘共用操作，先乐观移动，409 或失败回原列。父事项/项目或事项本身不可写时禁止拖放/键盘写入，服务端继续保留最终权限/终态门槛。
- 成功 PATCH 先更新已确认 Query 缓存，再失效读取；避免后续读取失败把已提交状态误显示回原列。键盘跨列后焦点保留，后续普通刷新不抢焦点。
- 人工跟进支持新增、编辑、删除确认、显式查看修订；系统事件不提供编辑/删除。删除 204 后立即清除缓存正文并显示删除标记，即使后续读取失败也不回显旧正文；历史仅由修订 API 显式读取。
- 编辑开始时固定正文和 expected_version；普通 Query 失效不能抬升 dirty 草稿 CAS。409 保留草稿并禁止盲重试，显式重新读取当前页版本、显示服务端正文供核对后可重试。其他写失败保留输入；取消编辑不发写请求。
- 活动与修订都用不透明游标，读取期间禁止连续翻页。按任务 ID 重挂载，Query key 带任务 ID、分页游标及修订身份，旧任务迟到响应不串入新任务。

## RED → GREEN

日志均在 `/data/knoted-dev/tasks/t40/evidence/`。

- `red.log`：先建立 t40_1/t40_2/t40_3 与 CAS/重试，共 4 项真实失败，原因是基线缺少看板和时间线。该初次 shell 最后一条为 tail，未单独保存 Vitest 子命令退出码；日志明确 `4 failed`，不是导入/编译错误。
- `additional-red.log`：状态保存成功但后续列表读取失败时，事项错误回原列；有效失败，退出码 1，后修复缓存更新次序。
- `focus-red.log`：键盘移动后普通刷新抢走用户焦点；有效失败，退出码 1，后改为仅挂载/焦点目标变化时恢复焦点。
- `delete-refresh-red.log`：DELETE 成功但刷新失败时回显旧正文；有效失败，退出码 1，后在 204 后同步删除标记及清空缓存正文。
- 最终 `focused.log`：11 项通过，覆盖卡片三个指定场景和上述异常路径、显式重取 CAS 后保存、取消、修订翻页、归档只读、切任务迟到响应。

## 实际门禁输出

执行目录为当前 Worktree。完整日志保留在上述 evidence 路径。构建目录为 `/data/knoted-dev/tasks/t40/build/web`；任务自己的 Vite/Vitest 缓存已迁入 `/data/knoted-dev/tasks/t40/cache/`，Worktree node_modules 仅保留到该目录的链接，不修改共享主目录缓存。

```text
pnpm install --frozen-lockfile --offline
Done in 345ms using pnpm v12.9.0
exit 0

pnpm --dir apps/web exec vitest run tests/tasks/t40.test.tsx
Test Files  1 passed (1)
Tests       11 passed (11)
exit 0

pnpm --dir apps/web test
Test Files  15 passed (15)
Tests       234 passed (234)
exit 0

pnpm --dir apps/web typecheck
$ tsc --noEmit && tsc --noEmit -p tsconfig.node.json
exit 0

pnpm --dir apps/web lint
$ eslint src tests vite*.ts
exit 0

pnpm --dir apps/web build --outDir /data/knoted-dev/tasks/t40/build/web --emptyOutDir
✓ built
exit 0
```

测试环境有既存 JSDOM `Window.scrollTo()` 未实现提示，测试全部通过；生产构建有既存 chunk 超过 500 kB 提示，不影响 build exit 0。没有因此放宽检查。Rust 文件未改，本实施 Agent 按委派不重复 Rust 门禁；主 Agent 在集成前独立执行完整门禁。

## 交接与限制

- 实际接口：`PATCH /tasks/{taskId}`、`GET/POST /tasks/{taskId}/activities`、`PATCH/DELETE /tasks/{taskId}/activities/{activityId}`、`GET .../revisions`；均消费已生成的 DTO。
- 未修改宿主/服务/数据库/生产；未 push、创建 PR、合并或清理分支/Worktree。
- 浏览器原生 drag/drop、键盘、页面视觉和真实认证 HTTP 验收仍须主 Agent 在 dev 固定 SHA 验证；本证据不把 JSDOM 模拟拖放等同于浏览器验收。
