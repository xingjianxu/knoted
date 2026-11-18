# T61–T65 主线集成与 dev 验收

[PR #213](https://github.com/xingjianxu/knoted/pull/213) 普通合并至 `867b004910efd5711e26a9578421054c76413ba0`。合并版本的 Rust、Web、OpenAPI、生成客户端、dev 脚本与审查过的分支 `6cb4315` 一致。生成协议沿用已有契约，搜索命中增加可选 `session_id`，供会话内消息定位。

## 本地验证

| 命令或检查 | 结果 |
|---|---|
| `cargo fmt --check` | exit 0，主线复跑通过 |
| `cargo test --locked --workspace` | exit 0，492 passed、0 failed、1 个既有 doctest ignored，包含真实 OCR 和 Office 提取 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo test --locked -p knoted-core --test t61 --test t63 -p knoted-api --test t62 --test t64` | 主线复跑 25 项通过（T61 7、T62 8、T63 6、T64 4），用例使用真实 PostgreSQL 和可控回环模型 |
| `pnpm api:check`、`pnpm api:verify`、API 客户端 typecheck | exit 0 |
| Web lint、typecheck、test、build | exit 0，23 个文件共 439 项通过，T65 定向 10 项通过，构建保留分包大小提示 |
| pcwrap 测试 | 41 项通过，覆盖本机模型配置和凭据文件路径 |

主 Agent 独立复核生成终态、模型端点绑定与引用完成事务。T62 用例验证幂等、取消保留生成位、Worker 接手、SSE 持久化和聊天投影。T63 用例验证整轮裁剪、超预算拒绝、来源撤销与恢复后的历史隔离。T64 用例验证引用授权、固定快照以及鉴权。T65 用例验证中断、切会话、重试、失效引用与搜索后的消息定位。

## 固定版本

从主线 `867b004` 构建 Rust 和 Web，Python 服务源码与前一部署版本相同，复用已校验产物。运行包包含 38,341 项文件校验，`SHA256SUMS` 的 SHA-256 为 `e9fa0c46776c26dc715ebc33c7d006e981c1079c10d262ba0f69fe226a576f0c`。切换前备份 dev 数据库和资源、复核校验，dev 依次应用迁移 0021–0024。

测试记录位于 `/data/knoted-dev/tasks/t65/evidence/`，包括 `integrated-rust-suite.log`、`integrated-web-test.log`、`integrated-pc.log`、`dev-release.log` 和 `dev-acceptance.log`。

## dev 场景

临时回环模型仅处理合成笔记和合成问题。会话、队列、Worker、Meilisearch、PostgreSQL 和 Chromium 均使用实际 dev 链路。

- Worker 发布当前合成笔记的词法分块，临时 Chromium 会话选定笔记和唯一合成标签后提交问题。
- 模型夹具确认请求携带对应笔记、服务端发放的引用标记及 4096 输出预留。浏览器收到 SSE 流，重放验证 `queued`、`started`、`delta`、`citation`、`completed` 的持久顺序。重新加载后正文与数据库终态一致，答案移除引用标记。
- 从聊天全文命中跳转同一会话并聚焦原回答。当前来源有直达链接，软删除后保留同一引用快照，显示来源已删除并移除链接。
- 临时会话授权撤销，合成会话、笔记及标签软删除。恢复常规 dev 运行方式，实际运行仍为 `867b004`。宿主 Caddy、PostgreSQL、OnlyOffice、Strata 进程 PID 保持一致。

在上述集成验收后，宿主配置已为 dev 运行身份授予既有 Strata 客户端组权限。T61 在同一 dev 发布版本完成真实鉴权、生成流和停止确认，见[真实 Strata 验收](t61-providers.md#真实-strata-dev-验收)。
