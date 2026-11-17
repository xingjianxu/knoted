# T37 事项归档和回收站：实现与验证

任务：[T37](../tasks/T37.md)，Issue [#114](https://github.com/xingjianxu/knoted/issues/114)，[已确认设计与计划](../../2026-11-17-t37.md)。

当前：实现已提交，主 Agent 已复跑门禁，独立审查 PASS；PR 集成与测试实例部署尚待完成，不标 completed。

## 设计与范围

用户在本次会话选择「最小兼容方案」，确认：

- archive/unarchive 只改变请求事项，不限终态、不级联子事项；父/项目归档时，正文、跟进、标签和资料关联的 source 写入均拒绝，读仍允许。归档不等于删除，仍可检索。
- restore 保留当前 parent/project，父缺失/删除/归档/非顶层、继承不一致或项目不可写均 409；先恢复父，不自动移根、复活父或重开 done 父。仍需校验 T35 终态不变量。
- 30 天是物理清理资格线，不是恢复硬截止；未被清理仍可恢复。精确到 `deleted_at <= now - 30 days`，使用 AppContext 注入时钟。
- 只永久清理事项及其 activities/activity_revisions/task_tags，系统时间线随所属事项永久回收；保留 entity_registry、失效目标 entity_links、共享 tags、durable outbox。任何尚存子行都阻止父删除，先叶后父，不提前删除未满期子、不解除 FK。
- 不引入父子归档批次、存储迁移或新 HTTP 字段，不实现笔记/项目物理清理、T42 页面或 T53/T57 检索模块。

## 实现与接口

| 文件 | 职责 |
|---|---|
| `crates/knoted-core/src/tasks/lifecycle.rs` | archive/unarchive/delete/restore 与 purge_trash 的事务、校验和回滚 |
| `tasks/repository.rs` | 生命周期 CAS 同时写归档状态/批次，过期行有序锁定、叶子查询和显式依赖清理 SQL |
| `tasks/service.rs`、`tasks/mod.rs` | 共享 relations_writable 校验、原 delete 兼容转发、模块注册 |
| `tasks/activities.rs` | 父/项目只读保护，复用完整有序关系行锁；状态拒绝不返回外来 activity current_version |
| `links/repository.rs` | source 写校验父与项目；端点行锁前取得关系锁，避免反向链接先锁端点再等待关系锁 |
| `crates/knoted-api/src/routes/tasks.rs` | 三个既有 POST 动作，会话/CSRF、严格 body、Task+ETag、既有错误码 |
| `crates/knoted-worker/src/handlers/trash.rs`、`handlers/mod.rs`、`main.rs` | 独立启动即执行与每 30 秒清理循环，和 note_history 并发；错误分类日志，无正文/数据库诊断 |
| `crates/knoted-api/tests/t37.rs` | 17 个真实 HTTP/PG/TCP 用例 |
| `crates/knoted-worker/tests/t37.rs` | 2 个真实 Worker 进程启动/周期/故障重试用例 |
| `api/openapi.yaml`、生成客户端 | 仅补四个生命周期操作的已确认语义说明；无类型、路径或字段变化 |

`lifecycle::{ArchiveTask,UnarchiveTask,RestoreTask}` 均为 `{ id: Uuid, expected_version: i64 }`；DeleteTask 复用原 service 类型。四个操作采用 `(ctx: &AppContext, input: 对应类型) -> Result<Task, DomainError>`（delete 返回 `()`），均为 async。`purge_trash(ctx, PurgeTrash {}) -> Result<PurgeTrashReport, DomainError>`，报告含 `purged: u64`。

HTTP POST `/tasks/{taskId}/{archive,unarchive,restore}` 仍只接受 expected_version，DELETE 保留 query expected_version。所有生命周期在同一事务更新 state/version/updated_at，并追加已有 `task.changed`（source_id/content_version/operation）。purge 不清除 durable outbox，也不虚构新搜索事件类型。

## 并发与检索边界

- 复用 relation advisory lock → 按 UUID 有序锁行 → CAS → outbox 的既有协议，restore 与 purge 只有可串行化结果。purge 一轮原子执行，依赖清理或任务删除失败则整轮回滚。
- links 端点校验在任何端点行锁前取得关系锁；为防 reciprocal link 先遇到非 task 端点，这也覆盖只读端点检查。代价是保守地串行化 link/tag 读取与项目/事项写入；没有吞吐承诺或性能测试。
- 当前尚无全文/向量投影，T37 验证的是 PG 当前删除状态立即不可见、版本更新与 durable outbox 同事务，不是检索引擎端到端撤出。T53/T57 必须复核当前业务行状态；物理清理后行缺失同样不可见，registry 身份本身不授予正文可见性。

## 测试证据

主 Agent 在 `.worktrees/knoted-t37` 独立复跑；完整日志 `/data/knoted-dev/tasks/t37/logs/`，实施报告 `/data/knoted-dev/tasks/t37/task-1-report.md`。

| 命令 | 主 Agent 实际结果 |
|---|---|
| `cargo test --workspace --locked t37_` | exit 0，19 passed / 0 failed（API 17 + Worker 2） |
| `cargo test --workspace --locked` | exit 0，255 passed / 0 failed / 1 既有 ignored doctest；基线 236 passed |
| `cargo fmt --check` | exit 0 |
| `cargo clippy --locked --all-targets -- -D warnings` | exit 0 |
| `pnpm api:check` | exit 0，83 operations, 16 examples (8 invalid rejected) |
| `pnpm run typecheck`、`pnpm run lint` | 均 exit 0 |
| `pnpm run test` | exit 0，12 files / 199 tests passed |
| `pnpm run build` | exit 0；已有 >500kB chunk 警告，不在本任务改动范围 |
| `git diff --check` | exit 0 |

所有 cargo 命令限制 CARGO_BUILD_JOBS=2；默认 128 核并发编译曾两次超时（180s/600s），限制后基线与实现均通过。测试运行没有以超时伪装失败行为 RED。

### 场景矩阵

| 场景 | 覆盖 |
|---|---|
| t37_1（5例） | 单项不限终态归档、取消归档、可读与列表过滤、父只读所有子写、项目只读所有写、严格 body/path/version、匿名 401/CSRF 403、旧版本冲突 |
| t37_2（1例） | 29 天恢复、版本加一、关联/历史保留；未物理清理超过 30 天仍可恢复 |
| t37_3（2例） | 已删父先恢复、done 父拒绝恢复非终态子、当前迁移项目、历史非法层级与继承、父/项目只读拒绝 |
| t37_4（3例） | 满 30 天资格、前 1 微秒不删、先叶后父、年轻/存活子阻止老父、依赖显式清理、registry/links/shared tags/outbox保留、幂等、触发器故障整轮回滚 |
| t37_5（5例） | 四种 lifecycle outbox 失败回滚；项目批次不取消手动归档；同版本 archive/PATCH 竞争；父归档与 activity/tag/反向link竞争；restore/purge竞争无孤儿 |
| t37_6（1例） | 实际 axum TCP listener 上的生命周期、读取隐藏、CSRF/session、ETag |
| Worker（2例） | 真实二进制启动与下一周期清理；数据库失败分类日志不泄露正文、下轮重试且历史维护独立运行 |

### TDD 记录

实施 Agent 先看到缺少 archive 路由 404、父归档后活动 201/标签 204 等真实 RED；restore 在路由缺失时 RED；purge 先签名编译缺失，再用 no-op 观察真实 PG 行为失败，随后实现为 GREEN；真实 Worker 在未接线时启动清理超时失败，接线后启动/周期/故障重试通过。相关 `impl-red-*`、`impl-green-*`、`impl-delivery-*` 日志保留。追加回归/竞争本来通过的用例只记 regression，不伪造 RED；TCP fixture 的非直接依赖编译错误不计产品行为 RED，最终使用已有 std TCP 接口，无新增依赖。

## 审查判定与覆盖限度

- task spec/quality 与单任务完整分支均通过独立审查；无代码修改要求，因此没有虚构审查修复 RED→GREEN。
- T37 join/timeout 竞争测试只确认实际出现的可串行结果，不强制两种赢家都出现；子→父 link 与父归档竞争不等价于同时 A→B/B→A 的双 link 测试。锁序还经过 reviewer 调用链分析，以及 T32 真实 DB 门闩的 note-link/tree 回归；不声称穷举全部 interleaving。
- 缺失父由代码路径审阅；正常 FK 不允许构造引用不存在父的 fixture，非法层级/继承及已删父有真实场景。
- Reviewer 未判断全文/向量、T38/T42 UI、硬件 Passkey、生产部署或性能容量；这些均在任务外，主 Agent 保持此边界，不以测试会话冒充硬件 ceremony，不据此提出产品范围扩张。
- 保守的 link/tag 读串行化是当前锁协议安全性取舍；未来若修改该协议，再增加受控双向 link 排队回归。此为覆盖增强建议，不是已发现缺陷或当前阻断。

## 提交、审查、主线与实例

- 基线：`c059943132be22e60e1e24e8386050b586d6540c`；T36 `f26af66`、T08 `9bd5bcd` 均为祖先。
- 计划：`410be28`；实施：`4442f55cc4f9fdbbdb440827a80a4e6a29a23eae`。
- 独立审查：`review.md`（同任务证据根）Spec PASS / Quality APPROVED，Critical/Important/Minor = 0/0/0；reviewer 独立运行 9 个针对性 PG 测试全部通过，包括 purge 边界/回滚、竞争以及 T32 受控 note-link/tree 锁序回归。无待修 findings。
- PR/主线：待集成。
- 测试实例：部署前运行 `f26af66`、迁移 11；9 个事项均已软删，过期 30 天事项 0；未对真实数据做老化或手工永久清理。
- 新增自动物理清理启用前须停本实例 API/Worker 做数据库+assets一致备份，再部署固定产物；不改 host、不停共享数据库/Caddy/Strata。
- 运行 SHA、HTTPS 冒烟、Worker日志、备份与工作区清理：待实际执行后补记。
