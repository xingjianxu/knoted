# Knoted 一致性备份

以实例的宿主数据库用户调用固定产物中的命令。先核对 API、Worker 与命令均来自相同支持维护准入的产物，数据库迁移已完成。首次切换仍使用停写备份，见 [ADR 0011](../../docs/decisions/0011-consistent-backup-and-isolated-restore.md) 和 [原实例升级回退要求](../../docs/decisions/0007-execution-exit-confirmation.md)。

```sh
knoted-api admin backup --config /data/knoted-dev/service/config/knoted.toml \
  --output /data/knoted-dev/service/backups/2026-11-19-manual
```

输出目录的父目录须已存在。命令创建私有目录，并拒绝已有输出、活跃资产目录、暂存目录和只读来源下的路径。实例数据库使用 [ADR 0004](../../docs/decisions/0004-database-owned-by-host-user.md) 的宿主用户连接，工具链提供兼容版本的 `pg_dump`。执行窗口中暂停手动迁移和外部 SQL 写入。

新写入和 Worker 领取在独占排队时暂停。已准入写入完成后，命令核对作业与外部服务，建立独立只读快照并归档数据库和登记图片。维护期间已有会话可继续读取，写接口返回 `503 dependency_unavailable`。正常或失败退出后释放本次独占锁。

仅在 `manifest.json` 存在时视为完成。归档由该清单、`database.dump` 和 `assets/<UUID>` 组成。数据库与各图片使用 SHA-256 和字节数校验。清单记录格式、应用版本、迁移校验和及恢复隔离所需的非密来源信息。使用固定发布包时另记录 `REVISION`。数据库归档含业务和认证记录，按实例数据权限保存。

缺图、摘要不符、锁已断开或无法核实外部退出时，命令以非零退出。保留诊断和已生成的部分文件，重试使用新的输出目录。若维护记录显示未确认退出，先排查对应进程、数据库和外部副作用。原实例升级回退所需的 Python SQLite 账本、退出凭证和暂存文件依 [ADR 0007](../../docs/decisions/0007-execution-exit-confirmation.md) 单独处理。
