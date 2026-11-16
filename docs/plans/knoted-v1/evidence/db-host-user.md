# 数据库改由宿主用户直接创建（Issue #66）

状态：已完成（PR [#67](https://github.com/xingjianxu/knoted/pull/67) 普通 merge 至 master `46d8eb9`；属主迁移与实例重启见文末）。

决策：[ADR 0004](../../../decisions/0004-database-owned-by-host-user.md)（部分替代 [ADR 0003](../../../decisions/0003-test-instance-process-management.md) 的数据库 provisioning 归属）。

## 变更

| 侧 | 内容 |
|---|---|
| app | `tools/dev/pc/service.sh` 默认连接串改为 `postgres://xingjian@127.0.0.1:5432/knoted_dev`；迁移失败时打印 `createdb -h 127.0.0.1 knoted_dev`；`tests/pc/` 新增「默认连接串用宿主用户」与「失败给建库命令」用例 |
| host（`/etc/nixos`） | 删除 `systemd.services.knoted-db` 与 `features.server.knoted.app.instance.enable`；生产分支的连接串同步为 `xingjian`；`hosts/wks-lab` 只剩 `app.enable = false` |
| 文档 | ADR 0004、ADR 0003 部分替代说明、ADR 索引、开发与部署指南、AGENTS/README/架构现状/计划入口 |

## 验证（代码与配置）

见门禁日志与 `nix eval` 输出：

| 检查 | 结果 |
|---|---|
| `pnpm test:pc` | 9 套件全绿（含新增两个用例） |
| 本地门禁 | `/data/knoted-dev/tasks/pcwrap/logs/gates-dbowner2.log`：Rust 125 / 前端 54 / `test:pc` 21 / typecheck / lint / build / api / `git diff --check` |
| `nix eval …systemd.services`（knoted*） | `["knoted-https-reload"]`（`knoted-db` 已消失），输出存档 `/data/knoted-dev/tasks/pcwrap/host-evidence/nix-eval-knoted-units.json` |
| boot 候选 | `wiqsw3wd…`（host `0e95cde`），日志在 `/data/knoted-dev/tasks/pcwrap/host-evidence/boot-install.log`；非全机 switch |

## 数据库属主迁移与实例重启（2026-11-16 22:1x）

备份：`/data/knoted-dev/tasks/pcwrap/evidence/knoted_dev-pre-dbowner-20261116T135656Z.dump`（46990 B），还原演练通过（独立库 10 表 / 1 行笔记 / 迁移 1—5，演练库已删除）。

迁移（以 `xingjian` 身份，`ON_ERROR_STOP=1`）：

```sql
ALTER DATABASE knoted_dev OWNER TO xingjian;          -- ALTER DATABASE
REASSIGN OWNED BY knoted_dev TO xingjian;             -- 52 个对象（10 表 + 索引/序列）
DROP OWNED BY knoted_dev;                             -- 撤销残留授权
DROP ROLE knoted_dev;                                 -- DROP ROLE
```

实例重启：主检出 `pnpm pc down && pnpm pc up -D`（`prepare` 重新渲染配置，默认连接串即 `postgres://xingjian@127.0.0.1:5432/knoted_dev`）。

| 检查 | 结果 |
|---|---|
| `pg_database` 属主 | `knoted_dev → xingjian` |
| public 表属主 | 10/10 属 `xingjian` |
| `notes` 行数 | 1（切换前同为 1，未丢数据） |
| `pg_roles` 中 `knoted_dev` | 0 行（角色已删除） |
| HTTPS `/health/ready` / `/` | 200 / 200 |
| `X-Knoted-Revision` | `4a546b2f0efdc07a3c9fe0527654a89b6f8c4eb4`（应用产物；本次提交只改 dev 脚本与文档，未重建产物） |
| `/api/v1/auth/session` | 401 |
| 共享服务 | `postgresql` 1167036 / `caddy` 1706466 / `strata` 1180032，PID 与迁移前一致 |
| 应用进程身份 | `xingjian`，`/data/knoted-dev/runtime/current/bin/knoted-api` |
| `systemctl list-unit-files '*knoted*'` | 仅 `knoted-https-reload.service` |

## 回退

重建角色并把属主与对象转回：

```sql
CREATE ROLE knoted_dev LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
ALTER DATABASE knoted_dev OWNER TO knoted_dev;
-- 在库内：REASSIGN OWNED BY xingjian TO knoted_dev;   -- 仅回退本次转移的对象
```

并把 `DATABASE_URL`/模板改回旧值；数据不丢失。
