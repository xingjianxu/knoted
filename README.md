# Knoted

Knoted 提供 Passkey 单账号登录、带版本历史的笔记、项目与事项、只读文件采集和预览、中文全文检索及带引用的问答。应用由 Rust API 与 Worker、React/TypeScript Web、Python intelligence 组成，使用 PostgreSQL 和 Meilisearch。具体能力、验证版本和限制见 [Knoted v1 任务](docs/plans/knoted-v1/README.md) 与 [架构现状](docs/architecture/README.md)。

UI 方向已确认：通用组件直接消费 npm `@bixlab/mtone`，本项目只做业务组合与适配。T17—T19 已全部完成并集成主线：T17 精确接入 `@bixlab/mtone@0.2.0`、公开 Provider/CSS 并移除 `packages/ui` 占位包（PR #42 / `e3e1db2`）；T18 注册应用通知 Provider 并验收弹层菜单（PR #46 / `4425ab9`）；T19 交付应用游标 reducer 并验收树/列表/反馈（PR #48 / `6de79a8`）。当前精确消费版本为 **`@bixlab/mtone@0.5.0`**，消费复验见 [0.5.0 升级记录](docs/development/ui-mtone.md#050-升级与当前消费约定)；上述 0.2.0 为 T17—T19 历史接入版本。详见 [T17](docs/plans/knoted-v1/evidence/t17.md)、[T18](docs/plans/knoted-v1/evidence/t18.md)、[T19](docs/plans/knoted-v1/evidence/t19.md) 证据。见 [UI 指南](docs/development/ui-mtone.md) 与 [ADR 0002](docs/decisions/0002-mtone-ui-ownership.md)。

## 开始使用

1. 阅读 [AGENTS.md](AGENTS.md) 和 [Agent 工作流](docs/development/agent-workflow.md)。
2. 检查 Superpowers、git-commit 的实际安装与加载；参照 [Agent 适配](docs/development/agent-adapters.md)。
3. 选择已由人工批准的任务，按 [Worktree 指南](docs/development/git-worktree.md) 在 `.worktrees/` 隔离开发。
4. 在任务 Worktree 执行本地门禁（仓库不使用 GitHub CI，这些检查是合并前的唯一门禁）：

   ```bash
   cargo fmt --check
   cargo test --workspace --locked
   cargo clippy --workspace --all-targets -- -D warnings
   pnpm install --frozen-lockfile
   pnpm api:check          # OpenAPI 契约与样例
   pnpm api:verify         # 生成客户端是否与契约同步（需干净工作区）
   pnpm run typecheck && pnpm run lint && pnpm run test && pnpm test:pc && pnpm run build
   ```

## 测试实例

测试实例（`https://wlab.132140.xyz:15601/`）由 [pcwrap](https://www.npmjs.com/package/@bixlab/pcwrap) + process-compose 管理，控制端口 15600：

```bash
pnpm pc up -D                        # prepare → migrate → intelligence、API 和 Worker
pnpm pc process list
pnpm pc process restart knoted-api   # 只重启 API，不重跑迁移
pnpm pc process logs knoted-api -n 200
pnpm pc down                         # 停止实例（不影响 PostgreSQL 等外部依赖）
```

运行根是固定提交产物（`tools/dev/build-artifact.sh` → `/data/knoted-dev/runtime/<sha>`），配置由 `tools/dev/pc/*.tmpl` 渲染。pcwrap 初次接管测试实例见 [PR #64 的历史验证](docs/plans/knoted-v1/evidence/pcwrap-dev-service.md)。HTTPS 由宿主共享 Caddy 提供。dev 使用 `knoted_dev`，数据库由宿主用户创建。独立生产实例由 NixOS 和共享 Caddy 运行，使用另一数据库和数据根。固定发布、生产入口与回退见 [T72 交接记录](docs/plans/knoted-v1/evidence/t72-deployment.md)。[ADR 0003](docs/decisions/0003-test-instance-process-management.md) · [ADR 0004](docs/decisions/0004-database-owned-by-host-user.md)

## 流程入口

| 需要做什么 | 入口 |
|---|---|
| 记录发现的问题、查重 | [issue-create](.agents/skills/issue-create/SKILL.md) |
| 检查任务是否可执行 | [issue-triage](.agents/skills/issue-triage/SKILL.md) |
| 完成已批准任务 | [issue-work](.agents/skills/issue-work/SKILL.md) |
| 评估重要架构变更 | [architecture-review](.agents/skills/architecture-review/SKILL.md) |
| 记录决策、分配 ADR 编号 | [adr-create](.agents/skills/adr-create/SKILL.md) |
| 判断组件归属、交接 mtone 能力缺口 | [mtone-component-request](.agents/skills/mtone-component-request/SKILL.md) |
| 按范围审计项目 | [project-audit](.agents/skills/project-audit/SKILL.md) |
| 查看 GitHub 远程配置现状与复核命令 | [远程配置记录](docs/development/github-setup.md) |
| 查看能力检查与初始化交付记录 | [初始化报告](docs/development/initialization-report.md) |

调用示例：Codex 使用 `$issue-work` 并给出 Issue 编号或任务描述；Pi 使用 `/skill:issue-work`。不支持自动发现时直接读取该 `SKILL.md`，按同一规则执行。原生工具语法只在适配文档中维护。

## GitHub 默认授权

Issue、Labels、Projects、push、PR 创建与合并默认已授权，不再逐次索取许可；仅 ruleset、分支保护、webhook、协作者、标签名增删与仓库设置类操作需要单独确认。保留的人工确认点只有重大架构决策和不可逆/生产变更。详见 [AGENTS.md](AGENTS.md) 的「远程操作授权」。
