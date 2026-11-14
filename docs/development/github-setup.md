# GitHub 远程配置状态与操作记录

远程配置已在 2026-11-14 经用户明确授权执行：首次推送、标签增补、Projects 建立、分支规则集配置。本文只记录**实际执行的命令、实测结果和仍待人工处理的项**；未实际运行的检查明确标注，不得把本文当成无条件脚本执行。后续写操作按根 [AGENTS.md](../../AGENTS.md)「远程操作授权」执行：常规 Issue、Labels 与 Projects、push 非默认分支、PR 创建与普通合并默认授权、不再逐次许可；仓库级设置（ruleset、分支保护、webhook、协作者与权限、Actions、可见性与删除、标签名增删）仍须人工确认。

## 当前实际状态

| 项 | 实际值 |
|---|---|
| 默认分支 | `master`（GraphQL `defaultBranchRef` 实测） |
| 首次推送 | `refs/heads/master` → `c34db46` |
| 标签 | 27 个 = GitHub 默认 10 个 + 新增 17 个 |
| Projects | #1「Knoted Development」，已 link 到本仓库 |
| 规则集 | `27256364` master protection（active；仅 `deletion`、`non_fast_forward`、`pull_request` 三条，已移除必需检查） |
| Quality CI | **已移除**（不再运行 GitHub Actions 质量检查，改由本地检查把关） |
| Actions | enabled，`allowed_actions: all`，`sha_pinning_required: false` |
| 仓库合并策略 | 仅 merge commit；`delete_branch_on_merge: true` |

## 1. 身份与授权范围

`gh project` 需要 `project` scope，默认 OAuth 流程不含它：

```bash
gh auth refresh -h github.com -s project
gh auth status   # 实测 scopes: gist, project, read:org, repo, workflow
```

- 缺 scope 时 `gh project list` 直接失败：`missing required scopes [read:project]`。
- Git 推送凭据与 gh token **互相独立**（本机 `credential.helper = store --file ${XDG_RUNTIME_DIR}/agenix/git-creds`）；刷新 gh scope 不影响推送凭据，反之亦然。
- scope 具备不等于放开授权范围：常规远程写操作按默认授权执行，仓库级设置与规则 12 的边界仍需人工确认。

## 2. 首次推送与默认分支

```bash
git push -u origin master
```

- 空仓库上**第一个被推送的分支自动成为默认分支**，不需要 `gh repo edit --default-branch`；实测 `defaultBranchRef.name = master`。
- **易误判点：** 首次推送**之前**观察到，REST `repos/{owner}/{repo}` 的 `default_branch` **字段**在空仓库返回 `"master"` 占位值，而同时 GraphQL `defaultBranchRef` 为空。该观察发生在首推前的窗口内，现在无法只读复现。结论只针对这一个字段：判断主线请用 GraphQL `defaultBranchRef`、`git ls-remote --heads` 或 `GET /repos/{owner}/{repo}/branches`。

### 历史重建说明（必须保留的事实）

执行本次配置前，本地 `.git` 已被重新初始化：`objects` 为空、无 `logs/`、无 `packed-refs`、无任何 ref，远程当时也为空，因此原 4 个提交对象无副本可恢复。现 `master` 的 4 个提交按文档记录的分组与提交信息重建：

| 新哈希 | 提交 | 覆盖内容 |
|---|---|---|
| `096d2eb` | `chore: 纳管现有 Rust 项目骨架` | `Cargo.toml`、`Cargo.lock`、`src/main.rs`、原始 `.gitignore` |
| `db39b2e` | `chore(skills): 纳管已安装的提交 Skill` | `.agents/skills/git-commit`、`.claude` 链接、`skills-lock.json` |
| `e7bc8ed` | `chore(workflow): 建立 Agent 驱动开发管理体系` | AGENTS.md、六个 Skill、模板、CI、文档 |
| `c34db46` | `docs(workflow): 记录初始化验证与本地集成结果` | 初始化记录与计划账本 |

旧哈希 `e9c3363` / `9171137` / `d04ea56` / `ad6363d` 已不存在。`docs/development/initialization-report.md` 与 `docs/plans/2026-11-14-agent-workflow.md` 中的失效哈希已改为文字描述，本节是唯一保留映射的位置；引用旧值时不能当作有效证据。

## 3. 标签

`docs/development/github-labels.json` 的 19 项中，`bug`、`enhancement` 已由 GitHub 默认存在且配色一致，未覆盖；实际新增 17 项：`feature, tech-debt, refactor, performance, security, agent-found, human-reported, needs-triage, needs-info, agent-ready, in-progress, blocked, human-required, P0, P1, P2, P3`。

