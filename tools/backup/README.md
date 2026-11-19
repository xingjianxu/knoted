# Knoted 一致性备份

以实例的宿主数据库用户调用固定产物中的命令。先核对运行中的 API、Worker、Python intelligence 服务与备份命令均来自同一固定修订，并确认数据库迁移已完成。用 API、Worker 的进程可执行文件路径及 Python 服务进程实际启动的解释器路径（命令行第一个参数）定位各自产物，再比对产物的 `share/knoted/REVISION`。旧版 Python 服务的健康响应字段与新版相同，其响应可能遗漏尚未确认退出的执行。首次切换仍使用停写备份，见 [ADR 0011](../../docs/decisions/0011-consistent-backup-and-isolated-restore.md) 和 [原实例升级回退要求](../../docs/decisions/0007-execution-exit-confirmation.md)。

```sh
knoted-api admin backup --config /data/knoted-dev/service/config/knoted.toml \
  --output /data/knoted-dev/service/backups/2026-11-19-manual
```

输出目录的父目录须已存在。命令创建私有目录，并拒绝已有输出、活跃资产目录、暂存目录和只读来源下的路径。实例数据库使用 [ADR 0004](../../docs/decisions/0004-database-owned-by-host-user.md) 的宿主用户连接，工具链提供兼容版本的 `pg_dump`。执行窗口中暂停手动迁移和外部 SQL 写入。

新写入和 Worker 领取在独占排队时暂停。已准入写入完成后，命令核对作业与外部服务，建立独立只读快照并归档数据库和登记图片。维护期间已有会话可继续读取，写接口返回 `503 dependency_unavailable`。正常或失败退出后释放本次独占锁。

命令成功退出且 `manifest.json` 存在时，归档才完成。归档由该清单、`database.dump` 和 `assets/<UUID>` 组成。数据库与各图片使用 SHA-256 和字节数校验。清单记录格式、应用版本、迁移校验和及恢复隔离所需的非密来源信息。无论检索是否启用，清单都记录其开关、来源实例地址和配置的索引名称，供恢复时核对隔离目标。使用固定发布包时另记录 `REVISION`。数据库归档含业务和认证记录，按实例数据权限保存。

缺图、摘要不符、锁已断开或无法核实外部退出时，命令以非零退出。保留诊断和已生成的部分文件，重试使用新的输出目录。若目录同步与撤销发布均未能确认，命令报告发布不确定。隔离该目录，即使其中可见清单也应使用新目录重试。若维护记录显示未确认退出，先排查对应进程、数据库和外部副作用。原实例升级回退所需的 Python SQLite 账本、退出凭证和暂存文件依 [ADR 0007](../../docs/decisions/0007-execution-exit-confirmation.md) 单独处理。

## 恢复到独立实例

使用由成功返回的备份命令生成、来源可信的本地归档。准备两个独立的 TOML 文件。目标应用配置沿用普通 API 和 Worker 的配置格式。恢复目标描述文件只含两个绝对路径，例如：

```toml
instance_root = "/srv/knoted-rehearsal/run-01"
app_config = "/srv/knoted-rehearsal/config/run-01.toml"
```

在目标应用配置中设置新数据库名，使用与原实例相同的 RP ID，并使用该 RP ID 域名或其子域名下的独立 HTTPS origin。目标 API 使用空闲的 loopback 监听地址。`assets.root`、`files.staging_root` 分别使用实例根下的 `assets`、`staging`。实例根的父目录须存在，实例根须为新路径。文件源仅可指向获授权的只读目录。目标 PostgreSQL 须支持 pgvector，操作者使用本机账号连接并具有建库权限。

为演练单独配置 Python 服务和空的 Meilisearch 实例，分配独立的 loopback 端口、两个新的索引 UID 和独立生成的搜索凭据。旧归档不保存源凭据，密钥独立性由供应过程保证。恢复程序读取目标 Meilisearch 的索引清单和排队中、处理中的任务，均须为空。恢复期间目标 API 和 Python 端口须空闲，命令保留这两个监听地址直至结束。启动 Python 服务时将 `KNOTED_INTELLIGENCE_STATE_DIR` 设为新实例根下的 `python`，`KNOTED_INTELLIGENCE_STAGING_DIR` 设为新实例根下的 `staging`。模型共享只读。

```sh
knoted-api admin restore --input /srv/backups/complete-01 --target /srv/knoted-rehearsal/config/target.toml
```

命令验证清单、数据库和登记图片的摘要，在新目录和私有临时数据库恢复，核对迁移、图片及引用，撤销旧授权并冻结旧任务。最后重命名数据库以启用目标名。命令不启动服务。失败时保留的 `knoted_install_*` 数据库和新建目录供离线诊断，再次演练使用新路径。成功后以目标配置启动独立服务，Worker 从正常任务队列重新发布全文、词法及向量索引，文件扫描负责重新确认源文件。用专用 HTTPS origin 和既有 Passkey 登录验收。
