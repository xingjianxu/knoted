# Agent 开发管理体系初始化交付记录

检查日期：2026-11-14。目标仓库：`xingjianxu/knoted`。本次交付范围是本地文件、配置、验证和安全的本地 Git 集成；任何远程写操作仍待明确授权。

> **时点说明：** 本记录固定描述 2026-11-14 初始化时点的状态。其中远程部分——首次推送、标签、Projects #1、ruleset——已由同日的后续授权任务完成，最新事实和可复核命令见 [GitHub 配置记录](github-setup.md)。下文提到的具体提交哈希均在 `.git` 重建后失效，现行对应关系同样见该文档的“历史重建说明”。

## 文件与完整结构

新增 AGENTS.md、README.md、六个项目管理 Skills、三份 Issue Forms、PR 模板、Quality workflow、开发指南、ADR/RFC/计划模板及本记录。修改 `.gitignore`，保留 `/target`，增加 `.worktrees/`、`.superpowers/`。构建生成 Cargo.lock（无第三方依赖）。

原样保留 Cargo.toml 和 src/main.rs。用户在任务期间安装的 git-commit、skills-lock.json 和 `.claude` 链接原样纳入基线，不复制或改造其实现。无已有 AGENTS.md、ADR、RFC、CI 可合并；没有删除其他 Worktree。

```text
knoted/
├── .agents/skills/
│   ├── adr-create/SKILL.md
│   ├── architecture-review/SKILL.md
│   ├── git-commit/SKILL.md              # 用户安装，原样保留
│   ├── issue-create/SKILL.md
│   ├── issue-triage/SKILL.md
│   ├── issue-work/SKILL.md
│   └── project-audit/SKILL.md
├── .claude/skills/git-commit           # 用户安装的链接 → ../../.agents/skills/git-commit
├── .github/
│   ├── ISSUE_TEMPLATE/
│   │   ├── bug_report.yml
│   │   ├── feature_request.yml
│   │   └── technical_debt.yml
│   ├── PULL_REQUEST_TEMPLATE.md
│   └── workflows/quality.yml
├── .gitignore
├── .worktrees/                        # 本次任务在 task-agent-workflow；不跟踪
├── AGENTS.md
├── Cargo.toml
├── Cargo.lock
├── README.md
├── skills-lock.json
├── src/main.rs
└── docs/
    ├── architecture/README.md
    ├── decisions/
    │   ├── README.md
    │   └── template.md
    ├── rfcs/
    │   ├── README.md
    │   └── template.md
    ├── plans/
    │   ├── README.md
    │   ├── template.md
    │   └── 2026-11-14-agent-workflow.md
    └── development/
        ├── agent-workflow.md
        ├── task-planning.md
        ├── git-worktree.md
        ├── adr-guide.md
        ├── agent-adapters.md
        ├── subagent-task-template.md
        ├── github-setup.md
        ├── github-labels.json
        ├── validation.md
        └── initialization-report.md
```

Git 元数据、构建 target、临时验证缓存不列入交付文件；不能提交 `.worktrees/` 或 `.superpowers/`。

## 规则与流程交付

AGENTS.md 统一入口和强制约束：先查规则/决策/授权；复杂任务先规划；修改只在主根 `.worktrees/`；写 Subagent 各自隔离；人工批准任务和重要架构；实际读取 git-commit；测试和独立审查；主线验证后才清理；远程写操作不能由 gh 登录状态或 Skill 调用推定。

| Skill | 触发 / 结果 |
|---|---|
| issue-create | 新发现问题 → 查重、结构化草稿；有授权才发布 needs-triage |
| issue-triage | 现有 Issue → 有效性、信息缺口、优先级及准入建议；不自动批准 |
| issue-work | 已批准任务 → 调用 Superpowers 并协调计划、分支、审查、集成和清理 |
| architecture-review | 长期约束或 Accepted ADR 变化 → 比较方案、RFC/Proposed 草稿、人工决定点 |
| adr-create | 需要记录决策/替代关系 → 串行编号、确认依据、索引和双向替代链接 |
| project-audit | 明确范围审计 → 可复现证据、查重与待分诊草稿；不自行修复 |

