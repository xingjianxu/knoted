# T35 事项终态与上海日期证据

- Issue：[#92](https://github.com/xingjianxu/knoted/issues/92)；分支 `feat/t35`；基线 `aa58c1ab0e6986fb0a7545d66843e826aa363fb7`（T34 PR #98）。
- 范围：`tasks::{transitions,dates}` 和必要 service/repository 注册，真实 API/PostgreSQL 测试；复用 T34 唯一 DTO/枚举，未新增 HTTP 字段/路由/迁移。
- 当前：implemented，本地门禁全部通过；主 Agent 负责独立审查、PR/主线集成、dev 更新及清理，未提前宣称完整生命周期完成。

## 实现与不变量

`service::update` 实际进入公开 `transitions::transition_task(ctx, UpdateTask)`，再执行唯一事务命令。create 和 PATCH 在同一个调用方事务内复用 `validate_final_state`；状态、标题、日期、父/项目先合并再检查，包含无变化 PATCH。done 父的所有未删除直属子必须为 done/cancelled；归档活子仍计入，删除子忽略。显式重开父、终态化子、解除关系可按最终候选状态修复历史不一致；不自动修改其他实体状态/版本。

沿用关系锁 33001：先获取事务 advisory lock，再发现目标、旧/新父、目标及父的全部直属子和涉及项目，跨 tasks/projects 按全局 UUID 顺序锁行，然后重读、检查 CAS/只读/关系和终态。失败显式回滚；成功的目标版本/outbox 与 T34 项目迁移子版本/outbox 保持原子性，保留活子项目不匹配 fail-closed 和删除子关系迁移例外。

`dates::validate_dates(start, due)` 统一保留 T34 的真实日历/时分/上海 timezone 校验和最终开始/截止顺序校验。标准 IANA 库 `chrono-tz =0.10.4` 将 `Asia::Shanghai` 本地时刻映射至 UTC；Ambiguous/None 返回 422。原始 DateValue 仍原样持久化。`is_overdue(&Task, now)` 接收显式注入时间，done/cancelled 或无截止返回 false；其他状态在 `now >= cutoff` 逾期，非法旧日期返回错误。纯日期先日历加一天取当地午夜，再转换 UTC，绝不固定加 24 小时。

## RED 与 GREEN

所有 cargo 命令均使用本任务 `CARGO_HOME=/data/knoted-dev/tasks/t35/cache/cargo`、`TMPDIR=/data/knoted-dev/tasks/t35/tmp`、`CARGO_TARGET_DIR=/data/knoted-dev/tasks/t35/build/rust-target`、`CARGO_BUILD_JOBS=2`。完整输出在 `/data/knoted-dev/tasks/t35/evidence/`。

- `cargo test --locked --workspace t35_` 首次 exit 101：7 个真实 HTTP 断言失败，父完成/子创建/重开/重挂和双成功并发缺少 409，历史 DST gap/fold 缺少 422。日志 `red-http.log`。
- `cargo test --locked -p knoted-api --test t35 t35_reparent_locks` 首次 exit 101：旧/新父兄弟行未加入锁集合，闸门未观察到等待而超时。日志 `red-sibling-locks.log`。
- 新日期 helper 测试最初缺符号编译失败，单列 `red-date-interface.log`，不冒称行为 RED。IANA 写入行为的真实 RED 已见 HTTP 日志。
- 首轮 `cargo test --locked --workspace t35_` exit 0：8 API + 5 core 日期测试，日志 `green-focused.log`。
- `cargo test --locked --workspace --test t32 --test t33 --test t34 --test t35` exit 0：8 + 10 + 20 + 9 + 1 = 48 例；保留 T34 枚举/日期 presence/项目迁移和 T32/T33 锁协议回归，日志 `green-regression.log`。

新增测试覆盖 parent done 全部活子终态；done 父下默认 todo/create/reopen/reparent；混合 PATCH 回滚与修复、同值 PATCH；三个真实 HTTP 并发对各两种先后次序，使用行锁闸门/pg_blocking_pids，无 sleep、stress 或产品测试钩子。额外闸门证明旧/新父兄弟按全局 UUID 先于较大父/目标锁定；状态冲突后关系锁可重新获取。

日期验证包含 2026-11-17 本地次日午夜截止 `16:00:00Z` 的前一微秒/到点/之后；10:30 上海对应 `02:30:00Z`；无截止、终态与生命周期独立；真实持久化 Task 配合 AppContext/FixedClock；1991-07-01 12:00 对应 `03:00Z`，1991-04-14 02:30 gap 和 1991-09-15 01:30 fold 拒绝；1991 春季相邻当地午夜仅相距 23 小时。

## 全量门禁与交接

| 实际命令 | 结果与完整输出 |
|---|---|
| `cargo fmt --check` | exit0，无输出；`full-fmt.log` |
| `cargo test --locked` | exit0，229 passed/0 failed/1 既有 ignored doctest；`full-test.log` |
| `cargo clippy --locked --all-targets -- -D warnings` | exit0；`full-clippy.log` |
| `git diff --check` | exit0 |
| 本卡/证据 Markdown 相对链接检查 | 9 个链接存在；`docs-links.log` |

全量测试实际包含最终 T35 的 9 API + 5 core 日期用例。命令退出码在 `full-results.json`，精确总数在 `test-counts.json`。完整输出摘录（core 单元共 12 例，其中 5 个为新增日期测试）：

```text
running 9 tests
test t35_date_persistence_and_context_clock ... ok
test t35_mixed_update_atomic_and_repair ... ok
test t35_3_parent_done_requires_all_live_children_terminal ... ok
test t35_done_parent_child_create_reopen_reparent ... ok
test t35_reparent_locks_old_and_new_siblings_in_global_order ... ok
test t35_real_iana_invalid_local_time_http ... ok
test t35_completion_vs_reopen ... ok
test t35_completion_vs_create ... ok
test t35_completion_vs_reparent ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.12s

running 12 tests
test config::tests::line_column_is_one_based_and_char_aware ... ok
test context::tests::t06_fixed_clock_advances_and_is_shared ... ok
test tasks::dates::tests::t35_real_iana_history_and_invalid_local_time ... ok
test tasks::dates::tests::t35_exact_time_boundary ... ok
test tasks::dates::tests::t35_2_no_due_and_terminal ... ok
test tasks::dates::tests::t35_existing_date_order_regression ... ok
test tasks::dates::tests::t35_1_date_only_midnight_boundary ... ok
test context::tests::t06_app_context_exposes_injected_clock_and_pool ... ok
test projects::tests::t33_relation_lock_release_on_error ... ok
test projects::tests::t33_archive_vs_task_state_change ... ok
test projects::tests::t33_archive_vs_task_admission ... ok
test projects::tests::t33_archive_vs_task_migration ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.53s
```

全量总计按所有真实 `test result` 行汇总为 229 passed。依赖锁仅增加 chrono-tz/phf/phf_shared/siphasher 四包，未升级已有 chrono 0.4.45；结构比较记录 `dependency-review.json`。作者已自查，未发现待修问题；独立审查由主 Agent 执行，不用作者自查冒称独立审查。

没有前端或 OpenAPI 变更，未重复执行无关前端/生成客户端检查。未 push/PR/merge/deploy，保留本任务分支和 Worktree，供主 Agent 完成主线验证、真实 dev 验收及清理。实施报告：`/data/knoted-dev/tasks/t35/t35-report.md`。