在仓库根执行下列脚本（它读取仓库内相对路径，移植时先核对 `repo` 与标签数据）：

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

- 通过 argv 传参，不执行正文或 shell 插值；已存在的标签保持原值，不加 `--force`。
- 错误即停止；超时先读回远程状态再重试。
- **顺序约束：** Issue Forms 的 `labels:` 引用不存在的标签会被 GitHub **静默丢弃**（官方语法文档要求标签必须已存在于仓库），不报错也不自动创建。因此标签必须早于任何 Issue 创建。
- `gh issue create --body-file` 不套用 YAML 表单字段、也不自动加标签，正文与标签必须显式提供。

## 4. 远程验证结果

- **push 不再触发 CI：** 原 `Quality` 工作流（run `43576185739`）在 2026-11-14 确认可用后，按用户要求删除；仓库不再有质量门禁的远程环节，本地 `cargo fmt/test/clippy` 是唯一门禁。分支保护原先引用的必需检查 job 名 `Rust baseline` 已同步从 ruleset 移除，否则 PR 会永久卡在 “Expected — Waiting for status to be reported”。
- **Issue 路径：** #1 以 CLI 创建并附加 `feature`、`agent-found`、`needs-triage`，验证后以 `not planned` 关闭，作为审计痕迹保留。
- **PR 路径：** 本次修订即以 PR 提交，验证 PR 模板字段、`pull_request` 触发与必需检查。
- **未验证项：** Issue Forms **选择页的渲染**只能由已登录浏览器人工确认；CLI 与 API 均不渲染表单，不能声称已验证。

## 5. Projects

看板 #1 <https://github.com/users/xingjianxu/projects/1>，已 link 本仓库；13 个默认字段 + 5 个新增字段。

| 字段 | 类型 | 取值 |
|---|---|---|
| Status | SINGLE_SELECT | Backlog / Ready / In Progress / Review / Done |
| Priority | SINGLE_SELECT | P0–P3 |
| Task Type | SINGLE_SELECT | bug / feature / enhancement / tech-debt / refactor / performance / security |
| Source | SINGLE_SELECT | agent-found / human-reported |
| Module | TEXT | 当前可用 core/docs/workflow，随实际模块调整 |
| Agent | TEXT | 执行者标识 |

判读规则：Ready 由人工批准；Review 仍对应 in-progress；blocked / human-required 保留原阶段并显示标签；Done 仅在已合并后。

实测修正（相对本文件早期版本）：

- **字段名 `Type` 是保留名**，`gh project field-create --name Type` 返回 `Name cannot have a reserved value`；实际字段名为 `Task Type`。
- **内置 Status 的选项可以脚本化改写**，无需在 UI 手工配置。`gh project` 只有 `field-create` / `field-delete` / `field-list`，没有 `field-edit`，必须走 GraphQL：

```bash
# 先读回既有 fieldId、option id 与 description；再整表回写，仅新选项省略 id
gh api graphql -f query='
query { viewer { projectV2(number: 1) {
  field(name: "Status") {
    ... on ProjectV2SingleSelectField { id options { id name color description } } } } } }'

# 实际执行的 mutation 结构；此处把真实 fieldId/option id 换成占位符
gh api graphql -f query='
mutation {
  updateProjectV2Field(input: {
    fieldId: "<STATUS_FIELD_ID>"
    singleSelectOptions: [
      { id: "<EXISTING_ID>" name: "Backlog" color: GRAY description: "待人工分诊与批准" }
      { name: "Ready" color: BLUE description: "人工已批准，可交由 Agent 实施" }
      { id: "<EXISTING_ID>" name: "In Progress" color: YELLOW description: "实施或审查中" }
      { name: "Review" color: ORANGE description: "已提 PR，等待审查与合并" }
      { id: "<EXISTING_ID>" name: "Done" color: PURPLE description: "已合并到默认分支" }
    ]
  }) { projectV2Field { ... on ProjectV2SingleSelectField { id name options { id name } } } }
}'
```

`ProjectV2SingleSelectFieldOptionInput` 的 `name`、`color`、`description` 都是**非空无默认值**字段：每个选项都必须写 `description`（可写空字符串清空说明），缺少任一字段会被 GraphQL 直接拒绝。

`singleSelectOptions` 是**覆盖式**写入 id/name/color/description 四项：必须回写完整列表；保留既有 option id 才能保住条目的已选值；想保留既有说明就必须把原 description 一并回写，省略或留空会静默清空。

