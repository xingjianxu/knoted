# T12 部署与可运行性证据：Strata IQ3_S 本地 provider

- 任务卡：[T12](../tasks/T12.md)。本文只覆盖 **app 子任务**：Knoted 侧 provider 契约、Nix 声明模块、
  接口样例与验证清单。`/etc/nixos` 的安装/启用属于独立 host 子任务。
- 当前状态：**host 安装未执行**。`/data/llm/strata` 内没有已安装的 Strata 程序或 IQ3_S 模型，
  `127.0.0.1:15604` 上没有服务，因此 **t12_1 / t12_2 未通过**；`t12_3` 的约束（未完成部署不得写通过）
  已满足，但这不代表部署验收通过。
- 已完成的 app 侧证据：
  1. `nix/strata-client.nix` 通过语法解析、NixOS 完整求值与反向断言测试；
  2. 用上游自带的 mock 引擎在临时端口上采集了真实的 OpenAI 兼容请求/流式响应与断连取消行为；
  3. 按上游固定值定义模型/上游摘要表，实测列全部标“未验证”。
- mock 预检不是 IQ3_S 运行验证，也不能替代 15604 上的部署验收。没有任何一处把“预期值”写成“已通过”。

## 1. 上游与固定身份

只读检查的上游快照：`/tmp/pi-github-repos/runtime-7YkPt7/d28d017…`（目录名是快照标签，**不是** git HEAD）。
目录内 `git rev-parse HEAD` 为 `13eedf552417a6a791a7d13c6597e24300ee3a09`（2026-11-11，`serve: the #4677 stop bound…`）；
`strata_cli/__init__.py` 的 `PRODUCT_VERSION` 为 `0.2.13.1`（`docs/INSTALL.md` 写作 0.2.13）。
接口结论以 git HEAD 为准，版本以 `PRODUCT_VERSION` 为准。

| 项 | 固定值 | 来源 |
| --- | --- | --- |
| Strata 版本 | `0.2.13.1` | `strata_cli/__init__.py` `PRODUCT_VERSION` |
| 接口核对 commit | `13eedf552417a6a791a7d13c6597e24300ee3a09` | 上游 git HEAD |
| 安装方式 | 上游 release 安装器/预安装程序，**不由 Knoted fetch** | `docs/INSTALL.md`、`docs/TERMINAL_LAUNCH.md` |
| 安装根 | `/data/llm/strata` | `development-and-deploy.md` |
| 服务入口 | `strata serve --config <run config> --host 127.0.0.1 --port 15604 --offline` | `strata_cli/cli.py` `serve` 子命令 |
| 模型仓库 | `ISTA-DASLab/Qwen3.8-Flash-Next-GSQ-RCO-GGUF` | 上游 `setup.py` `HF_REVISIONS` |
| 模型 revision | `ed59f92082b1e93c0e96d60a8b11aab089b52f09`（2026-09-29） | 同上 |
| 量化/别名 | `IQ3_S` / `qwen3.8-flash-next-iq3_s` | `setup.py`、`strata_control/drafting.py` |

说明：上游 release 安装器本身不按 commit 固定，因此 host 安装后必须记录实际安装的
`strata --version`、组件版本以及每个下载/安装文件的 size+sha256；与本表不一致时记录偏差，不静默替换。

## 2. 已实际执行的接口预检（mock，非 IQ3_S）

目的：在不动主机、不下载模型的前提下，确认真实 wire format（请求体、SSE 事件、错误体、断连取消），
供 T62 的 provider 客户端实现使用。运行的是上游自带 `serve.server --engine mock`，**不是** Strata 引擎，
没有 GPU、没有模型权重。

环境与命令（任务临时目录 `/data/knoted-dev/tasks/t12/`；Python 3.14.10；任务本地 venv 只装
`jinja2==3.1.6`、`regex==2026.9.10`，不是仓库依赖）：

