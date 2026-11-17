# T40 看板与跟进时间线实施证据

- 日期：2026-11-17；Issue：[#124](https://github.com/xingjianxu/knoted/issues/124)。
- 基线：`bbdef3674d7b5cae1afd75e6da03a223669e6390`（T39 PR #123），Web 基线 223 项测试。
- 实施 Worktree：`/wrd/prj/knoted/.worktrees/knoted-t40`；分支：`feat/t40`（均已安全清理）。
- 范围：TaskBoard、ActivityTimeline、TasksPage 业务接线、T40 测试与本卡证据。依赖/锁、OpenAPI、后端、README 主状态均未改。
- 当前阶段：done（PR [#136](https://github.com/xingjianxu/knoted/pull/136)，主线 `aee7c2e`，dev `aee7c2e`；审查、实例验收和任务工作区清理完成）。

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

## 实施子 Agent 交接时的限制

以下保留集成前的历史记录，最终交付见下节。

- 实际接口：`PATCH /tasks/{taskId}`、`GET/POST /tasks/{taskId}/activities`、`PATCH/DELETE /tasks/{taskId}/activities/{activityId}`、`GET .../revisions`；均消费已生成的 DTO。
- 未修改宿主/服务/数据库/生产；未 push、创建 PR、合并或清理分支/Worktree。
- 浏览器原生 drag/drop、键盘、页面视觉和真实认证 HTTP 验收仍须主 Agent 在 dev 固定 SHA 验证；本证据不把 JSDOM 模拟拖放等同于浏览器验收。


## 主 Agent 集成、实例与收尾

- PR [#136](https://github.com/xingjianxu/knoted/pull/136) 普通 merge，主线 `aee7c2eb5f2ff40805b069d24d6a8b57665acf8d`，包含实施 `54a09d9`。Issue #124自动关闭。
- fresh独立review_t40结论 Spec PASS / Quality APPROVE，无Critical/Important/Minor。
- 主Agent独立Rust门禁 fmt/test/clippy全部exit0，255 passed / 1既有ignored；Rust源码与root `bfeb8ab`一致性由git diff --exit-code确认。独立Web typecheck/lint/test（15文件234 passed）/build全部exit0（`/data/knoted-dev/tasks/t40/logs/parent-*.log`）。
- 主线受影响Web typecheck/lint/test234/build全部exit0（`main-*.log`）；Rust与上述已验证源码一致，无新增Rust改动，未无故重复完整Rust门禁。
- 已提交主线archive构建固定产物 `/data/knoted-dev/runtime/aee7c2eb5f2ff40805b069d24d6a8b57665acf8d`，完整SHA256SUMS校验通过（`build-artifact.log`）。
- 无迁移，schema保持11。停本实例API/Worker并确认旧PID退出后，一致备份 `/data/knoted-dev/service/backups/20261117T152841Z-t40-`（PG/assets/摘要及pg_restore目录校验），原子切current并pcwrap down/up；HTTPS ready200，真实API/Worker路径固定到该SHA，共享PG/Caddy/Strata PID前后一致（`deploy.log`、`runtime-{before,after}.json`）。
- 真实HTTPS Chromium无路由拦截：`live-browser.log` exit0；native_drag_failure_rollback、native_drag_success、keyboard_status_and_focus、manual_create_system_readonly、manual_edit_and_revisions、activity_draft_cas_and_retry、deleted_marker_history_only、switch_no_leak、passkey_count_stable均true，cleanup_ok=true。包含真实父完成门槛409与原列回滚，Alt方向键不触发浏览器返回且焦点随卡片移动，普通刷新后的跟进草稿仍受CAS保护，删除正文仅在显式历史中展示。
- 首次浏览器运行 `timeline-create` 阶段因测试选择器同时匹配输入textarea和已发布p而strict mode失败（`live-browser-first.log`）；边界证据确认POST成功和正文已显示，限定正文段落选择器后同脚本通过。未改产品代码或跳过验收。
- 夹具仅通过API创建/软删事项和跟进，临时会话已撤销。最终读回活跃事项0、未撤销验收会话0、Passkey1（前后不变），未做硬件Passkey ceremony。
- `.worktrees/knoted-t40`干净且feat/t40为主线祖先后安全移除，分支-d删除；任务缓存留在其授权/data目录，他人worktree保留。后续纯文档提交不改变dev运行SHA。
