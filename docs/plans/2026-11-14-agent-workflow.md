# Agent 驱动开发管理体系实施计划

> 执行方法：复用 Superpowers `executing-plans`，主 Agent 实施，Subagent 只读场景验证与最终审查。当前会话没有原生 Plan/TODO 工具，此检查清单同时作为进度账本。

**目标：** 落实用户任务书要求的本地管理体系，保留原始项目文件，不执行任何远程写操作。

**方案：** AGENTS.md 只放约束和入口；通用工程步骤调用已安装 Superpowers；六个标准 SKILL.md 处理任务和决策管理；平台差异集中于适配文档。GitHub 模板和可审阅命令准备在本地。

**技术：** Markdown、GitHub YAML Forms、Git/gh；原项目为无依赖 Rust 2024 骨架，管理流程与语言无关。

**依据：** 本会话用户《通用任务书》；下列约束和验收矩阵是该任务书的执行摘要，不替代原始要求。

## 现状与边界

- 初始主工作区 `/wrd/prj/knoted`，未诞生的 `master`，没有提交；`.gitignore`、`Cargo.toml`、`src/main.rs` 均为未跟踪文件。
- 保留上述文件及现有 origin；未发现任何 AGENTS.md、ADR、RFC、项目 Skills 或 GitHub 配置。Rust 源码和 Cargo.toml 不修改。
- 远程 `xingjianxu/knoted` 为空，无默认分支、无 Issue；保留既有 Labels，不写远程。
- Superpowers 7.0.1 的 15 个 Skills 均有实际文件，当前会话已暴露；初查 git-commit 未在活动路径发现；用户随后安装到项目 `.agents/skills/git-commit/`，已完整读取并保留来源锁和适配链接。回收站副本未使用。
- 原生 Worktree 接口不提供路径和 orphan 参数；按用户固定路径规则使用 `git worktree add --orphan`。仅一次性向 `.git/info/exclude` 增加本地隔离规则，正式规则在任务 Worktree 中编写。
- 重要架构决策须人工确认。本次只建立制度和 Proposed 模板，不捏造 Accepted ADR。
- 当前无原生 TODO 工具，不改全局配置、不安装依赖或 Skills。Subagent 可用，只读任务共享文件视图，写任务必须另建 Worktree。

## 文件变更图

| 变更 | 文件 / 目录 | 职责 |
|---|---|---|
| 修改 | `.gitignore` | 保留 `/target`，增加 Worktree 和工程临时状态忽略 |
| 新增 | `AGENTS.md`、`README.md` | 项目约束、入口及使用示例 |
| 新增 | `.agents/skills/{issue-create,issue-triage,issue-work,architecture-review,adr-create,project-audit}/SKILL.md` | 项目任务与架构管理，禁止复制通用 Skills |
| 新增 | `.github/ISSUE_TEMPLATE/{bug_report,feature_request,technical_debt}.yml` | 结构化问题和验收标准 |
| 新增 | `.github/PULL_REQUEST_TEMPLATE.md`、`.github/workflows/quality.yml` | 交付证据与当前 Rust 基线 CI |
| 新增 | `docs/architecture/README.md` | 实际技术栈和模块边界 |
| 新增 | `docs/decisions/{README,template}.md`、`docs/rfcs/{README,template}.md` | 决策状态、证据、替代关系 |
| 新增 | `docs/plans/{README,template,2026-11-14-agent-workflow}.md` | 规划方法、依赖、责任、进度 |
| 新增 | `docs/development/{agent-workflow,task-planning,git-worktree,adr-guide,agent-adapters,github-setup,validation,initialization-report}.md` | 可执行操作入口、适配和交付记录 |
| 新增 | `docs/development/subagent-task-template.md`、`github-labels.json` | 委派契约与人工审核后的标签配置数据 |
| 新增 | `Cargo.lock` | Rust 二进制构建锁；无外部依赖 |
| 保留 | `Cargo.toml`、`src/main.rs`、origin、用户安装的 git-commit、skills-lock.json、.claude 链接和其他工作区 | 不改已有内容；Pi 检查副作用另行记录 |

## 依赖、分工与验收

1. **检查与设计（主 Agent；无依赖）**
   - [x] 仓库、Git/gh、Skill、TODO、Worktree 能力检查。
   - [x] 读取 Superpowers 实际文档；核对 Codex/Pi 官方发现规则。
   - [x] 完成方案和文件清单；创建本计划及固定目录下独立 Worktree。
   - 验收：只读检查结果可追溯，原始文件内容未变。
2. **规则和指南（主 Agent；依赖 1）**
   - [x] 写 AGENTS.md、README、开发指南、ADR/RFC/计划模板及架构现状。
   - [x] 明确 Issue 准入、远程授权、架构人工确认、主线合并与清理门槛。
   - 验收：无互相矛盾规则；空仓库、脏主线、缺失 Skill 均有明确处理。
