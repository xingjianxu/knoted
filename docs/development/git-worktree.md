# Worktree 隔离、集成与清理

所有文件编辑（含文档与配置）均在主仓库根目录 `.worktrees/<编号>-<slug>` 的独立任务分支中完成；主工作区仅用于协调、集成和同步 `master`，保留用户未提交修改，不回退到主工作区直接开发。

## 定位与预检

在主仓库根目录执行：

```bash
git rev-parse --show-toplevel
git rev-parse --path-format=absolute --git-common-dir
git worktree list --porcelain
git status --short --untracked-files=all
git check-ignore -v .worktrees/
gh auth status
gh repo view --json nameWithOwner,url,defaultBranchRef
```

- 当前目录若已是链接 Worktree，`--show-toplevel` 返回的是任务目录，不得在其内部嵌套创建 `.worktrees/`；务必通过 `git worktree list` 与 `--git-common-dir` 定位**主仓库根目录** `PROJECT_ROOT`。
- 本项目默认分支为 `master`（以 `defaultBranchRef.name` 实测为准）。`git fetch origin` 不覆盖用户未提交内容；主工作区若有未提交/未跟踪文件，记录范围，不 stash、不 reset、不 checkout 覆盖，也绝不动其他任务的 Worktree。

## 创建与复用（`gh issue develop`）

分支命名统一为 `issue-<编号>-<slug>`，Worktree 路径固定为 `$PROJECT_ROOT/.worktrees/<编号>-<slug>`（`slug` 取自任务卡的简短英文描述）。先检查是否已有该 Issue 的关联分支、Worktree 或 Issue 评论记录，接续同一任务时直接复用：

```bash
issue=270
slug=github-issue-workflow
PROJECT_ROOT=$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")
base=$(gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name')
branch="issue-$issue-$slug"
tree="$PROJECT_ROOT/.worktrees/$issue-$slug"

git -C "$PROJECT_ROOT" fetch origin
git -C "$PROJECT_ROOT" check-ignore -v .worktrees/
gh issue develop "$issue" --list
git -C "$PROJECT_ROOT" worktree list

# 若尚未创建关联分支：
gh issue develop "$issue" --base "$base" --name "$branch"
git -C "$PROJECT_ROOT" fetch origin "$branch"
git -C "$PROJECT_ROOT" worktree add --track -b "$branch" "$tree" "origin/$branch"
cd "$tree"
git rev-parse HEAD
```

- 若 `gh issue develop "$issue" --list` 已列出目标分支：直接 `git fetch origin "$branch"`，若尚无对应 Worktree 再执行 `git worktree add --track -b "$branch" "$tree" "origin/$branch"`（本地分支已存在时用 `git worktree add "$tree" "$branch"`）。
- 根 `.gitignore` 必须持续忽略 `.worktrees/` 且 `git check-ignore -v .worktrees/` 成功。
- 创建或接续后，立即在对应 GitHub Issue 评论记录分支名、Worktree 路径与默认分支起点 SHA。

## 子任务隔离与集成

写文件的每个 Subagent 均使用主仓库根下的独立路径 `.worktrees/agent-<name>-<编号>` 与独立分支，禁止嵌套到主任务 Worktree 下。主 Agent 协调文件范围并串行处理强耦合改动；子 Agent 只提交任务分支，主 Agent 独立审查 diff 和测试后在主任务 Worktree 集成，不让多个 Agent 写主线。

## PR 交付与合并

推送前在干净的任务 Worktree 核实当前 SHA、默认分支基线 SHA 并运行全部适用本地门禁：

```bash
git push -u origin "$branch"
# BODY_FILE 为按 .github/PULL_REQUEST_TEMPLATE.md 填写的 UTF-8 正文文件
gh pr create --base "$base" --head "$branch" --body-file "$BODY_FILE" --title "$PR_TITLE"
gh pr view "$PR_NUMBER" --json number,url,state,headRefOid,baseRefName,mergeCommit,mergedAt
```

- 仓库不运行 GitHub CI，`gh pr checks` 为空不代表检查通过；合并依据是 PR 正文记录的本轮真实本地检查输出与独立审查结果。
- 若审查后更新了 head 或默认分支已前进，先同步基线并重新运行本地检查。
- 本仓库仅允许普通 merge commit（`allow_merge_commit=true`、`allow_squash_merge=false`、`delete_branch_on_merge=true`）。默认授权使用精确版本锁合并：

  ```bash
  gh pr merge "$PR_NUMBER" --merge --match-head-commit "<已验证的HEAD_SHA>"
  ```

- 不直推 `master`、不使用 `--admin`；用户明确要求先看 PR 时保留待合并状态与 Worktree。

## 主线同步、回写与清理

1. 确认 PR 已合入 `master` 并记录 `mergeCommit` SHA。
2. 回到干净的主工作区同步并做主线复核验证：

   ```bash
   cd "$PROJECT_ROOT"
   git fetch origin
   git pull --ff-only origin "$base"
   git rev-parse HEAD
   ```

   无法快进时停下调查，不 `reset`。在主线上复核必要检查并记录主线 SHA。
3. 将 PR 链接、合并 SHA 和验证摘要回写原 GitHub Issue 评论，确认 Issue 已关闭（单次/最终 PR 由 `Closes #<编号>` 自动关闭）。
4. 仅在主线验证通过后清理**本次任务**的 Worktree 与已合并分支：

   ```bash
   git -C "$tree" status --short --untracked-files=all
   git -C "$tree" status --short --ignored
   git -C "$PROJECT_ROOT" merge-base --is-ancestor "$branch" "$base"
   git -C "$PROJECT_ROOT" worktree remove "$tree"
   git -C "$PROJECT_ROOT" branch -d "$branch"
   git -C "$PROJECT_ROOT" worktree list
   ```

- 若任务 Worktree 仍有未提交或需保留的忽略文件、`merge-base --is-ancestor` 失败或 `git branch -d` 拒绝，绝不使用 `--force` 或 `-D`，保留工作区并报告原因。
- `git worktree prune` 仅在 `--dry-run` 确认无其他任务记录时才可执行；不清理其他任务的 Worktree 或分支。
