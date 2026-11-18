# T61 模型服务适配器实施证据

- 任务卡：[T61](../tasks/T61.md)｜Issue：[#196](https://github.com/xingjianxu/knoted/issues/196)
- 基线：`341b9a5`，分支 `feat/t61`，工作区 `.worktrees/knoted-t61`
- 本地命令输出：`/data/knoted-dev/tasks/t61/evidence/`

`ChatProvidersConfig` 从 `[chat.local]` 及 `[chat.external.<name>]` 读取 `base_url`、`model`、`credential_file`。配置接受本地 IP 回环及显式选定的外部 HTTPS 服务。凭据从各自文件读取，鉴权头标记敏感，错误保留固定分类。默认选择 `local`，本地配置缺失、鉴权错误或请求失败都不会调用其他服务。

Strata 通过 `/v1/status` 和 `/v1/models` 核对模型与上下文设置，在 32768 总预算内给输出预留 4096 token。返回的 `ProviderInfo` 提供实际模型、所选服务和输入上限，供生成运行记录与上下文构建消费。模型别名由部署配置显式传入，T12 部署契约继续提供固定值。

`ChatProvider::stream` 读取 SSE 实际正文并区分结束原因、服务报错、传输中断和取消。成功终止要求结束原因为 `stop` 或 `length`，之后收到 `[DONE]`。思考通道不进入正文。取消先关闭连接，本地轮询运行状态到 `activity.in_flight=0` 后提供停止证明，通用外部端点返回停止未确认，由生成队列保留对应资源状态。

| 验证 | 结果 |
|---|---|
| `cargo test --locked --workspace t61_` | exit 0，7 passed / 0 failed |
| `cargo fmt --check` | exit 0 |
| `cargo clippy --locked --all-targets -- -D warnings` | exit 0 |
| `git diff --check` | exit 0 |

定向用例覆盖默认选择、无自动回退、凭据/提示词脱敏、显式错误先于结束标记、断连、SSE 跨块 CRLF/UTF-8、本地上下文降限及取消确认差异。实现前测试在缺少 provider 模块时编译失败，随后连接两个独立的本机协议模拟服务执行。

T62 生成任务已接入该适配器，[PR #213](https://github.com/xingjianxu/knoted/pull/213) 合入 `867b004`，主线复跑 `cargo test --locked -p knoted-core --test t61` 为 7 passed / 0 failed。pcwrap 支持显式设置 `KNOTED_CHAT_LOCAL_BASE_URL`、`KNOTED_CHAT_LOCAL_MODEL` 和 `KNOTED_CHAT_LOCAL_CREDENTIAL_FILE`，三项齐备时启用本机配置。

## 真实 Strata dev 验收

用户在 2026-11-19 授权修改宿主配置。`/etc/nixos` 本地主线提交 `a5e2f5b` 为 `wks-lab` 的 `xingjian` 配置宿主既有 `strata-clients` 组。Nix 语法检查、组成员求值和系统候选构建均通过。候选还包含 dae 与 Home Manager 单元差异，本轮通过 `gpasswd` 定向激活组成员资格。宿主共享 Strata、Caddy、PostgreSQL、OnlyOffice 和 dae 的 PID 核对一致。

以继承新组的 `xingjian` 启动 pcwrap dev，API 与 Worker 运行 `867b004`。`[chat.local]` 使用 `http://127.0.0.1:15604`、`qwen3.8-flash-next-iq3_s` 和 `/run/strata/api-key`。鉴权后的 `/v1/models` 和 `/v1/status` 均返回 HTTP 200，上报模型已加载，实际上下文窗口 32768。凭据由进程直接从宿主运行期文件读取。

通过真实 dev HTTPS 和 Chromium 使用合成资料及独立标签。取消前观察到 Strata `activity.in_flight > 0` 且 Worker 已发送请求。取消后观察到 Strata `activity.in_flight = 0`、运行与任务均为 `cancelled`、生成位已释放，SSE 持久事件以 `cancelled` 结束。

取消后的下一轮问答在同一会话生成 41 条增量和 1 条有效引用，收到 `completed`，持久回答包含预期合成验收词。数据库记录模型终态证明和 `succeeded`，Strata 终态 `activity.in_flight = 0`。临时授权已撤销，合成会话、笔记与标签已软删除。`cargo fmt --check` 和 `cargo test --locked -p knoted-core --test t61` 复跑通过，专项 7 项。

dev 保留已启用的本机模型配置。今后完整重建 pcwrap 编排时使用上述本机端点、模型和凭据路径对应的三项环境变量。

验证输出保存在 `/data/knoted-dev/tasks/t61/evidence/`。

- 宿主候选构建及组成员为 `host-build.log`、`host-groups.json`，共享服务 PID 为 `shared-before.txt`、`shared-after.txt`。
- 真实模型 HTTP 状态和上下文为 `strata-authenticated-status.json`，浏览器场景为 `real-strata-smoke.log`，持久终态、生成位与 SSE 末事件为 `real-strata-terminal-rows.json`。
- `real-strata-cleanup.txt` 的 `t|0|0|0` 依次表示持久回答匹配和未删除的合成会话、笔记、标签数。专项输出为 `t61-providers-recheck.log`。
