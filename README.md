# Knoted

当前代码为 Rust 2024 初始骨架，无外部依赖。项目功能尚未定义；开发管理规范可移植到其他语言和 Agent。

## 开始使用

1. 阅读 [AGENTS.md](AGENTS.md) 和 [Agent 工作流](docs/development/agent-workflow.md)。
2. 检查 Superpowers、git-commit 的实际安装与加载；参照 [Agent 适配](docs/development/agent-adapters.md)。
3. 选择已由人工批准的任务，按 [Worktree 指南](docs/development/git-worktree.md) 在 `.worktrees/` 隔离开发。
4. 在任务 Worktree 执行：

   ```bash
   cargo fmt --check
   cargo test
   cargo clippy --all-targets -- -D warnings
   ```

## 流程入口

| 需要做什么 | 入口 |
|---|---|
| 记录发现的问题、查重 | [issue-create](.agents/skills/issue-create/SKILL.md) |
| 检查任务是否可执行 | [issue-triage](.agents/skills/issue-triage/SKILL.md) |
| 完成已批准任务 | [issue-work](.agents/skills/issue-work/SKILL.md) |
| 评估重要架构变更 | [architecture-review](.agents/skills/architecture-review/SKILL.md) |
| 记录决策、分配 ADR 编号 | [adr-create](.agents/skills/adr-create/SKILL.md) |
| 按范围审计项目 | [project-audit](.agents/skills/project-audit/SKILL.md) |
| 配置 GitHub 看板、标签、CI | [待授权配置方案](docs/development/github-setup.md) |
| 查看本次能力检查和交付 | [初始化报告](docs/development/initialization-report.md) |

调用示例：Codex 使用 `$issue-work` 并给出 Issue 编号、仓库和授权范围；Pi 使用 `/skill:issue-work`。不支持自动发现时直接读取该 `SKILL.md`，按同一规则执行。原生工具语法只在适配文档中维护。
