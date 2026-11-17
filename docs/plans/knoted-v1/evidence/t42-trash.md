# T42 统一回收站实施证据

状态：done（PR [#141](https://github.com/xingjianxu/knoted/pull/141)，主线与 dev `d7c74ec982ea980b92a37612ba66a2e5923b7808`）。下列实施阶段记录保留历史，最终集成验收见文末。

- Issue：[#127](https://github.com/xingjianxu/knoted/issues/127)。
- 基线：`770263e`；任务分支 `feat/t42`；Worktree `/wrd/prj/knoted/.worktrees/knoted-t42`。
- 输入：[T42](../tasks/T42.md)、[ADR 0006](../../../decisions/0006-file-pipeline-and-aggregate-contracts.md)、主 Agent brief 与 `/data/knoted-dev/tasks/t39-t50/reports/t42-readonly-research.md`。
- 计划：依次完成只读聚合/资格、HTTP 注册、Web 恢复动作与真实路由、回归与门禁，交主 Agent 集成。复用 T24/T37 restore，消费 T41 基线及主 Agent 已生成的契约；不改锁文件、迁移或共享契约。

## 验收清单

- [x] API 真实 RED：缺失 `/trash` 返回 404 而期望 200，2 项失败；日志 `api-red.log`；当时 tail 包装 shell 退出 0，未独立捕获 cargo 子进程退出码，日志明确 2 failed，不把包装退出码算 GREEN。
- [x] Web 真实 router RED：访问 `/trash` 无回收站标题，1 项失败；日志 `web-red.log`；当时 tail 包装 shell 退出 0，未独立捕获 vitest 子进程退出码，日志明确 1 failed，不把包装退出码算 GREEN。追加行测试的首次 import 缺失是收集错误，不把它计为行为 RED。
- [x] Core SQL 只列实际实体表软删除行；排序 `deleted_at DESC, entity_type ASC, id ASC`，完整 tuple seek + limit+1；游标绑定 version/scope/order/filter（含无筛选）。
- [x] `REPEATABLE READ READ ONLY` 一致快照；笔记父链谓词只增加读取模式，命令保留原锁；事项复用关系与完整终态谓词。无 restore 探测、无 GET 写锁、无迁移。
- [x] 30 天只提示清理资格；40 天软删除对象仍能恢复；只归档对象与 registry-only 身份排除。
- [x] 前端类型筛选/前后翻页/加载期禁用，逐行 `type:id` 状态；笔记显式根恢复，确认/取消；各领域 CAS restore 路由。
- [x] 写成功立即移除全部 trash 缓存中的本行，再刷新；其它行保持挂载/位置选择/错误。读取失败与写失败分开反馈。
- [x] 真实 `/trash` 路由、启用侧栏、safeReturnTo 注册；本卡 router 测试 stub scrollTo，避免新增 jsdom 噪声。
- [x] 主 Agent 独立 spec/quality 审查、PR 合入主线、主线门禁、dev 浏览器验收及清理。

## 验证

日志目录：`/data/knoted-dev/tasks/t42/logs/`。Rust 共享 target：`/data/knoted-dev/tasks/t39-t50/build/rust-target`；TestDb 默认只访问 `knoted_test` 隔离 schema，无 dev 数据修改。

| 命令 | 结果 | 日志 |
| --- | --- | --- |
| `cargo test -p knoted-core -p knoted-api --test t42` | 0；core 3、API 3，全部通过 | `rust-t42-final.log` |
| `cd apps/web && pnpm exec vitest run tests/tasks/t42.test.tsx` | 0；6/6，通过且无 scrollTo 噪声 | `web-green.log` |
| `cargo fmt --check` | 0 | `rust-fmt.log` |
| `cargo test --locked` | 0；267 passed、0 failed、1 ignored | `rust-test-final.log` |
| `cargo clippy --locked --all-targets -- -D warnings` | 0 | `rust-clippy-final.log` |
| `cd apps/web && pnpm test` | 0；17 文件、248/248 | `web-test.log` |
| `cd apps/web && pnpm lint` | 0 | `web-lint.log` |
| `cd apps/web && pnpm typecheck` | 0 | `web-typecheck.log` |
| `cd apps/web && pnpm build` | 0；既有编辑器大 chunk 警告 | `web-build.log` |
| `pnpm api:check` | 0；83 operations、20 examples，10 invalid rejected | `api-check.log` |

中间发现：初版复用 note `find_any` 自带 `FOR UPDATE`，只读事务实际报 25006（`api-debug.log`）；修为独立只读查询且保留写命令锁，现已通过持有 notes 写锁时读取的 PostgreSQL 验证。首轮 core/full suite 有测试夹具误用 registry 列名 `entity_id/deleted_at`，按实际 schema 修正为 `id`；第二轮 full `cargo test` 267 passed、0 failed、1 ignored。最终门禁以 `*-final.log` 与对应 `.exit` 为准。

全 Web suite 仍有其他既有 router 测试的 3 条 jsdom scrollTo 提示，本卡 6 项输出干净。全 Rust 1 项 ignored 为既有 auth middleware 文档示例。实施阶段尚未执行 dev 部署；实施 Agent 按授权未 push/PR/merge、未修改运行实例、未清理其它任务。这是交接时历史记录，最终结果见下。

## UI 消费与 dev 验收入口

实际锁定 `@bixlab/mtone@0.4.0`；已核查发布包公开声明，组合 List/ListItem、Select、Dialog、Button、Alert、Spinner、Empty 实现筛选、列表、确认与反馈，无通用能力缺口，无上游 Issue。

- 受保护 URL：`/trash`；main 名称“回收站”，heading“回收站”，侧栏 button“回收站”。
- 列表名“回收对象”；每行 region“回收对象 {title}”；类型筛选 combobox“对象类型”，选项“全部/笔记/项目/事项”。
- 笔记位置 combobox“恢复位置 {title}”，选项“原位置/根目录”；恢复 button“恢复 {title}”。
- 确认 dialog“恢复 {title}”；button“确认恢复/取消”；成功 status“已恢复 {title}”；刷新 button“刷新列表”。
- 翻页 button“上一页/下一页”；当前受限原因在对应 region 内显示。删除时间按上海时区展示。

## 主 Agent 最终集成与实例验收

- 契约提交 `770263e7e9e531a6ed0806782547529dbba5f55d`、实施提交 `073d4df60e61a09d0bc8d974cb9cab735260164e`；独立 reviewer `review_t42` 给出 Spec PASS / Quality APPROVE，无新增缺陷。报告 `/data/knoted-dev/tasks/t39-t50/reviews/t42-task-review.md`。
- PR [#141](https://github.com/xingjianxu/knoted/pull/141) 普通 merge，实际主线/dev SHA `d7c74ec982ea980b92a37612ba66a2e5923b7808`；Issue #127 通过关闭关系处理。
- 主 Agent 在实现最终字节和主线分别执行完整 Rust/Web/API 门禁。两轮均 exit 0：Rust 267 passed / 1 ignored；Web 17 文件 / 248 passed；fmt、clippy all-targets -D warnings、typecheck、lint、build、api:check（83 operations / 20 examples / 10 invalid rejected）、api:verify。日志 `/data/knoted-dev/tasks/t42/logs/{parent,main}-*.log`。既有文档忽略、其他 router scrollTo 提示和编辑器 chunk 提示如实保留，本卡未新增这些警告。
- `tools/dev/build-artifact.sh` 仅用已提交主线构建固定产物，完整 SHA256SUMS 校验通过；日志 `build-artifact.log`。停本实例 API/Worker 写进程后备份 PG/assets：`/data/knoted-dev/service/backups/20261117T164835Z-t42-`，摘要与 pg_restore --list 校验通过；本卡 schema 仍 0011。
- 原子切换 current 并通过 pcwrap down/up 更新 dev；HTTPS ready 200，实际 API/Worker exe 均来自上述固定 SHA，PostgreSQL/Caddy/Strata MainPID 未变。日志 `deploy.log`、`runtime-before.json`、`runtime-after.json`。
- 真实 Chromium/HTTPS 无 HTTP mock、无 HAR/trace/storageState/凭据输出。脚本 `/data/knoted-dev/tasks/t42/live-browser.mjs`，日志 `live-browser.log`，exit 0：

```json
{"result":"passed","checks":{"auth_missing_parent_reason_archive_exclusion":true,"cursor_bound_to_filter_and_invalid_filter_rejected":true,"enabled_trash_navigation_and_explicit_root_choice":true,"cas_conflict_visible_and_other_row_choice_preserved_on_refresh":true,"project_task_note_restore_domain_actions_and_root_parent_readback":true,"other_restore_preserves_blocked_row_choice":true,"entity_filter_and_parent_restore":true,"own_fixture_soft_delete_cleanup":true,"passkey_count_stable":true},"cleanup_ok":true}
```

- 浏览器仅创建/软删除本次专用样本，不做物理清理；回读当前未删除 notes/projects/tasks 均 0，本次验收未撤销 session 为 0，Passkey 仍 1。
- 2026-11-18 主 Agent 确认任务 Worktree tracked/untracked clean、feat/t42 为 master 祖先后，安全移除 `.worktrees/knoted-t42` 与本地 `feat/t42`；保留协调 Worktree 及他人 `feat-https-shared-caddy`。
