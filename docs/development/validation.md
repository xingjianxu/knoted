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

当前骨架没有测试用例；cargo test 成功只证明测试目标能构建和执行，不表示业务覆盖。行为新增时补有意义的回归测试。Cargo.lock 是当前二进制包的构建锁文件，不是新增第三方依赖。

## 文档、Skills 与 GitHub 配置

- 检查所有 Markdown 本地链接指向存在的文件；模板中的待填文字不作为真实路径。检查 ADR 编号唯一、状态合法、Accepted 有人工证据、替代关系双向完整。
- 六个自定义 Skill 均须 `name` 和 `description` YAML frontmatter；目录名与 name 一致，name≤64、description≤1024，无非法连字符；正文包含输入、执行步骤、输出和边界。用户安装的 git-commit 保持原文。
- 使用已有 YAML 1.2 解析器解析三份表单与 workflow，检查 name/description、body、唯一 id、支持的 type、非空 label、dropdown options、布尔 required；Forms 必须包含来源、验收和 needs-triage。不能仅用正则把 YAML 语法视为已验证。
- 模板依赖的 Labels 必须在远程启用前创建；本地检查无法证明远程渲染和标签应用成功。gh CLI 不自动套用表单字段。
- CI 检查 `on` 事件、contents: read、checkout 固定 commit 和 persist-credentials: false；不含未经授权的发布、标签、批准或合并操作。
- 检查标签 JSON 的名称唯一、颜色六位十六进制、描述完整，并与模板一致。
- 实际发现验证使用 [Agent 适配](agent-adapters.md) 的 Codex skills/list 和隔离 Pi get_commands。当前工具读取并应用与仅有目录是不同证据。

本次利用环境已有 Node `yaml` 模块完成 YAML/schema 子集检查，没有为验证修改项目依赖或安装全局解析器。该模块不是本项目运行时依赖；其他机器可使用已有等价解析器。GitHub 全服务校验和真实 CI 要在远程授权后运行，不虚称已验证。

## Skills 场景验收

| 输入 | 必需结果 |
|---|---|
| 疑似重复问题，只有本地初始化授权 | 查重并出草稿，不发布 |
| needs-triage 且 Agent 认为容易修复 | 人工批准前不自行 ready/执行 |
| CI通过但 PR 未合并 | 保留 Worktree，不关闭 Issue |
| 新方案与 Accepted ADR 冲突 | 提案及人工确认，保留旧决策历史 |
| 两个 Agent 同时需要下一 ADR号 | 主 Agent 串行分配，批准与编号分离 |
| 审计发现凭据 | 脱敏报告，未授权不发布或自动修复 |
| gh 查询失败 | 标明查重未完成，不断言无重复 |
| 批准任务但未批准架构 | 不推定架构 Accepted |
| git-commit 缺失 | 不伪称已使用；无替代授权则保留待提交 |
| 无 TODO / 无 Subagent | 持久化清单 / 相同契约串行执行 |
| 审计命令会生成文件 | 使用专用 Worktree |

本次由只读 Subagent 做基线和规则场景复核；基线没有违规，因此不称 RED→GREEN，也不把模拟结果称为端到端执行。主 Agent 仍需检查指南与 Skills 的一致性和最终差异。

## 提交、集成和证据

使用 verification-before-completion；报告命令、退出码、用例数量、检查范围、未执行项。使用 requesting-code-review 做独立审查，重要问题处理完才能集成。

最终在目标主线再次运行必要检查，确认任务 commit 被包含。清理前确认无未提交成果或需要保存的忽略文件、无 Agent 占用，仅移除本任务 Worktree。完整证据应包含主线 SHA、Worktree list 和 status。不以文件数量、模板勾选或 Agent 自述替代实际验证。
