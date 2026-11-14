# Worktree 隔离、集成与清理

本指南是 `using-git-worktrees` / `finishing-a-development-branch` 的项目路径和交付约束，不是新 Skill。主工作区只协调和集成，所有文件编辑在任务 Worktree。

## 定位和预检

```bash
git rev-parse --show-toplevel
git rev-parse --path-format=absolute --git-common-dir
git worktree list --porcelain
git status --short --untracked-files=all
git branch -avv
git remote -v
git symbolic-ref --quiet refs/remotes/origin/HEAD
gh auth status
gh repo view --json nameWithOwner,url,defaultBranchRef
```

当前目录可能已经是链接 Worktree；`--show-toplevel` 这时返回任务目录，不能用它再创建嵌套 `.worktrees/`。从 worktree 列表与 common-dir 交叉核对**主工作区**，设置绝对 `PROJECT_ROOT`。已在正确路径的专用任务 Worktree 时复用，不重复创建。检查是否为 submodule，不能将它误判为链接 Worktree。

远程默认分支、仓库说明及本地跟踪关系共同确定 `BASE_BRANCH`；本项目默认主分支为 `master`，不存在 `main`，以实际配置为准。远程为空时没有可推断的默认分支，不能伪造 origin/master。记录目标分支后按授权读取最新状态：`git fetch origin`，不覆盖用户未提交内容，不用无条件 `git pull`。

主工作区有未提交/未跟踪文件：记录范围，不 stash、不 reset、不 checkout 覆盖。若可从现有提交创建隔离分支，继续不依赖这些修改的工作；最终集成会触碰这些路径时停止集成，保留成果并请求处理。其他人的 Worktree 不动。

## 创建

以下变量须先替换为已核实值；示例在普通非空仓库执行，不能直接套用本次空仓库：

```bash
PROJECT_ROOT=/absolute/path/to/repository
BASE_BRANCH=master
TASK_BRANCH=feat/21-markdown-import
TASK_DIR="$PROJECT_ROOT/.worktrees/issue-21-markdown-import"
git -C "$PROJECT_ROOT" check-ignore -v .worktrees/
```

根 `.gitignore` 必须有 `.worktrees/` 或等效规则，且 `git check-ignore` 成功。若缺失：先在已有合规 Worktree 修正并安全集成；完全没有 Worktree 时，可在仓库 `.git/info/exclude` 加一次本地引导规则，再于新任务 Worktree 修改正式 `.gitignore`，集成后移除本次引导规则。不要将这种引导当成长期替代；主根 `.gitignore` 未落地时必须报告待集成。

核验 `.worktrees/` 不是指向项目外的符号链接、路径不存在、不对应其他任务后：

```bash
mkdir -p "$PROJECT_ROOT/.worktrees"
git -C "$PROJECT_ROOT" worktree add -b "$TASK_BRANCH" "$TASK_DIR" "$BASE_BRANCH"
git -C "$TASK_DIR" status --short
git -C "$TASK_DIR" check-ignore -v .worktrees/
```

需要最新远程基线且本地主线落后时，使用已核实的 `origin/<base>` 为起点，不强制移动本地主线。先记录 fork 点，再进入 Worktree 分析、设计、修改、测试、提交和准备 PR。

原生 Worktree 工具只有能满足固定路径、独立分支和保留现有工作的条件时才使用；否则明确记录原因，使用 Git。权限失败或路径不满足要求时，不回退到主工作区开发。

## 空仓库引导

确认本地 HEAD 无提交且远程无分支后，本机 Git 支持：

```bash
git -C "$PROJECT_ROOT" worktree add --orphan -b chore/agent-workflow \
  "$PROJECT_ROOT/.worktrees/task-agent-workflow"
```

这是首次初始化的特殊路径，不用于已有历史仓库绕开集成。现有初始文件可以只读复制作为开发基线，原文件保持原样；先记录散列。不能把任务文件复制回主工作区替代 Git 集成。首次提交仍必须读取 git-commit。若初始未跟踪文件阻碍首次主线建立，先交付可审阅结果，再由用户确认原文件纳管和目标主线；不能强制覆盖。旧 Git 不支持 orphan 时报告限制，请求确认首次基线提交，不能擅自提交用户文件。

