# Agent 任务工作流

## 启动与路由

先检查仓库、适用 AGENTS.md、用户授权和真实工具能力。读取相关 Issue、ADR、代码，验证 Superpowers 和 git-commit 文件可读及当前 Agent 可加载。未安装时报告所缺能力，不改全局环境。安装/加载/读到内容/实际执行是四种不同证据，不能互相代替。

| 任务环节 | 复用 Superpowers |
|---|---|
| 新功能或复杂变更 | brainstorming → writing-plans |
| 按依赖执行 | executing-plans 或 subagent-driven-development |
| 独立研究、审查或隔离子任务 | dispatching-parallel-agents |
| 隔离 | using-git-worktrees（固定主仓库 `.worktrees/`） |
| 行为实现、问题定位 | test-driven-development、systematic-debugging |
| 审查与反馈 | requesting-code-review、receiving-code-review |
| 验收与集成 | verification-before-completion、finishing-a-development-branch |

只调用实际存在且适用的 Skill。需求和方案已在用户请求中明确、且实现已获授权时，不重复索要常规执行许可；重大架构和危险操作仍按 AGENTS.md 的边界处理。

## Issue 生命周期与 Projects

| 阶段 | Issue 状态标签 | Projects Status | 触发与完成证据 |
|---|---|---|---|
| 新发现 | needs-triage | Backlog | Agent 查重后直接创建，来源 agent-found |
| 信息不足 | needs-info | Backlog | 列出缺失复现、版本、验收信息 |
| 可执行 | agent-ready | Ready | 人工批准或 Agent 自查重确认范围、优先级与验收；保留依据 |
| 已领取 | in-progress | In Progress | Agent 明确责任人、分支、Worktree、计划 |
| 技术依赖阻塞 | blocked | 保留当前阶段 | 记录依赖、影响、恢复条件 |
| 人工决定/权限阻塞 | human-required | 保留当前阶段 | 仅用于规则 12 的两类边界或仓库设置类操作 |
| PR 审查中 | in-progress | Review | PR 链接、本地检查结果和验收证据 |
| 已合并 | 移除活动状态标签 | Done | PR 合并默认分支，关闭关系自动关闭 Issue |

状态标签原则上互斥，每次转换移除旧状态，保留来源/类型；恢复阻塞时回到记录的阶段。状态、优先级与看板字段的维护默认已授权，按本表和 [AGENTS.md](../../AGENTS.md) 的「远程操作授权」执行，不再逐次索取许可。看板位置不构成授权依据，也不能反推批准。

远程操作默认已授权（push 到非默认分支、Issue/Label/Projects/PR 的全部常规动作、合并普通 PR）；无需逐次询问。仍需逐次授权的是仓库级设置：ruleset、分支保护、webhook、协作者与权限、Actions 设置、仓库可见性与删除、标签名的增删（标签赋值不在此列）。用户在当前会话明确禁止某个对象或动作时，以用户指令为准并在报告中记录该限制。默认授权不豁免本地检查：合并前必须已运行并留存必需命令的输出。

上游 `xingjianxu/mtone` 的默认授权只覆盖创建 feature Issue：先查重、英文正文、仅用既有标签、回写编号与 URL，且不写入 Knoted 内部决策、凭据或私有数据。其标签增删、仓库设置、PR 和 npm 发布仍需逐次授权，按 [UI 指南](ui-mtone.md) 的缺口交接流程执行。

创建前使用 `gh issue list --state all --search '关键词 in:title,body'`，打开候选 Issue 和关联 PR 核实范围；搜索不到不等于一定不存在，换同义词/模块名。发现重复后关联已有任务，不重复开单；确属重复且已由新任务覆盖时才关闭并写明理由。安全问题仅保留脱敏摘要，通过已有私密渠道处理。

## 提交与审查

提交前定位当前有效的 `git-commit/SKILL.md`（从 Agent 技能清单、用户技能目录和项目目录查找），实际阅读全文；记录路径。不能使用回收站副本冒充当前安装，也不能复制其实现进本项目。找不到时保留修改和草稿，明确“未使用、未提交”；若用户随后明确指定替代流程，按新授权执行，记录实际采用的方法。

当前用户已安装 [git-commit](../../.agents/skills/git-commit/SKILL.md)（来源由根 `skills-lock.json` 记录为 github/awesome-copilot）。实际规则为 Conventional Commits、首行不超过 72 字符且不以句点结尾、从差异决定类型/范围、按逻辑拆分、提交前检查进行中的 Git 操作、不得擅改历史或绕过 hooks。其规则允许仓库规定语言，因此与本项目中文摘要/正文兼容。Superpowers 没有独立的 git-commit Skill，仍单独复用这一安装。

Skill 决定结构、类型、范围等细节；项目指定语言：Type/Scope 英文，Summary/Body 中文，Footer 和格式元数据英文。以下只是用户要求的示例，最终应与有效 Skill 核对：

```text
chore(workflow): 建立 Agent 开发管理规范

统一任务准入、架构决策和隔离开发入口。
提供项目管理 Skills、协作模板和本地验证说明。
```

Body 解释原因和影响，不复述标题，不声称未验证结果。只有真实关联 Issue 时才写 `Closes #123`；没有 Issue 时不生成虚构 Footer。提交前检查 `git diff`、`git diff --cached`、未跟踪文件与测试输出，显式暂存本任务文件，不使用会包含其他工作区的盲目 `git add .`。

PR 使用 [.github/PULL_REQUEST_TEMPLATE.md](../../.github/PULL_REQUEST_TEMPLATE.md)，写清关联任务、实现、本地检查输出、验收、限制、ADR和子任务。主 Agent 检查子任务 commit 范围并独立验证；独立 Reviewer 不得批准自己的实现。仓库不运行 GitHub CI，PR 页面不会有检查状态，合并依据是 PR 正文记录的本地命令输出。

## 交付定义

先核对本地检查，再走任务分支 + PR 集成（不直推 `master`）；具体命令见 [Worktree](git-worktree.md)。默认授权允许合并自己完成的 PR；用户要求先看 PR 时保留待合并状态。通过合并到默认分支的 PR 关闭关联 Issue；合并到非默认集成分支不会完成 Issue 生命周期。

完整交付报告包括：实际改动、提交/PR、测试命令及结果、主线集成证据、Worktree/分支清理结果、未解决项。缺少任何必需阶段则说明“待提交”“待审查”或“待集成”，不能把 PR 创建视为任务完成。