```sh
cd <上游只读快照>
TMPDIR=/data/knoted-dev/tasks/t12/tmp \
  /data/knoted-dev/tasks/t12/probe-venv/bin/python -m serve.server \
  --engine mock --host 127.0.0.1 --port 19095 --api-key t12-probe-key \
  --script "probe ok"
```

`t12-probe-key` 是本次临时 mock 服务的一次性探测值（已在探测后废弃），不是任何真实凭据；
真实 key 只存在于 host 创建的运行凭据文件。

### 2.1 认证与错误体

缺少 key：

```http
POST /v1/chat/completions HTTP/1.1
Content-Type: application/json

{"model":"qwen3.8-flash-next-iq3_s","messages":[{"role":"user","content":"ping"}]}
```

```http
HTTP/1.1 401 Unauthorized
{"error": {"type": "authentication_error", "message": "missing or wrong API key"}}
```

`Authorization: Bearer <key>` 与 `x-api-key: <key>` 都被接受（上游 `api_key` 检查，见 `serve/server.py`）。
Knoted 统一使用 `Authorization: Bearer`，key 从运行凭据文件读取，不写进仓库、日志或 nix store。

### 2.2 模型列表

```sh
curl -sS -H 'Authorization: Bearer t12-probe-key' http://127.0.0.1:19095/v1/models
```

```json
{
  "object": "list",
  "data": [
    {
      "id": "qwen3.8-flash-next",
      "object": "model",
      "status": {"value": "loaded"},
      "meta": {"n_ctx": 32768},
      "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]}
    }
  ]
}
```

注意：mock 的 run config 没有 `model_name`，所以这里报告的是家族默认名 `qwen3.8-flash-next`。
**这是必须由 host 验证的点**：IQ3_S 的 run config 必须带 `"model_name": "qwen3.8-flash-next-iq3_s"`，
否则 `/v1/models` 与响应里的 `model` 字段不会等于 Knoted 配置的别名。t12_2 以此判失败，而不是静默改别名。

### 2.3 请求体（完整）

```json
{
  "model": "qwen3.8-flash-next-iq3_s",
  "messages": [
    {"role": "system", "content": "Answer in two words."},
    {"role": "user", "content": "Reply with: probe ok"}
  ],
  "max_tokens": 8,
  "temperature": 0,
  "stream": true,
  "stream_options": {"include_usage": true}
}
```

要点：
- 路径是 `/v1/chat/completions`，配置里的 base URL 不带 `/v1`。
- `stream_options.include_usage=true` 时，最后一个 chunk 带 `usage`。
- 上游会把思考内容放进 `delta.reasoning_content`，答案正文在 `delta.content`；Knoted 只能把
  `content` 当作答案，不能把 `reasoning_content` 拼进回答或引用证据。

### 2.4 流式响应（完整事件序列，原样采集）

响应头：

```http
HTTP/1.0 200 OK
Content-Type: text/event-stream
Cache-Control: no-cache
X-Accel-Buffering: no
Connection: close
```

事件序列（`stream-small.sse` 全文，11 个 `data:` 事件 + `[DONE]`；为便于阅读保留原始 JSON 单行）：