Issue 生命周期：发现 → 查重 → needs-triage → 人工决定/agent-ready → in-progress → PR Review → 合并到默认分支后通过 PR 关系关闭。blocked/human-required 记录原因和恢复阶段，Projects 只展示状态。普通修改不写 ADR；重大方案先 RFC，Proposed → 人工确认 Accepted；替代时新 ADR 和旧 Superseded 双向关联。

规划采用简单/中等/复杂三级。非简单任务优先原生 TODO，本会话无该工具，已用 [持久化计划](../plans/2026-11-14-agent-workflow.md) 追踪。复杂任务明确输入输出、依赖、分工、并行边界、风险、测试、集成、清理；每个阶段必须有验收证据。

Subagent 用于六类边界场景模拟、独立能力核验和最终只读审查。写任务的通用契约要求独立分支/Worktree和限定修改范围；高度耦合任务串行；主 Agent 独立验收并承担最终主线集成。本次不安排并行写文件，避免对同一规则交叉修改。

Worktree 完整操作见 [指南](git-worktree.md)：核对主根/主线/状态 → 忽略规则 → 独立分支 → 基线测试 → 实施/提交/审查 → 按授权合并 → 主线验证 → 安全清理本任务。空仓库采用 orphan Worktree，未在主工作区编辑源码或配置；一次性 `.git/info/exclude` 仅用于引导，正式 `.gitignore` 规则通过集成落地。

## 环境与真实能力

| 检查 | 实际结果 |
|---|---|
| Git | 2.56.0，支持 `worktree add --orphan` |
| 原始状态 | 未诞生 master，三份 Rust 骨架文件未跟踪；远程为空、无默认分支 |
| Codex | CLI 0.167.0；本宿主会话有 Subagent，原生 Plan/TODO 未暴露 |
| Codex 项目 Skills | app-server skills/list 实测发现六个自定义 Skill及用户安装的 git-commit，共7个，enabled=true、errors=[] |
| Superpowers（当前宿主） | 7.0.1，技能清单可见、15 份 SKILL.md 实际存在并可读；适用 Skills 已读取并用于任务 |
| Pi | 1.5.0；官方随包文档原生支持 `.agents/skills`；隔离配置 RPC get_commands 实测发现六个自定义 Skill及 git-commit，共7个 |
| Pi TODO/Subagent | 用户 packages 有配置；未在完整用户环境做运行验证，不能据此宣称可调用 |
| git-commit | 用户安装于 `.agents/skills/git-commit/SKILL.md`，已完整读取；来源 github/awesome-copilot，锁记录保留 |
| gh | 2.103.3，认证用户 xingjianxu，origin 与 gh repo view 都指向 xingjianxu/knoted |
| 远程配置 | 无 Issue、无默认分支；保留既有 Labels；未执行 Issues/Labels/Projects/PR/设置写操作或 push |
| Projects 权限 | 当前认证列出 repo/workflow 等，未列 project scope；未请求新 scope 或假设已有权限 |

Superpowers 实际安装的 15 个 Skills：using-superpowers、brainstorming、writing-plans、executing-plans、subagent-driven-development、dispatching-parallel-agents、using-git-worktrees、test-driven-development、systematic-debugging、requesting-code-review、receiving-code-review、verification-before-completion、finishing-a-development-branch、writing-skills、diagnosing-superpowers。

本次实际复用：using-superpowers、brainstorming、writing-plans、executing-plans、using-git-worktrees、dispatching-parallel-agents、writing-skills、requesting-code-review、receiving-code-review、verification-before-completion、finishing-a-development-branch。文档配置任务未新增业务行为，不声称运行 TDD 或 subagent-driven-development。另用 skill-creator 编写标准元数据、openai-docs 核实官方发现规范。

## git-commit 兼容性与示例

实际 Skill 采用 Conventional Commits，标题≤72字符，不以句点结束，从 diff 判断 type/scope；允许仓库规则优先，因此中文摘要/正文兼容。提交前分别检查 MERGE_HEAD、CHERRY_PICK_HEAD、REVERT_HEAD、冲突和 diff；按逻辑暂存，不跳过 hooks、不改历史、不顺带 push。

本次逻辑拆分：原始 Rust 骨架基线、用户安装的提交 Skill、管理体系初始化各一个提交，另以文档提交记录最终验证。实现已快进集成到本地 `master`；无远程推送。具体哈希在 `.git` 重建后失效，现行对应关系见 [历史重建说明](github-setup.md)。管理体系提交示例：

