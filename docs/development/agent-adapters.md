# Coding Agent 适配与能力检查

业务规则只维护在 AGENTS.md 和 `.agents/skills/`。这里集中平台调用方式，不把某个 Agent 的私有工具语法放进通用 Skill。检查日期：2026-11-14。

## 当前 Codex 会话

- 本机 CLI：0.167.0；当前宿主会话通过插件提供 Superpowers 7.0.1，15 个 Skill 元数据可见，文件可读，实际已调用适用流程。不能由宿主能力推断每个独立 CLI 进程也自动启用同一插件。
- 项目发现路径：从 cwd 向仓库根扫描 `.agents/skills/`。在任务 Worktree 启动时读取其中的 AGENTS.md 和 Skills；不能指望未提交的主工作区 Skills 自动出现在另一 Worktree。
- 项目 Skill 用 `$issue-work`、`$adr-create` 等选择，也可直接要求读取对应 SKILL.md。使用 `/skills` 检查是否出现。官方规则见 [Build skills](https://learn.chatgpt.com/docs/build-skills)。
- 本次通过本机 `codex app-server` 的 `initialize` → `initialized` → `skills/list`（`cwds` 指定任务 Worktree、`forceReload: true`）实际验证七个项目 Skills（六个自定义加 git-commit）被识别且 enabled，未发起 `turn/start` 或模型请求。[App Server 协议](https://learn.chatgpt.com/docs/app-server)
- 当前会话实际工具有 `collaboration.spawn_agent`、`send_message`、`followup_task`、`list_agents`、`wait_agent`。当前最多 4 个并发槽含主 Agent；其他环境必须重新检查，不写死为项目要求。
- 未提供原生 `update_plan` / TODO 工具；使用 `docs/plans/` 检查清单替代。Plan 模式、提问工具不等同于可更新的 TODO。

当前宿主的只读委派示例（不是跨平台 API）：

```text
collaboration.spawn_agent({
  task_name: "review_workflow",
  fork_turns: "none",
  message: "按完整委派契约审查指定 Worktree；只读，不写文件，不操作远程。返回文件位置、问题、证据与验收结论。"
})
```

使用当前工具实际 schema；不要复制旧版本 `agent_type`、模型名或其它未提供参数。写子任务先由主 Agent 创建独立 `.worktrees/<task>` 和分支，将绝对路径及禁止修改范围明确传入。`followup_task` 用于继续修复；消息工具不自动证明任务已执行。主 Agent 独立验收。

原生 Worktree 创建接口本次只接受 `ref` 和异步参数，不能指定 `.worktrees/` 或空分支；因此采用用户要求的 Git CLI 固定路径，不使用宿主自动选址。未来版本若支持目标路径，可优先原生工具，但先核验。

## Pi 1.5.0

本机官方随包 `docs/skills.md`、`docs/configuration.md` 和 `docs/rpc-commands.md` 已实际读取：原生支持项目 `.agents/skills/`，向祖先扫描至仓库根；无需创建 `.pi/skills` 副本或符号链接。

在可信项目里使用 `/skill:issue-work`，修改后 `/reload`。显式调用附任务号与权限，例如“读取任务并输出计划，暂不写远程”。不支持此命令的旧版本，应按其官方发现配置显式指向同一目录，或直接读取文件，不复制业务规则。

本次通过**隔离配置、离线且禁用扩展**的 RPC `get_commands` 实测发现项目 Skills，scope=project。没有发送 `prompt`；这证明发现路径和元数据加载，不代表完成真实 Issue 开发。配置目录设为本任务 `.superpowers/pi-discovery/`，没有复用用户包列表：

```bash
# PI_CHECK_DIR 必须是当前任务 Worktree 下的临时目录，不能复用用户配置目录
PI_CHECK_DIR="$PWD/.superpowers/pi-discovery"
mkdir -p "$PI_CHECK_DIR"
PI_CODING_AGENT_DIR="$PI_CHECK_DIR" PI_OFFLINE=1 \
  pi --offline --mode rpc --no-session --no-extensions --no-mcp --no-tools --approve
# RPC 输入仅发送：{"type":"get_commands"}，检查 response 后退出；不发送 prompt。
```

`--approve` 仅在此隔离探测中信任已审阅项目资源，不作为未来任务的自动批准策略。完整用户环境中 TODO/Subagent 扩展虽在 packages 配置中出现，未做实际开发调用，不能据配置宣称运行时可用；检查工具清单后决定，缺失就串行和使用文本计划。

**本次检查偏差：** 最初只读检查调用用户环境的 `pi --help`，Pi 为发现扩展参数自动同步 packages，意外克隆 Superpowers 7.0.2 到 `~/.pi/agent/git/github.com/obra/superpowers` 并生成 package-lock.json（HEAD `22f0b463e7a5e579904dd6211a520321035526f7`）。未改 settings、未发模型请求，已停止该方式，未自行删除用户目录内容。不能将这次副作用说成预先已安装/加载。后续核验采用上述隔离模式。

## git-commit 与现有适配

用户安装的 `.agents/skills/git-commit/SKILL.md` 来自 `github/awesome-copilot`，根 `skills-lock.json` 记录来源及锁信息。实际读取后使用，不改其正文、不重复实现。已有 `.claude/skills/git-commit` 是指向同一文件的用户安装链接，予以保留；六个自定义 Skills 不另建副本。

提交格式：`type(scope): 中文摘要`，首行最多 72 字符；正文中文解释原因与影响；英文 Footer 如 `Closes: #21`（必须真实关联）。Scope 可省略，标题不加句点，不捏造未验证成果。Skill 不负责 push/PR，授权不能由提交扩展到远程。

## 其他 Agent 与缺失能力

1. 检查根/子目录规则继承、标准 SKILL.md 支持、发现路径和当前工具清单；不要假设插件跨平台自动同步。
2. Superpowers 分别核对安装位置、版本、技能目录和运行时可见性；缺失时提供所需安装方案，未经授权不安装或修改全局目录。
3. 支持通用 `.agents/skills` 就直接复用；否则用官方配置引用同一目录。无法自动发现时，显式读取 Skill 并报告手动加载。
4. 无 Subagent：按同样任务边界串行；无原生 TODO：计划检查清单；无 gh/权限：本地草稿并报告，不能声称远程已同步。
5. 用最小只读场景验证发现、加载、应用，分别记录结果；格式遵循 [Agent Skills 标准](https://agentskills.io/specification)。