```text
data: {"id": "chatcmpl-a28b6f8797824884adfcbcbb", "object": "chat.completion.chunk", "created": 1794669478, "model": "qwen3.8-flash-next", "choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": null}]}

data: {"id": "chatcmpl-a28b6f8797824884adfcbcbb", "object": "chat.completion.chunk", "created": 1794669478, "model": "qwen3.8-flash-next", "choices": [{"index": 0, "delta": {"reasoning_content": "p"}, "finish_reason": null}]}

data: {"id": "chatcmpl-a28b6f8797824884adfcbcbb", "object": "chat.completion.chunk", "created": 1794669478, "model": "qwen3.8-flash-next", "choices": [{"index": 0, "delta": {"reasoning_content": "r"}, "finish_reason": null}]}

data: {"id": "chatcmpl-a28b6f8797824884adfcbcbb", "object": "chat.completion.chunk", "created": 1794669478, "model": "qwen3.8-flash-next", "choices": [{"index": 0, "delta": {"reasoning_content": "o"}, "finish_reason": null}]}

data: {"id": "chatcmpl-a28b6f8797824884adfcbcbb", "object": "chat.completion.chunk", "created": 1794669478, "model": "qwen3.8-flash-next", "choices": [{"index": 0, "delta": {"reasoning_content": "b"}, "finish_reason": null}]}

data: {"id": "chatcmpl-a28b6f8797824884adfcbcbb", "object": "chat.completion.chunk", "created": 1794669478, "model": "qwen3.8-flash-next", "choices": [{"index": 0, "delta": {"reasoning_content": "e"}, "finish_reason": null}]}

data: {"id": "chatcmpl-a28b6f8797824884adfcbcbb", "object": "chat.completion.chunk", "created": 1794669478, "model": "qwen3.8-flash-next", "choices": [{"index": 0, "delta": {"reasoning_content": " "}, "finish_reason": null}]}

data: {"id": "chatcmpl-a28b6f8797824884adfcbcbb", "object": "chat.completion.chunk", "created": 1794669478, "model": "qwen3.8-flash-next", "choices": [{"index": 0, "delta": {"reasoning_content": "o"}, "finish_reason": null}]}

data: {"id": "chatcmpl-a28b6f8797824884adfcbcbb", "object": "chat.completion.chunk", "created": 1794669478, "model": "qwen3.8-flash-next", "choices": [{"index": 0, "delta": {"reasoning_content": "k"}, "finish_reason": null}]}

data: {"id": "chatcmpl-a28b6f8797824884adfcbcbb", "object": "chat.completion.chunk", "created": 1794669478, "model": "qwen3.8-flash-next", "choices": [{"index": 0, "delta": {}, "finish_reason": "length"}], "usage": {"prompt_tokens": 286, "completion_tokens": 8, "total_tokens": 294, "prompt_tokens_details": {"cached_tokens": 0}}}

data: [DONE]
```

（`finish_reason: "length"` 是因为 `max_tokens=8` 截断；未截断时应为 `"stop"`。mock 脚本内容被模板放进了
思考通道，所以正文 `content` 为空；真实 IQ3_S 的 `content`/`reasoning_content` 分布必须由 host 实测记录。）

非流式响应形状（同端点 `stream:false`，mock 实测）：

```json
{
  "id": "chatcmpl-410ce6003b2e4c4da42978c5",
  "object": "chat.completion",
  "created": 1794669409,
  "model": "qwen3.8-flash-next",
  "choices": [{
    "index": 0,
    "message": {"role": "assistant", "content": null, "reasoning_content": "Strata interface probe: alpha be"},
    "finish_reason": "length"
  }],
  "usage": {"prompt_tokens": 263, "completion_tokens": 32, "total_tokens": 295,
            "prompt_tokens_details": {"cached_tokens": 0}}
}
```

### 2.5 仅回环（mock 预检）

```sh
ss -ltnp | grep 19095
# LISTEN 0 256 127.0.0.1:19095 0.0.0.0:* users:(("python",pid=…,fd=6))
curl --max-time 2 http://<本机非回环地址>:19095/health
# curl: (7) Failed to connect …; HTTP 000
```

这只证明 `--host 127.0.0.1` 的上游行为，**不证明 15604 已部署**；t12_1 仍在部署后重跑。

### 2.6 断连取消（mock 实测）

方法：mock 脚本 120 个 token、`--script-delay 0.05`（单个请求约 6 秒）；客户端流式读取 0.6 秒后关闭连接。

```sh
timeout 0.6 curl -sS -N -H 'Authorization: Bearer t12-probe-key' \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen3.8-flash-next-iq3_s","messages":[{"role":"user","content":"long"}],"max_tokens":200,"stream":true}' \
  http://127.0.0.1:19095/v1/chat/completions > stream-disconnect.partial.sse
```

