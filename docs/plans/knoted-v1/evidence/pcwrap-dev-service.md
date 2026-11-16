# 测试实例 pcwrap 迁移证据（Issue #63）

状态：已完成（PR [#64](https://github.com/xingjianxu/knoted/pull/64) 普通 merge 至 master `4a546b2`；真实切换与主线验证见文末）。

计划：[2026-11-16-pcwrap-test-service.md](../../2026-11-16-pcwrap-test-service.md)；
决策：[ADR 0003](../../../decisions/0003-test-instance-process-management.md)。

## 范围

- 测试应用的进程（配置渲染、迁移、API、独立 HTTPS）从 `/etc/nixos` 的 systemd 单元迁到本仓库的 `@bixlab/pcwrap` + process-compose，控制端口 `127.0.0.1:15600`。
- PostgreSQL 角色/库 provisioning、Strata、机器级模型共享根、ACME 证书、防火墙仍由 `/etc/nixos` 管理；生产实例（`app.enable`）保持关闭。
- 端口与地址不变：HTTPS 15601、API 127.0.0.1:15602、库 `knoted_dev`。

## 本仓库产物

| 文件 | 作用 |
|---|---|
| `pcwrap.config.json` | name/port 15600/日志/keep-project/env 遮蔽提醒 |
| `process-compose.yaml` | `prepare` → `migrate` → `knoted-api` → `knoted-https`，无 prod profile |
| `tools/dev/pc/service.sh` | 产物清单校验、模板渲染、目录准备、证书/端口校验、迁移、启动、探测、ACME reload 入口安装 |
| `tools/dev/pc/check-certificate.sh` | 证书可读/生效/未过期/hostname 匹配，失败即拒绝启动 |
| `tools/dev/pc/{knoted.toml,Caddyfile}.tmpl` | 运行配置模板（无密钥） |
| `tests/pc/service.test.mjs` | 8 个场景套件 / 20 个用例（配置、渲染、篡改、符号链接、缺 REVISION、证书、端口、迁移失败、缺依赖、工作树守卫、密码掩码、默认路径） |

## 隔离验证（未接触运行实例）

在 `/data/knoted-dev/tasks/pcwrap/isolated`、控制端口 15690、API 15692、HTTPS 15693 上以现有产物 `7dc601a` 跑通整链：

| 检查 | 结果 |
|---|---|
| `pcwrap doctor`（仓库根） | 配置合法、process-compose v1.122.0、控制端口 15600 空闲、跨项目端口扫描通过 |
| 控制端口监听地址 | `ss -ltn` 实测 `127.0.0.1:15690`（隔离）/ 15600（真实），未见 `0.0.0.0` 绑定 |
| `pc up -D` | prepare（产物清单逐字节一致）/migrate（1—5 已安装）/api/https 全部起来 |
| `/health/ready` | API 200、HTTPS 200 |
| `X-Knoted-Revision` | `7dc601a6afb15c977972f0179c65f51103c33251`（等于产物 REVISION） |
| `process restart knoted-api` | 重启后 readiness 仍 200，未重跑迁移 |
| `bin/knoted-https-reload`（`env -i`，PATH 仅取单元 `path=` 声明） | 证书校验 + `caddy reload` 成功，HTTPS 仍 200；入口不写文件，可在 `ProtectSystem=strict` 下运行。独立复核另以 transient unit 按单元真实属性（`ProtectSystem=strict`、`PrivateTmp`、无登录 PATH）复现 status 0/SUCCESS |
| `pc down` | 15690/15692/15693 全部释放 |

失败路径由 `tests/pc/` 断言：篡改产物（改 `index.html`）拒绝且不创建实例目录、缺少证书拒绝、hostname 不匹配拒绝、端口被占拒绝、`sqlx` 失败原样传播退出码、缺少 `sqlx` 给出明确错误、`paths` 默认值指向本机测试实例。

## 本次门禁（app worktree）

命令与输出见 `/data/knoted-dev/tasks/pcwrap/logs/gates.log`（本机检查是唯一合并门禁）：

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | exit 0 |
| `cargo test --workspace --locked` | 125 passed，0 failed（1 既有 ignored） |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `pnpm install --frozen-lockfile` | exit 0（含 `@bixlab/pcwrap@0.1.0`） |
| `pnpm api:check`、`pnpm api:verify` | exit 0 |
| `pnpm typecheck`、`lint` | exit 0 |
| `pnpm test` | 54 passed |
| `pnpm test:pc` | 8 套件 / 20 用例全绿（默认环境无 `openssl`，测试用临时 profile 目录提供工具） |
| `pnpm build` | exit 0 |
| `git diff --check` | exit 0 |

## host 侧（`/etc/nixos`，独立提交）

- `228d8d8` feat(knoted): 测试实例应用撤出 NixOS，只保留依赖与续期触发
- `94dd141` fix(knoted): knoted-db 由 multi-user.target 拉起
- `9498093` fix(knoted): 续期 reload 单元显式声明解释器与工具（`path = [bash coreutils gnused openssl caddy]`）

求值与构建证据在 `/data/knoted-dev/tasks/pcwrap/host-evidence/`：

- `nix eval`：`knotedUnits = [knoted-db, knoted-https-reload]`；`reloadServices = [caddy.service, knoted-https-reload.service]`；防火墙含 15601；systemPackages 含 caddy/sqlx-cli/openssl；tmpfiles 将 `/data/knoted-dev/service*` 声明为 `0750 xingjian users`。
- 候选构建 `1vmr7b6f…`（`candidate-build.log`）：普通 `pkgs.sqlx-cli` 会因 `-lsqlite3` 链接失败，沿用既有 override 后通过。
- 非 Knoted 单元差异（`unit-list.diff`、`content-diff.list`）：仅 `knoted-api/https/migrate/prepare` 移除、`knoted-https-reload` 新增；其余差异为激活/系统路径哈希（tmpfiles、home-manager 生成、acme-postrun 路径），共享服务（postgresql、caddy、strata）单元内容一致。
- boot profile 逐次安装为 `plx4r25…`（首次）→ `zgh9s6zg…`（补 `wantedBy`）→ `200mywwgi…`（补续期单元 `path`，当前生效），均只做 boot，不执行全机 `switch`（`boot-install.log`）。

## 真实切换（2026-11-16 21:24—21:27）

固定产物：`4a546b2f0efdc07a3c9fe0527654a89b6f8c4eb4`（合并后的 master），`tools/dev/build-artifact.sh` 构建到 `/data/knoted-dev/tasks/pcwrap/artifact-4a546b2f0efd`，安装为 `/data/knoted-dev/runtime/4a546b2f0efd/` 并由 `current` 指向（`artifact-build.log`）。

顺序与结果（`evidence/cutover-pre.txt`、`cutover-post.txt`）：

1. 备份：`pg_dump --format=custom knoted_dev`（46990 B，`pg_restore --list` 校验）写入 `service/backups/20261116T125428Z-pcwrap/`，assets 为空（`assets.count`）。
2. 撤除旧单元：`knoted-https/api/migrate/prepare` 停止、`disable`、删除 `/etc/systemd/system` 与 `multi-user.target.wants` 软链；`/etc/systemd/system` 只剩 `knoted-db.service` 与 `knoted-https-reload.service`（从 boot 候选重新链接）。
3. 实例目录 `chown -R xingjian:users`，旧的 Nix store 配置链接删除，由 `prepare` 重新渲染为 `0600` 真实文件。
4. `pnpm pc up -D`（从主检出 `/wrd/prj/knoted` 运行，非工作树）：`prepare` 校验产物清单、渲染配置与 Caddyfile、校验证书与端口；`migrate` 报告 1—5 已安装；API 与 HTTPS 就绪。

切换中发现并修正一处操作错误：首次 `cutover.sh` 用了 `sudo bash`，导致 pcwrap 以 root 启动、`prepare` 写出 root 拥有的 `bin/`，续期单元以 `xingjian` 执行时 permission denied（126）。已 `pcwrap down` 停止该实例、`chown -R xingjian:users` 并把脚本改为“以宿主用户运行、内部按需 sudo”后重跑；此后一切以宿主用户身份运行。

| 检查 | 结果 |
|---|---|
| HTTPS `/health/ready` | 200 `{"status":"ready"}` |
| HTTPS `/`（静态站点） | 200，返回应用 index |
| `X-Knoted-Revision` | `4a546b2f0efdc07a3c9fe0527654a89b6f8c4eb4`（= 运行产物） |
| `/api/v1/auth/session`（未登录） | 401（认证仍生效） |
| 明文 HTTP 到 15601 | 400（无 HTTP 回退） |
| 监听地址 | 15600 `127.0.0.1`、15602 `127.0.0.1`、15601 `*`（公网入口） |
| `systemctl start knoted-https-reload` | exit 0（真实单元属性：`ProtectSystem=strict`、`PrivateTmp`、无登录 PATH） |
| `pnpm pc process restart knoted-api` | 重启后 readiness 200，`sqlx migrate info` 仍为 1—5（未隐式迁移） |
| `pnpm pc down` → `pnpm pc up -D` | 端口释放后完整重建成功，HTTPS 200、版本头不变 |
| 共享服务 | `postgresql` PID 1167036、`caddy` 1706466、`strata` 1180032，切换前后一致 |
| 旧单元 | 四个均 inactive，软链已移除 |

## 主线验证

- PR #64 普通 merge 至 master `4a546b2`；Issue #63 随合并关闭。
- 切换后主线门禁与运行读回：见收尾 PR 的复验记录（前端 54、Rust 125、`test:pc` 20 用例、typecheck/lint/build/api 与 `git diff --check`）。

## 已知限制

- 未加「证书已过期」的自动化用例：需要真实过期 fixture，`openssl req -days 0` 不支持；脚本自身的可读/生效/hostname/非 PEM 分支有测试，过期判定依赖 `openssl x509 -checkend 0`（第三方行为）。

- 测试实例不再开机自启（ADR 0003 的取舍）；重启后需手工 `pnpm pc up -D`。
- ACME 续期的 reload 引用要在新 boot 配置生效后更新；运行中的 acme 单元仍持旧引用，续期发生在下次开机之后才是新链路。
- 真实硬件 Passkey 人工验证仍是既有跨任务限制；切换验收只覆盖 HTTPS、readiness 与匿名/受保护路由行为。