- Iteration 未启用；需要排期时在看板设置中配置。Projects 自动化（自动加入看板、写字段）未建立执行引擎，`gh project item-add` / `item-edit` 属默认授权的常规写操作，不再逐次许可。

## 6. 分支规则集与合并策略

公开个人仓库在 GitHub Free 即可使用 ruleset 与经典保护。`gh ruleset` 只有 `list` / `view` / `check`，**没有 create**，创建规则集必须用 REST：

```bash
gh api -X POST repos/xingjianxu/knoted/rulesets --input ruleset.json
```

下面是创建时提交的请求体（最小必需集合，未列出的属性取服务端默认值）：

```json
{
  "name": "master protection",
  "target": "branch",
  "enforcement": "active",
  "conditions": { "ref_name": { "include": ["refs/heads/master"], "exclude": [] } },
  "rules": [
    { "type": "deletion" },
    { "type": "non_fast_forward" },
    { "type": "pull_request",
      "parameters": { "required_approving_review_count": 0,
                      "allowed_merge_methods": ["merge"],
                      "dismiss_stale_reviews_on_push": true,
                      "require_code_owner_review": false,
                      "require_last_push_approval": false,
                      "required_review_thread_resolution": true } }
  ]
}
```

读回（`GET /repos/{owner}/{repo}/rulesets/{id}`）后可见服务端补全的默认值：`bypass_actors: []`、`current_user_can_bypass: never`、`do_not_enforce_on_create: false`、`required_reviewers: []`、`require_extra_approval_for_unattributed_changes: true`。也就是说本仓库 owner 也没有绕过通道。

- **单维护者约束：** `required_approving_review_count` 保持 0，不要求人工审批；结合默认授权的 PR 合并，Agent 可在本地检查通过后自行合入。提高审批数前必须存在第二个可用审查者。
- **无 bypass 列表**：`bypass_actors: []`、`current_user_can_bypass: never`，保留的 `deletion` 与 `non_fast_forward` 对仓库 owner 同样生效，即 push 直推 `master` 会被拒绝，必须走 PR。
- 合并策略：`allow_squash_merge=false`、`allow_rebase_merge=false`、`delete_branch_on_merge=true`，即仅普通 merge commit。
- **已移除必需检查（2026-11-14 实测）：** 删除 `Quality` 工作流后，`required_status_checks` 会让 PR 永久停在 “Expected — Waiting for status to be reported”，因此同步执行：

  ```bash
  gh api -X PUT repos/xingjianxu/knoted/rulesets/27256364 --input ruleset.json
  ```

  请求体为上述完整四字段结构（`name`/`target`/`enforcement`/`conditions`/`rules`）。读回确认 `rules[].type` 现仅含 `deletion`、`non_fast_forward`、`pull_request`，响应 `updated_at` 为 `2026-11-14T20:50:32.010+08:00`。规则集修改属仓库设置类，不在默认授权范围。

## 7. 仍需人工决定

- 是否提高审批数量、是否为人工操作配置 bypass 列表。
- Issue Forms 选择页渲染的人工确认（提交一份真实表单 Issue）。
- 是否建立 Projects 自动化，以及 Iteration 排期。

已处理（后续授权任务）：失效提交哈希已改为文字描述并指向本节映射表；`README.md` 与 `AGENTS.md` 的入向/授权措辞改为不绑定具体时间的通用规则；`Quality` 工作流已删除并同步移除 ruleset 必需检查，本地检查成为唯一门禁。

## 8. 复核命令（只读）

```bash
gh auth status
gh api graphql -f query='{repository(owner:"xingjianxu",name:"knoted"){defaultBranchRef{name}}}'
gh label list --repo xingjianxu/knoted --limit 100 --json name --jq 'length'
gh project field-list 1 --owner xingjianxu --format json
gh ruleset list --repo xingjianxu/knoted
gh api repos/xingjianxu/knoted/rulesets/27256364
gh run list --repo xingjianxu/knoted --limit 5
```

## 9. 限制

- Quality 工作流已删除；仓库不再有 CI 任务，因此不存在 Actions 执行面风险。
- 仓库当前只有 Rust 骨架，`cargo test` 为 0 个测试；fmt/test/clippy 在本地通过，但**不代表业务覆盖**。
- 未设置组织级规则集、Actions 必需 pinning、分支/标签保护之外的仓库级策略。
- 表单语法依据 [GitHub Issue Forms 文档](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms)，CLI 依据本机 `gh 2.103.3` 实测。
