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

本机 Strata `/health` 返回 HTTP 200。dev 运行用户对 `/run/strata/api-key` 无读权限，真实鉴权调用、流和停止确认待授权本机凭据路径。
