# T36 跟进时间线与历史：实现与验证

任务卡：[T36](../tasks/T36.md)。本页记录实现范围、判定与真实运行结果；未通过的项不写“通过”。

当前：PR [#110](https://github.com/xingjianxu/knoted/pull/110) 已普通 merge 至主线 `f26af66`，主线门禁与测试实例 HTTPS 验收通过，任务工作区/分支已清理。

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
- **删除只留标记**：删除时先把当前正文写入历史，再把行置为“标记”——`deleted_at` 置位、`body` 清空、`version+1`。列表**包含**标记行（`deleted_at` 非空、`body` 为空字符串），所以时间线能显示“该跟进已删除”而不泄露内容；正文只在 `revisions` 可读，符合“旧内容仅历史查看，不再检索”。行级约束 `CHECK (deleted_at IS NOT NULL OR char_length(body) >= 1)` 保证活记录必有正文。
- **系统事件不可改写**：状态变更（todo/in_progress/done/cancelled 之间）在 `service::update_command` 的同一事务内追加 `author_kind='system'` 事件，正文形如 `status: todo -> in_progress`（状态名复用 DTO 序列化，不另立映射）。PATCH/DELETE 系统事件返回 409 且不写入任何修订；CAS 失败的事项更新会回滚整笔事务，不留下事件（`t36_1`）。
- **删除撤出检索投影**：当前尚无检索投影（T53/T57 未实施），本卡不自造 `outbox` 主题；删除通过“列表排除 + 历史保留”满足契约的可见性要求，投影落地时应读取 `deleted_at` 与历史表，而不是依赖新事件类型。
- **排序与游标**：列表与修订均按 `(created_at,id)` 倒序（与 `note_snapshots` 历史列表一致），游标为 `base64url` 版本化 JSON，绑定 `scope`+`task_id`(+`activity_id`)，跨任务/跨集合复用返回 400。
- **写入门槛**：与事项写一致——删除的事项 404；归档事项、归档或删除的项目 409（归档 API 属 T37，本卡用 SQL 置位验证该分支）。这类“状态拒绝”与系统事件不可改写一样返回 `409 version_conflict` 且**不带** `current_version`：调用方的 `expected_version` 指备注本身，不是事项或项目版本，带上外来版本会让前端把它当成可重试的版本冲突。只有备注自身版本不匹配才返回 `current_version`。
- **锁顺序**：先取 `33001/projects` 关系事务锁，再 `tasks` 行 `FOR UPDATE`，与 T33—T35 既有顺序一致。

## 验证

在 `.worktrees/knoted-t36`（基线主线 `ccf1fa6`）实际运行，日志保存在 `/data/knoted-dev/tasks/t36/logs/`。

| 命令 | 结果 |
|---|---|
| `cargo test --workspace --locked t36_` | exit 0；`t36_` 用例 7 passed / 0 failed |
| `cargo test --workspace --locked` | exit 0；236 passed / 0 failed / 1 既有 ignored（35 个测试二进制；基线 `ccf1fa6` 为 229 passed，本卡新增 7） |
| `cargo fmt --check` | exit 0 |
| `cargo clippy --locked --all-targets -- -D warnings` | exit 0 |
| `pnpm api:check`（worktree 契约） | `api:check ok — 83 operations, 16 examples (8 invalid rejected)` |
| `pnpm api:verify` 等价检查 | 生成客户端仅 `createTaskActivity` 新增 `404: NotFound`，其余无差异 |

场景覆盖（`crates/knoted-api/tests/t36.rs`，真实 axum 路由 + 真实 PostgreSQL schema）：

| 用例 | 覆盖 |
|---|---|
| `t36_1` | 系统事件在状态变更事务内追加；PATCH/DELETE 均 409（无 `current_version`）且记录不变；失败的 CAS 事项更新不追加事件；跨事项不可见；再次终态变更各追加一次 |
| `t36_2` | 人工备注编辑后旧正文可从 revisions 查到；删除后列表保留标记（`deleted_at` 非空、`body` 为空、`version` 3）、行正文清空、最终正文仍只在历史；已删除记录不能再改 |
| `t36_3` | CAS 冲突、空正文、NUL、超长正文、越界 `expected_version`、缺字段、跨事项路径、未知 id、已删事项均不产生孤立修订；重放旧版本不新增修订 |
| `t36_4` | 倒序游标分页、跨任务/跨活动/跨集合游标 400、`limit` 0 与 101 为 400、未知游标 400、不存在事项 404、跨任务 DELETE 404、创建时正文校验 422、匿名 401、无 CSRF 写 403 |
| `t36_5` | 归档事项可读不可写；归档项目与已删除项目拒绝新备注（无 `current_version`），恢复可写；被拒写入不新增行 |
| `t36_6` | 两个同版本并发 PATCH 在真实后端等待后一个 200、一个 409，历史恰好 1 行，无 503（证明的是关系锁/行锁顺序，不是行 CAS 本身） |
| `t36_7` | 注入 `BEFORE INSERT` 触发器让追加失败：事项 PATCH 返回 503，`tasks.status/version` 未变、无事件行，证明事件与事项更新同事务；改名/说明/优先级更新不追加事件，只有状态变更追加一条 |

TDD 记录：先在空实现上运行 `cargo test --locked --test t36`，4 个用例因路由不存在得到 404（RED），实现后转绿；随后补齐 `t36_5`、`t36_6`（后者首次运行即通过，它验证的是锁顺序而非新代码路径）。实施过程中发现并修复的真实缺陷：`version` 列误用 `integer` 导致 `i64` 解码失败（所有请求 503），改为 `bigint` 与 `projects`/`tasks` 一致。

独立审查（PASS）提出的 Important 与选定 Minor 已按同样方式修复并各自先看到 RED：墓碑对客户端不可见导致 `Activity.deleted_at`（契约必填字段）恒为 null、T40 无法显示“已删除标记”；状态类 409 误带外来 `current_version`；`POST` 缺 `404` 声明。审查记录的其余 Minor 与“declined to judge”项作为判定保留在下方。

## 交叉卡的改动与理由

- `crates/knoted-api/tests/t34.rs`：`t34_migration_empty_upgrade_and_unknown_rows_preserved` 通过 `DROP TABLE tasks` 回放到 T33 形态。`activities` 以 `task_id` 外键引用 `tasks` 后，该语句会被 PostgreSQL 拒绝（`2BP01`）。改为显式列出依赖表 `DROP TABLE IF EXISTS activity_revisions, activities, tasks`，不隐藏依赖、不用 `CASCADE` 削弱断言；断言内容未变。外键保留，不用去掉约束来迁就测试。
- `crates/knoted-core/src/tasks/service.rs`：只在“状态实际变化且更新成功”的同一事务里追加系统事件（任务卡步骤 4 要求事务追加），未改动既有 DTO、版本或错误码。

## 判定（审查未定项）

- **跟进写入不看父事项状态**：只校验本事项存活与所属项目可写。父事项的状态不阻止给存活子事项追加跟进——跟进不改变子事项状态，若 T37 归档语义要求更严，再在同一处补父检查（成本：一处查询 + 一条用例）。
- **系统事件正文**：固定为 `status: <旧> -> <新>`（英文枚举名，取自 DTO 序列化）。`Activity` 契约没有结构化事件字段，本卡不自造扩展；T40 若要中文化需先改契约。
- **DB 级不可改写强度**：数据库约束覆盖“系统事件不能带删除标记”，编辑/删除的 `author_kind='manual'` 条件在 service 的 UPDATE 里；仓库既有迁移不使用触发器，本卡不引入例外。
- **未写 `entity_registry`**：跟进记录目前不是可打标签/关联的实体；若 T57 的独立片段沿用 registry 外键约定，需在 T57 补枚举值与回填迁移，不在本卡自造。
- **未写 outbox**：投影协议属 T53/T57。对下游的前置提醒：T53/T57 落地时必须排除 `deleted_at` 非空的标记行，并对已有数据做一次全表回填/对账，而不是假设历史事件已经存在。

## 主线与实例

| 项目 | 结果 |
|---|---|
| 主线 SHA | `f26af666c78eb3e302276a22cf9458255ab13faf`（PR #110 普通 merge） |
| 主线门禁 | `cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --workspace --locked` 均 exit0；236 passed / 0 failed / 1 既有 ignored（日志 `/data/knoted-dev/tasks/t36/logs/main-*.log`） |
| 固定产物 | `/data/knoted-dev/runtime/f26af66…faf`（`tools/dev/build-artifact.sh` exit0，`SHA256SUMS` 校验通过，含 `migrations/0011`） |
| 一致备份 | `/data/knoted-dev/service/backups/20261117T112156Z-t36`（停 API/Worker 后 `pg_dump` + assets，`SHA256SUMS` 自检通过） |
| 迁移 | 实例 `_sqlx_migrations` 升至 11（`t36 activities`），由编排 `migrate` 进程应用 |
| 运行核对 | `current` → 固定产物；API/Worker `/proc/<pid>/exe` 均指向该产物的 `bin/`（`logs/t36-runtime-readback.txt`） |
| HTTPS 验收 | `live-smoke.py` exit0：真实 `https://wlab.132140.xyz:15601` 完成备注创建/编辑/删除标记与历史、系统事件 409、并发与游标分页、匿名 401/CSRF 403；临时会话撤销、Passkey 计数不变、fixture 软删（日志 `logs/t36-live-smoke.log`） |

首次 `deploy.py` 因 `pnpm pc down` 在任务目录缺少工作区而失败（API/Worker 已由备份脚本停止、`current` 已切换）；修正为 `pnpm --dir /wrd/prj/knoted pc` 后跳过重复备份完成 `down`/`up`，备份仍是停写后的一致快照。首轮冒烟未通过的两处是脚本自身口径（`passkeys` 表名应为 `passkey_credentials`、分页期望集漏掉删除标记），均按真实 schema/行为修正后通过，不属产品缺陷。

## 已知限制

- 归档/恢复/回收站 API 属 T37，本卡只按 SQL 置位验证拒绝分支；届时 T37 复用同一 `appended only` 约定即可（归档不是状态变更，不需要额外事件）。
- 检索投影与 RAG 片段在 T53/T57，本卡不写 `source_documents`，也不新增 outbox 主题。
- 前端列表/时间线页面属 T40；本卡只交付 API 与契约实现，未新增 Web 组件或类型。
