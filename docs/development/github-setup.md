# GitHub 远程配置状态与操作记录

本文记录 `xingjianxu/knoted` 的**实际远程状态、验证命令与历史变更**；不把本文当成无条件脚本执行。写操作遵循 [AGENTS.md](../../AGENTS.md) 的「远程操作授权」：常规 Issue（原生 Type、`blocked-by`、评论与关闭）、`gh issue develop`、push 到非默认分支、PR 创建与普通合并默认已授权；仓库级设置（ruleset、分支保护、webhook、协作者与权限、Actions 设置、仓库可见性与删除、标签名增删）仍需逐次人工确认。

## 当前实际状态

| 项 | 实际值 |
|---|---|
| 默认分支 | `master`（GraphQL `defaultBranchRef` 实测） |
| 原生 Issue Types | `Feature`、`Bug`、`Task`（GraphQL `repository.issueTypes` 实测可用；新流程全面采用） |
| Issue 模板 | `.github/ISSUE_TEMPLATE/{feature,bug,task}.yml`（原生 `type:` 字段） |
| 关联分支工作流 | `gh issue develop <编号> --base master --name issue-<编号>-<slug>`（已实测可用） |
| 历史自定义标签 | 远端仍保留初始化时的 27 个标签作为历史审计，新流程不再用状态/来源/优先级标签驱动流转 |
| 历史 Projects | #1「Knoted Development」保留为历史记录，新流程不再同步看板字段 |
| 规则集 | `27256364` master protection（active；仅 `deletion`、`non_fast_forward`、`pull_request` 三条，无远程必需检查） |
| Quality CI | 已移除（不运行 GitHub Actions 质量检查，本地门禁是唯一合并门槛） |
| Actions | enabled，`allowed_actions: all`，`sha_pinning_required: false` |
| 仓库合并策略 | 仅普通 merge commit（`allow_merge_commit=true`、`allow_squash_merge=false`、`allow_rebase_merge=false`、`delete_branch_on_merge=true`） |

## 1. 身份与凭据

- `gh auth status` 实测包含 `repo` 等常规写权限（如需读取历史 Projects 才需 `project` scope）。
- Git 推送凭据与 `gh` token 互相独立（本机 `credential.helper = store --file ${XDG_RUNTIME_DIR}/agenix/git-creds`）。
- 具备 token scope 不等于放开仓库级设置授权；规则集、分支保护、标签定义增删等仍须人工确认。

## 2. 原生 Issue Type、依赖与关联分支

2026-11-23 起流程切换为 GitHub 原生 Issue 协作方式（见 [#270](https://github.com/xingjianxu/knoted/issues/270)）：

- 建单统一使用 `gh issue create --repo xingjianxu/knoted --type Feature|Bug|Task --title '...' --body-file <正文文件>`。
- 直接前置依赖使用 GitHub 原生 `blocked-by` 关系维护。
- 开工使用 `gh issue develop <编号> --base master --name issue-<编号>-<slug>` 建立与 Issue 关联的分支，再在主仓库 `.worktrees/<编号>-<slug>` 检出工作区。
- 2026-11-23 已为迁移时的 15 个历史 open Issue 补设原生 Issue Type（`Bug` 5 个、`Task` 10 个），保留原有标签未做仓库级增删。
- **未验证项：** GitHub Web 端 Issue Forms 选择页渲染需由已登录浏览器人工确认；CLI 与 YAML 解析器不替代浏览器表单渲染。

## 3. 分支规则集与合并策略

规则集 `27256364`（`master protection`）保护 `refs/heads/master`：

- `deletion`、`non_fast_forward`、`pull_request`（`required_approving_review_count: 0`、`allowed_merge_methods: ["merge"]`、`dismiss_stale_reviews_on_push: true`、`required_review_thread_resolution: true`）。
- `bypass_actors: []`、`current_user_can_bypass: never`，仓库 owner 也不能直推 `master`，所有变更必须走 PR。
- 合并使用 `gh pr merge <PR> --merge --match-head-commit <已验证SHA>`；合并后服务端按 `delete_branch_on_merge=true` 自动删除远程任务分支。
- 规则集修改属仓库设置类，不在默认授权范围。

## 4. 历史记录（首次初始化与历史重建）

2026-11-14 首次推送前本地 `.git` 曾重新初始化，`master` 最初 4 个提交按文档记录重建：

| 新哈希 | 提交 | 覆盖内容 |
|---|---|---|
| `096d2eb` | `chore: 纳管现有 Rust 项目骨架` | `Cargo.toml`、`Cargo.lock`、`src/main.rs`、原始 `.gitignore` |
| `db39b2e` | `chore(skills): 纳管已安装的提交 Skill` | `.agents/skills/git-commit`、`.claude` 链接、`skills-lock.json` |
| `e7bc8ed` | `chore(workflow): 建立 Agent 驱动开发管理体系` | 初版 AGENTS.md、Skills、模板、CI、文档 |
| `c34db46` | `docs(workflow): 记录初始化验证与本地集成结果` | 初始化记录与计划账本 |

同日经用户确认删除 `Quality` 工作流并从 ruleset `27256364` 移除 `required_status_checks`（`updated_at: 2026-11-14T20:50:32.010+08:00`），此后本地检查成为唯一合并门禁。

## 5. 复核命令（只读）

```bash
gh auth status
gh api graphql -f query='{repository(owner:"xingjianxu",name:"knoted"){defaultBranchRef{name} issueTypes(first:10){nodes{name description}}}}'
gh api repos/xingjianxu/knoted --jq '{default_branch,allow_merge_commit,allow_squash_merge,allow_rebase_merge,delete_branch_on_merge}'
gh ruleset list --repo xingjianxu/knoted
gh api repos/xingjianxu/knoted/rulesets/27256364
gh issue list --repo xingjianxu/knoted --state open --json number,title,issueType
```
