# 部署进程映像判据修复（Issue #268）

**Goal:** `pnpm deploy:dev` 在操作者主组与 dev 托管进程主组不一致时仍可验证部署；缺少版本证据时给出可执行诊断，不因这一点自动回滚。

**Issue:** [#268](https://github.com/xingjianxu/knoted/issues/268)（bug / agent-found，人工已批准实施）
**分支:** `fix/dev-deploy-process-check`，Worktree `.worktrees/dev-deploy-process-check`

## 根因（已证实）

`tools/dev/deploy-dev.sh` 的 `check_processes` 用 `readlink /proc/<pid>/exe` 判定进程映像。内核解析 `/proc/<pid>/exe` 走 `PTRACE_MODE_READ_FSCREDS`，要求调用者与目标的 uid **和 gid** 匹配（`kernel.yama.ptrace_scope=1` 只额外限制 ATTACH 模式，不是本次原因）。

dev 实例的进程由 pcwrap 托管的 process-compose 拉起，主组 `strata-clients(980)`；操作者 shell 主组 `users(100)` → `readlink` 返回权限错误。脚本把它当成"进程不存在"，空转到 `KNOTED_DEPLOY_READY_TIMEOUT` 后回滚一个本来正确的部署。

证据：`sg strata-clients -c 'readlink /proc/<pid>/exe'` 立即成功；同一次部署用该组重跑 8s 收敛成功。

## 设计决定

1. **判据分层，不猜**：优先 `/proc/<pid>/exe`（权威）；读不到时退回 `/proc/<pid>/cmdline`（世界可读，不受 ptrace 限制）。
2. **让 cmdline 自描述**：`tools/dev/pc/service.sh` 在 exec 前把运行根解析成具体产物目录（`readlink -f`）。语义不变（execve 本来就会解析符号链接），但进程表从此能看出进程来自哪个 `<sha>`，与 `bin/knoted-intelligence` 包装器自己解析路径的既有行为一致。
3. **判不出就不猜**：cmdline 只给到 `runtime/<…>/current/…` 这类非具体目录时视为"无法验证"。
   - 切换前发现 → 检查实际负责重启的主检出 launcher 是否会解析产物路径；支持则继续升级，否则停止，指针不变；
   - 切换后发现 → 立即失败（不等满超时）、**不回滚指针**（回滚会扩大空窗且无法改善判断），打印日志入口。
4. **诊断可执行**：读不到 exe 时打印调用者 gid 与进程 `Gid`，并给出 `sg <进程主组> -c 'pnpm deploy:dev'` 这一恢复路径（换组后 exe 判据与 `environ` 抓取都恢复可用）。
5. 保留"重启成功但映像没换"的强检测：exe 可读时行为与现在完全一致。

## 文件与任务

| 文件 | 改动 |
| --- | --- |
| `tests/pc/deploy.test.mjs` | 覆盖 exe 优先、40 位 SHA 与别名区分、主检出 launcher 版本、env 文件打开失败 |
| `tools/dev/deploy-dev.sh` | `identify_process` 分层判据、`preflight_process_evidence`、收敛等待快速失败、`fail_after_switch` 不回滚、抓取运行变量的权限诊断 |
| `tools/dev/pc/service.sh`、`tests/pc/service.test.mjs` | 将运行根解析为具体产物目录；测试 API/Worker 实际以固定路径启动 |
| `docs/plans/knoted-v1/development-and-deploy.md` | 验证项写清新判据与执行前置条件 |
| 本文件 | 计划与复盘 |

## 验收（对应 Issue）

- [x] exe 不可读、cmdline 指向具体产物 → 部署成功并提示所用判据
- [x] 现有进程只提供运行根别名 → 主检出 launcher 能解析时继续（含历史短 SHA 目录）；不能时在切换前退出
- [x] 切换后无法验证 → 立即失败、无回滚重启、指针保持新产物
- [x] `environ` 实际打开失败时给出诊断；不采信未经 exe 确认的进程环境
- [x] 既有"重启后映像仍在旧目录"用例继续检出真实退化
- [x] `pnpm test:pc` 67 项通过；`pnpm lint`、`pnpm typecheck`、`pnpm test` 通过（Web 468 项）
- [x] `cargo fmt --check`、`cargo test --locked`、`cargo clippy --locked --all-targets -- -D warnings` 退出 0；真实 intelligence 测试使用已有 Python 虚拟环境和只读模型根
- [ ] 真机 `pnpm deploy:dev` 在默认 `users` 主组下可用（合并后验证）
