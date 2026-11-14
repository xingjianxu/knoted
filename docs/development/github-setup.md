# GitHub 配置方案（尚未执行远程写操作）

本次仅完成本地模板与命令准备。仓库 `xingjianxu/knoted` 公开，origin 对应一致；检查时远程无分支/默认分支、无 Issues。gh 认证有效，拥有 repo/workflow 等 scope，但未看到 project scope；认证不代表当前任务已授权写操作或具备所有管理权限。

以下命令是**审核后由明确授权触发的操作**。取得范围内已有授权后可直接执行，不重复审批；本次没有该授权。不得把整个文档无条件当脚本执行。

## 顺序与保留项

1. 确认首次提交、目标主线名和原始文件纳管方式；完成本地验证。空远程不能直接创建正常 PR。
2. 明确授权首次推送后，将审核过的主线推送到 origin，确认远程默认分支。只推送已审阅内容。
3. 增补缺失标签，不改写已有 bug/enhancement 等标签，不删除用户现有分类。
4. 检查默认分支上的三份 Issue Forms、PR 模板及 Quality 工作流是否可见，实际提交一个经授权的试验 Issue/PR 才能验证远端渲染和 Actions。未经授权不制造试验数据。
5. 如需 Projects，先确认 owner、复用已有看板、project scope 和字段；再授权创建或增补。
6. 仓库维护者决定 CI 必需检查和 PR 审批策略。本次不修改分支保护、不启用自动合并或自动审批标签。

## 标签

[github-labels.json](github-labels.json) 是待审阅的 19 个标签定义。优先级可以改用 Projects 字段；采用字段时不必创建 P0–P3 标签。来源标签和状态标签必须保留其语义，Agent 新任务固定 agent-found + needs-triage。

先只读：

```bash
gh auth status
gh repo view --json nameWithOwner,url,defaultBranchRef
gh label list --repo xingjianxu/knoted --limit 100 --json name,color,description
```

获“增补标签”授权后，在仓库根执行以下代码。通过 argv 传参数，不执行正文或 shell 插值；已存在的标签保持原值：

```python
import json
import subprocess
from pathlib import Path

repo = "xingjianxu/knoted"  # 移植时先核对仓库，再修改此值
labels = json.loads(Path("docs/development/github-labels.json").read_text())
existing = json.loads(subprocess.check_output(
    ["gh", "label", "list", "--repo", repo, "--limit", "1000", "--json", "name"], text=True))
names = {row["name"].casefold() for row in existing}
for label in labels:
    if label["name"].casefold() in names:
        continue
    subprocess.run(["gh", "label", "create", label["name"], "--repo", repo,
                    "--color", label["color"], "--description", label["description"]], check=True)
    names.add(label["name"].casefold())
```

错误即停止；超时先读回远程状态再重试。不要添加 `--force` 去覆盖用户标签。表单引用 feature、tech-debt、needs-triage；标签未创建时无法期待自动加标成功。CLI `gh issue create --body-file` 不自动套用 YAML 表单字段或标签，因此项目 Skill 必须显式提供正文和标签。

## Projects

建议复用一个看板，Status = Backlog / Ready / In Progress / Review / Done。Ready 由人工批准；Review 仍对应 in-progress；blocked/human-required 保留原阶段并显示阻塞标签；Done 仅在已合并后。

字段建议：Priority（P0–P3）、Type（七类任务）、Module（当前可用 core/docs/workflow，随实际模块调整）、Source（agent-found/human-reported）、Agent（文本），Iteration 仅有排期需要时启用。不要为了表格完整虚构模块或迭代。

```bash
# 只读先检查；若 scope 不足则报告，不自行 gh auth refresh
gh project list --owner xingjianxu --format json
# 以下命令分别需要明确写授权；先确认不存在合适的看板/字段
gh project create --owner xingjianxu --title 'Knoted Development' --format json
# PROJECT_NUMBER 从 create 返回或人工指定，不能猜测
gh project field-list "$PROJECT_NUMBER" --owner xingjianxu --format json
gh project field-create "$PROJECT_NUMBER" --owner xingjianxu \
  --name Priority --data-type SINGLE_SELECT --single-select-options 'P0,P1,P2,P3'
gh project field-create "$PROJECT_NUMBER" --owner xingjianxu \
  --name Type --data-type SINGLE_SELECT \
  --single-select-options 'bug,feature,enhancement,tech-debt,refactor,performance,security'
gh project field-create "$PROJECT_NUMBER" --owner xingjianxu \
  --name Source --data-type SINGLE_SELECT --single-select-options 'agent-found,human-reported'
gh project field-create "$PROJECT_NUMBER" --owner xingjianxu --name Module --data-type TEXT
gh project field-create "$PROJECT_NUMBER" --owner xingjianxu --name Agent --data-type TEXT
```

Projects 自带 Status，不重复创建同名字段。由人工在看板设置中将其选项配置为上述五态；本机 `gh project field-create` 可新建字段，不能据此假设能重设既有 Status 选项。Iteration 同样由看板设置配置。需要自动化时另行审阅 API 方案，不在本次建立执行引擎。

将 Issue 加入看板、修改字段也属于写操作。获授权后可用当前 CLI 的：

```bash
gh project item-add "$PROJECT_NUMBER" --owner xingjianxu --url "$ISSUE_URL"
gh project item-edit "$PROJECT_NUMBER" --owner xingjianxu --url "$ISSUE_URL" \
  --field Status --value 'In Progress'
```

旧版 gh 未提供按名称编辑时，用 field-list / item-list 读取 ID，再按 `gh project item-edit --help` 使用 ID 形式，不能硬编码别的项目 ID。

## 模板、CI 与保护

Forms 默认 needs-triage，来源为显式输入；人工/Agent 的来源由分诊或已授权 Agent 添加，不自动把所有创建者标 human-reported。PR 必填 Issue、实现、验证、验收、ADR和限制，但模板复选框本身不是强制校验或批准。

Quality 仅 contents: read，checkout 不持久化凭据，执行当前 Rust fmt/test/clippy，不执行 pull_request_target、不发布、不修改任务。checkout 固定到核验过的 v4 commit；未来升级单独审查。Actions 尚未远端运行，不能声称 CI 已通过。

维护者确认远端 checks 名称后再设置保护规则、必需审查及合并策略。表单语法依据 [GitHub Issue Forms 文档](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms)，CLI 依据本机 2.103.3 的 `gh project --help` 和子命令帮助。
