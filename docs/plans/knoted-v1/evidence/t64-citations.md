# T64 引用快照与验证

- 对应 [任务卡](../tasks/T64.md) 与 [Issue #203](https://github.com/xingjianxu/knoted/issues/203)。
- 工作区 `.worktrees/knoted-t64` 和分支 `feat/t64`。以 `35ff7c4` 为主线基线，合入 T62 持久层 `f50aa3d` 和 T63 依赖记录 `c3ae523`。
- 验证输出保存在 `/data/knoted-dev/tasks/t64/evidence/`。

## 实现

服务器为本轮资料分配 UUID 引用标记。`validate_citations` 按首次出现顺序接纳白名单成员，去除原始标记。无有效引用或无可展示答复时输出「依据不足。」。URL、裸 UUID 和未知标记不产生引用记录。

`0023_t64_citation_snapshots.sql` 保存引用的标题、定位、片段和来源版本，并绑定生成运行、助手消息及 T63 记录的来源可见性代次。读取时复核当前来源、版本和提取身份，已更新的来源返回 `missing`，已删除的来源返回 `deleted`。两种情形都保留原快照。

`GET /api/v1/chat/messages/{messageId}/citations` 使用已登记的 `CitationPage` 契约，以消息绑定的 `(ordinal,id)` 游标分页。路由继承会话鉴权，检查消息和会话当前可见性，未完成答复的草稿不返回引用。后续提示从消息正文和 T63 依赖记录构建。

## 验收范围

- `t64_1` 验证伪造 URL 和未知 ID 无法形成证据，并按出现顺序去重合法引用。
- `t64_2` 通过 PostgreSQL 持久化及真实 HTTP 路由检查旧版标题、定位和片段，更新和删除来源后验证当前状态变化。
- `t64_3` 从上一轮的实际聊天持久层读取历史并构建新上下文，保留来源依赖，快照正文不进入提示。
- `t64_4` 验证匿名读取、分页、跨消息游标和会话删除后的访问限制。

HTTP 路由未实现时，`t64_2` 和 `t64_4` 分别在成功读取与匿名拒绝断言得到 404。补齐路由和分页后，`cargo test --locked --workspace t64_` exit 0，4 passed / 0 failed。`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`git diff --check` 均为 exit 0。

生成完成事务接线与 dev 验收随上游 T62、T63 运行链集成后记录。
