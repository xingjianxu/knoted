# T25 历史快照与 Worker 验收

Issue [#78](https://github.com/xingjianxu/knoted/issues/78)，实现 `8429aedc57bcb526ea09ac10e9ac8f1fd1388944`；PR [#79](https://github.com/xingjianxu/knoted/pull/79) 普通 merge 主线 `337090e67d2494424510e454fdfc514d7d889e6a`。实施与主线均已验证，dev 同时运行该固定产物的 API/Worker；实施 Worktree/分支清理完成。完整计划与RED/GREEN见[实施清单](../../2026-11-17-t25-history.md)。

## 实现与独立审查

Snapshot ID 与内容版本分离、首次未快照变化持久化计时、持续编辑不延后五分钟截止、幂等 flush/启动补存、恢复先保存当前再写新版本、90天清理保留当前版本。现有 T03 HTTP DTO 不变，唯一新增迁移0006，旧迁移及锁文件未改。API/Worker 采用同一固定产物，pcwrap四进程；无宿主或生产修改。

`requesting-code-review` 独立审查报告 `/data/knoted-dev/tasks/t25/review.md`：无Critical、Important或Minor缺陷，可合并。锁顺序、SKIP LOCKED、CAS与回滚、分页、回收站和清理经代码复核。非阻断覆盖建议：启动首轮失败后恢复、多批次含软删除清理、restore/prune确定性屏障；这些场景未声称已测试。

## 真实门禁

实施者、主 Agent、主线各自运行全量 Rust 门禁和 pc 场景；实施者另运行过滤的 `t25_` 命令，主 Agent/主线的全量运行也执行这14个用例（完整输出 `/data/knoted-dev/tasks/t25/`）：

| 命令 | 结果 |
| --- | --- |
| cargo fmt --check | exit0，无输出 |
| cargo test --workspace --locked | exit0，153passed0failed，1既有ignored doctest |
| cargo test --workspace --locked t25_（实施者） | exit0，14passed（HTTP5/core6/真实Worker3） |
| cargo clippy --locked --all-targets -- -D warnings | exit0，无告警 |
| pnpm test:pc | exit0，24passed0failed |
| shell语法、YAML/Markdown结构与本地链接、diff/cached检查 | exit0 |

实际 Worker 测试等待30秒下一次轮询，确认不重启也会保存到期内容；CLI参数回显先RED后修复。HTTP反向恢复断言标题及不同中文段落正文，测试未用持久化mock。

## 一致备份、迁移与运行验收

停止 Knoted API 后，确认该dev数据库其他连接0；备份为 `/data/knoted-dev/service/backups/20261117T010317Z-t25`（0700，数据库custom dump及assets，摘要与pg_restore目录检查通过，不提交备份内容）。当时旧编排尚无Worker，不停止共享PostgreSQL或Caddy。

已提交SHA构建的 `/data/knoted-dev/runtime/8429aedc57bcb526ea09ac10e9ac8f1fd1388944` 含API、Worker、网页与迁移；完整SHA256SUMS校验通过。pc down后原子切换current并up新四进程编排，迁移1—6全部success；API/Worker真实可执行路径均匹配该固定SHA。旧二进制readiness不接受新schema，回退不能只切旧指针。

真实HTTPS输出（`live-smoke.json`）：

```json
{"result": "passed", "revision": "8429aedc57bcb526ea09ac10e9ac8f1fd1388944", "https_ready": 200, "flush": 201, "idempotent_flush": true, "restore": 200, "reverse_restore": 200, "restored_version": 4, "stale_restore": 409, "anonymous": 401, "worker_crash_recovery": true, "worker_restart_idempotent": true, "startup_backfill_immediate": true, "credentials_changed": false}
```

停Worker→通过HTTPS创建本任务笔记且不flush→启动Worker，立即补齐最新版本；再次restart无重复快照。全部测试会话秘密仅存内存，最终撤销；笔记正常软删除，用户Passkey不变。未新增硬件Passkey人工验证声明。

## 收尾

Issue78自动关闭、Projects Done。实施 `.worktrees/knoted-t25` 与本地feat/t25正常移除，远程分支由仓库自动删除；忽略的依赖/scratch归档在任务 `/data/.../worktree-generated`。保留其他任务HTTPS工作区。此纯文档收尾分支合并后也清理，运行行为与固定SHA不变。
