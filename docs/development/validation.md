# 验证与质量证据

每次变更只运行与范围相关的验证。验证失败先定位根因，不要修改测试或忽略错误以制造通过。仓库不运行 GitHub CI，**本地门禁输出是唯一合并依据**，PR 正文必须记录本轮真实命令与结果。

## 代码与契约检查

在任务 Worktree 按改动范围运行并检查每条命令退出码：

```bash
# Rust
cargo fmt --check
bash tools/test/rust.sh
cargo clippy --workspace --all-targets -- -D warnings

# Web 与 OpenAPI 契约
pnpm install --frozen-lockfile
pnpm api:check
pnpm api:verify
pnpm run typecheck && pnpm run lint && pnpm run test && pnpm test:pc && pnpm run build

# Python intelligence（改动 services/intelligence 时）
# 在 services/intelligence 目录执行：
# uv run --group dev pytest && uv run --group dev ruff check && uv run --group dev ruff format --check

# Git 基础卫生
git diff --check
git diff --cached --check
git check-ignore -v .worktrees/
git status --short --untracked-files=all
```

测试通过只证明已运行用例覆盖的场景；新增或修改行为时补写有意义的回归测试，涉及运行行为变更时，在隔离实例复核并记录实际 SHA。开发实例的写入验证需要用户明确要求。

## 程序测试环境

- Rust 全量入口 `bash tools/test/rust.sh` 调用 `cargo test --workspace --locked`，额外参数原样传给 Cargo。安装带中文分词的 Meilisearch，或用 `KNOTED_TEST_MEILI_BIN` 指定可执行文件。入口在 `127.0.0.1:17830` 启动一次性搜索进程，把 Meilisearch 数据及 `TMPDIR` 放入 `/data/knoted_dev/rust.*`，退出后清理。已有监听者时直接失败。真实分块和 intelligence 用例需要固定 BGE-M3 模型根与 Python 环境，分别通过 `KNOTED_INTELLIGENCE_BGE_M3_DIR` 和 `KNOTED_TEST_PYTHON` 设置。
- PostgreSQL 默认通过本机 socket 连接 `knoted_test`，每用例建临时 schema。可用 `KNOTED_TEST_DATABASE_URL` 指定其他以 `_test` 结尾的库。直接运行单项真实搜索测试时，显式将 `KNOTED_TEST_MEILI_URL` 指向专用测试实例。
- [T71 夹具](../../tests/fixtures/t71/instance.sh) 在 `/data/knoted_dev/t71/instance.*` 内启动一次性的 API、Worker、Python、PostgreSQL、Meilisearch 和 Caddy。以固定构建产物设置 `T71_ARTIFACT`，以安装前缀设置 `T71_PGVECTOR`，准备锁定模型根后运行 `bash tests/fixtures/t71/instance.sh pnpm exec playwright test tests/e2e/t71.spec.ts`。夹具使用 `17820–17829`，退出后停止进程并回收当次状态。

## 文档、Skills、钩子与 GitHub 模板

- **Markdown 链接与 ADR**：检查所有本地 Markdown 链接指向存在的文件（模板占位符除外）；检查 ADR 编号唯一、状态合法、Accepted 有人工证据、替代关系双向完整。
- **任务索引与卡片**：核对 `docs/tasks/<编号>.md` 的文件名、GitHub Issue 号、原生 Type、前置关系与 `docs/tasks/README.md` 索引条目一致。
- **Skills**：当前仓库保留 4 个项目专用 Skill（`adr-create`、`architecture-review`、`mtone-component-request`、`project-audit`）及带 AI 署名规范的 `git-commit`；均须具备合法 YAML frontmatter（`name`、`description`、`name` 与目录名一致且 ≤ 64、`description` ≤ 1024）。
- **Git 钩子**：对 `.githooks/prepare-commit-msg` 执行 `bash -n` 语法检查并确认具备可执行权限（`test -x`），必要时用临时消息文件验证 trailer 注入与防重复逻辑。
- **Issue 模板**：使用真实 YAML 1.2 解析器（如环境已有 Node `yaml` 或 Python `PyYAML`）解析 `.github/ISSUE_TEMPLATE/{feature,bug,task}.yml`，检查 `name`/`description`/`type`（`Feature`/`Bug`/`Task`）以及 `body` 中各字段的 `id` 唯一、类型合法、`label` 非空与布尔 `required`；不使用仅靠正则的伪校验。浏览器选择页渲染仍需人工确认。

## 协作流程场景核对

| 场景 | 预期结果 |
|---|---|
| 新发现缺陷或新目标 | 先查重，确认无重复后用 `gh issue create --type Bug\|Feature\|Task` 建单并按返回编号建 `docs/tasks/<编号>.md` |
| 已有任务准备开工或接续 | 先查 `gh issue develop <编号> --list`、`git worktree list` 与 Issue 评论；复用已有工作或新建 `issue-<编号>-<slug>` 与 `.worktrees/<编号>-<slug>` 并回写 Issue |
| 存在直接前置依赖 | 在 GitHub Issue 设置原生 `blocked-by` 关系，并在任务卡与索引同步列出 |
| 本地检查通过但 PR 尚未合入 | 保留 Worktree 与分支，等待合入并完成主线复核后才清理 |
| 新方案与 Accepted ADR 冲突 | 走 `architecture-review` 并提交提案待人工确认，保留旧 ADR 历史 |
| 审计发现凭据或漏洞 | 仅提供脱敏摘要，不将凭据写入任何提交、Issue、PR 或日志 |
| `gh` 查询失败 | 标明查重未完成，不断言无重复 |
| 缺少提交钩子配置 | 按 `AGENTS.md` 执行 `git config --local core.hooksPath .githooks`，并在提交时显式使用 `--trailer` 写 `Co-authored-by:` 与 `Model:` |

## 提交、集成与收尾证据

- PR 创建前完成独立审查并核实重要反馈，PR 正文写明 head/base SHA、环境、实际命令与结果、验收情况及未覆盖范围。
- 合入 `master` 后按 [Agent 任务工作流](agent-workflow.md#交付定义与远程授权) 同步主线、复核并更新开发服务器，将 PR 链接、合并 SHA、验证摘要和部署结果回写原 Issue。确认 Issue 已关闭且部署验证通过后，再安全清理本任务 Worktree 与已合并本地分支。不以文件数、模板勾选或口头声称代替真实命令输出。