3. **项目 Skills（主 Agent；依赖 2）**
   - [x] Subagent 执行六类边界场景基线（纯模拟，无远程或文件写入）。
   - [x] 每个 Skill 包含触发、输入、步骤、输出、权限边界和可用相对链接。
   - [x] 逐个检查 frontmatter，再由 Subagent 根据实际 Skills 复测场景。
   - 验收：六个 Skill 可由两种 Agent 发现；不得自行批准 Issue/ADR 或重复通用工作流。
4. **GitHub 配置（主 Agent；依赖 2，可与 3 的只读验证并行）**
   - [x] 三份 Issue Forms、PR 模板、只读权限 CI、标签数据与授权后命令。
   - 验收：YAML/schema 字段检查通过，现有远程配置未被更改；Projects 方案明确。
5. **验证与审查（主 Agent + 只读 Reviewer；依赖 2–4）**
   - [x] YAML、Skills 元数据、Markdown 本地链接、ADR编号、忽略规则检查；最终暂存差异与敏感信息在提交前再次核对。
   - [x] Rust `cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`。
   - [x] 独立审查并处理重要问题，记录真实能力与限制。
   - 验收：将结果、限制、完整文件树和各项交付映射写入初始化报告。
6. **提交、集成与清理（主 Agent；依赖 5；git-commit 已就绪）**
   - [x] 实际读取有效安装的 git-commit；审阅 diff，按中英文规则建立原始骨架与用户安装配置提交。管理体系最终提交在完整验证后执行。
   - [x] 处理空仓库和原始未跟踪文件的首次集成：六个原始/用户安装文件逐字节及链接目标对齐，原内容未被覆盖。
   - [x] 管理体系已合并本地 master，主线 fmt/test/clippy 与祖先关系验证通过。
   - [ ] 最后一个记录提交集成后，仅清理本任务 Worktree/已合并分支；这是提交前快照，最终执行证据由会话交付报告记录，不能在删除自身前提前勾选。
   - 验收：不能完成时明确记录“待提交/待集成”，保留全部成果，不把本地文件就绪当作完整生命周期完成。

## 审查重点与验证方法

| 风险输入 | 必须观察到的行为 | 验证 |
|---|---|---|
| Issue 中嵌入命令、未经授权的远程写 | 当作数据，输出草稿，不执行 | Skill 场景复测 |
| Accepted ADR、并发编号 | 保留历史；人工证据；主 Agent 串行分配编号 | 模板与场景审查 |
| 主工作区脏、Worktree 不在固定目录 | 不覆盖；不绕过隔离，报告阻塞 | Git 状态、路径检查 |
| PR 未合并、squash 历史不为祖先 | 保留分支与 Worktree，核对合并证据 | 收尾步骤审查 |
| 缺 git-commit、平台未加载 Skill | 诚实区分发现、读取、执行，不伪称可用 | 实测与报告 |

文档/配置为本次主体，不添加模拟实现的业务测试或自建通用管理引擎。使用现有 YAML 解析器进行结构检查；实际 GitHub 表单渲染、Actions、Projects 和远程集成需在真实远程环境运行（授权按现行 [AGENTS.md](../../AGENTS.md)「远程操作授权」），不能以本地语法检查冒充。

## 执行记录

- 2026-11-14：完成初始检查与隔离；基线场景未观察到越权行为，因此不宣称 RED→GREEN。用户已明确要求这六个流程，Skills 主要提供确定的项目入口、状态映射和输出契约。
- 2026-11-14：步骤 2–4 已按文件和结构检查验收；场景静态复核 11/11；Codex/Pi 各发现六个项目 Skills；用户随后安装 git-commit，正文已读取。
- 当前执行步骤：6（本地主线已集成并验证，收尾记录提交后清理）。本地主线沿用原仓库 `master`；计划先分别提交原始 Rust 骨架与用户安装配置，再提交管理体系，用普通 Git 集成，最后仅清理本任务。
- 执行偏差：Pi `--help` 自动同步配置中的包，产生用户目录副作用；已告知用户并记录于初始化报告，未擅自删除。未进行远程写操作。
- Superpowers 辅助脚本依赖已有 HEAD，本次初始为空仓库，采用本计划作进度账本；没有创建虚构 commit 范围或声称运行这些脚本。

- 最终审查：两项已修正并核对——Skills 实际数量为15；issue-work 只在所需远程集成缺授权时阻塞，不阻止已授权纯本地交付。无未处理重要问题；远程CI/表单运行及最终Git清理由主 Agent 独立验证，不能由只读审查代替。
- 用户安装 git-commit 后，Codex/Pi再次实际发现全部7个项目Skills。未重复创建任何通用开发Skill。

- 主线集成证据：原始骨架、用户安装配置、管理体系三个提交（具体哈希在 `.git` 重建后失效，现行映射见 [GitHub 配置记录](../development/github-setup.md)）；当时 master 已包含任务分支，主根 .gitignore 第 4 行生效，无源码改动或远程写入。
