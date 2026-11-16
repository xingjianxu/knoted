# ADR 0003: 测试实例由 pcwrap 编排，NixOS 只保留外部依赖与未来生产

- Date: 2026-11-16
- Status: Accepted
- Decision owner: xingjianxu（仓库 owner）
- Confirmation: 2026-11-16 会话中人工直接指示「knoted 的测试服务，使用 `@bixlab/pcwrap` 工具管理，使用方式参考 `../receiptd` 项目，process compose 管理端口使用 15600。只有以后的生产环境，采用 nixos 的配置进行管理（暂时不部署生产环境）。依赖的外部服务，如 postgresql，使用 nixos 管理」，并在同会话批准本文与实施计划（[Issue #63](https://github.com/xingjianxu/knoted/issues/63)）。
- Related issues: [#63](https://github.com/xingjianxu/knoted/issues/63)
- Related PRs: 见 Issue #63 的合并记录
- Supersedes: 无
- Superseded by: 无
- 部分替代：[ADR 0004](0004-database-owned-by-host-user.md) 取代本文第 1 条中「PostgreSQL 角色/库 provisioning 由 `/etc/nixos` 管理」的部分；其余边界（测试应用由 pcwrap、生产应用将来由 NixOS、Strata/模型根/ACME/防火墙归 host）仍然有效。

## 背景与问题

T21（[证据](../plans/knoted-v1/evidence/t21-deployment.md)）把测试实例做成 `/etc/nixos` 的 `features.server.knoted.app`：固定 SHA + artifact 哈希 → Nix 包 → `knoted-db/prepare/migrate/api/https` 五个 systemd 单元，ACME 续期通过 `security.acme.certs.<domain>.reloadServices` 触发。

维护中出现三个问题：

1. **日常运维跨仓库**：应用每次更新都要在 `/etc/nixos` 改 pin、重建候选、做定向单元激活（全机 `switch` 已被 T21 证据证明会隐式重启共享 PostgreSQL/Strata，因此不能常规使用）。
2. **进程生命周期不归开发侧**：`pnpm pc process restart` 这类即时重启、日志查看在 systemd 下需要 sudo 和单元名知识；开发/测试实例的调试循环因此变长。
3. **配置来源分裂**：运行 TOML 由 host 的 `pkgs.writeText` 生成并 symlink 到 `/data`，模板、证书校验与编排知识同时散落在两个仓库。

同机其它项目（`receiptd`/`labtrack`/`thesisd`）已经用 process-compose 管理开发/测试进程，并有发布版包装器 `@bixlab/pcwrap`（固定控制端口与日志、`up` 自动 `--keep-project`、env 遮蔽提醒、子命令透传）。

## 决策内容

1. **管理边界**
   - Knoted 测试应用的进程（渲染运行配置与 Caddyfile、执行迁移、运行 API、运行独立 HTTPS Caddy）由 Knoted 仓库的 `@bixlab/pcwrap` + `process-compose` 管理；控制端口 **`127.0.0.1:15600`**，其他端口保持 15601（HTTPS）/15602（API）。
   - 外部依赖与基础设施仍由 `/etc/nixos` 管理：PostgreSQL 角色/库 provisioning（`knoted-db`）、Strata、机器级模型共享根（ADR 0001）、ACME 证书、防火墙、宿主账号与目录属主；并提供运行期可执行文件 `caddy`、`sqlx-cli`、`openssl`。
   - **生产实例**将来才由 `/etc/nixos` 的 `features.server.knoted.app.enable` 运行；`nix/module.nix`、`nix/package.nix` 作为该路径的契约保留，本次不启用、不部署。
2. **运行固定产物**：实例运行 `tools/dev/build-artifact.sh` 从已提交 SHA 构建的产物（`REVISION` + `SHA256SUMS`），`prepare` 逐文件重算摘要后才启动；不做源码热更新，稳定运行根在 `/data/knoted-dev/runtime/<sha>/`，指针 `current` 原子切换。
3. **配置归仓库**：运行 TOML 与 Caddyfile 由 `tools/dev/pc/*.tmpl` 渲染到 `/data/knoted-dev/service/config/`，值来自环境变量（默认即本机测试实例）。证书校验脚本移入本仓库（`tools/dev/pc/check-certificate.sh`），保留「缺失/过期/域名不符拒绝启动、无 HTTP 回退」。
4. **迁移不隐含**：`migrate` 是编排里的一次性进程，`knoted-api` 依赖其成功；重启 API 不触发迁移。
5. **续期 reload**：ACME 续期由 host 的 `knoted-https-reload.service`（oneshot）调用实例稳定入口 `<dataRoot>/bin/knoted-https-reload`；该入口由 `prepare` 安装，含 `flock`、证书校验与 `caddy reload`。host 只触发，不启动/停止应用。
6. **目录属主**：`/data/knoted-dev/service` 归宿主用户（编排由该用户运行）；`knoted` 系统账号保留给将来的生产实例。
7. **端口登记**：本机 `15600–15609` 归 Knoted（控制/HTTPS/API/Python/Strata 沿用既定分配），登记进 `@bixlab/pcwrap` 的端口表。

不涵盖：Worker 常驻与 Python 执行服务编排（后续任务）、生产部署与主机切换、源码热更新/文件监视、把数据库或模型服务纳入编排。

## 备选方案

| 方案 | 收益 | 成本 / 风险 | 结论 |
| --- | --- | --- | --- |
| 保持 T21 的 systemd 单元 | 零改动 | 日常更新跨仓库 + 需 sudo；配置来源分裂 | 否（用户明确要求改） |
| 用 pcwrap 管理测试应用（本决策） | 开发侧统一入口、日志/重启即时；host 只剩依赖与基础设施 | 需要一次无损切换；host 需提供可执行文件与续期触发 | **采纳** |
| 测试与生产都用 NixOS | 一致 | 日常调试仍受 sudo/单元名限制 | 否（用户指定仅生产用 NixOS） |
| 测试与生产都用 pcwrap | 单一心智模型 | 生产需要开机自启、最小权限、声明式回滚，pcwrap 面向交互式编排 | 否 |
| 测试用 docker/podman compose | 环境隔离 | 本机无容器运行时先例；模型/证书/GPU 挂载成本高 | 否 |

## 决策理由

- 与同机其它项目一致，`pnpm pc up -D / process list / restart / logs / down` 已是本机通用操作；`@bixlab/pcwrap` 是发布包，不需要复制手写包装脚本。
- host 侧保留的正是「跨应用共享、需要开机自启或 root 权限」的部分（数据库、证书、防火墙、账号），而 Knoted 应用进程不再需要 root。
- 固定产物 + 清单校验保留了 T21 的部署可信度（不接受未提交源码或篡改产物），切换只改「谁来拉起进程」。
- 保留 `nix/` 的生产契约，避免生产部署时重新设计。

## 影响

- 正面：应用更新只需 `git pull` + 重新构建产物 + `pc process restart`；host 仓库不再跟随每次应用发布改 pin；配置与证书校验进入应用仓库受审查。
- 负面：测试实例不再有开机自启（需手工或后续显式约定拉起）；进程日志由编排持有（`pc process logs`），不进 journal；host 仍需一次切换与一次 boot profile 更新。
- 迁移与回退：切换前备份 `knoted_dev` 与 assets，并把旧运行态（`/run/current-system`、五个单元、`config/knoted.toml` 的 store 链接、迁移版本）记录到任务证据。回退步骤（顺序固定，不能只 `systemctl start`）：
  1. `pnpm pc down`，确认 15600/15601/15602 释放；
  2. 从记录的旧 system 闭包恢复 `knoted-{prepare,migrate,api,https}.service` 软链到 `/etc/systemd/system` 与 `multi-user.target.wants`，`systemctl daemon-reload`；
  3. 把 `/data/knoted-dev/service` 属主改回 `knoted:knoted`，并把 `config/knoted.toml` 恢复为记录的 store 链接（切换时被渲染成编排用户拥有的真实文件，旧单元读不到）；
  4. `chown -R knoted:knoted /data/knoted-dev/service`（含 `config`/`state`/`logs`），然后 `systemctl start knoted-prepare knoted-migrate knoted-api knoted-https`，核对 readiness 与版本头；
  5. 恢复续期链路：删除实例安装的 `bin/knoted-https-reload`（或把 `security.acme.certs.<domain>.reloadServices` 改回 `knoted-https.service` 并重建 system），否则新 boot 配置里的 `knoted-https-reload.service` 仍会命中旧入口、指向已停止的 pcwrap socket，续期后旧 Caddy 不会 reload；最后 `systemctl reload knoted-https`。
  本次无新迁移，数据库不降级。
- 安全：编排以宿主用户运行，证书依赖既有 ACL（`xingjian` 可读 acme 目录）；密钥不入仓库/日志；HTTPS 仍是唯一公网入口，15600 只监听回环。

## 验证与后续任务

已完成（证据：[pcwrap 迁移证据](../plans/knoted-v1/evidence/pcwrap-dev-service.md)）：编排配置与失败路径测试、固定产物校验、临时根/临时端口上的端到端验证（readiness、TLS、版本头、重启不重跑迁移、reload 入口在受限 PATH 下可用），以及 host 侧 `nix eval`、候选构建、非 Knoted 单元差异与 boot profile 安装。

真实切换是本次交付的最后一步，结果追加到同一证据文件：撤除旧单元、宿主用户运行、HTTPS/readiness/受保护路由、共享服务 PID 比对、`knoted-https-reload.service` 实际触发。真实硬件 Passkey 人工验证与 CRUD 冒烟沿用既有跨任务限制（既有会话/凭据不受影响），未在本决策中新增声明。

后续：生产实例（T70）启用时重新给出固定 revision/artifact，并按 ADR 0002/0001 的既有约定接入。

## 状态历史

- 2026-11-16：Proposed，Agent 起草。
- 2026-11-16：Accepted。人工在本会话直接指示管理边界并批准实施计划（Issue #63）。
