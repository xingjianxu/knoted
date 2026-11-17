# T43 目录扫描实施记录

状态：done（PR [#144](https://github.com/xingjianxu/knoted/pull/144)，主线/dev `987ef077328f5a37092bf326f47703554a945607`）。以下保留实施/修复阶段历史，最终结果见文末。

- Issue：[#128](https://github.com/xingjianxu/knoted/issues/128)。实施基线 `294dac5a957206540386acf7f916930254c3b898`；整体审查基线 `46515b3ca56154326f6d35ad81ba736638d9b2fb`。
- 工作区：`/wrd/prj/knoted/.worktrees/knoted-t43`，分支 `feat/t43`。依赖由父 Agent 登记，本次没有修改依赖、锁、OpenAPI 或生成客户端。
- 范围：ADR0006 roots/scans/observations/sources、安全只读 FD、真实通知与 jobs/Worker 入口、配置/概览/现有夹具适配。不采集、移动、解析或索引。
- 分工：本 Agent 独立实施；父 Agent 独立审查、集成、dev 迁移、样本初始化及验收。未委派 subagent、push、PR 或 merge。

## 实施检查清单

- [x] 阅读任务卡、批准 ADR、三份安全/入口研究，核对隔离分支和基线。
- [x] RED：根失联、中断、链接过滤；配置/概览；真实通知/独立时钟；Worker实际落库；staging正常及dangling别名；锁等待租约过期。
- [x] GREEN：扫描与事务 reconciliation、运行链和通知。
- [x] 完整本地门禁及实际证据，见下表与外部原始日志。
- [x] 父 Agent 审查、合入主线、dev 验收及工作区清理。

## 运行链与持久化

`worker main` 显式注入 files 配置并运行 scan 服务。单一 `WorkerRunner<Dispatcher>` 领取共享 jobs，`jobs::load_claimed` 读取当前有效任务，kind=`file.scan` 分派到 `observe_root` → `reconcile_complete_scan`。其他 kind 明确失败，不启动互抢的单 kind runner。既有 history/trash 循环保留。默认配置 disabled；pc 固定模板显式启用批准根，staging 在本实例私有根。

`0012_t43_files.sql` 创建：

- `file_roots`：单根 `configured_path/identity/generation/scan_token/last_complete_generation/last_complete_at/paused_reason`。identity JSON 含 device/inode/可选完整 opaque handle。根不存在时 identity 保持 NULL 且暂停；首次成功安全打开后登记；已登记身份不自动替换。
- `file_scans`：`id/root_id/generation/token/job_id/lease_token/started_at/finished_at/complete/error_class`。令牌不在日志打印。
- `file_observations(scan_id,relative_path,metadata)`：尚无稳定摘要的待采集集合，不创建 FileSource。
- `file_sources`：ADR0006 必需字段及 `observation/observation_version/content_fresh`。本卡只维护已登记来源的观察和缺失状态，首次稳定来源登记由 T44 负责。

打开根时逐段 DIRECTORY/NOFOLLOW；目录枚举来自 FD；条目先 O_PATH/NOFOLLOW，再以 fstat 判定普通文件，不读 FIFO/socket/device 或 symlink。完成前复核配置路径、根身份和所有遍历目录的身份/mtime/ctime/nlink；读失败、权限、目录变化、取消、根变化均不判缺失。完整标记与 missing reconciliation 同事务，锁 jobs/root，复核扫描代次/根令牌；等待数据库锁后以及提交前再次检查真实租约到期时间。旧代失败也不能覆盖新代暂停状态。

通知使用持久 Linux inotify 描述符，根失联时监视最近可安全打开祖先；身份登记后安装递归目录 watch，刷新保留已排队事件并移除不再属于树的 watch。2秒 debounce 和600秒补扫分别计时；入队遇到活动 job 不清 pending，持续通知不能重置补扫期限。overflow 同样请求完整扫描。观察消息不直接推断缺失；通知错误时独立完整补扫仍保留。

## T44/T47 必须复用的观察语义

- 新观察 JSON 与现存 `file_sources.observation` 不同时：更新 observation，`observation_version += 1`、`version += 1`、`content_fresh=false`；即使扫描随后不完整也先禁止旧内容。相同观察的重试不重复递增。
- 这里不更新 sha256 或 content_version，不将新观察冒充稳定采集。T44 采集稳定后更新摘要/内容版本，并在事务中复核当前 observation_version 和完整 metadata；成功后才将 content_fresh 设为 true。T47 发布/读取不能只检查 content_version，必须检查当前 present、content_fresh、观察版本及采集观察。
- 完整扫描缺失：`status=missing`、`version += 1`、`content_fresh=false`；保留旧摘要、位置和历史身份。不会自动复活 missing，也不执行移动识别。
- opaque handle 仅为当前已登记 root/FS 范围内的内部观察细节。unsupported 为 None；不以 ctime/btime 作为 incarnation 证明，不声称跨克隆/回滚绝对唯一。
- 配置拒绝源与 staging/assets 双向重叠，包含已有/尚不存在目录上的 symlink 别名。配置检查不代替 T44 打开 staging 时防御运行期路径替换。

## 验证

原始日志：`/data/knoted-dev/tasks/t43/logs/`。所有临时文件在本卡 tmp，Rust target 使用父 Agent 指定共享 build。

| 命令/场景 | 结果 | 日志 |
| --- | --- | --- |
| `cargo test --workspace t43_` 初始占位行为 | exit 101，3个实际断言失败 | `red-initial.log` |
| 配置及概览 RED | exit 101，10通过/2失败 | `red-config-dashboard.log` |
| inotify/独立补扫 RED | exit 101，2失败 | `red-notifications.log` |
| Worker真实 scan 落库 RED | exit 101，1失败 | `red-runner.log` |
| staging alias / dangling alias | 各 exit 101，1失败 | `red-staging-alias.log` / `red-dangling-alias.log` |
| 等根锁后租约到期 RED | exit 101，1失败（错误提交为 true） | `red-expiry-wait.log` |
| `cargo test --workspace t43_` | exit 0，21通过 | `t43-final.log` |
| `cargo test --workspace` | exit 0，288通过、1既有doc忽略 | `cargo-test-final.log` |
| `cargo fmt --check` | exit 0 | `fmt-final.log` |
| `cargo clippy --all-targets -- -D warnings` | exit 0 | `clippy-final.log` |
| `pnpm --filter @knoted/web test` | exit 0，17文件/248用例 | `web-test.log` |
| Web `typecheck` / `lint` | 各 exit 0 | `web-typecheck.log` / `web-lint.log` |
| `pnpm test:pc` | exit 0，24用例 | `pc-final.log` |
| `bash -n tools/dev/pc/service.sh` | exit 0 | `shell-final.log` |

补充回归覆盖：实际权限失败、根/子目录替换、过期租约、取消、旧代成功/失败迟到、partial观察失效、重复观察版本不变、FIFO/socket、隐藏 include 无法突破链接、实际嵌套通知、活动扫描期间通知、overflow解码及独立补扫时钟。overflow 用内核格式事件帧测试真实解码器，未进行队列洪泛或性能测试。

首轮 clippy 的3处样式诊断已修正；pc 首轮23/24（缺sqlx用例继承真实PATH）失败，最小隔离该负向用例PATH后24/24。Web输出3条既有 jsdom scrollTo 未实现提示，测试无失败。

## 实施交接时待集成/dev（历史）

父 Agent 停本实例写进程、备份后应用 schema12，构建固定 SHA 并切换。先观察批准根 absent 的 paused_reason=root_unavailable，再由父 Agent 仅初始化批准样本子目录。扫描读取样本后应有完整scan与observations，files计数仍为0，直到T44稳定采集登记真实source。不能把T43扫描成功当成采集、解析或索引已接通。

详细交接报告：`/data/knoted-dev/tasks/t39-t50/reports/t43-implementation.md`。任务包报告 `task-43-report.md` 指向同一报告。

## 独立审查修复 round1（基线 04431da）

审查原文：`/data/knoted-dev/tasks/t39-t50/reviews/t43-task-review.md`，Spec FAIL / Quality CHANGES_REQUESTED。下面保留初始门禁未覆盖时序的实际失败，并补对应修复，不以此前288通过否认问题。

- **R1 — Heartbeat can self-deadlock against the scan transaction's jobs row lock.** 真实 WorkerRunner 使用20ms心跳；外部事务持root锁250ms，原实现释放root后仍等待至2秒超时。runner 的 heartbeat 分支改为并发轮询 heartbeat SQL 和 handler，不派生后台任务，不改变 Handler 接口、取消确认或丢租约规则。
- **R2 — An expired observation attempt can still publish root generation/token.** 领取观察在等待root锁后与提交前重新校验lease。回归保留并逐字段比较整个root JSON（含NULL identity、原generation/token）及scan数量，过期时事务完整回滚。
- **R3 — Changed observations do not invalidate old content until the entire traversal ends; real cancellation discards them.** 每个普通文件观察在继续下一条目录项前，独立事务写观察并使旧内容失效；事务以jobs lease、root generation/token/identity和安全路径检查限制发布。完成reconcile只负责最终完整标记/缺失推断，不再补写此前内存观察。相同观察仍不重复增版本；过期或旧代不能写观察。

R3 的core用例在更晚目录的INSERT以真实PG advisory锁暂停，证明早先文件已在遍历结束前失效；丢弃后续future仍保留失效且scan不完整。worker用例持有真实observe_root结果、暂停于reconcile之前，经真实 `request_cancel` → heartbeat → Runner丢弃execute future（DropSignal验证）→ cancelled；没有手工调用reconcile代替取消清理，来源仍present且content_fresh=false。另有等待source行锁期间lease过期的回归，观察及失效写入均回滚。

测试文件：`crates/knoted-core/tests/t43.rs`、`crates/knoted-worker/tests/t43_runner.rs`。日志目录仍是本卡 `/data/knoted-dev/tasks/t43/logs/`，每条round1命令都保存独立`.exit`文件。

| 精确命令 | 实际结果/日志 |
| --- | --- |
| `cargo test -p knoted-worker --test t43_runner t43_r1_` | RED exit101：1失败（超时）；GREEN exit0：1通过；`r1-red.log` / `r1-green.log` |
| `cargo test -p knoted-core --test t43 t43_r2_` | RED exit101：1失败；GREEN exit0：1通过；`r2-red.log` / `r2-green.log` |
| `cargo test --workspace t43_r3_` | RED exit101：core1失败后Cargo停止，尚未跑worker；`r3-red.log`。修复后exit0：core/worker共2通过；`r3-green.log` |
| `cargo test -p knoted-worker --test t43_runner t43_r3_` | 单独证明worker RED exit101：1失败，实际仍fresh=true；`r3-worker-red.log` |
| `cargo test --workspace t43_` | exit0：26通过；`round1-t43.log` |
| `cargo test -p knoted-worker --test t07_runner --test t08_runner` | exit0：4通过；`round1-existing-runner.log` |
| `cargo fmt --check` | exit0；`round1-fmt.log` |
| `cargo clippy --all-targets -- -D warnings` | exit0；`round1-clippy.log` |

`cargo test --workspace` 最终 exit0：293通过、1既有doc忽略，`round1-cargo-test.log` / `.exit`；提交SHA在交接报告追加。本轮未修改或重跑Web/pc，也没有修改依赖锁/公开契约或执行dev/远程操作。R4既有jsdom提示按父Agent指令留独立ledger，本轮不处理。状态仍为待父Agent限定diff复审，不自行宣称Spec/Quality已通过。

## 主 Agent 最终集成与实例验收

- 准备提交 `294dac5a957206540386acf7f916930254c3b898`、实施 `04431da9e91189f32283395aad4c3614c324041c`、修复 `cff42a333f49414c4c3f1450114a3ac6a3c9e70d`。独立初审发现三项 Important，经真实失败回归修复；限定复审 R1/R2/R3 全部 ADDRESSED、Spec PASS / Quality APPROVE，无新重要问题。报告 `/data/knoted-dev/tasks/t39-t50/reviews/t43-{task,round1}-review.md`。
- PR [#144](https://github.com/xingjianxu/knoted/pull/144) 普通合入主线 `987ef077328f5a37092bf326f47703554a945607`，dev 运行相同 SHA；Issue #128 由 PR 关闭。既有测试环境噪声 R4 单独登记 [#143](https://github.com/xingjianxu/knoted/issues/143)，未混入扫描实现。
- 主 Agent 初次完整 Rust/Web/API/pc 检查均 exit0；修复后独立最终 Rust 293 passed / 1 ignored、fmt/clippy exit0（`parent-fix1-*`）。实际 diff 证明修复轮未改 Web/API/pc/依赖锁，沿用其已通过的 Web248/pc24 与静态证据。合并后主线再次完整验证：Rust293/1 ignored、Web248、pc24，fmt/clippy/typecheck/lint/build/api:check（83 operations/20 examples/10 invalid rejected）/api:verify/bash语法全部 exit0；日志 `/data/knoted-dev/tasks/t43/logs/main-*.log`。
- 固定提交产物 `/data/knoted-dev/runtime/987ef077328f5a37092bf326f47703554a945607` 完整 SHA256SUMS 通过；停 API/Worker 写进程后备份 `/data/knoted-dev/service/backups/20261117T173815Z-t43-`，摘要与 pg_restore --list 校验通过。pcwrap prepare→migrate 应用新增0012，再启动该固定产物。旧二进制会拒绝schema12，不把切回current冒充数据库回滚。
- `deploy.log`、`runtime-before.json`、`runtime-after.json` 记录 HTTPS ready200、实际 API/Worker exe 为同一固定 SHA，PostgreSQL/Caddy/Strata MainPID 未变。没有宿主配置、共享服务、生产或破坏性数据库操作。
- 批准根初始不存在时，真实根状态暂停 `root_unavailable`、identity未登记、0个complete scan、0个source；这一新库运行样本没有已登记来源，不能用它替代隔离PG测试中“已有来源零missing”的证明。随后主 Agent 仅在批准根初始化 `agent-fixtures/t43/<专用UUID>`，应用始终只读。
- 扫描实际只观察 visible.txt；隐藏/工具/临时文件、源外symlink和FIFO均未进入观察。等scanner真正空闲后修改样本，独立通知触发新代完整扫描（空闲代4→修改代5，文件size29→35）；删除自己的样本后代6完整重扫、观察为空。没有用手工入队或重启冒充通知。2秒/600秒规则另由真实通知及独立时钟测试覆盖，未声称实际等待10分钟或做性能测试。
- 各运行阶段脚本 `/data/knoted-dev/tasks/t43/runtime-probe.py`，原始JSON `/data/knoted-dev/tasks/t43/evidence/runtime-{missing,observed,changed,cleaned}.json`，均 exit0：

```json
{"schema":12,"generation":2,"last_complete_generation":null,"registered":false,"paused_reason":"root_unavailable","complete_scans":0,"incomplete_scans":2,"sources":0,"observations":[],"phase":"missing","result":"passed"}
```

```json
{"schema":12,"generation":3,"last_complete_generation":3,"registered":true,"paused_reason":null,"complete_scans":1,"incomplete_scans":2,"sources":0,"observations":[{"path":"agent-fixtures/t43/ed9920de-72b5-46f2-81e4-58d885047a10/visible.txt","size":29}],"phase":"observed","result":"passed"}
```

```json
{"schema":12,"generation":5,"last_complete_generation":5,"registered":true,"paused_reason":null,"complete_scans":3,"incomplete_scans":2,"sources":0,"observations":[{"path":"agent-fixtures/t43/ed9920de-72b5-46f2-81e4-58d885047a10/visible.txt","size":35}],"phase":"changed","result":"passed"}
```

```json
{"schema":12,"generation":6,"last_complete_generation":6,"registered":true,"paused_reason":null,"complete_scans":4,"incomplete_scans":2,"sources":0,"observations":[],"phase":"cleaned","result":"passed"}
```

- 真实 HTTPS Chromium 概览验收 exit0：`dashboard_and_stage_status=true`、`passkey_count_stable=true`、`session_revoked=true`，日志 `dashboard-browser.log`。文件计数仍0，因为首次稳定摘要登记尚属T44；不把扫描当成采集/解析/索引完成。
- 专用source样本和外部sentinel已清理，批准根及已登记身份保留供T44；没有重命名/替换真实vault根。2026-11-18确认任务Worktree tracked/untracked clean、feat/t43为master祖先后，安全移除 `.worktrees/knoted-t43` 与本地 `feat/t43`；协调及他人Worktree保留。
