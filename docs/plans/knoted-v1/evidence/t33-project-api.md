# T33 项目 API 与归档：本地实施证据

- 任务：[T33](../tasks/T33.md)，Issue [#90](https://github.com/xingjianxu/knoted/issues/90)。
- 基线 `cef4e7df87cccedc6cb4d30e8454e24d2ba51a76`，Worktree `.worktrees/knoted-t33`，分支 `feat/t33`。
- 范围：Project CRUD / archive / unarchive / restore、最小 tasks 归属表、T32 真实表 fixture 兼容。没有实现 T34 Task CRUD；没有改依赖、锁文件、前端或 OpenAPI。
- 状态：本地实施交接；主 Agent 独立审查、PR 集成、主线复验、迁移前一致备份、dev 固定 SHA 部署与 HTTPS 验收尚待进行。

## 实现边界

`0009_t33_projects.sql` 首次创建完整 projects 与最小 tasks，不给项目标题/时间制造占位默认值；后续 T34 必须扩展同一 tasks 表。软删除保留 registry、tags、links 与已删除事项，不进入笔记/资产服务。既有已归档但未完成事项也阻止项目归档；删除必须无任何未删除事项；restore 只恢复项目。

所有项目写事务先调用 `projects::repository::lock_relations(&mut PgConnection)`（`pub(crate)`）：`pg_advisory_xact_lock(33001,'projects'::regclass::oid::integer)`。发现涉及项目及事项后，将跨表行按 UUID 全局升序锁定，之后重读 CAS/归属/状态并变更。T34/T35/T37 复用该 helper 和协议；不得先锁项目再锁事项，不进入 notes tree / asset 服务。

`project.changed` / `task.changed` outbox payload 固定 `{source_id,content_version:entity.version,operation}`；operation 为 created / updated / archived / unarchived / deleted / restored 中的实际动作。事件同事务且无正文，无当前 Worker 不认识的 job kind。相同值 PATCH 不递增版本/时间、不发事件。归档每次生成 UUID 批次，取消归档只处理同项目同批次未删除事项；不用时间猜测批次。

列表省略 archived 表示所有非 deleted 项目，true/false 过滤；`(created_at,id)` 升序，base64url 版本/类型/过滤 scope 游标。PATCH description 省略保留、null 清空。详情和写响应按完整 Project DTO，ETag 带双引号，不泄露 archive_batch_id。

## RED / GREEN 与并发证据

完整输出在 `/data/knoted-dev/tasks/t33/logs/`，进度 `/data/knoted-dev/tasks/t33/progress.md`，完整交接报告 `/data/knoted-dev/tasks/t33/t33-report.md`。临时目录、Cargo target 均在本任务 `/data` 目录；编译并发固定 `CARGO_BUILD_JOBS=2`。

- `01-red.log`：初始四场景均在真实 HTTP 得到 404，预期创建 201，exit 101。
- `02-red.log`：九场景在缺失项目路由处失败，exit 101。
- `03-green.log`：9 个 API/PG 场景通过。
- `04-concurrency.log`：10 API + 4 core 场景通过；事项创建、迁入、done→todo 与归档分别验证两种先后结果，实际等待由 PostgreSQL `pg_blocking_pids` 观测，不用 sleep/压力测试/产品测试钩子。
- `06-t32.log`：`cargo test --locked --workspace t32_`，9 T32 + 2 同时匹配名称的 T33 测试通过；T32 原有业务断言保持不变，fixture 使用真实项目创建和最小事项字段。
- `07-lock-order-mutation-red.log`：显式 mutation RED，将 UUID 顺序临时替换为项目先行后，T32 关联与归档的受控交叠产生 archive 503，exit 101；finally 立即恢复生产排序。该证据没有冒充初始测试 RED。
- `08-t33-green.log`：恢复 UUID 排序后 `cargo test --locked --workspace t33_`，14 项通过。

## 最终本地门禁

命令：`cargo fmt --check`、`cargo test --locked`、`cargo clippy --locked --all-targets -- -D warnings`；初轮 `09-fmt.log` / `10-full-test.log` / `11-clippy.log` 全部 exit 0，195 个 Rust 测试通过、1 个既有文档测试 ignored。自查补强全部 Project 生命周期事件断言后，最终三项门禁均 exit 0（195 passed、0 failed、1 个既有 ignored doc-test）；读回结果见交接报告及 `13-final-fmt.log` / `14-final-test.log` / `15-final-clippy.log` / `final-exit-codes.txt`。未改前端/OpenAPI，因此无需重复前端脚本或 API 生成。

新迁移只在隔离测试 schema 应用。部署时先一致备份，按既有 pcwrap 固定产物流程迁移和验收；本地通过不是 dev 已部署或任务已完成的证明。