```text
chore(workflow): 建立 Agent 驱动开发管理体系

统一任务准入、架构确认、规划和隔离开发规则。
提供项目管理 Skills、GitHub 模板和跨 Agent 适配说明，
保留原有 Rust 骨架和用户安装配置。
```

没有真实 Issue，所以不虚构 Closes Footer。Superpowers 中无独立 git-commit 工具；当前使用用户已安装 Skill 指导普通 Git 提交，不将其冒称为 Superpowers 的 Skill。

## 验证结果与边界

- 六个自定义 Skill 的 YAML frontmatter、命名和描述长度检查通过；只读 Subagent 的六类场景和五项补充检查共 11/11 有明确规则支持。基线未违规，不宣称 RED→GREEN 或端到端运行。
- 三份 Issue Forms、Quality YAML 和 19 个标签定义通过本地 YAML 解析/结构检查。远端表单渲染、真实 CI 和 Projects 尚未执行。
- Rust fmt、test、clippy 通过；当前 0 个测试，结果仅证明骨架编译与工具检查通过。
- 独立审查已完成，两项发现均已修正并核对：Superpowers 实际数量15，以及纯本地授权交付的完成条件。27份 Markdown 的66个本地链接无缺失；暂存差异及凭据特征检查通过。
- 管理体系提交已进入当时本地主线 master，祖先关系检查成功；主线 fmt/test/clippy 再次通过，主工作区干净。最后的文档记录提交仍按相同 Git 集成与检查流程完成。现主线提交见 [GitHub 配置记录](github-setup.md) 的历史重建说明。
- `.worktrees/` 位于主根 `/wrd/prj/knoted/.worktrees/`；任务路径为 `task-agent-workflow`。主根 `git check-ignore -v .worktrees/` 实际输出 `.gitignore:4:.worktrees/`。
- 清理在最后一个文档提交安全集成后进行：确认工作区干净、没有 Agent 使用、仅有本任务生成的 target 和隔离 Pi 探测缓存，再用 git worktree remove 和 branch -d。该记录提交时不提前声称自有 Worktree 已被删除；最终清理结果见会话交付报告与 `git worktree list`。

**实际发生的环境偏差：** 能力核验的 `pi --help` 触发用户已配置包的自动同步，生成 `~/.pi/agent/git/github.com/obra/superpowers`（7.0.2，HEAD `22f0b463e7a5e579904dd6211a520321035526f7`）和安装文件。已停止使用该检查方式，未修改 settings、未发模型请求、未自行删除新增包。后续探测使用任务目录内隔离配置和离线模式；不能声称本次完全没有用户目录副作用。

## 使用、人工后续与限制

在目标 Worktree 启动 Codex 后，用 `$issue-work` 加 Issue/授权范围；Pi 用 `/skill:issue-work`。两者共用同一目录；其他 Agent 按官方发现机制指向 `.agents/skills`，或显式读取文件。不能把宿主插件安装状态自动套用到独立 CLI、Pi 或其他 Agent。

需要人工明确授权的远程工作已准备在 [GitHub 配置方案](github-setup.md)：首次主线推送及默认分支、缺失标签、看板及字段、CI首次运行和保护规则。项目默认不做这些写操作。优先级可用 Labels 或 Projects 字段，避免双重维护。

当前限制：Pi 完整用户环境中的 TODO/Subagent 未验证；没有真实远程 PR/CI 或关闭 Issue 实测；零业务测试；不自动恢复/删除 Pi 包同步副作用。正式 ADR 为空是因为尚无已确认产品决策，不是缺漏。下一任务应由人工确定产品目标和可验证验收，再建立批准的 Issue。

## 交付要求对应入口

1–2 文件清单/树：本记录首节；3 规则：[AGENTS.md](../../AGENTS.md)；4–5 Superpowers：本记录环境表；6 Skills：上表；7 git-commit：兼容性说明；8 Issue/ADR：[工作流](agent-workflow.md) / [ADR指南](adr-guide.md)；9 规划：[task-planning](task-planning.md)；10 子任务：[契约](subagent-task-template.md)；11–12 Worktree：[操作指南](git-worktree.md)；13 提交：上方示例；14–15 gh/TODO：环境表；16–17 跨 Agent：[适配](agent-adapters.md)；18 GitHub：[配置方案](github-setup.md)；19–20 限制和建议：上一节。
