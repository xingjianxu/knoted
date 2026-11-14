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
| 查看 GitHub 远程配置现状与复核命令 | [远程配置记录](docs/development/github-setup.md) |
| 查看能力检查与初始化交付记录 | [初始化报告](docs/development/initialization-report.md) |

调用示例：Codex 使用 `$issue-work` 并给出 Issue 编号或任务描述；Pi 使用 `/skill:issue-work`。不支持自动发现时直接读取该 `SKILL.md`，按同一规则执行。原生工具语法只在适配文档中维护。

## GitHub 默认授权

Issue、Labels、Projects、push、PR 创建与合并默认已授权，不再逐次索取许可；仅 ruleset、分支保护、webhook、协作者与仓库设置类操作需要单独确认。保留的人工确认点只有重大架构决策和不可逆/生产变更。详见 [AGENTS.md](AGENTS.md) 的「远程操作授权」。
