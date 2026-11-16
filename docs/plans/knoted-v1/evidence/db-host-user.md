# 数据库改由宿主用户直接创建（Issue #66）

状态：实施完成；数据库属主迁移与实例重启是交付最后一步，完成后在本文件追加实测结果。

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
| 本地门禁 | `gates-dbowner.log`：Rust 125 / 前端 54 / 20+ test:pc / typecheck / lint / build / api / `git diff --check` |
| `nix eval …systemd.services`（knoted*） | `["knoted-https-reload"]`（`knoted-db` 已消失） |
| boot 候选 | 见 host 证据（`/data/knoted-dev/tasks/pcwrap/host-evidence/`），非全机 switch |

## 数据库属主迁移与实例重启

待执行：备份 `knoted_dev` → `ALTER DATABASE knoted_dev OWNER TO xingjian` + `REASSIGN OWNED BY knoted_dev TO xingjian` → `DROP ROLE knoted_dev` → 用默认连接串 `pnpm pc down && pnpm pc up -D` → readiness/数据/共享服务核对。

## 回退

重建角色并把属主与对象转回：

```sql
CREATE ROLE knoted_dev LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
ALTER DATABASE knoted_dev OWNER TO knoted_dev;
-- 在库内：REASSIGN OWNED BY xingjian TO knoted_dev;   -- 仅回退本次转移的对象
```

并把 `DATABASE_URL`/模板改回旧值；数据不丢失。
