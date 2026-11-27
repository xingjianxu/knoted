# Knoted

Knoted 提供 Passkey 单账号登录、带版本历史的笔记、项目与事项、自动采集和预览只读文件，以及笔记、项目和事项的中文全文搜索。应用由 Rust API 与 Worker 和 React/TypeScript Web 组成，使用 PostgreSQL 与 Meilisearch。文件索引和 RAG 的旧实现可从归档分支取回。[当前架构](docs/architecture/README.md) · [退出决策](docs/decisions/0014-retire-file-index-rag.md)

UI 方向已确认：通用组件直接消费 npm `@bixlab/mtone`，本项目只做业务组合与适配。当前锁定版本及升级记录见 [UI 指南](docs/development/ui-mtone.md#现状与目标) 与 [ADR 0002](docs/decisions/0002-mtone-ui-ownership.md)。

## 开始使用

1. 阅读 [AGENTS.md](AGENTS.md) 和 [任务索引](docs/tasks/README.md)。
2. 新克隆或首次启用钩子时在仓库根目录执行：`git config --local core.hooksPath .githooks`。
3. 查重后通过 GitHub Issue（原生 Type：`Feature` / `Bug` / `Task`）与 `docs/tasks/<编号>.md` 确定交付目标，使用 `gh issue develop` 创建关联分支 `issue-<编号>-<slug>` 并在主仓库 `.worktrees/<编号>-<slug>` 隔离开发（详见 [Worktree 指南](docs/development/git-worktree.md)）。
4. 在任务 Worktree 执行本地门禁（仓库不使用 GitHub CI，这些检查是合并前的唯一门禁）：

   ```bash
   cargo fmt --check
   bash tools/test/rust.sh
   cargo clippy --workspace --all-targets -- -D warnings
   pnpm install --frozen-lockfile
   pnpm api:check          # OpenAPI 契约与样例
   pnpm api:verify         # 生成客户端是否与契约同步（需干净工作区）
   pnpm run typecheck && pnpm run lint && pnpm run test && pnpm test:pc && pnpm run build
   ```

## 开发实例

开发实例（`https://wlab.132140.xyz:15601/`）由 [pcwrap](https://www.npmjs.com/package/@bixlab/pcwrap) + process-compose 管理，控制端口 15600：

```bash
pnpm deploy:dev                      # 取最新主线：构建 → 切换 → 重启 → 验证
pnpm pc up -D                        # prepare → migrate → API 和 Worker
pnpm pc process list
pnpm pc process restart knoted-api   # 只重启 API，不重跑迁移
pnpm pc process logs knoted-api -n 200
pnpm pc down                         # 停止实例（不影响 PostgreSQL 等外部依赖）
```

运行根是固定提交产物（`tools/dev/build-artifact.sh` → `/data/knoted-dev/runtime/<sha>`），配置由 `tools/dev/pc/*.tmpl` 渲染。日常发版用 `pnpm deploy:dev`。HTTPS 由宿主共享 Caddy 提供。dev 使用 `knoted_dev`，数据库由宿主用户创建。独立生产实例由 NixOS 和共享 Caddy 运行，使用另一数据库和数据根。生产实例由宿主为当前提交构建固定产物。[旧部署交接](docs/plans/knoted-v1/evidence/t72-deployment.md)[ADR 0003](docs/decisions/0003-test-instance-process-management.md) · [ADR 0004](docs/decisions/0004-database-owned-by-host-user.md) · [ADR 0005](docs/decisions/0005-https-owned-by-caddy.md)

## 程序测试

Rust 全量测试通过 `bash tools/test/rust.sh` 运行。文件夹具和临时 Meilisearch 位于 `/data/knoted_dev`，数据库使用 `knoted_test` 的临时 schema。浏览器集成夹具在同一测试根下启动独立 PostgreSQL、搜索和应用进程，退出后删除此次状态。依赖和命令见 [验证方法](docs/development/validation.md#程序测试环境)。

## 流程入口

| 需要做什么 | 入口 |
|---|---|
| 浏览与领取开发任务 | [任务索引](docs/tasks/README.md) |
| 查看完整协作规范与 GitHub Issue 流程 | [AGENTS.md](AGENTS.md) · [工作流详情](docs/development/agent-workflow.md) |
| 隔离开发、PR 交付与主线清理 | [Worktree 指南](docs/development/git-worktree.md) |
| 提交代码与 AI 署名规范 | [git-commit](.agents/skills/git-commit/SKILL.md) |
| 评估重要架构变更 | [architecture-review](.agents/skills/architecture-review/SKILL.md) |
| 记录决策、分配 ADR 编号 | [adr-create](.agents/skills/adr-create/SKILL.md) |
| 判断组件归属、交接 mtone 能力缺口 | [mtone-component-request](.agents/skills/mtone-component-request/SKILL.md) |
| 按范围审计项目 | [project-audit](.agents/skills/project-audit/SKILL.md) |
| 查看 GitHub 远程配置现状与复核命令 | [远程配置记录](docs/development/github-setup.md) |
| 查看历史规划归档（v1 / Web 重设计） | [实施计划归档](docs/plans/README.md) |

## GitHub 默认授权

GitHub Issue（原生 Type、`blocked-by`、评论与关闭）、`gh issue develop`、push 到非默认分支、PR 创建与普通合并默认已授权，不再逐次索取许可；仅 ruleset、分支保护、webhook、协作者、标签名增删与仓库设置类操作需要单独确认。保留的人工确认点只有重大架构决策和不可逆/生产变更。详见 [AGENTS.md](AGENTS.md) 的「远程操作授权」。
