# T51 OnlyOffice 只读预览授权交付证据

后端由 [PR #181](https://github.com/xingjianxu/knoted/pull/181) 普通 merge 至 `dadbef7934eceb42e7f08d6bf300857b2b9c5dd4`；[Issue #179](https://github.com/xingjianxu/knoted/issues/179) 已关闭，Projects Done。网页接入和 XLSX 切换工作表属于 [T52](../tasks/T52.md)，本卡不替它验收。

## 契约和审查

- 实现 `a2b59ee`、凭据修复 `7e68c55`、整合 `18e9236`；合并提交与整合工作树的内容无差异。
- [ADR 0008](../../../decisions/0008-office-preview-authorization.md) 与 [ADR 0009](../../../decisions/0009-office-source-jwt-header.md)：DOCX/XLSX/PPTX 固定版本副本，每进程最多两份、每份最多 50 MiB、最长 120 秒。创建/撤销受会话和 CSRF 保护；交付须仍持有效创建会话。已发出的字节不可收回。无编辑或保存回调、迁移、原件写入、普通下载变更。
- 首审指出共享转换器在失败时记录完整源 URL。用户批准官方出站 JWT 请求头；改为无凭据 URL、只收官方 `Authorization: Bearer` JWT、校验时限和精确 URL，不接受查询凭据、Cookie 或浏览器配置 JWT。初版查询凭据实现未部署。独立复审确认原 P1 已修，无新 Critical/Important。建议后续补上发布前和会话查验期间的确定性竞态用例；当前专项已有并发容量、传输中限额和登出拒绝。

## 实际本地门禁

日志和对应退出码位于 `/data/knoted-dev/tasks/t51/evidence/`。以下是主 Agent 独立核对记录；Rust 全量、静态分析与 pnpm 门禁在整合头执行。

| 命令 | 结果 |
| --- | --- |
| `cargo test --workspace --locked t51_` | exit 0，5/5；修复轮 TDD RED 5 项预期失败 |
| `cargo test --workspace --locked t49_` | exit 0，12/12 |
| `cargo fmt --check` | exit 0 |
| `cargo test --workspace --locked` | exit 0，420 顶层 + 8 嵌套通过，0 失败，1 既有 ignored doctest |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | exit 0 |
| `pnpm api:check` / `pnpm api:verify` | exit 0；84 operations / 29 examples / 14 反例拒绝；生成无漂移 |
| `pnpm --filter @knoted/api-client run lint` / `run typecheck` | exit 0 |
| `pnpm test:pc` | exit 0，38/38 |

第一次主 Agent 全量因隔离 PostgreSQL 测试库是 SQL_ASCII，既有 T23 中文样本失败；改用 UTF-8 库后定向通过。随后一次命令超时，不计通过；最终整合头 UTF-8 完整运行结果见上表。自动测试未操作 dev 业务库。

## dev 固定产物与真实 HTTPS 后端验收

- `/data/knoted-dev/runtime/dadbef7934eceb42e7f08d6bf300857b2b9c5dd4`：Rust release 与 Web 来自主线归档构建。标准打包在下载大型 Python wheel 时超时；经核对 Python 源码、锁与构建配方不变，复用上一固定版本经清单逐文件校验的 Python 载荷，重新生成 38,320 项校验清单并逐文件验证（清单 SHA-256 `ef349aabd7bdeed537ccc23acf89c1db6cfe5456f97e75cd85e9387bdf96c225`）。三服务进程解析到这一完整 SHA。无迁移差异；切换前保存可读取的 dev PostgreSQL 备份。
- 第一次 fixture capture 超时归因于 pcwrap 重启后漏传既有只读模型路径，intelligence 没有启动。恢复相同路径再运行，API 和 intelligence 就绪；失败证据保留，不计为通过。
- 最终 `t51-live-shared-office-r2.json`/`.exit`：严格 HTTPS，短期测试会话在单独的 Knoted HTTP 客户端创建并撤销预览，浏览器载入真实共享 OnlyOffice 10.0.0 官方 SDK 且不带 Knoted Cookie。XLSX `onDocumentReady`；无请求头直接访问来源 HTTP 404；另开同文档的新 grant，在转换前撤销，再用官方 SDK 得到预期 `onError` `-4`。此处验证授权和真实取源，不是认证页面集成或工作表切换。
- `t51-revoked-fetch-log-redaction.json`：共享转换日志中仅统计对应撤销实例，3 条取源失败，未匹配到 query 秘密或 Bearer 凭据；保存统计，不保存原始请求头/JWT 或转换日志。没有修改共享 OnlyOffice、Caddy、PostgreSQL 或宿主；相应共享服务 PID 在验收前后相同。自有测试会话已撤销、源夹具已移除。
- 验收后保持同一主线固定产物，清除 Knoted dev 可选 Office 配置并核验 API/intelligence 就绪。[T52](../tasks/T52.md) 通过真实验收前不启用网页预览。
