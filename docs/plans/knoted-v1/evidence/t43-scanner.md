# T43 目录扫描实施记录

状态：implementation-ready，待父 Agent 独立审查、集成与 dev 验收；未运行 dev 迁移或部署。

- Issue：[#128](https://github.com/xingjianxu/knoted/issues/128)。实施基线 `294dac5a957206540386acf7f916930254c3b898`；整体审查基线 `46515b3ca56154326f6d35ad81ba736638d9b2fb`。
- 工作区：`/wrd/prj/knoted/.worktrees/knoted-t43`，分支 `feat/t43`。依赖由父 Agent 登记，本次没有修改依赖、锁、OpenAPI 或生成客户端。
- 范围：ADR0006 roots/scans/observations/sources、安全只读 FD、真实通知与 jobs/Worker 入口、配置/概览/现有夹具适配。不采集、移动、解析或索引。
- 分工：本 Agent 独立实施；父 Agent 独立审查、集成、dev 迁移、样本初始化及验收。未委派 subagent、push、PR 或 merge。

## 实施检查清单

- [x] 阅读任务卡、批准 ADR、三份安全/入口研究，核对隔离分支和基线。
- [x] RED：根失联、中断、链接过滤；配置/概览；真实通知/独立时钟；Worker实际落库；staging正常及dangling别名；锁等待租约过期。
- [x] GREEN：扫描与事务 reconciliation、运行链和通知。
- [x] 完整本地门禁及实际证据，见下表与外部原始日志。
- [ ] 父 Agent 审查、合入主线、dev 验收及工作区清理。

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

## 待集成/dev

父 Agent 停本实例写进程、备份后应用 schema12，构建固定 SHA 并切换。先观察批准根 absent 的 paused_reason=root_unavailable，再由父 Agent 仅初始化批准样本子目录。扫描读取样本后应有完整scan与observations，files计数仍为0，直到T44稳定采集登记真实source。不能把T43扫描成功当成采集、解析或索引已接通。

详细交接报告：`/data/knoted-dev/tasks/t39-t50/reports/t43-implementation.md`。任务包报告 `task-43-report.md` 指向同一报告。