观测：
- 客户端侧只收到约 2.7 KB 的部分 SSE，**没有 `[DONE]`**；
- 断连 0.3 秒后 `GET /v1/status`：`activity.in_flight = 0`（该次请求已不在执行中）；
- 紧接着的第二个请求（16 token，mock 第二条脚本）**818 ms** 返回；若第一次生成未被中止，按
  120 token × 50 ms 应阻塞约 6 秒。说明服务端在客户端断连后停止了生成并归还了槽位。

取消语义（Knoted 侧的硬要求，写进 T62 客户端）：

1. 客户端断连即取消：不自动重发、不把部分输出标记为完成或成功。
2. 服务端必须停止该次生成（engine STOP/取消事件），不得在无人读取时继续消耗 GPU。
3. 只在**确认生成已停止**后归还并发槽；归还前后都不把这次半成品交给其他请求/任务复用。
4. 若引擎不响应停止，槽位保持占用并显式失败，不能假装已释放。
5. Knoted SSE 对外事件仍按 `data-and-api.md`：`queued/started/delta/citation/completed/failed/cancelled`；
   断连后客户端用 GET 查询状态，只有显式 `POST /cancel` 才产生取消任务；部分正文标未完成。

停止整个服务时的取消：`strata serve` 默认 `--shutdown-timeout 30` 排空并取消自有请求；Nix 单元
`TimeoutStopSec=300`、`KillMode=control-group`，确保引擎子进程不会在服务停止后继续占卡。
host 必须实测：长生成进行中 `systemctl stop knoted-strata`，然后
`nvidia-smi --query-compute-apps=pid,process_name,gpu_uuid --format=csv` 不再有该引擎进程。

## 3. Knoted 侧 provider 契约

| 项 | 值 | 说明 |
| --- | --- | --- |
| base URL | `http://127.0.0.1:15604` | 仅回环；客户端自行拼 `/v1/chat/completions` |
| 模型别名 | `qwen3.8-flash-next-iq3_s` | 必须与 `/v1/models` 和响应 `model` 字段一致 |
| 凭据文件 | `/run/knoted/strata-api-key` | root:knoted 0440；服务端 LoadCredential 与 Knoted 客户端共用 |
| 认证头 | `Authorization: Bearer <key>` | 也接受 `x-api-key`，Knoted 只固定一种 |
| 流式 | `stream:true` + SSE | `delta.content` 是答案；`delta.reasoning_content` 不进入答案 |
| 结束 | `data: [DONE]` | 未收到即视为失败/取消，不把部分内容当完整 |
| 预算 | 32K 总预算（含输出），系统+问题+引用+历史 ≤28K | 见 `indexing-and-rag.md`；tokenizer 优先用模型自带 |
| 失败 | 连接失败/401/5xx 不自动 fallback 到 external | 默认 local；显式选择才用 external |

`/v1/models` 的 `id`、响应 `model` 字段和配置别名三者不一致时，provider 配置视为无效，报依赖不可用，
不得静默改用服务端报告的名字。

## 4. host 子任务接口约定（安装/启用时需要对齐）

1. 安装 Strata `0.2.13.1`（记录实际 build/组件版本）到 `/data/llm/strata`；Knoted 不 fetch 二进制。
2. 安装第 5 节的 IQ3_S 文件到 `/data/llm/strata/models/IQ3_S/`，实测每个文件的 size/sha256。
3. 生成 `/data/llm/strata/config/iq3_s.json`，必须包含 `"model_name": "qwen3.8-flash-next-iq3_s"`；
   建议 `vision:false`（Knoted 只用文本），并记录实际下载的 MTP drafter 组件与摘要。
4. 创建 `/run/knoted`（0750 root:knoted）和 `/run/knoted/strata-api-key`（root:knoted 0440）；
   密钥不得进入 nix store、git、日志或 Issue。
