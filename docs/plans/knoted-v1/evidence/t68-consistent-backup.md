# T68 一致性备份验收

维护者确认的 [ADR 0011](../../../decisions/0011-consistent-backup-and-isolated-restore.md) 通过 [PR #221](https://github.com/xingjianxu/knoted/pull/221) 合入。T68 实现 [PR #227](https://github.com/xingjianxu/knoted/pull/227) 合入 `c94232cb7f7cae922a08569ff411a9b8ca024713`，自动关闭 [Issue #220](https://github.com/xingjianxu/knoted/issues/220)。

## 门禁与审查

| 实际执行 | 结果 |
| --- | --- |
| `cargo fmt --check` | 任务分支与主线均退出 0 |
| `cargo test --locked --workspace` | 任务分支与主线均为 537 项通过，0 失败，1 项既有忽略 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 任务分支与主线均退出 0 |
| `ruff check services/intelligence` 和 `ruff format --check services/intelligence` | 退出 0，30 份文件符合格式 |
| `python -m pytest -q services/intelligence/tests` | 177 项通过，保留 9 项第三方弃用提示 |
| `git diff --check` | 退出 0 |

Rust 门禁使用隔离的官方 Meilisearch 1.56.1 和真实 PostgreSQL，并运行了真实 OCR 和 Office 用例。首次隔离使用的 Nix Meilisearch 构建缺少中文分词。使用项目既有验收构建后，中文检索回归和全量任务分支检查通过。CLI 专项核验实际快照中的在途提交及其图片、同长度损坏、非 ready 引用、协调连接失联、未决外部请求和关闭检索后保留来源信息。维护准入、外部证明和归档分别经过独立审查及专项复验，并通过整体集成复审。

## 固定产物与 HTTPS dev

部署源提交 `75ff4caac8fec444479f3af3fd45604b71c9d56d` 与合并提交的 Git 树相同。按合并 SHA 固定产物后校验 38,345 个文件，`SHA256SUMS` 摘要为 `6affb6feb9a62869b3ca6636ac8738d6d8997101bd2fd33f574f01123ec14e9c`。首次切换停本实例 API、Worker 与 Python intelligence，核对终止证明并备份 PostgreSQL、图片、SQLite 账本及其凭证。保留原实例回退备份。切换后通过进程可执行文件核对 API、Worker 与实际 Python 解释器都运行固定 SHA，HTTPS readiness 和 intelligence 健康检查通过。共享 Caddy、PostgreSQL、Strata 和 OnlyOffice 进程保持同一 PID。

使用一份临时合成图片和带快照的笔记完成 HTTPS 验收：

- 真实备份排队等待独占时，已登录读请求返回 200，写请求返回 503。
- 成功归档的 11 个登记图片及 PostgreSQL custom dump 的字节数和摘要均匹配。实际 `pg_restore` 查询档案中的图片引用成功。
- 临时挪开合成图片后，备份退出 1，失败目录没有完成清单。图片复位后，HTTPS 写请求返回 200。合成笔记移入回收站，临时会话撤销。

本地证据位于 `/data/knoted-dev/tasks/t68/evidence/`。主要记录为 `t68-head-final-rust.log`、`t68-head-python.log`、`t68-mainline-rust.log`、`t68-fixed-artifact.log`、`t68-dev-cutover.log` 和 `t68-dev-acceptance.log`。恢复演练见 [T69](../tasks/T69.md)。
