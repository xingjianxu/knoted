# Knoted 测试实例改由 pcwrap 管理（实施计划）

> 执行技能：superpowers:executing-plans（主 Agent 实施 + 独立审查）。
> Issue：[#63](https://github.com/xingjianxu/knoted/issues/63)
> 决策记录：[ADR 0003](../decisions/0003-test-instance-process-management.md)
> 基线：master `2ceb833`（应用代码等同于运行中的 `7dc601a`）。

## 目标与边界

| 对象 | 管理方式 |
|---|---|
| Knoted 测试应用（API、专用 HTTPS 入口、配置渲染、迁移步骤） | `@bixlab/pcwrap` → process-compose，控制端口 `127.0.0.1:15600` |
| PostgreSQL（`knoted_dev` 角色/库 provisioning）、Strata、共享模型根、ACME 证书 | `/etc/nixos`（不变） |
| 生产实例 | 将来用 NixOS 管理，本次不部署、不启用 |

保持不变：公网入口 `https://wlab.132140.xyz:15601/`、API `127.0.0.1:15602`、数据库与实例数据目录，避免 Passkey 的 origin/RP ID 与既有数据失效。

不做：源码热更新/监视、Worker 常驻、Python 执行服务编排、生产部署、编排外部依赖。

## 端口与路径

| 用途 | 值 |
|---|---|
| pcwrap 控制端口 | `127.0.0.1:15600` |
| 测试 HTTPS 入口 | `wlab.132140.xyz:15601`（证书 `/var/lib/acme/wlab.132140.xyz/`） |
| 测试 API | `127.0.0.1:15602` |
| 实例根（数据/配置/日志） | `/data/knoted-dev/service/{config,state,assets,staging,logs,backups,bin}` |
| 固定产物运行根 | `/data/knoted-dev/runtime/<full-sha>/`（`bin/knoted-api`、`share/knoted/{web,migrations}`） |
| 当前运行指针 | `/data/knoted-dev/runtime/current` → 上述目录（原子切换） |
| 本任务构建/证据 | `/data/knoted-dev/tasks/pcwrap/` |

## 文件结构

| 文件 | 职责 |
|---|---|
| `pcwrap.config.json` | pcwrap 固定项：name/port/logFile/keepProject/env 模式 |
| `process-compose.yaml` | 4 个进程：`prepare`、`migrate`（一次性）、`knoted-api`、`knoted-https`；仅 dev profile |
| `tools/dev/pc/service.sh` | 子命令 `prepare`/`migrate`/`api`/`https`/`reload-https`/`install-launchers`：路径解析、配置与 Caddyfile 渲染、前置校验、exec 目标进程 |
| `tools/dev/pc/check-certificate.sh` | 证书存在/有效期/hostname 校验，失败输出原因并 exit 1；不打印私钥 |
| `tools/dev/pc/knoted.toml.tmpl` | 运行配置模板（无密钥；值由 `@…@` 占位符渲染） |
| `tools/dev/pc/Caddyfile.tmpl` | 独立 Caddy 实例（`auto_https off`、私有 admin socket、15601 反代 15602、静态站点） |
| `tests/pc/*.test.mjs` | 编排配置、渲染、证书校验与失败路径场景（`node --test`） |

`tools/dev/pc/service.sh` 中所有可变值来自环境变量，默认即上表测试实例取值；隔离验证用同一脚本指向临时根与临时端口，不接触正在运行的实例。

## 关键设计决定

1. **固定产物运行**：`tools/dev/build-artifact.sh` 从已提交 SHA 构建（Rust release + pnpm build），产物含 `REVISION` 与 `SHA256SUMS`；`prepare` 逐文件重算并比对后才允许启动，产物目录不含源码。
2. **配置由本仓库渲染**：`prepare` 把 `knoted.toml` 与 `Caddyfile` 渲染到 `/data/knoted-dev/service/config/`，不再依赖 Nix store 里的配置 symlink；`knoted` 系统账号不再需要，目录属主改为宿主用户。
3. **迁移不隐含在重启里**：`migrate` 是独立一次性进程，`knoted-api` 依赖其成功；`pnpm pc process restart knoted-api` 不触发迁移。
4. **HTTPS 归编排**：Caddy 作为编排进程运行（宿主用户身份，证书目录已有 `xingjian` 读 ACL），保留 `X-Knoted-Revision` 与证书校验；ACME 续期由 host 的 oneshot 调用稳定入口 `/data/knoted-dev/service/bin/knoted-https-reload`（本仓库脚本安装产生）。
5. **运行期工具来源**：`caddy`/`sqlx-cli`/`openssl` 由 host 提供；`service.sh` 把 boot profile 的 `sw/bin` 追加到 PATH 末尾（调用者 PATH 优先），保证「刚装 boot 候选、尚未 switch」时也能启动；reload 入口在 prepare 时烘焙绝对目录，不依赖调用者 PATH。
6. **生产保留**：`nix/module.nix`、`nix/package.nix` 保留为未来生产契约并在文档标注默认不启用；不在其上加测试实例的临时补丁。

## 任务

### 任务 1：编排与脚本（app）

- [x] `package.json` 增加 `"pc": "pcwrap"`、`@bixlab/pcwrap` 精确版本与 `test:pc`。
- [x] 写 `pcwrap.config.json`、`process-compose.yaml`、`tools/dev/pc/*`。
- [x] 写 `tests/pc/`：端口只回环、渲染值、证书三类失败、产物校验拒绝篡改。
- [x] `pnpm install --frozen-lockfile` 更新锁文件后 `pnpm test:pc` 全绿（8 个套件 / 20 个用例）。

### 任务 2：隔离验证（app）

> 测试不依赖调用者 PATH：`tests/pc/` 用临时「profile 目录」（真实 openssl + caddy stub）模拟 NixOS 提供的运行期工具，因此 `pnpm test:pc` 在默认环境即可通过。

- [x] 用临时根/临时端口跑 `prepare → migrate → api → https`，验证 readiness、静态站点与 TLS（15692/15693/15690，`X-Knoted-Revision` 与产物一致，`process restart`、reload 入口、`down` 均验证）。
- [x] 验证失败路径：产物篡改、符号链接、缺 REVISION、证书缺失/域名不符/非 PEM、端口被占、迁移失败不启动 API、缺运行期依赖、工作树守卫（`tests/pc/` 覆盖）。

### 任务 3：host 交接（`/etc/nixos`）

- [x] 保留 `knoted-db` provisioning、Strata、共享模型根、ACME；提供 `caddy`、`sqlx-cli`（沿用 sqlite 链接库 override）、`openssl`。
- [x] 配置层面停用 `knoted-api/https/migrate/prepare` 单元与 ACME reload 引用，改为 `knoted-https-reload.service` oneshot（`knotedUnits = [knoted-db, knoted-https-reload]`）。
- [x] `/data/knoted-dev/service` 属主声明为宿主用户；`nix eval` + 候选构建 + 非 Knoted 单元差异检查；boot profile 已安装（`plx4r25…`），未做全机 `switch`。运行态撤除旧单元属于任务 4。

### 任务 4：切换（运行态）

- [ ] 记录旧状态（单元、PID、运行 SHA、迁移版本、共享服务）、备份 `knoted_dev` 与 assets。
- [ ] 构建固定产物 `2ceb833`，`prepare` 校验通过。
- [ ] 停旧单元 → `pnpm pc up -D` → readiness/HTTPS/Passkey/CRUD 验证。
- [ ] 共享服务 PID/状态比对为空差异；记录证据。

### 任务 5：文档与收尾

- [ ] ADR 0003、本计划、`docs/plans/knoted-v1/development-and-deploy.md`、架构现状、AGENTS 端口段与运行入口。
- [ ] 本地门禁：`pnpm test:pc`、`pnpm typecheck/lint/test/build`、`cargo fmt --check`、`cargo test --workspace --locked`、`cargo clippy --all-targets -- -D warnings`、`git diff --check`。
- [ ] 独立审查（requesting-code-review）、PR、普通 merge、主线复验、Worktree/分支清理。

## 风险与回退

- **回退**：`pnpm pc down` → 恢复旧单元（`systemctl start knoted-prepare knoted-migrate knoted-api knoted-https`，旧包装仍指向 `7dc601a` 产物）。数据库不降级；本次无新迁移。
- **风险**：宿主用户与旧 `knoted` 账号并存期间目录属主混淆 → 切换前一次性 chown 并在文档写死属主。
- **风险**：ACME 续期与编排重启竞争 → reload 脚本加 `flock`，失败只记录不改证书。
- **停止条件**：证书不可读、端口无法释放、共享服务受牵连、迁移无备份时停止并保留成果。