5. 用 `nix/strata-client.nix` 的 `knoted.strata` 选项启用 `knoted-strata.service`；单元以
   `CUDA_VISIBLE_DEVICES=<两张 3090 UUID>` 运行，`strata serve` 只绑定 `127.0.0.1:15604`。
6. 安装后回报：host 提交 SHA、实际 `strata --version`、组件版本、实测模型摘要、`/v1/models` 的 `id`、
   `ss -ltnp` 的 15604 行、一次流式调用记录与断连取消记录。

## 5. 固定模型记录表（实测值待 host 填充）

规则：
- “期望值”来自上游 pin（`setup.py` `ISTA_FILES`/`HF_REVISIONS`），只能填在期望列。
- “实测值”必须来自 host 在 wks-lab 上执行 `stat -c '%s %n'` 与 `sha256sum` 的原始输出；没跑就写 **未验证**。
- 不得把期望值抄进实测列；不得用“应该一致”代替命令输出。
- 只有实测 size+sha256 全部匹配、`/v1/models` 别名匹配、t12_1/t12_2/t12_3 全部真实执行通过，
  才能把状态改为“通过”；任一不匹配即记录偏差并停止，不静默替换文件或别名。

| 文件 / 组件 | 角色 | 期望 size (bytes) | 期望 sha256 | 实测 size | 实测 sha256 | 上游 revision | 状态 |
| --- | --- | ---: | --- | --- | --- | --- | --- |
| `Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00001-of-00002.gguf` | 权重 shard 1 | 54,817,524,224 | `4c1eb2ceb4915e1192f4f386021897bde56a97f40a0bb78bb86465e0f7d2aca3` | 未验证 | 未验证 | `ed59f92…f09` | 未验证 |
| `Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00002-of-00002.gguf` | n-gram 表（shard 2） | 28,800,138,432 | `316b46f3a2dbd68c900f43136ab9449f9dcc3725dfd8c794847c204bc161e113` | 未验证 | 未验证 | `ed59f92…f09` | 未验证 |
| MTP drafter pack（名称以 `strata launch --plan` 为准） | 投机解码草稿层 | 未验证 | 未验证 | 未验证 | 未验证 | 未验证 | 未验证 |
| Strata engine 组件（CUDA，按卡） | 推理引擎 | 未验证 | 未验证 | 未验证 | 未验证 | 未验证（随 release） | 未验证 |
| tokenizer pack（`pack/full/tokenizer`） | 分词/计数 | 未验证 | 未验证 | 未验证 | 未验证 | 未验证（随组件） | 未验证 |

说明：Knoted 只用文本，`mmproj-Qwen3.8-Flash-Next-BF16.gguf`（视觉投影，907,543,008 bytes）不是必需项，
若 host 一并安装则另记一行；`vision:false` 时不得加载以省 VRAM。

## 6. 验收清单（部署未执行，当前未通过）

| 场景 | 要求 | 当前状态 | 部署后必须执行的命令 |
| --- | --- | --- | --- |
| `t12_1` | 服务仅回环 | **未通过（未部署）**；仅在 mock 19095 上预检过 `--host 127.0.0.1` | `ss -ltnp \| grep ':15604'` 必须只有 `127.0.0.1:15604`；从非回环地址连接必须失败 |
| `t12_2` | 记录模型与上游摘要 | **未通过（未验证）**；第 5 节表已建立，实测列全为“未验证” | `sha256sum /data/llm/strata/models/IQ3_S/*.gguf`；`curl -H "Authorization: Bearer $(cat /run/knoted/strata-api-key)" http://127.0.0.1:15604/v1/models` |
| `t12_3` | 未完成部署不写通过 | **满足约束**：全文未把任何部署/运行项写成通过 | 本文件与 T12 卡状态保持 pending，直到 host 证据补齐 |

host 安装后的最小功能验证（必须真实执行并回填命令/退出码/输出）：