## 子任务与集成

写文件的每个 Subagent 均使用主根 `.worktrees/agent-<name>-<task>` 和独立分支，禁止在主 Agent 的任务目录下嵌套。主 Agent 先协调文件范围，再安排并行；强耦合修改串行。子 Agent 只提交任务分支，主 Agent 审查 commit 后在集成 Worktree 执行普通 merge 和整体验证，不让多个 Agent 写主线。

## PR 交付路径

仅获得该仓库推送、创建 PR 的明确授权后，在任务目录操作：

```bash
git push -u origin "$TASK_BRANCH"
# BODY_FILE 是已填写并审核的 UTF-8 PR 正文文件，不是未经处理的 Issue 内容
gh pr create --base "$BASE_BRANCH" --head "$TASK_BRANCH" --body-file "$BODY_FILE" --title "$PR_TITLE"
gh pr checks "$PR_NUMBER"
gh pr view "$PR_NUMBER" --json state,mergedAt,mergeCommit,baseRefName,reviewDecision,statusCheckRollup
```

仓库不使用 GitHub CI，`gh pr checks` 会为空，这不代表检查通过。合并前确认 PR 正文记录的命令输出是本任务真实运行的结果，再执行 `gh pr merge "$PR_NUMBER" --merge`（默认已授权）。不直推 `master`、不使用 `--admin`。若仓库禁用 merge commit，遵循其既有策略。用户要求先看 PR 时保持 Worktree 与待合并状态。

## 本地主线集成及清理

1. 检查任务状态和忽略文件，确认所有应保留成果已提交。运行测试和质量检查。
2. fetch 后核对主线新提交，审查差异；普通 merge 优先。冲突时保留 Worktree，在任务工作区解决；语义不清须人工确认。
3. 确认主工作区及目标主线可安全更新，没有会被覆盖的用户修改。已授权本地合并时，在主工作区 `git switch "$BASE_BRANCH"`，再 `git merge "$TASK_BRANCH"`。不添加 `--force`。
4. PR 路径则先确认 PR 已合并到正确主线，fetch 后用 `git merge --ff-only "origin/$BASE_BRANCH"` 更新干净本地主线；无法快进时调查，不 reset。
5. 在主线上执行必要集成验证，并记录 SHA、命令和结果。只有验证成功才清理。
6. 对普通 merge，`git merge-base --is-ancestor "$TASK_BRANCH" "$BASE_BRANCH"` 必须成功。对 squash/rebase merge，不能只看祖先判断：同时核实 PR mergeCommit、目标主线包含该提交、PR 文件范围和最终内容一致；不能证明时保留分支和工作区。`git branch -d` 拒绝时不自动换成 `-D`，报告保留原因。
7. 确认任务 Worktree 无暂存、修改、未跟踪或需要保留的忽略文件，且所有 Agent 已停止使用：

   ```bash
   git -C "$TASK_DIR" status --short --untracked-files=all
   git -C "$TASK_DIR" status --short --ignored
   git -C "$PROJECT_ROOT" worktree remove "$TASK_DIR"
   git -C "$PROJECT_ROOT" worktree prune --dry-run
   ```

   dry-run 若包含别人的注册记录，不 prune；仅确认本任务残留后才 `git worktree prune`。不使用 `remove --force`。
8. 安全合并证据成立后 `git -C "$PROJECT_ROOT" branch -d "$TASK_BRANCH"`；远程分支遵从仓库策略和授权，不能因本地清理顺带删除。
9. 再次查看 worktree list、主线状态和 `.worktrees/`。只核对本任务残留；只有没有其他活动任务时才能期待目录为空。

仓库级设置限制、用户要求仅创建 PR 或无法证明合并时，报告“待集成”；不能提前关闭 Issue，也不能提前清理未合并成果。
