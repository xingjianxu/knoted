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
- 测试：`crates/knoted-core/tests/t53.rs`（6 例）。

## 实现要点

- 每来源/版本一个确定性文档键 `<type>_<uuid 无连字符>_v<version>`，满足 Meilisearch 文档 id 字符限制；重试是同键 upsert。
- 发布事务：`source_documents` 当前版本校验（advisory lock 串行化同一来源）→ `fulltext_publications` → `jobs::enqueue`（同时写 `outbox`，主题 `job.enqueued.fulltext.publish`）。
- 状态机 `pending → submitted（task id 落库）→ published | failed`，另有 `superseded`；`published_at` 只能与 `published` 同事务出现。
- 只有 Meili 任务 `succeeded` 才写 `published`；`enqueued`/`processing` 返回可重试失败，且重试复用已落库的 task id，不重复提交。
- 旧版本发布标记 `superseded` 并入队 `fulltext.cleanup`；清理按 `keep_version` 批量删除旧文档键，成功后删除旧投影与发布记录。可见性不依赖删除速度（读时复核属 T54）。
- 引擎拒绝（4xx）为永久失败、传输/5xx 为可重试失败；租约丢失（接管）时归还 `Cancelled`，不写 `published`。
- 索引设置：`searchableAttributes=[title, body]`、9 个过滤字段、`sortableAttributes=[updated_at, version]`；设置漂移时才重写并等待任务确认。

## 验证

环境：真实 PostgreSQL 测试库 + 真实 Meilisearch 1.56.1（官方 release 二进制，含 charabia 全语言 tokenizer；nixpkgs 默认包 `buildNoDefaultFeatures` 缺中文分词，实测只能命中前缀）。

| 命令 | 结果 |
|---|---|
| `cargo test --workspace --locked t53_` | exit 0，`t53_` 6 passed / 0 failed |
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

基线失败（与本卡无关，未修改主线）：`knoted-worker --test t47_pipeline` 的 `t47_real_scan_capture_ocr_publication_and_unchanged_no_retry` 在未修改的主检出上同样失败（`models_missing`/`artifacts_unset`），属本机 OCR/嵌入模型环境缺失；`t47_http` 的真实 Python 测试需要 `KNOTED_TEST_PYTHON` 指向主检出 venv（工作区无 `.venv`）。

## 部署判断

- dev 实例**未部署**：运行期检索依赖是 Meilisearch，而编排仍将 `127.0.0.1:15605` 预留且未包含该服务（`docs/plans/knoted-v1/development-and-deploy.md`）；接入属外部依赖/host 任务，本卡不修改 `/etc/nixos`，也不把未运行的服务写成已启用。
- 迁移 `0016` 为新增表，不改变现有读写路径；`[search]` 默认 `enabled = false`，未配置时 worker 仍正常运行（调度器只在有任务时领取）。下次部署产物会一并应用迁移，届时无 `[search]` 段即保持关闭。

## 限制

- 未实现 `/search`（T54）、索引 generation 重建与前端（T55）。
- 引擎中可能短暂保留被取代版本的文档；统一搜索必须按 PG 当前版本/删除状态复核（T54 验收项）。
- 中文分词质量取决于 Meilisearch 的 charabia 构建；部署必须使用含全语言 tokenizer 的构建（官方 release），否则只能命中词首前缀。
