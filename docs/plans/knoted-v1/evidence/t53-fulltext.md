# T53 全文索引发布证据

- 任务卡：[T53](../tasks/T53.md)｜Issue：[#185](https://github.com/xingjianxu/knoted/issues/185)
- 分支/工作区：`feat/t53` / `.worktrees/knoted-t53`
- 基线：主线 `e18ae55b6437134a170d2c5b161d201170d9cdc8`
- 临时目录：`/data/knoted-dev/tasks/t53/`（`tmp`、`build`、`meili`、`evidence`）

## 交付内容

- `migrations/0016_t53_fulltext.sql`：`source_documents` 统一投影（`(source_type, source_id)` 当前版本部分唯一）与 `fulltext_publications` 发布账本（`(engine, index_uid, document_key)` 唯一，`published_at` 与 `published` 状态同事务约束）。
- `crates/knoted-core/src/search/meili.rs`：`publish_fulltext`、`get_publication`、`document_key`、Meilisearch 适配器与 `fulltext.publish` / `fulltext.cleanup` 执行逻辑。
- `crates/knoted-core/src/search/mod.rs`、`src/lib.rs`：注册 `search` 模块。
- `crates/knoted-core/src/config.rs`：`[search]`（`enabled`、`base_url`、`index_uid`、`api_key_credential`），沿用 loopback HTTP 与凭据文件约束。
- `crates/knoted-core/src/context.rs`：`with_search` / `with_search_api_key`；`Debug` 不输出密钥。
- `crates/knoted-worker/src/handlers/fulltext.rs`、`handlers/mod.rs`、`handlers/scan.rs`：注册两个任务类型；调度器在未启用文件扫描时仍领取任务（全文发布不依赖文件扫描）；`cancel_and_wait` 按任务类型分派。
- `crates/knoted-worker/src/main.rs`：装载检索配置与凭据。
- 测试：`crates/knoted-core/tests/t53.rs`（13 例）。

## 实现要点

- 每来源/版本一个确定性文档键 `<type>_<uuid 无连字符>_v<version>`，满足 Meilisearch 文档 id 字符限制；重试是同键 upsert。
- 发布事务：`source_documents` 当前版本校验（advisory lock 串行化同一来源）→ `fulltext_publications` → `jobs::enqueue`（同时写 `outbox`，主题 `job.enqueued.fulltext.publish`）。
- 状态机 `pending → submitted（task id 落库）→ published | failed`，另有 `superseded`；`published_at` 只能与 `published` 同事务出现。
- 只有 Meili 任务 `succeeded` 才写 `published`；`enqueued`/`processing` 返回可重试失败，且重试复用已落库的 task id，不重复提交。
- 旧版本发布标记 `superseded` 并入队 `fulltext.cleanup`；清理按 `keep_version` 批量删除旧文档键，成功后删除旧投影与发布记录。可见性不依赖删除速度（读时复核属 T54）。
- 引擎拒绝（4xx）为永久失败、传输/5xx 为可重试失败；永久失败与重试预算耗尽都会把账本落 `failed`。租约丢失（接管）时归还 `Cancelled`，不写 `published`。
- 平台手动重试（`retry_failed`）通过 `retry_of` 认领并发起一次性改绑，发布不会因旧账本 `job_id` 而静默失效。
- 版本即内容：`title/body/tags/archived/deleted/path/format/updated_at` 变化必须使用新的 `source_version`；同版本重发沿用已存投影。
- `source_type` 词表与共享来源注册表一致（`chat_message`）。
- 索引设置：`searchableAttributes=[title, body]`、9 个过滤字段、`sortableAttributes=[updated_at, version]`；设置漂移时才重写并等待任务确认。

## 验证

环境：真实 PostgreSQL 测试库 + 真实 Meilisearch 1.56.1（官方 release 二进制，含 charabia 全语言 tokenizer；nixpkgs 默认包 `buildNoDefaultFeatures` 缺中文分词，实测只能命中前缀）。

| 命令 | 结果 |
|---|---|
| `cargo test --workspace --locked t53_` | exit 0，`t53_` 13 passed / 0 failed |
| `cargo test --workspace --locked`（`KNOTED_TEST_PYTHON=/wrd/prj/knoted/services/intelligence/.venv/bin/python`） | 除既有环境失败外全部通过，详见下 |
| `cargo fmt --check` | exit 0 |
| `cargo clippy --locked --all-targets -- -D warnings` | exit 0 |

逐场景：

- `t53_1`：长中文句子中「知识管理」命中（非前缀），英文 `meeting` 命中，过滤（类型/标签/归档/删除/版本）与 `updated_at:desc` 排序真实生效；`get_publication` 为 `published` 且 task id 存在。
- `t53_2`（wiremock）：任务仅 `enqueued` 时返回 `TemporaryFailure`，账本保持 `submitted` 且 `published_at IS NULL`；同一 task 变为 `succeeded` 后重试只轮询、不二次提交，随后 `published`。
- `t53_3`：重复 `publish_fulltext` 同版本只保留一个任务；提交响应丢失后重试，同一文档键在引擎中仍只有 1 条可见文档。
- `t53_4`：租约被接管后执行返回 `Cancelled`，账本未写 `published`/`published_at`。
- `t53_5`：空白标题在事务前拒绝，投影/发布/任务/outbox 均 0 行。
- `t53_6`：发布 v1、v2 后旧版本标记 `superseded`、引擎暂有 2 条；清理任务成功后引擎剩 1 条且旧投影行删除。
- `t53_7`：`[search]` 拒绝非 loopback/含凭据/非法 `index_uid`；缺省时 `enabled=false` 且无 API key。
- `t53_8`：平台手动重试（`jobs::service::retry_failed` 新建带 `retry_of` 的任务）能把失败发布改绑到新任务并真正发布。
- `t53_9`：引擎 422 永久拒绝后账本落 `failed` 且记录错误；同版本再次 `publish_fulltext` 会重新入队真实任务。
- `t53_10`：引擎已接收但响应丢失（无 task id）的旧版本文档，仍被后台清理按投影版本重新推导文档键删除。
- `t53_11`：已有 v2 后再发布 v1 返回 `Stale`，当前投影仍只有 v2。
- `t53_12`：并发发布 v1/v2 时较新版本必然成功，旧版本只可能 `Stale`，当前投影恰好一行。
- `t53_13`：`MeiliClient` 的 `Debug` 不包含 API key。
- `t53_1` 另含分词负例：跨分词边界的「识管」命中 0 条，证明命中来自真实中文分词而非前缀/子串。

基线失败（与本卡无关，未修改主线）：`knoted-worker --test t47_pipeline` 的 `t47_real_scan_capture_ocr_publication_and_unchanged_no_retry` 在未修改的主检出上同样失败（`models_missing`/`artifacts_unset`），属本机 OCR/嵌入模型环境缺失；`t47_http` 的真实 Python 测试需要 `KNOTED_TEST_PYTHON` 指向主检出 venv（工作区无 `.venv`）。

## 独立审查与修复

独立审查（reviewer）结论 CHANGES REQUESTED，4 项 Important 已全部修复并各自补测：

1. 账本→任务绑定会让平台手动重试永远 `Cancelled`：改为 `retry_of` 匹配时在旧任务终态下一次性改绑，`t53_8` 覆盖。
2. 提交前/后的永久失败（含重试预算耗尽）没落 `failed`，账本永久停留 `pending`/`submitted`：统一在永久失败路径记账，`publish_fulltext` 在 owning job 终态时重新入队，`t53_9` 覆盖。
3. 清理只删有 task id 的键，响应丢失的旧版本会永久残留：清理键改为按 `source_documents` 版本重新推导，`t53_10` 覆盖（回退旧谓词该用例失败，已验证可证伪）。
4. `MeiliClient` 的 `Debug` 会打印 API key：手写 `Debug` 只输出 `api_key_set`，`t53_13` 覆盖。

P2/P3 采纳：提交前复核当前版本、`t53_4` 补 `record_published` 栅栏、补 Stale/并发/分词负例测试、`chat_message` 词表统一、`get_publication` 按 `index_uid` 限定、advisory lock 改用 `hashtextextended(current_schema()…)`、无旧版本不再入队清理任务、取消失败封闭、`ensure_index` 4xx 归类为永久、配置诊断改为「IP 字面量」。未采纳/保留说明：清理与在途提交交错时引擎可能短暂残留文档键，已用提交前复核把窗口收窄；剩余窗口由 T54 的 PG 复核保证可见性，符合「绝不靠删除速度保证可见性」。

## 部署判断

- dev 实例**未部署**：运行期检索依赖是 Meilisearch，而编排仍将 `127.0.0.1:15605` 预留且未包含该服务（`docs/plans/knoted-v1/development-and-deploy.md`）；接入属外部依赖/host 任务，本卡不修改 `/etc/nixos`，也不把未运行的服务写成已启用。
- 迁移 `0016` 为新增表，不改变现有读写路径；`[search]` 默认 `enabled = false`，未配置时 worker 仍正常运行（调度器只在有任务时领取）。下次部署产物会一并应用迁移，届时无 `[search]` 段即保持关闭。

## 限制

- 未实现 `/search`（T54）、索引 generation 重建与前端（T55）。
- 引擎中可能短暂保留被取代版本的文档；统一搜索必须按 PG 当前版本/删除状态复核（T54 验收项）。
- 中文分词质量取决于 Meilisearch 的 charabia 构建；部署必须使用含全语言 tokenizer 的构建（官方 release），否则只能命中词首前缀。


## 主线复核与 dev 发布验收

本节记录第 2 次交付，更新上文实现分支的部署判断。

- 代码先经 [PR #187](https://github.com/xingjianxu/knoted/pull/187) 合入，再由 [PR #192](https://github.com/xingjianxu/knoted/pull/192) 修复人工重试和聊天消息键。两条新测试在原代码分别观察到终态任务不可恢复和约束错误 23514。重新提交同一文档键后只保留一个发布账本，新任务 UID 为已确认成功的 UID。迁移 0017 允许 `chat_message_` 键，保留既有键约束。
- `cargo test --locked -p knoted-core --test t53` 在 PR 和主线均 exit 0，15 passed / 0 failed。PR 的 `cargo test --workspace --locked` 整轮复跑 exit 0，日志共 443 个 passed（含嵌套子进程报告）。首轮在 T46 捕获场景偶发失败，单例和整轮复跑通过。T47 真实 OCR 测试补齐既有共享模型路径后通过。
- `cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings` 均 exit 0，`node --test tests/pc/service.test.mjs` 39 passed / 0 failed。独立复核无阻塞。详细原始输出存于 `/data/knoted-dev/tasks/t53/evidence/`。
- dev 部署前停止本实例两个写入进程并验证无在途任务，备份 PostgreSQL 与 assets 且重算摘要。固定提交 `de8fbcfbe48441e69a872b6d2193c52cbd625b47` 从 Git 归档构建 API/Worker，复用经 Git 差异验证的 Web/Python 字节。校验 38,325 项，清单 SHA-256 为 `87129c2aefa8af59b716b104ff3ad98192613f8ec76e1ce341e4d1e1a8102246`。备份和部署结果保存在本机任务证据，API 和 Worker 的执行路径均指向该 SHA。共享 Caddy、OnlyOffice 和 PostgreSQL PID 前后一致。
- 迁移 0016 和 0017 应用成功。pcwrap 以 `KNOTED_SEARCH_ENABLED=1` 渲染全文检索配置，API HTTPS/Python readiness 都通过。一个带中文长句的合成聊天投影经真实 Worker 从队列发布，账本返回 `published:851:succeeded`。实际 Meilisearch task 851 为 `succeeded`，查询「知识管理」只命中该文档键一次。
- 验收使用宿主现有回环 Meilisearch 1.56.1。持久化服务管理仍由 [#186](https://github.com/xingjianxu/knoted/issues/186) 跟踪。T54 将接入真实业务投影和搜索 API。重启本 dev 编排时显式传入 `KNOTED_SEARCH_ENABLED=1`。如引擎有访问密钥，`KNOTED_SEARCH_CREDENTIAL_FILE` 接受持有 `[secrets].meilisearch` 的受保护 TOML 文件路径。
