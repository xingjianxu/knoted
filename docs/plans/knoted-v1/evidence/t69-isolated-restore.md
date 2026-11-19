# T69 隔离恢复验收

[PR #229](https://github.com/xingjianxu/knoted/pull/229) 合入 `5308378799a768ded33d26d1e08c37473f30eca5`，关闭 [Issue #226](https://github.com/xingjianxu/knoted/issues/226)。恢复操作见 [备份运行手册](../../../../tools/backup/README.md)，方案依据 [ADR 0011](../../../decisions/0011-consistent-backup-and-isolated-restore.md)。

## 门禁与审查

| 实际执行 | 结果 |
| --- | --- |
| `cargo fmt --check` | 分支与主线退出 0 |
| `cargo test --locked --workspace` | 分支与主线均为 534 项通过，0 失败，1 项既有忽略。包含 5 项 T69 专项 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 分支与主线退出 0 |
| `git diff --check` | 退出 0 |

测试使用隔离的官方 Meilisearch 1.56.1 和实际 PostgreSQL。T69 专项覆盖归档摘要和核验后归档替换、目标路径及端口占用、旧认证状态、业务历史以及重复恢复后派生状态清理。实施和逐轮复修经过独立规格、代码及集成审查，Cargo.lock 由主 Agent 核对。Python、Web、迁移和 OpenAPI 输入未变。

## 固定产物与隔离演练

测试提交 `b4179805b30f4b453856fa97a587bf9ddeeb6f3e` 与合并提交的 Git 树相同。按合并提交生成固定产物并验证 38,345 个文件，`SHA256SUMS` 摘要为 `24c193ea3c7bbf53bab5a2ca13b39fc7383b7c0c5f8346537cfb7bb2d583d6ec`。候选和固定主线各执行一次完整演练。主线演练的源和目标均用同一固定产物，独立 PostgreSQL 18.6 + pgvector 0.8.7，两个本地 Meilisearch 1.56.1 和独立 Python 状态。通过同一个临时 Caddy 为 `source.knoted.test:17700` 与 `restore.knoted.test:17701` 提供受信任的测试 HTTPS，使用 Chromium 的虚拟 CTAP2 认证器和相同 RP ID `knoted.test`。

| 实际操作 | 观察到的结果 |
| --- | --- |
| 源实例登录，登记合成附件、笔记快照及回收站与文件链接，运行 T68 备份 | 生成可恢复的数据库和资产归档 |
| 恢复摘要错误的归档 | 退出 1，目标数据库和根目录均未创建 |
| 恢复有效归档并启动目标 | 旧会话在源返回 200、在目标返回 401，旧恢复令牌与仪式被拒绝。同一虚拟 Passkey 在目标重新登录 |
| 读取历史、回收站、来源关联和附件 | 业务标识和历史匹配，附件 SHA-256 一致 |
| 从配置为只读的同一夹具目录重新采集并重建索引 | 目标中文全文检索命中，检索分块存在，实际 BGE-M3 生成 1024 维向量。源索引任务和原件核对一致 |
| 再向已存在目标恢复 | 拒绝覆盖 |

演练使用合成数据与虚拟认证器，覆盖实际 HTTP 和签名校验。目标发布完成后，另行启动目标服务并等待索引恢复。继承的未完成任务被取消并保留历史。原任务的重试请求被拒绝，目标通过新扫描和投影任务重建派生数据。

## HTTPS dev

切换前用上一固定版 `c94232c` 完成一次真实一致性备份，归档包括 11 张登记图片。通过 pcwrap 切换至 `5308378` 后，`https://wlab.132140.xyz:15601/health/ready` 和本机 intelligence 健康检查均返回 200。进程可执行路径与固定产物相符，Caddy、PostgreSQL、Strata 和 OnlyOffice 的 PID 保持一致。恢复验收使用上述隔离实例。

本地证据位于 `/data/knoted-dev/tasks/t69/evidence/`，包括 `t69-head-rust.log`、`t69-mainline-rust.log`、`t69-fixed-artifact.log`、`t69-mainline-restore.log` 和 `t69-dev-rollout.log`。
