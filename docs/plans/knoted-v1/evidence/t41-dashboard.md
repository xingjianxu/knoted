# T41 工作概览与到期：实施证据

- Issue：[#126](https://github.com/xingjianxu/knoted/issues/126)。依据 [T41](../tasks/T41.md)、[ADR 0002](../../../decisions/0002-mtone-ui-ownership.md)、[ADR 0006](../../../decisions/0006-file-pipeline-and-aggregate-contracts.md)。
- 实施工作区：主仓库 `.worktrees/knoted-t41`，分支 `feat/t41`。worker 基线 `5df8aa8a259ebe2ede73f48966ff9e11963580b9`，含主 Agent 已串行补齐的 DashboardSummary 契约；整卡审查基线 `4d08f4b093aa0d7db4d2250d611499dea2bddbb0`。
- 状态：实施与本地验证，待主 Agent 独立审查、PR 集成、主线验证、dev HTTPS 验收与清理；不声称已完成生命周期。

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

Web `run build` 的 `tsc --noEmit` 已由 typecheck 的同一命令实际通过，随后执行上表带外部 outDir 的 Vite 构建，避免输出写入任务 Worktree。测试出现既有 jsdom `scrollTo` 提示、构建出现 chunk 大小提示，均 exit 0。独立审查、集成及部署待主 Agent 执行。无迁移、依赖或锁文件变更；API/生成客户端由主 Agent 预先补齐，本实施未改动。

## 限制

概览读取当前未归档事项以复用领域日期语义，未增加全文/向量索引或外部通知。到期标签表达本次读取时刻的状态；重新进入首页会刷新。dev 部署与原生 HTTPS 的最终业务验收尚未执行。
