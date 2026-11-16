# ADR 0004: 开发/测试与生产数据库由宿主用户直接创建

- Date: 2026-11-16
- Status: Accepted
- Decision owner: xingjianxu（仓库 owner）
- Confirmation: 2026-11-16 会话中人工直接指示「删除 knoted-db 服务，直接使用 xingjian 用户创建和管理开发测试数据库即可，后续生产环境也是直接使用 xingjian 用户创建对应数据库」。
- Related issues: [#66](https://github.com/xingjianxu/knoted/issues/66)
- Related PRs: 见 Issue #66 的合并记录
- Supersedes: [ADR 0003](0003-test-instance-process-management.md) 中「PostgreSQL 角色/库 provisioning 由 `/etc/nixos` 管理」的部分（其余仍然有效）
- Superseded by: 无

## 背景与问题

[ADR 0003](0003-test-instance-process-management.md) 把测试应用的进程迁到 pcwrap，但保留 `/etc/nixos` 的 `knoted-db.service`：以 `postgres` 身份创建 `knoted_dev` 角色与库，并断言「可登录、非超级用户、不能建库/建角色、库属主匹配」。

实际使用后认为这层声明多余：

1. 本机 `xingjian` 已是 PostgreSQL 超级用户（`pg_roles.rolsuper = t`），`createdb`/`psql` 直接可用，本机使用 trust 认证、没有独立凭据文件；专用角色不提供实际隔离。
2. 它带来一个额外的 NixOS 单元、一次开机 oneshot、以及一条跨仓库耦合（角色名同时出现在 host 单元、pcwrap 默认连接串、迁移失败提示里）。
3. 开发/测试库经常被重建（清空测试数据、换机器），声明式 provisioning 并不能降低风险，反而在「库已存在但属主被改」时让整个 `multi-user.target` 失败。

## 决策内容

1. **数据库由宿主用户 `xingjian` 直接创建与管理**：`createdb knoted_dev`（开发/测试）与将来的 `createdb <prod-db>`（生产）都以该用户执行；连接串是 `postgres://xingjian@127.0.0.1:5432/<db>`，沿用本机 trust 认证。
2. **删除 `knoted-db` 服务与 `features.server.knoted.app.instance.enable` 选项**；`/etc/nixos` 不再声明数据库角色/库，也不再断言其权限。
3. **schema 仍由应用管理**：迁移是 pcwrap 编排里的 `migrate` 进程（`sqlx migrate run`），与建库解耦；库不存在时迁移失败并在日志里给出 `createdb -h 127.0.0.1 knoted_dev` 命令。
4. **不引入数据库密码或凭据文件**；若将来生产需要独立凭据，另开决策，不在本 ADR 假定。
5. **迁移现有实例**：备份后把 `knoted_dev` 的属主与对象转给 `xingjian`，删除 `knoted_dev` 角色，再用新连接串重启实例。

不涵盖：不改生产部署时机（仍不部署生产）；不改迁移工具、schema 或 `sqlx` 用法；不改变「测试应用由 pcwrap 管理、生产应用将来由 NixOS 管理」的其余边界。

## 备选方案

| 方案 | 收益 | 成本 / 风险 | 结论 |
| --- | --- | --- | --- |
| 保留 `knoted-db` 声明式 provisioning | 新机器开箱即用 | 多一个单元与跨仓库耦合；属主漂移会让 target 失败；本机无实际隔离收益 | 否（用户明确要求删除） |
| 由宿主用户直接建库（本决策） | 边界最少；与 `knoted_test` 现状一致 | 新机器需手工 `createdb` 一步；无声明式校验 | **采纳** |
| 在 pcwrap `prepare` 里自动建库 | 免手工 | 应用进程需要建库权限（本机超级用户），把 host 权限带入应用侧 | 否 |
| 独立角色 + 密码文件 | 更贴近生产最小权限 | 本机 trust 认证下没有实际收益，增加凭据管理负担 | 否（将来生产另议） |

## 决策理由

- 本机信任模型下专用角色不产生隔离，只增加单元、oneshot 与命名耦合；`knoted_test`（测试库）本来就由 `xingjian` 拥有，实例与测试库统一到一个属主更简单。
- 建库是一次性动作，迁移是可重复动作；把一次性动作留在人手里、把可重复动作留在编排里，失败面更小。
- 删除后 `systemctl list-units '*knoted*'` 只剩 ACME 续期触发，host 仓库不再持有任何应用数据对象。

## 影响

- 正面：host 少一个单元与一个选项；角色/库/属主不再跨仓库重复声明；实例与测试库同属主，`psql`/`createdb` 直接可用。
- 负面：新机器或换库需要手工 `createdb`（一行命令，写进部署指南）；没有声明式权限校验，属主漂移只能靠迁移报错发现。
- 可移植性成本：默认连接串把用户名写死为 `xingjian`（迁移失败提示用 `id -un`，两者在非本机环境会不一致）；`createdb` 需要 `CREATEDB` 权限或超级用户。换机器时这两点都要按宿主用户重新确认。
- 已知取舍（明确接受）：连接用户是本机 PostgreSQL 超级用户，且依赖本机 trust 认证，**没有按服务最小权限**。这意味着本机上任何能以 `xingjian` 身份连接数据库的进程都拥有全库权限，审计粒度止于系统账号。若将来生产需要独立凭据或按服务授权，应另开决策（例如 per-service 角色 + 凭据文件），本 ADR 不预设其结论。
- 迁移与回退：迁移前 `pg_dump`；回退 = 重建 `knoted_dev` 角色、把属主与对象转回、恢复旧连接串。数据不丢失（`ALTER DATABASE ... OWNER` + `REASSIGN OWNED` 保留字节）。
- 安全：连接串无密码，仅本机回环；生产若需独立凭据另行决策。

## 验证与后续任务

已完成并复核，证据见 [db-host-user.md](../plans/knoted-v1/evidence/db-host-user.md)：单元与选项删除（`nix eval` 只剩 `knoted-https-reload`）、默认连接串与迁移失败提示、测试与门禁；备份与还原演练；`ALTER DATABASE` / `REASSIGN OWNED`（52 个对象）/ `DROP OWNED` / `DROP ROLE knoted_dev` 后库与表属主均为 `xingjian`、笔记数据保留、实例用默认连接串启动且 readiness 200、共享服务 PID 不变。

## 状态历史

- 2026-11-16：Proposed，Agent 起草。
- 2026-11-16：Accepted。人工在本会话直接指示删除 `knoted-db` 并统一由 `xingjian` 建库（Issue #66）。
