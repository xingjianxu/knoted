# Coding Agent 适配与能力检查

业务规则只维护在 [AGENTS.md](../../AGENTS.md) 和 `.agents/skills/`。这里集中说明多 Agent 平台的 Skill 发现、提交署名与降级规则，不把某个 Agent 的私有工具语法放进通用规则。

## 当前项目 Skills 清单

本仓库在 `.agents/skills/` 下维护 5 个 Skill：

- `git-commit`：Conventional Commits 提交流程及 AI 署名（`Co-authored-by:` + `Model:` trailer），配套兜底钩子位于 `.githooks/prepare-commit-msg`。`.claude/skills/git-commit` 指向同一文件，予以保留。
- `architecture-review`：重大架构变更评估与 RFC 起草。
- `adr-create`：ADR 编号分配、记录与替代维护。
- `mtone-component-request`：判断 `@bixlab/mtone` 组件归属与上游需求交接。
- `project-audit`：按授权范围执行项目审计并登记脱敏 Issue。

原 `issue-create` / `issue-triage` / `issue-work` 标签流 Skill 已移除，常规 GitHub Issue 协作（原生 Type、`blocked-by`、`docs/tasks/<编号>.md`、`gh issue develop`）已直接收拢至 [AGENTS.md](../../AGENTS.md) 与 [工作流详情](agent-workflow.md)。

## 提交署名与钩子适配

新克隆或首次启用钩子时在主仓库根目录执行一次：

```bash
git config --local core.hooksPath .githooks
```

- 各 Agent 按 `.agents/skills/git-commit/SKILL.md` 写入对应 `Co-authored-by:`（`claude` / `codex` / `agy` / `pi` 等）与 `Model:` trailer。
- `.githooks/prepare-commit-msg` 会检测运行环境（环境变量与父进程树），仅在 AI Agent 提交且缺少对应 trailer 时自动补全，跳过手工人工提交与 `merge`/`squash` 提交。

## 平台发现与降级规则

1. **目录发现**：Codex、Pi、Antigravity 等支持 `.agents/skills/` 的工具从当前任务 Worktree 向仓库根扫描；在任务 Worktree 中读取该 Worktree 内的 `AGENTS.md` 与 Skills，不假定主工作区未提交的改动会自动出现在另一 Worktree。
2. **显式调用**：Codex 可用 `$adr-create` 等形式，Pi 可用 `/skill:adr-create`；不支持自动发现的平台直接读取对应 `SKILL.md` 并遵循同一规则。
3. **Worktree 路径**：无论平台是否提供原生 Worktree 工具，均遵守主仓库根目录 `.worktrees/<编号>-<slug>` 的固定路径约定；若平台原生工具无法指定该路径或独立分支，改用 `gh issue develop` + `git worktree add`。
4. **能力降级**：无 Subagent 能力时，主 Agent 按 [委派模板](subagent-task-template.md) 相同边界串行执行并在交付中说明；无远程权限或 `gh` 不可用时，保留本地成果与草稿并如实报告，不声称已同步远程。
5. **Skill 格式**：遵循 [Agent Skills 标准](https://agentskills.io/specification)，包含合法 `name` 与 `description` YAML frontmatter。
