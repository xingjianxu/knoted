# T41 工作概览与到期：实施证据

- Issue：[#126](https://github.com/xingjianxu/knoted/issues/126)。依据 [T41](../tasks/T41.md)、[ADR 0002](../../../decisions/0002-mtone-ui-ownership.md)、[ADR 0006](../../../decisions/0006-file-pipeline-and-aggregate-contracts.md)。
- 实施工作区：主仓库 `.worktrees/knoted-t41`，分支 `feat/t41`。worker 基线 `5df8aa8a259ebe2ede73f48966ff9e11963580b9`，含主 Agent 已串行补齐的 DashboardSummary 契约；整卡审查基线 `4d08f4b093aa0d7db4d2250d611499dea2bddbb0`。
- 状态：done（PR [#139](https://github.com/xingjianxu/knoted/pull/139)，主线 `f2423b4`，dev `f2423b4`；主线/实例验收和任务工作区清理完成）。

## 行为

认证 `GET /api/v1/dashboard` 在一个只读 `REPEATABLE READ` 事务中读取当前未删除笔记、未删除未归档事项和活跃项目；`tasks_by_status` 与 `tasks_open` 使用同一事项集合。最近笔记、最近事项以更新时间与 ID 降序最多 10 条；到期列表为未终态且上海日历日期不晚于今天的事项，按 T35 截止边界与 ID 升序最多 10 条。今日计数在截断前计算。T35 `date_boundary` 仅改为 `pub(crate)`，算法保持不变，时间源为 `ctx.now()`。

`job_failures` 为全部当前 `failed` jobs 数量，不返回 payload、last_error、正文或堆栈。数据库/遗留日期异常统一安全 503，匿名 401。概览不创建 job/outbox，不发送外部通知。文件源尚未实现，`files=0`；索引尚未启用，`indexing_enabled=false`，页面明确显示两项能力边界。

默认首页直接消费 mtone 0.4.0 已发布的 Card/List/ListItem/Empty/Alert/Button/Spinner/Badge，无新通用组件或上游缺口。笔记/事项按钮通过应用路由打开既有详情。失败可重试；重新进入首页 `refetchOnMount: "always"`，刷新期间隐藏旧聚合，防止完成事项仍显示旧逾期提示。前端截止展示复用 T39 `dateBoundary`，依据 Query 更新时刻显示此读取快照的到期标签。

## TDD 与回归

日志根 `/data/knoted-dev/tasks/t41/logs/`，完整命令与退出码见实施报告 `/data/knoted-dev/tasks/t39-t50/reports/t41-implementation.md`。

- `red-api.log`：实际 HTTP 请求返回 404，预期 200，1 failed，exit 101；随后 `api-t41.log` 2 passed。
- `red-web.log`：将原欢迎空态接入待实现业务入口后，6 个业务断言实际失败，exit 1；`green-web.log` 最终 7 passed。
- core `tests/t41.rs`：真实 PG + 固定上海日界 clock；今天日期型/精确时刻、次日变化、终态/归档/软删排除、更新时间/ID 排序、最近/到期 10 条截断、14 条计数不截断、failed 与 retry_wait/cancelled 区分、活跃项目、无新增 jobs/outbox、无错误正文泄露。
- API `tests/t41.rs`：真实 PG/HTTP 空态、匿名拒绝；完成前后聚合变化；仅改本测试 schema 的表名模拟依赖故障，503 不包含正文或 SQL 诊断。
- Web `tests/tasks/t41.test.tsx`：`t41_1` 完成事项不逾期，`t41_2` 零数据空态，`t41_3` 仅一次本站只读 GET；另测笔记导航、到期/失败计数、精确时刻与日期型边界、安全错误重试、再次进入强制刷新并隐藏旧到期数据。

## 本地门禁

以下最终命令均实际执行并 exit 0（Rust target `/data/knoted-dev/tasks/t41/build/cargo`，Web outDir `/data/knoted-dev/tasks/t41/build/web`）：

| 命令 | 实际结果 | 日志 |
| --- | --- | --- |
| `cargo fmt --check` | 通过 | `fmt.log` |
| `cargo test --workspace --locked` | 259 passed，1 ignored，0 failed | `rust-test.log` |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 | `clippy.log` |
| `pnpm --filter @knoted/web test` | 16 files，241 tests passed | `web-test.log` |
| `pnpm --filter @knoted/web run typecheck` | 两个 tsc 通过 | `typecheck.log` |
| `pnpm --filter @knoted/web run lint` | 通过 | `lint.log` |
| `pnpm --filter @knoted/web exec vite build --outDir /data/knoted-dev/tasks/t41/build/web` | 构建通过 | `build.log` |
| `pnpm api:check` | 83 operations，18 examples，9 invalid rejected | `api-check.log` |

Web `run build` 的 `tsc --noEmit` 已由 typecheck 的同一命令实际通过，随后执行上表带外部 outDir 的 Vite 构建，避免输出写入任务 Worktree。测试出现既有 jsdom `scrollTo` 提示、构建出现 chunk 大小提示，均 exit 0。实施阶段交接时独立审查、集成及部署待主 Agent执行；最终结果见收尾段。无迁移、依赖或锁文件变更；API/生成客户端由主 Agent 预先补齐，本实施未改动。

## 实施子 Agent 交接时的限制

以下保留集成前记录；最终部署和验收已在“主 Agent 集成、实例与收尾”完成。

概览读取当前未归档事项以复用领域日期语义，未增加全文/向量索引或外部通知。到期标签表达本次读取时刻的状态；重新进入首页会刷新。dev 部署与原生 HTTPS 的最终业务验收尚未执行。

## 主 Agent I1 复核修复（round 1）

主 Agent 在初审后发现合法历史精确截止时间被额外午夜映射误拒绝。实际 chrono/真实 PG 回归确认：`1919-04-13 03:00 Asia/Shanghai` 可通过 T35 校验并创建，core 概览却返回 `InvalidField("start/due")`，HTTP 概览 503。`i1-red-core.log`、`i1-red-api.log` 各 1 failed、exit 101，失败发生在创建成功后的概览断言，非 fixture 或编译错误。

修复仅移除概览将合法截止时间改为 `time=None` 再解析午夜的额外约束。先保留原 `date_boundary(value, true)` 验证/排序，然后使用 `NaiveDate` 读取已经验证的 `date` 字段用于日历比较；不修改 T35 时区算法。新增 core 回归固定到该上海当天验证今日计数与列表，新增 HTTP 回归验证合法任务创建后概览 200 且可见。

受影响命令 `cargo test --locked -p knoted-core -p knoted-api --test t41` 实测 6 passed、exit 0（`i1-green.log`）。重新执行 `cargo fmt --check`、`cargo test --workspace --locked`（261 passed、1 ignored、0 failed）与 `cargo clippy --locked --all-targets -- -D warnings` 全部 exit 0；对应 `i1-fmt.log`、`i1-rust-test.log`、`i1-clippy.log`，见实施报告的 I1 附录。Web 未改动，无需重复其已通过门禁。

首次 RED 的断言 panic 遇到既有 TestDb 同步 Drop 等待未完成回滚连接，schema 清理阻塞。仅终止本任务测试进程 `640179`、`640443`，新回归先 `await db.drop()` 再断言以取得干净失败证据。仅清理自身残留 `knoted_test_c42c5be30a1e`、`knoted_test_50bd8686bcc8`，读回 0；完整证据在 `/data/knoted-dev/tasks/t41/evidence/i1-cleanup-schemas.txt`。公共 TestDb 未修改，其他数据库/schema/进程未触碰。

## 主 Agent I2 导航接线修复（round 2）

真实 `createAppRouter` 路由测试从已实现的 `/settings/security` 页面点击侧栏概览：原实现仍将概览标为禁用待开放，点击后 pathname 保持 `/settings/security`，预期 `/`，有效 RED 1 failed、18 skipped、exit 1（`i2-red.log`）。最小修复在现有 authenticated Shell 注册已启用概览按钮，导航 `/`，仅根路径标记 active；从未来占位列表移除概览，不更改文件/搜索/问答。同步更新 T20 旧占位断言并给既有路由 harness 补充真实 DashboardSummary fixture。

回归使用现有应用 Router/AuthGate/Query/默认 index/DashboardPage，验证点击抵达根路由、概览标题与空态、实际 dashboard GET，以及离开根路径时概览不 active、抵达后 `aria-current=page`。T20/T41 targeted 26 passed、exit 0（`i2-green.log`）。Web 全量 `test` 242 passed、`typecheck`/`lint`/外部 outDir `vite build` 均 exit 0，对应日志 `i2-web-test.log`、`i2-typecheck.log`、`i2-lint.log`、`i2-build.log`，命令见实施报告 I2 附录；Rust 未变，不重复已通过的后端门禁。


## 主 Agent 集成、实例与收尾

- PR [#139](https://github.com/xingjianxu/knoted/pull/139) 普通merge，主线 `f2423b4e2cba882b93797fec7039d6c42cbf3f28`，包含契约5df8aa8、实现8ad3942、I1日期c35f0ef、I2导航6809dfb；Issue126自动关闭。
- fresh独立review_t41整体审查及两轮限定复审均PASS/APPROVE；I1/I2为主Agent额外验证发现，均真实RED→GREEN，无剩余重要问题。
- 主Agent独立最终Rust261 passed/1既有ignored、fmt/clippy；Web242/typecheck/lint/build；api:check83 operations/18 examples/9 invalid rejected、api:verify均exit0。主线再次运行相同完整Rust/Web/契约门禁全部exit0（`/data/knoted-dev/tasks/t41/logs/{parent-fix1-*,parent-fix2-*,main-*}.log`）。
- 固定主线archive产物 `/data/knoted-dev/runtime/f2423b4e2cba882b93797fec7039d6c42cbf3f28` 完整SHA256SUMS校验通过（build-artifact.log）。无迁移，schema11。
- 停本实例API/Worker并确认旧PID退出后，一致备份 `/data/knoted-dev/service/backups/20261117T161350Z-t41-`（PG/assets/摘要及pg_restore目录校验通过），原子切current并pcwrap down/up。HTTPS ready200、API/Worker实际可执行文件固定SHA，共享PG/Caddy/Strata PID不变（deploy.log、runtime-{before,after}.json）。
- 真实HTTPS Chromium无路由拦截：live-browser.log exit0，auth_default_home_empty_and_disabled_index、valid_historical_precise_creation、shanghai_due_historical_fix_terminal_future_excluded、enabled_overview_nav_refetch、completed_removes_overdue_after_reentry、recent_note_navigation、due_task_navigation、fixture_cleanup_readback、only_same_origin_no_external_notifications、passkey_count_stable全部true，cleanup_ok=true。合法1919-04-13 03:00事项实际创建201且概览200，完成后通过侧栏离开/返回不再列逾期；匿名401、默认首页/空态、笔记和事项真实路由均验证。
- 仅API创建/软删本任务笔记及事项，临时会话撤销；最终活跃笔记0、事项0、未撤销验收会话0、Passkey1且前后不变，无硬件Passkey ceremony。
- `.worktrees/knoted-t41`干净、feat/t41为主线祖先后安全移除并-d删除；他人worktree未改。dev继续固定f2423b4，后续证据文档不需部署。
- 公共TestDb panic/回滚清理阻塞已查重建[Issue138](https://github.com/xingjianxu/knoted/issues/138)，本卡未修改公共helper；只精确清理本任务两个_test遗留schema并读回0，未碰dev/其他任务。
- 当前首页文件计数/未启用提示是T41阶段策略；T43/T44接入文件后由其更新，不提前实现文件或索引阶段。
