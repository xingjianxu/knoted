# T36 跟进时间线与历史：实现与验证

任务卡：[T36](../tasks/T36.md)。本页记录实现范围、判定与真实运行结果；未通过的项不写“通过”。

## 范围与文件

| 文件 | 变化 |
|---|---|
| `crates/knoted-core/src/tasks/activities.rs`（新增） | `append`/`edit`/`delete`/`list`/`list_revisions` 与系统事件追加 `append_system`，SQL 与事务同文件 |
| `crates/knoted-core/src/tasks/service.rs` | 事项状态变更事务内追加系统事件（4 行 + 注释） |
| `crates/knoted-core/src/tasks/mod.rs` | 注册 `activities` 模块 |
| `crates/knoted-api/src/routes/activities.rs`（新增） | 契约中 5 个操作的 HTTP 路由，会话与 CSRF 守卫与事项一致 |
| `crates/knoted-api/src/routes/mod.rs` | 合并 `activities::router` |
| `migrations/0011_t36_activities.sql`（新增） | `activities`、`activity_revisions` |
| `crates/knoted-api/tests/t36.rs`（新增） | 6 个真实 HTTP + PostgreSQL 场景 |
| `crates/knoted-api/tests/t34.rs` | 迁移回放测试先删依赖 `tasks` 的新表（见下） |

`api/openapi.yaml` 与 `api/intelligence.openapi.yaml` 未改动：`/tasks/{taskId}/activities` 的 5 个操作与 `Activity`/`ActivityCreate`/`ActivityUpdate`/`ActivityPage`/`ActivityRevision`/`ActivityRevisionPage` 已由 T03 定稿，本卡按既有契约实现，因此生成客户端无需重新生成（`packages/api-client/src/generated` 无差异）。

## 契约与判定

- **人工备注可编辑留历史**：编辑或删除前先做 `version` CAS，成功后把被替换的内容写入 `activity_revisions`（`version` 为被替换的版本，`created_at` 为该版本最后一次写入时间）。`UNIQUE(activity_id,version)` 使同一历史版本不可能重复。
- **删除只留标记**：`deleted_at` 置位、`version+1`，行内容保留但不在列表出现；删除前的正文也写入历史，所以“旧内容仅历史查看”对已删除记录同样成立。
- **系统事件不可改写**：状态变更（todo/in_progress/done/cancelled 之间）在 `service::update_command` 的同一事务内追加 `author_kind='system'` 事件，正文形如 `status: todo -> in_progress`（状态名复用 DTO 序列化，不另立映射）。PATCH/DELETE 系统事件返回 409 且不写入任何修订；CAS 失败的事项更新会回滚整笔事务，不留下事件（`t36_1`）。
- **删除撤出检索投影**：当前尚无检索投影（T53/T57 未实施），本卡不自造 `outbox` 主题；删除通过“列表排除 + 历史保留”满足契约的可见性要求，投影落地时应读取 `deleted_at` 与历史表，而不是依赖新事件类型。
- **排序与游标**：列表与修订均按 `(created_at,id)` 倒序（与 `note_snapshots` 历史列表一致），游标为 `base64url` 版本化 JSON，绑定 `scope`+`task_id`(+`activity_id`)，跨任务/跨集合复用返回 400。
- **写入门槛**：与事项写一致——删除的事项 404；归档事项、归档或删除的项目 409（归档 API 属 T37，本卡用 SQL 置位验证该分支）。
- **锁顺序**：先取 `33001/projects` 关系事务锁，再 `tasks` 行 `FOR UPDATE`，与 T33—T35 既有顺序一致。

## 验证

在 `.worktrees/knoted-t36`（基线主线 `ccf1fa6`）实际运行，日志保存在 `/data/knoted-dev/tasks/t36/logs/`。

| 命令 | 结果 |
|---|---|
| `cargo test --workspace --locked t36_` | exit 0；`t36_` 用例 6 passed / 0 failed |
| `cargo test --workspace --locked` | exit 0；235 passed / 0 failed / 1 既有 ignored（35 个测试二进制；基线 `ccf1fa6` 为 229 passed，本卡新增 6） |
| `cargo fmt --check` | exit 0 |
| `cargo clippy --locked --all-targets -- -D warnings` | exit 0 |

场景覆盖（`crates/knoted-api/tests/t36.rs`，真实 axum 路由 + 真实 PostgreSQL schema）：

| 用例 | 覆盖 |
|---|---|
| `t36_1` | 系统事件在状态变更事务内追加；PATCH/DELETE 均 409 且记录不变；失败的 CAS 事项更新不追加事件；跨事项不可见；再次终态变更各追加一次 |
| `t36_2` | 人工备注编辑后旧正文可从 revisions 查到；删除后列表为空、行仅留标记、最终正文仍只在历史；已删除记录不能再改 |
| `t36_3` | CAS 冲突、空正文、NUL、超长正文、越界 `expected_version`、缺字段、跨事项路径、未知 id、已删事项均不产生孤立修订；重放旧版本不新增修订 |
| `t36_4` | 倒序游标分页、跨任务/跨集合游标 400、`limit` 0 与 101 为 400、未知游标 400、不存在事项 404、匿名 401、无 CSRF 写 403 |
| `t36_5` | 归档事项可读不可写；归档项目与已删除项目拒绝新备注，恢复可写 |
| `t36_6` | 两个同版本并发 PATCH 在真实后端等待后一个 200、一个 409，历史恰好 1 行，无 503 |

TDD 记录：先在空实现上运行 `cargo test --locked --test t36`，4 个用例因路由不存在得到 404（RED），实现后转绿；随后补齐 `t36_5`、`t36_6`（后者首次运行即通过，它验证的是锁顺序而非新代码路径）。实施过程中发现并修复的真实缺陷：`version` 列误用 `integer` 导致 `i64` 解码失败（所有请求 503），改为 `bigint` 与 `projects`/`tasks` 一致。

## 交叉卡的改动与理由

- `crates/knoted-api/tests/t34.rs`：`t34_migration_empty_upgrade_and_unknown_rows_preserved` 通过 `DROP TABLE tasks` 回放到 T33 形态。`activities` 以 `task_id` 外键引用 `tasks` 后，该语句会被 PostgreSQL 拒绝（`2BP01`）。改为显式列出依赖表 `DROP TABLE IF EXISTS activity_revisions, activities, tasks`，不隐藏依赖、不用 `CASCADE` 削弱断言；断言内容未变。外键保留，不用去掉约束来迁就测试。
- `crates/knoted-core/src/tasks/service.rs`：只在“状态实际变化且更新成功”的同一事务里追加系统事件（任务卡步骤 4 要求事务追加），未改动既有 DTO、版本或错误码。

## 已知限制

- 归档/恢复/回收站 API 属 T37，本卡只按 SQL 置位验证拒绝分支；届时 T37 复用同一 `appended only` 约定即可（归档不是状态变更，不需要额外事件）。
- 检索投影与 RAG 片段在 T53/T57，本卡不写 `source_documents`，也不新增 outbox 主题。
- 前端列表/时间线页面属 T40；本卡只交付 API 与契约实现，未新增 Web 组件或类型。
