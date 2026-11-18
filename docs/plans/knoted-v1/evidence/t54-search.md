# T54 统一搜索实施证据

- 任务卡：[T54](../tasks/T54.md)｜Issue：[#191](https://github.com/xingjianxu/knoted/issues/191)
- 基线：`341b9a5`，包含 T53 [PR #187](https://github.com/xingjianxu/knoted/pull/187)
- 分支与工作区：`feat/t54`，`.worktrees/knoted-t54`
- 本地测试输出：`/data/knoted-dev/tasks/t54/evidence/`

## 实现

- 经会话鉴权的 `GET /api/v1/search` 沿用 T03 OpenAPI 及生成客户端。返回纯文本摘要。聊天类型在 T62 接入前返回空页。
- Worker 消费现有笔记、项目、事项变更以及文件提取成功事件。在同一事务确认事件并调用 T53 建立投影、发布账本及任务。已索引的软删笔记、项目或事项生成无正文的删除标记，以入队清理旧版本。
- Meilisearch 负责词法命中和排名。查询只返回当前索引的已确认发布，按 PostgreSQL 的来源版本、软删除和文件新鲜度复核。标签、项目关系、归档、文件路径和后缀用 PostgreSQL 当前值。更新时间排序使用投影时间戳，按微秒精度排序，以文档键稳定处理同一更新时间。
- 版本化游标绑定查询、筛选、排序和索引，校验范围为 1 至 9999 的引擎起点。最多浏览 10000 个引擎候选，PostgreSQL 可能进一步过滤。响应无精确总数，索引改变期间无快照保证。

## 验收

| 场景 | 断言 |
|---|---|
| t54_1 | 原引擎文档存在时，软删后立即无法返回，并为已索引版本生成无正文删除标记 |
| t54_2 | 同一查询可用游标取下一页，换类型时返回 400 |
| t54_3 | 笔记提交新版本后旧正文立即隐藏，新全文独立于向量可见 |
| t54_4 | 保存笔记的现有变更事件仅生成一次最新版本的纯文本投影和发布任务 |
| t54_5 | 重复 `tags` 参数校验当前多标签关联，解除关联后从结果消失 |
| t54_6 | 显式更新时间排序优先于相关性，支持同一秒内按时间倒序或正序 |
| t54_7 | 文件提取事件经 T53 发布，移动后的路径过滤即时生效，采集失效后撤回旧结果 |

真实 PostgreSQL 与 Meilisearch 运行记录见任务区。首轮 TDD 的三个卡片测试在无路由时均得到 HTTP 404。更新时间排序及删除标记的新增断言在实现对应处理前已各观察到失败。

### 本地命令

| 命令 | 结果 |
|---|---|
| `cargo test --workspace --locked t54_` | exit 0，7 passed / 0 failed |
| `cargo test --locked -p knoted-core --test t53` | exit 0，13 passed / 0 failed |
| `cargo fmt --check` | exit 0 |
| `cargo clippy --locked --all-targets -- -D warnings` | exit 0 |

共享测试库等待 SQLx 迁移 advisory lock 时，改用任务目录内 PostgreSQL 18.6 私有 Unix socket 和独立 `_test` 库复核上述数据库用例。Meilisearch 继续使用本机真实引擎和随机测试索引。

## 范围

- 文件正文由成功提取投影生成。事项正文来自事项说明，跟进记录的独立检索分块按 T57 实现。
- 沿用 T53 单文档投影正文的 1 MiB UTF-8 上限。超过上限时仅索引前缀，来源及完整提取产物照常保留。统一搜索结果未提供分块定位字段。
- dev 更新、主线复验与工作区清理由主 Agent 在集成后记录。
