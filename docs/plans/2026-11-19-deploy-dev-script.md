# dev 一键部署脚本实施计划

**Goal:** 一条命令把 dev 测试实例从当前状态推进到最新主线提交：取最新 → 构建固定产物 → 原子切 `current` → 按迁移情况重启 → 全量验证。

**Architecture:** 新增 `tools/dev/deploy-dev.sh`，串起既有 `tools/dev/build-artifact.sh`（固定产物）、`tools/dev/pc/service.sh`（probe）、`@bixlab/pcwrap`（进程编排）三段既有能力，不复制其中任何逻辑。脚本只编排顺序、判迁移、切指针、验证与失败回滚；所有路径/端口可用环境变量覆盖，默认即本机 dev 实例。

**Tech Stack:** bash 5 + GNU coreutils（`flock`/`sha256sum`/`mv -T`）、node:test（`tests/pc/`）、pcwrap / process-compose。

**Spec:** 会话内已批准的设计（本文件「设计决定」节即规格来源）；相关 ADR [0003](../../decisions/0003-test-instance-process-management.md)、[0005](../../decisions/0005-https-owned-by-caddy.md)。

## 设计决定（已与维护者确认）

1. 全链：取最新主线 → 构建 → 切换 → 重启 → 验证。
2. 迁移处理：比对旧 `current` 与新产物的 `share/knoted/migrations/*.sql` 文件名集合；变化 → `down` + `up -D`（重跑 prepare/migrate），无变化 → 只 `process restart`。
3. 不提供 `--rollback` 子命令、不自动清理历史产物、不自动备份；只在本次运行内处理失败：迁移集合未变 → 复位指针并重启回旧版本；包含迁移 → 保留新产物并打印人工入口（只回退指针会与已应用 schema 冲突）。
4. 验证：`REVISION`、完整 `SHA256SUMS`、API readiness、intelligence health、三个进程 `/proc/<pid>/exe` 落在新目录、外网 HTTPS 200。
5. 启动变量持久化到 `<instance_root>/deploy.env`（0600）：首次从活动 `knoted-intelligence` 的 `/proc/<pid>/environ` 抓取，调用者环境变量优先。

## 全局约束

- 稳定实例不得运行在 `.worktrees/`：`instance_root`、运行根父目录、构建根任一命中即拒绝（与 `service.sh` 的 `guard_worktree` 同语义）。
- 产物只从已提交完整 SHA 构建；缺 `origin/master` 时先 `git fetch origin`。
- 不写凭据：`deploy.env` 只存路径与开关，权限 0600。
- 缺工具（`git`/`pnpm`/`curl`/`sha256sum`/`flock`）在改动任何状态前失败。
- 不顺带解决构建缓存清理、旧产物回收、数据库备份。

## 文件结构

| 文件 | 职责 |
| --- | --- |
| `tools/dev/deploy-dev.sh`（新增） | 编排入口；守卫、定版、变量、判迁移、切指针、重启、验证、复位 |
| `tests/pc/deploy.test.mjs`（新增） | 场景测试：8 个通过/拒绝路径，沿用 `service.test.mjs` 的 fixture 风格 |
| `package.json`（修改） | 新增 `deploy:dev` script |
| `docs/plans/knoted-v1/development-and-deploy.md`（修改） | 新增「一键部署」小节 |
| `README.md`（修改） | 运行入口加一行 |
| `process-compose.yaml`（修改） | 头部常用命令补一行 |

## Interfaces

- 调用：`bash tools/dev/deploy-dev.sh [<full-40-hex-sha>]`；`pnpm deploy:dev [<sha>]`。
- 环境覆盖（默认值即 dev）：`KNOTED_INSTANCE_ROOT`、`KNOTED_DEPLOY_RUNTIME_PARENT`（`/data/knoted-dev/runtime`）、`KNOTED_BUILD_ROOT`（`/data/knoted-dev/tasks/deploy`）、`KNOTED_DEPLOY_PROC_ROOT`（`/proc`）、`KNOTED_API_PORT`、`KNOTED_INTELLIGENCE_PORT`、`KNOTED_HTTPS_PORT`、`KNOTED_HTTPS_DOMAIN`、`CARGO_BUILD_JOBS`。
- 退出码：0 部署并验证通过（含「已是最新」）；2 用法/前置错误；1 构建、重启或验证失败（切换后失败会复位指针）。

## Task 1: 场景测试（TDD 先行）

**Files:** `tests/pc/deploy.test.mjs`

fixture：临时 `runtime_parent`（两个假产物：旧 `prev` 与新 `target`，均含 `bin/`、`share/knoted/{web,migrations,REVISION,SHA256SUMS}`）、临时 `instance_root`、假 `/proc` 根（`<proc>/<pid>/exe` 符号链接指向产物里的可执行文件）、`PATH` 前置 stub 目录（`pnpm` 记录 `cwd + argv` 到日志文件；`curl` 按用例返回 0/7）。

- [ ] 迁移集合有新增 → `pnpm pc down` 先于 `pnpm pc up -D`，`current` 指向新产物，退出 0
- [ ] 迁移集合相同 → 只有三次 `process restart`（api/worker/intelligence），没有 `down`
- [ ] `current` 已等于目标 → 不构建、不重启，仍验证，退出 0
- [ ] 产物缺失且构建失败 → 指针不变，退出非 0
- [ ] 切换后验证失败（curl 非 0）→ 指针复位到旧产物并再次重启，退出非 0
- [ ] 带迁移的部署验证失败 → 指针保持新产物、不再重启，退出非 0
- [ ] `deploy.env` 缺失 → 从假 `/proc` 里抓到 6 个变量，文件 0600 且内容正确
- [ ] 缺变量且无可抓取进程 → 在切换指针前以退化状态退出，指针不变
- [ ] `runtime_parent` 位于 `.worktrees/` → 拒绝并退出 2

## Task 2: 脚本实现

**Files:** `tools/dev/deploy-dev.sh`、`package.json`

- [ ] 写 `deploy-dev.sh`：`usage`/`--help`、参数校验、工具检查、路径守卫、`flock`、`git fetch` + 定版、`deploy.env` 解析与抓取、构建或复用产物、迁移集合比对、原子切指针、按需 `down`/`up -D` 或三次 `restart`、readiness 轮询、六项验证、失败复位
- [ ] `package.json` 加 `"deploy:dev": "bash tools/dev/deploy-dev.sh"`
- [ ] `bash -n` 语法检查 + `pnpm test:pc` 全绿
- [ ] 对真实旧产物跑一次 `sha256sum --check` 确认校验逻辑与真产物匹配

## Task 3: 文档

- [ ] `development-and-deploy.md`：新增「一键部署」小节（命令、6 步语义、`deploy.env`、失败行为、不做的事）
- [ ] `README.md` 与 `process-compose.yaml` 注释各加一行入口
- [ ] 结构检查：`grep -rn "deploy:dev" README.md docs process-compose.yaml package.json` 五处命中一致

## 验证

- `pnpm test:pc`（含新场景，输出留存）
- `bash -n tools/dev/deploy-dev.sh`
- 真机演练：合并后从主检出跑 `pnpm deploy:dev`，核对 REVISION/SHA256SUMS/进程 exe/HTTPS