```sh
# 1) 回环与健康
ss -ltnp | grep ':15604'
curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:15604/health

# 2) 模型身份
curl -sS -H "Authorization: Bearer $(cat /run/knoted/strata-api-key)" \
  http://127.0.0.1:15604/v1/models

# 3) 小样本流式（记录完整 SSE，确认 content 与 [DONE]）
curl -sS -N -H "Authorization: Bearer $(cat /run/knoted/strata-api-key)" \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen3.8-flash-next-iq3_s","messages":[{"role":"user","content":"Reply with: ok"}],"max_tokens":16,"temperature":0,"stream":true,"stream_options":{"include_usage":true}}' \
  http://127.0.0.1:15604/v1/chat/completions

# 4) 断连取消（长输出，读 1 秒后关闭；随后查 /v1/status 与 GPU 进程）
timeout 1 curl -sS -N ...（同上，max_tokens 较大）> partial.sse
curl -sS -H "Authorization: Bearer $(cat /run/knoted/strata-api-key)" http://127.0.0.1:15604/v1/status
nvidia-smi --query-compute-apps=pid,process_name,gpu_uuid --format=csv

# 5) 单元停止不残留引擎
systemctl stop knoted-strata
nvidia-smi --query-compute-apps=pid,process_name,gpu_uuid --format=csv
```

任一步失败即 t12 未通过，不得以“接口能连”替代模型加载/生成/停止的实测。

## 7. Nix 模块验证（已实际执行）

文件：`nix/strata-client.nix`。它是标准 NixOS module（`options.knoted.strata`），host 可 `imports` 后启用，
不 fetch 二进制、不写密钥。

```sh
nix-instantiate --parse nix/strata-client.nix
# exit 0，输出完整 AST（带模块函数头，约 11 KB）

nix eval --impure --raw --expr '<nixpkgs> 完整 NixOS 求值，读取 config.knoted.strata.provider 等>'
# exit 0；关键值：
# baseUrl=http://127.0.0.1:15604
# model=qwen3.8-flash-next-iq3_s
# credentialFile=/run/knoted/strata-api-key
# CUDA_VISIBLE_DEVICES=GPU-6bc31459-…,GPU-9e2b6db5-…
# LoadCredential=strata-api-key:/run/knoted/strata-api-key
# ExecStart=/nix/store/…-knoted-strata-serve
```

反向断言（应失败，实际失败）：

| 覆盖 | 结果 | 输出 |
| --- | --- | --- |
| `listen.address = "0.0.0.0"` | exit 1 | `the provider must listen on 127.0.0.1 only` |
| `gpus = [ "0" "1" ]` | exit 1 | `every entry must be a GPU UUID from nvidia-smi (GPU-…), not a numeric index` |
| `extraArgs = [ "--api-key=leak" ]` | exit 1 | `never pass an API key on the command line; use apiKeyFile` |

限制：只做了 parse + NixOS 求值 + 断言，**没有** `nix build`、没有主机激活、没有修改 `/etc/nixos`；
host 仍需在自己的 flake 中实际构建并启用。

## 8. 未执行 / 限制

- Strata 程序、engine 组件、IQ3_S 权重均未安装；`/data/llm/strata` 未创建。
- `127.0.0.1:15604` 未监听；没有真实 GPU 推理、没有真实流式生成、没有真实断连取消。
- mock 预检用临时端口 19095，不写入任何系统配置；其任务目录证据（`/data/knoted-dev/tasks/t12/evidence/`）
  不提交到仓库，本文已内联关键原文。
- `/v1/models` 在 mock 下报 `qwen3.8-flash-next` 而非别名，这是 run config `model_name` 缺失的预期现象；
  host 必须让 IQ3_S run config 显式带别名，t12_2 以此验收。
- 本任务未修改 T12 任务卡状态；由主 Agent 在 host 证据齐全后更新。
- 未执行任何 GitHub / 远程操作；只做本地提交。
