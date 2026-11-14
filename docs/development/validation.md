# 验证与质量证据

每次变更只运行与范围相关的验证。验证失败先使用 systematic-debugging 定位；不要修改测试或忽略错误以制造通过。

## 当前项目

在任务 Worktree 运行并检查每条命令退出码：

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
git diff --check
git diff --cached --check
git check-ignore -v .worktrees/
git status --short --untracked-files=all
```

T01—T05 引入了真实回归测试：`cargo test --workspace --locked` 覆盖配置装载/脱敏与数据库迁移隔离（后者需要本机 PostgreSQL 测试库 `knoted_test`），前端 `pnpm run test` 覆盖空壳、客户端与查询入口。测试成功只证明这些场景，不表示未实现业务已有覆盖；行为新增时补有意义的回归测试。

## 文档、Skills 与 GitHub 配置

- 检查所有 Markdown 本地链接指向存在的文件；模板中的待填文字不作为真实路径。检查 ADR 编号唯一、状态合法、Accepted 有人工证据、替代关系双向完整。
- 六个自定义 Skill 均须 `name` 和 `description` YAML frontmatter；目录名与 name 一致，name≤64、description≤1024，无非法连字符；正文包含输入、执行步骤、输出和边界。用户安装的 git-commit 保持原文。
- 使用已有 YAML 1.2 解析器解析三份表单，检查 name/description、body、唯一 id、支持的 type、非空 label、dropdown options、布尔 required；Forms 必须包含来源、验收和 needs-triage。不能仅用正则把 YAML 语法视为已验证。
- 检查标签 JSON 的名称唯一、颜色六位十六进制、描述完整，并与模板一致。
- 仓库不运行 GitHub CI，因此本地 `cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings` 是唯一门禁；PR 正文必须粘贴本轮真实输出而不是“应该通过”。
- 实际发现验证使用 [Agent 适配](agent-adapters.md) 的 Codex skills/list 和隔离 Pi get_commands。当前工具读取并应用与仅有目录是不同证据。

本次利用环境已有 Node `yaml` 模块完成 YAML/schema 子集检查，没有为验证修改项目依赖或安装全局解析器。该模块不是本项目运行时依赖；其他机器可使用已有等价解析器。Issue Forms 的选择页渲染仍只能在浏览器人工确认，不虚称已验证。

## Skills 场景验收

| 输入 | 必需结果 |
|---|---|
| 疑似重复问题，需要新建 Issue | 查重后直接发布并标 agent-found、needs-triage |
| needs-triage 且 Agent 认为容易修复 | 可自行领取，但重大架构选择仍需人工确认 |
| 本地检查通过但 PR 未合并 | 保留 Worktree，合入后才清理 |
| 新方案与 Accepted ADR 冲突 | 提案及人工确认，保留旧决策历史 |
| 两个 Agent 同时需要下一 ADR号 | 主 Agent 串行分配，批准与编号分离 |
| 审计发现凭据 | 脱敏报告，不将凭据写入任何提交或 Issue |
| gh 查询失败 | 标明查重未完成，不断言无重复 |
| 批准任务但未批准架构 | 不推定架构 Accepted |
| git-commit 缺失 | 不伪称已使用；无替代授权时保留待提交 |
| 无 TODO / 无 Subagent | 持久化清单 / 相同契约串行执行 |
| 审计命令会生成文件 | 使用专用 Worktree |

本次由只读 Subagent 做基线和规则场景复核；基线没有违规，因此不称 RED→GREEN，也不把模拟结果称为端到端执行。主 Agent 仍需检查指南与 Skills 的一致性和最终差异。

## 提交、集成和证据

使用 verification-before-completion；报告命令、退出码、用例数量、检查范围、未执行项。使用 requesting-code-review 做独立审查，重要问题处理完才能集成。

最终在目标主线再次运行必要检查，确认任务 commit 被包含。清理前确认无未提交成果或需要保存的忽略文件、无 Agent 占用，仅移除本任务 Worktree。完整证据应包含主线 SHA、Worktree list 和 status。不以文件数量、模板勾选或 Agent 自述替代实际验证。
