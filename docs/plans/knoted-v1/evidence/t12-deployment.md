# T12 部署与可运行性证据：Strata IQ3_S 本地 provider

- 任务卡：[T12](../tasks/T12.md)。app 子任务（Knoted 侧 provider 契约、Nix 声明模块、接口样例）
  已合并：PR #21，master `b4a2d11`；`/etc/nixos` 的安装/启用由独立 host 子任务完成，提交
  `ae00563a3eb85e482b080049450e2b5103fffc85`（未 push）。
- 当前状态：**已部署，t12_1 / t12_2 / t12_3 全部通过**（2026-11-15 在 wks-lab 复验）。
  `127.0.0.1:15604` 上运行上游 Strata 引擎（`strata serve --profile`），`/v1/models` 报告
  `id=qwen3.8-flash-next-iq3_s`、`meta.n_ctx=32768`；三个模型文件的 size+sha256 与上游 LFS 摘要一致。
- 证据分三层，来源不混用：
  1. **app 侧**（§2 mock 预检、§7 Nix 模块求值/断言）——app 子任务实际执行；
  2. **host 侧**（§8 部署实测）——引自 host 证据
     `/data/knoted-dev/tasks/knoted-t06-t16/evidence/t12-host.md`；
  3. **本轮复验**（§5、§6 的命令与输出）——2026-11-15 对本机部署重新执行的部分；未复跑的项一律注明
     “由 host 证据 §X 提供，本轮未复跑”，不把转述写成实测。
- mock 预检仍**不是** IQ3_S 运行验证：它只证明 wire format，真实端点证据在 §6/§8。全文没有任何一处把
  “预期值”写成“已通过”。
- 与 app 契约的偏差共 **10 条**（§9）：4 条需要人工裁决（承载来源、服务单元名、凭据传递、取消语义），
  2 条建议计划确认（投机解码关闭、并发 1），其余为部署事实与运维前提。**契约未被静默修改。**

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
| 实际产品版本 | `0.2.13.1` | `strata --version`（host §2、§7） |
| 实际 Terminal bundle | `0.1.59.1`（95 MB，自带 Python 3.12.15） | host §2 |
| 实际引擎组件 | `0.1.59 linux-x86_64-cuda-cuda13`（`build_id=712fec6b4297+cuda13.0`，`min_driver.nvidia=580.65`） | host §7 |
| 实际承载 | 上游 `strata serve --profile <run config>`（`--profile`/`--config` 是同一参数的别名） | host §3；本机 `strata serve --help` |
| run config | `/data/llm/strata/config/iq3_s.json`，`sha256=00220d6f4b332ae8970806151c64c0c6f3a236c572018f6190b00c71ca6855b5`（无 `api_key` 字段） | host §7 |
| host 提交 | `/etc/nixos` `ae00563a3eb85e482b080049450e2b5103fffc85` | host §6.5 |
| 驱动 | `595.112.07`（3×RTX 3090） | host §7 |

说明：上游 release 安装器本身不按 commit 固定。实际装到的是 feed 上的 Terminal `0.1.59.1` + engine
`0.1.59`（`0.2.13.1` 是产品号，与接口核对 commit `13eedf5…` 一致但不由 commit 固定）。要完全按 commit
固定得改用源码构建，本任务未做——记入 §9 偏差 7。

## 2. 已实际执行的接口预检（mock，非 IQ3_S）

目的：在不动主机、不下载模型的前提下，确认真实 wire format（请求体、SSE 事件、错误体、断连取消），
供 T62 的 provider 客户端实现使用。运行的是上游自带 `serve.server --engine mock`，**不是** Strata 引擎，
没有 GPU、没有模型权重。

环境与命令（任务临时目录 `/data/knoted-dev/tasks/t12/`；Python 3.14.10；任务本地 venv 只装
`jinja2==3.1.6`、`regex==2026.9.10`，不是仓库依赖）：

```sh
cd <上游只读快照>
export PROBE_KEY=$(python3 -c 'import secrets; print(secrets.token_hex(12))')  # 一次性 mock 值
TMPDIR=/data/knoted-dev/tasks/t12/tmp \
  /data/knoted-dev/tasks/t12/probe-venv/bin/python -m serve.server \
  --engine mock --host 127.0.0.1 --port 19095 --api-key "$PROBE_KEY" \
  --script "probe ok"
```

`PROBE_KEY` 只在本次 mock 探测中生成、探测后丢弃，不是任何真实凭据；真实 key 只存在于 host
创建的运行凭据文件。下方命令统一用 `$PROBE_KEY` 占位，避免把任何字面量密钥写进证据或触发扫描器。

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
curl -sS -H "Authorization: Bearer $PROBE_KEY" http://127.0.0.1:19095/v1/models
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
LAN_IP=$(ip -4 route get 1.1.1.1 | awk '{print $7; exit}')
curl --max-time 2 "http://$LAN_IP:19095/health"
# curl: (7) Failed to connect …; HTTP 000
```

这只证明 `--host 127.0.0.1` 的上游行为，**不证明 15604 已部署**。部署后的真实回环验收见 §6.1（本轮复跑）。

### 2.6 断连取消（mock 实测）

方法：mock 脚本 120 个 token、`--script-delay 0.05`（单个请求约 6 秒）；客户端流式读取 0.6 秒后关闭连接。
先重启 mock（保留 `$PROBE_KEY`）：

```sh
kill "$(cat mock-server.pid)"
LONG=$(python3 -c "print(' '.join(['alpha']*120))")
TMPDIR=/data/knoted-dev/tasks/t12/tmp \
  /data/knoted-dev/tasks/t12/probe-venv/bin/python -m serve.server \
  --engine mock --host 127.0.0.1 --port 19095 --api-key "$PROBE_KEY" \
  --script-delay 0.05 --script "$LONG" --script "short second answer" \
  > mock-server-cancel.log 2>&1 &
echo $! > mock-server.pid
```

```sh
timeout 0.6 curl -sS -N -H "Authorization: Bearer $PROBE_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen3.8-flash-next-iq3_s","messages":[{"role":"user","content":"long"}],"max_tokens":200,"stream":true}' \
  http://127.0.0.1:19095/v1/chat/completions > stream-disconnect.partial.sse || true
curl -sS -H "Authorization: Bearer $PROBE_KEY" http://127.0.0.1:19095/v1/status
start=$(date +%s%3N)
curl -sS -H "Authorization: Bearer $PROBE_KEY" -H 'Content-Type: application/json' \
  -d '{"model":"qwen3.8-flash-next-iq3_s","messages":[{"role":"user","content":"short"}],"max_tokens":16,"stream":false}' \
  http://127.0.0.1:19095/v1/chat/completions > after-cancel.json
end=$(date +%s%3N)
echo "second request elapsed_ms=$((end-start))"
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
host 已实测：长生成进行中 `systemctl stop strata.service`（实际单元名，见 §9 偏差 2），然后
`nvidia-smi --query-compute-apps=pid,process_name,gpu_uuid --format=csv` 不再有该引擎进程；
中断时客户端收到 `error` 块 + `[DONE]`，结果见 §8.6。

## 3. Knoted 侧 provider 契约

| 项 | 值 | 说明 |
| --- | --- | --- |
| base URL | `http://127.0.0.1:15604` | 仅回环；客户端自行拼 `/v1/chat/completions` |
| 模型别名 | `qwen3.8-flash-next-iq3_s` | 必须与 `/v1/models` 和响应 `model` 字段一致 |
| 凭据文件 | `/run/knoted/strata-api-key` | root:knoted 0440；服务端 LoadCredential 与 Knoted 客户端共用 |
| 认证头 | `Authorization: Bearer <key>` | 也接受 `x-api-key`，Knoted 只固定一种 |
| 流式 | `stream:true` + SSE | `delta.content` 是答案；`delta.reasoning_content` 不进入答案 |
| 结束 | `data: [DONE]`，且流内没有 `error` 块 | 未收到 `[DONE]` 视为失败/取消；**收到 `[DONE]` 但先出现 `error` 块也必须判失败**（停机实测，§8.6）。客户端不能只看 `[DONE]` |
| 预算 | 32K 总预算（含输出），系统+问题+引用+历史 ≤28K | 见 `indexing-and-rag.md`；tokenizer 优先用模型自带 |
| 失败 | 连接失败/401/5xx 不自动 fallback 到 external | 默认 local；显式选择才用 external |

`/v1/models` 的 `id`、响应 `model` 字段和配置别名三者不一致时，provider 配置视为无效，报依赖不可用，
不得静默改用服务端报告的名字。

服务单元名与凭据路径的实际落点见 §8：app 契约里的 `knoted-strata.service` 与 host 实际单元
`strata.service` 不一致（§9 偏差 2），凭据传递方式见 §9 偏差 3。

**别名唯一权威**：`nix/strata-client.nix` 的 `knoted.strata.model.alias` 是唯一默认值；T61 不得在应用配置里
另写一份默认别名。服务启动前的 preflight 会读取 run config 的 `model_name` 并拒绝与别名不一致的启动，
所以别名漂移会在部署时失败，而不是运行时静默改变模型身份。

### 3.1 GPU 分配（计划约束）

`requirements.md` R30 与 `indexing-and-rag.md` 规定应用最多使用两张 GPU：一张 Strata、一张 Python。
wks-lab 有 3 张 3090，默认分配如下：

| nvidia-smi 序号 | UUID | 用途 |
| --- | --- | --- |
| 0 | `GPU-6bc31459-053f-60ef-0615-3aec9593c9e7` | Strata（`knoted.strata.gpus` 的唯一默认项） |
| 1 | `GPU-9e2b6db5-acef-fa85-a28a-7214d1f4c6c0` | Python（由 Python 服务按 UUID 配置，不在本模块） |
| 2 | `GPU-bf8d2065-6d30-4a83-d9a7-efb41ab26cef` | 不使用 |

`knoted.strata.gpus` 接受 1—2 条 UUID，默认 1 条；列 2 条必须同时有 R30/计划变更记录。断言拒绝 0 条、
3 条和数字索引；UUID 只归一化十六进制部分，`GPU-` 前缀保持大写。模块用 `CUDA_VISIBLE_DEVICES` 把 UUID
重编号为 0..n-1，Strata 的 `gpu` 字段仍按进程内索引工作，不依赖主机上的永久 0/1。

### 3.2 若 host 采用 llama.cpp 承载（等价映射，未验证）

同一批 IQ3_S 文件是标准 GGUF（上游未对 llama.cpp 承载这一组合作出承诺）。如果 host 选择用 nixpkgs 的
llama.cpp 而不是 Strata 承载它们，必须保持下列等价映射，否则 Knoted 契约失效：

| 契约项 | Strata 实现 | llama.cpp 等价参数 |
| --- | --- | --- |
| 监听 | `strata serve --host 127.0.0.1 --port 15604` | `--host 127.0.0.1 --port 15604` |
| 模型别名 | run config `model_name` | `--alias qwen3.8-flash-next-iq3_s` |
| 凭据 | systemd LoadCredential → `STRATA_API_KEY` | `--api-key-file /run/knoted/strata-api-key` |
| GPU | `CUDA_VISIBLE_DEVICES=<UUID>` | 同值；UUID 而非 0/1 索引 |
| 流式 | `/v1/chat/completions` SSE | 同端点、同 SSE 形状 |

`t12_1`/`t12_2` 的命令与断言完全不变：只监听 `127.0.0.1:15604`，`/v1/models` 的 `id` 与响应 `model`
字段必须等于别名。此承载方式本任务**未验证**；host 若采用，必须记录 llama.cpp 版本与构建摘要，并重跑
同一套验收命令，不能引用 Strata 的验证结果。

## 4. host 子任务接口约定（安装/启用时需要对齐）

以下任一项缺失都视为部署失败，不得把服务标为“可用”；模块的 preflight 也会拒绝启动。
**实际 host 走的是另一套声明（`features.server.strata`），下列第 7 条的“用本模块启用”未被执行；
差异与影响见 §9 偏差 1、2、8、9。本节保留为契约与将来统一时的对齐清单，不代表当前落点。**

1. **账号**：创建 `knoted` 系统用户/组（与 API、Python 服务共用）；模块的 tmpfiles 规则假设该组已存在。
2. **目录**：`install -d -o root -g root -m 0755 /data/llm/strata`，以及 model/config 子目录；
   `install -d -o knoted -g knoted -m 0750 /data/llm/strata/state`（`/data` 为独立挂载，tmpfiles 在
   `local-fs.target` 之后重设为 0750 knoted:knoted）。
3. **程序**：安装 Strata `0.2.13.1`（记录实际 build/组件版本）到 `/data/llm/strata`；Knoted 不 fetch 二进制。
4. **模型**：安装第 5 节的 IQ3_S 文件到 `/data/llm/strata/models/IQ3_S/`，实测每个文件的 size/sha256。
5. **run config**：生成 `/data/llm/strata/config/iq3_s.json`，必须包含
   `"model_name": "qwen3.8-flash-next-iq3_s"`（模块 preflight 用 jq 校验，不一致拒绝启动）；
   建议 `vision:false`（Knoted 只用文本），并记录实际下载的 MTP drafter 组件与摘要。
6. **运行凭据**：
   ```sh
   install -d -o root -g knoted -m 0750 /run/knoted
   install -o root -g knoted -m 0440 /dev/null /run/knoted/strata-api-key   # 内容由 host 写入真实 key
   { printf 'Authorization: Bearer '; cat /run/knoted/strata-api-key; printf '\n'; } \
     > /run/knoted/strata-auth-header
   chown knoted:knoted /run/knoted/strata-auth-header
   chmod 0600 /run/knoted/strata-auth-header
   ```
   密钥不得进入 nix store、git、日志、Issue 或命令行参数；验证命令用 `-H @<头文件>` 读取。
7. **单元**：用 `nix/strata-client.nix` 的 `knoted.strata` 选项启用 `knoted-strata.service`；默认
   `CUDA_VISIBLE_DEVICES=GPU-6bc31459-…`（单卡 Strata，Python 用另一张，第三张不用），
   `strata serve` 只绑定 `127.0.0.1:15604`。
8. **回报**：host 提交 SHA、实际 `strata --version`、组件版本、实测模型摘要、`/v1/models` 的 `id`、
   `ss -ltnp` 的 15604 行、一次流式调用记录与断连取消记录。

## 5. 固定模型记录表（已实测填充）

规则：
- “期望值”来自上游 pin（`setup.py` `ISTA_FILES`/`HF_REVISIONS`），只能填在期望列。
- “实测值”只来自在 wks-lab 上真实执行 `stat -c '%s %n'` 与 `sha256sum` 的原始输出；没跑就写 **未验证**。
- 不得把期望值抄进实测列；不得用“应该一致”代替命令输出。
- 只有实测 size+sha256 全部匹配、`/v1/models` 别名匹配、t12_1/t12_2/t12_3 全部真实执行通过，
  才能把状态改为“通过”；任一不匹配即记录偏差并停止，不静默替换文件或别名。

2026-11-15 本轮实际执行（`/data/llm/strata/models`）：

```console
$ time sha256sum IQ3_S/*.gguf mmproj-Qwen3.8-Flash-Next-BF16.gguf
4c1eb2ceb4915e1192f4f386021897bde56a97f40a0bb78bb86465e0f7d2aca3  IQ3_S/Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00001-of-00002.gguf
316b46f3a2dbd68c900f43136ab9449f9dcc3725dfd8c794847c204bc161e113  IQ3_S/Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00002-of-00002.gguf
b1a82259702816a5330d7bd7607cd9676b11780e79ff7348c21103ff3ce49bd0  mmproj-Qwen3.8-Flash-Next-BF16.gguf

real    1m23.410s

$ stat -c '%s %n' IQ3_S/*.gguf mmproj-Qwen3.8-Flash-Next-BF16.gguf
54817524224 IQ3_S/Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00001-of-00002.gguf
28800138432 IQ3_S/Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00002-of-00002.gguf
907543008 mmproj-Qwen3.8-Flash-Next-BF16.gguf
```

与 host 证据 §1（`real 1m23.449s`）及主 Agent 核验 `t12-model-verification.md` 三次独立测量一致。

| 文件 / 组件 | 角色 | 期望 size (bytes) | 期望 sha256 | 实测 size | 实测 sha256 | 上游 revision | 状态 |
| --- | --- | ---: | --- | --- | --- | --- | --- |
| `Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00001-of-00002.gguf` | 权重 shard 1（`--native`） | 54,817,524,224 | `4c1eb2ce…d2aca3` | 54,817,524,224 | `4c1eb2ceb4915e1192f4f386021897bde56a97f40a0bb78bb86465e0f7d2aca3` | `ed59f92…f09` | **通过** |
| `Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00002-of-00002.gguf` | PLE n-gram 表（`--ple-gguf`，非权重分片） | 28,800,138,432 | `316b46f3…61e113` | 28,800,138,432 | `316b46f3a2dbd68c900f43136ab9449f9dcc3725dfd8c794847c204bc161e113` | `ed59f92…f09` | **通过** |
| `mmproj-Qwen3.8-Flash-Next-BF16.gguf` | 视觉投影（未使用，`vision off`） | 907,543,008 | `b1a82259…e49bd0` | 907,543,008 | `b1a82259702816a5330d7bd7607cd9676b11780e79ff7348c21103ff3ce49bd0` | `ed59f92…f09` | **通过（未加载）** |
| MTP drafter pack | 投机解码草稿层 | 未验证 | 未验证 | — | — | — | **不适用**：`drafting=off`，不再依赖 `/wrd/opt/Strata-data/mtp`（§9 偏差 5） |
| Strata engine 组件 | 推理引擎 | 未验证 | 未验证 | `0.1.59 linux-x86_64-cuda-cuda13`，`build_id=712fec6b4297+cuda13.0` | 未记录（release checksum+signature 校验） | 随 release | 版本已记录，摘要未测 |
| tokenizer pack（`pack/full/tokenizer`） | 分词/计数 | 未验证 | 未验证 | 随 `packs/iq3_s`（1.5 GB）生成 | 未单独记录 | 随组件 | 未单独核验 |

说明：
- Knoted 只用文本；`mmproj` 虽已安装但 `vision.enabled=false`，引擎不加载它（省 VRAM）。
- 第二个文件按 GGUF 头解析为 `tensors=3 / kv=8` 的 PLE n-gram 表，由 `--ple-gguf` 加载（host §1）；
  它不是权重分片，`strata launch` 的 `--native` 只吃 shard 1。
- engine/tokenizer 未做 sha256：本任务范围是“模型与上游摘要”，引擎只记版本与 build id，不把版本号当摘要。

## 6. 验收清单（三条全部通过）

| 场景 | 要求 | 状态 | 证据 |
| --- | --- | --- | --- |
| `t12_1` | 服务仅回环 | **通过** | §6.1 本轮复验：`ss -ltnp` 只有 `127.0.0.1:15604`；非回环 `115.24.115.250:15604` 连接被拒（rc=7）；无密钥 `/v1/models` = 401 |
| `t12_2` | 记录模型与上游摘要 | **通过** | §5 + §6.2 本轮复验：三文件 size+sha256 与上游 LFS 一致；`/v1/models` `id=qwen3.8-flash-next-iq3_s`、`meta.n_ctx=32768`；`/v1/status` 别名/上下文/并发一致 |
| `t12_3` | 未完成部署不写通过 | **通过** | PR #21 阶段的 app 证据把 t12_1/t12_2 明确标“未通过（未部署）”；host 部署且 §5/§6 实测完成后才改为通过；未把期望值抄进实测列 |

所有 curl 用 `-H @/run/knoted/strata-auth-header`（0600，内容 `Authorization: Bearer <key>`）读取凭据，
**key 不进 curl argv**（本机其他用户可读 `/proc/*/cmdline`）。`curl -H @file` 需要 curl ≥ 7.55。

### 6.1 t12_1 仅回环（本轮复跑）

```console
$ ss -ltnp | grep 15604
LISTEN 0      256                             127.0.0.1:15604      0.0.0.0:*

$ curl -sS --max-time 5 -o /dev/null -w 'health=%{http_code}\n' http://127.0.0.1:15604/health
health=200

$ LAN_IP=$(ip -4 route get 1.1.1.1 | awk '{print $7; exit}'); echo "LAN_IP=$LAN_IP"
LAN_IP=115.24.115.250
$ curl -sS --max-time 5 -o /dev/null -w 'lan_health=%{http_code}\n' "http://$LAN_IP:15604/health"
curl: (7) Failed to connect to 115.24.115.250:15604 after 0 ms: Could not connect to server
lan_health=000

$ curl -sS --max-time 5 -o /dev/null -w 'models_nokey=%{http_code}\n' http://127.0.0.1:15604/v1/models
models_nokey=401
```

本命令带 `-p`，但非 root 时进程信息被隐藏（输出里没有 `users:(…)` 列）；绑定地址本身已足以判定。加 root
复跑能看到进程名，结论不变。401 说明认证仍生效：“回环可达”不等于“无认证可用”。

### 6.2 t12_2 模型身份与摘要（本轮复跑）

模型文件命令与输出见 §5（同一轮执行）。`/v1/models` 与 `/v1/status` 本轮重新执行：

```console
$ sudo curl -sS -H @/run/knoted/strata-auth-header http://127.0.0.1:15604/v1/models
{"object": "list", "data": [{"id": "qwen3.8-flash-next-iq3_s", "object": "model", "status": {"value": "loaded"}, "meta": {"n_ctx": 32768}, "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]}}]}

$ sudo curl -sS -H @/run/knoted/strata-auth-header http://127.0.0.1:15604/v1/status
{"service": "strata", "model": "qwen3.8-flash-next-iq3_s", "loaded": true, …, "engine": "0.1.59",
 "cache_max_tokens": 32768, "context": {"native": 32768, "max_positions": 32768},
 "concurrency": {"serving": 1, "requested": 1},
 "dialects": ["/v1/chat/completions", "/v1/messages", "/v1/responses"],
 "vision": {"enabled": false, "available": false, "error": null},
 "activity": {"requests": 0, "in_flight": 0, "last_request_at": null}, …}
```

（`/v1/status` 为可读性省略 `structured_output`/`machine` 等与本验收无关的字段，未改动任何值。
`activity.requests=0` 是因为服务在本轮验证前不久重启过，不是“没跑过请求”。）

模型身份、别名与上下文三项与 host 证据 §4 一致；host 侧 `/v1/models` 原始输出见 §8.3。

### 6.3 t12_3 未完成部署不写通过

- PR #21 合入时（master `b4a2d11`）本文件把 t12_1/t12_2 标为“未通过（未部署）”，T12 卡为 pending；
  当时没有把任何部署项写成通过。
- host 部署完成后才在本轮填实测值并改状态；先执行命令（§5、§6.1、§6.2），再改文字，顺序可查。
- host 未复跑的项（流式时间点、断连取消、停机语义）保留“由 host 证据 §X 提供，本轮未复跑”的标注，
  没有转写成实测。

### 6.4 部署后功能验证清单的执行状态

| 原清单步骤 | 命令 | 执行状态 |
| --- | --- | --- |
| 1 回环与健康 | `ss -ltnp \| grep 15604`、`curl …/health` | **本轮复跑**，§6.1 |
| 2 模型身份 | `curl -H @… /v1/models` | **本轮复跑**，§6.2 |
| 3 小样本流式 | `curl -N …` / 探针脚本 | 由 host 证据 §5 提供（探针 `/tmp/strata_probe.py`，密钥读文件不进 argv），**本轮未复跑** |
| 4 断连取消 | `timeout 1 curl …` + `/v1/status` + `nvidia-smi` | 由 host 证据 §5 提供，**本轮未复跑** |
| 5 单元停止不残留引擎 | `systemctl stop …` + `nvidia-smi` | 由 host 证据 §6 提供，**本轮未复跑**；实际单元名是 `strata.service`（§9 偏差 2） |

## 7. Nix 模块验证（已实际执行）

文件：`nix/strata-client.nix`。它是标准 NixOS module（`options.knoted.strata`），host 可 `imports` 后启用，
不 fetch 二进制、不写密钥。以下命令均在仓库根目录执行（`$(pwd)` 换成实际 worktree 根即可复现）。

```sh
nix-instantiate --parse nix/strata-client.nix
# exit 0，输出完整 AST，13,574 bytes
```

完整可复制的求值命令（本任务实际执行；`--impure` 只为允许 `<nixpkgs>` 与 `$(pwd)`）：

```sh
nix eval --impure --raw --expr "
let sys = import <nixpkgs/nixos> { configuration = {
  imports = [ $(pwd)/nix/strata-client.nix ];
  nixpkgs.hostPlatform = \"x86_64-linux\"; system.stateVersion = \"24.11\";
  knoted.strata.enable = true; }; };
in builtins.toJSON {
  baseUrl = sys.config.knoted.strata.provider.baseUrl;
  model = sys.config.knoted.strata.provider.model;
  package = sys.config.knoted.strata.package;
  runConfig = sys.config.knoted.strata.runConfig;
  modelPath = sys.config.knoted.strata.model.path;
  stateDir = sys.config.knoted.strata.stateDir;
  device = sys.config.systemd.services.knoted-strata.environment.CUDA_VISIBLE_DEVICES;
  tmpfiles = builtins.filter (r: builtins.match \".*knoted.*\" r != null)
    sys.config.systemd.tmpfiles.rules;
}"
```

实际输出（exit 0；JSON 的换行只为展示，字段值与原始单行一致）：

```json
{"baseUrl":"http://127.0.0.1:15604",
 "device":"GPU-6bc31459-053f-60ef-0615-3aec9593c9e7",
 "model":"qwen3.8-flash-next-iq3_s",
 "modelPath":"/data/llm/strata/models/IQ3_S",
 "package":"/data/llm/strata/bin/strata",
 "runConfig":"/data/llm/strata/config/iq3_s.json",
 "stateDir":"/data/llm/strata/state",
 "tmpfiles":["d /run/knoted 0750 root knoted -",
             "d /data/llm/strata/state 0750 knoted knoted -"]}
```

完整系统求值（会触发所有断言；需要最小 `fileSystems`/grub 才能过 NixOS 自身断言）：

```sh
nix eval --impure --raw --expr "
let sys = import <nixpkgs/nixos> { configuration = {
  imports = [ $(pwd)/nix/strata-client.nix ];
  nixpkgs.hostPlatform = \"x86_64-linux\"; system.stateVersion = \"24.11\";
  knoted.strata.enable = true;
  fileSystems.\"/\" = { device = \"/dev/vda1\"; fsType = \"ext4\"; };
  boot.loader.grub.devices = [ \"/dev/vda\" ]; }; };
in sys.config.system.build.toplevel.drvPath"
# exit 0；输出 /nix/store/…-nixos-system-nixos-…drv（只求值，未 build）
```

模块还在服务启动前做两项运行时校验（eval 不会触发；host 启动时由 preflight 实际执行）：
run config 的 `model_name` 必须等于别名（`jq -e`），`model.files` 每个文件的 `stat -c %s` 必须等于期望
size；任一不符即拒绝启动。sha256 留给 host 证据（t12_2）。

preflight 脚本单独构建并检查过失败闭合（不是系统构建）：

```sh
nix build --impure --no-link --print-out-paths --expr "
let sys = import <nixpkgs/nixos> { configuration = {
  imports = [ $(pwd)/nix/strata-client.nix ];
  nixpkgs.hostPlatform = \"x86_64-linux\"; system.stateVersion = \"24.11\";
  knoted.strata.enable = true; }; };
in sys.config.systemd.services.knoted-strata.serviceConfig.ExecStartPre"
# exit 0，输出 /nix/store/…-knoted-strata-preflight

bash -n /nix/store/…-knoted-strata-preflight
# exit 0

CUDA_VISIBLE_DEVICES=GPU-6bc31459-053f-60ef-0615-3aec9593c9e7 /nix/store/…-knoted-strata-preflight
# exit 1；knoted-strata: Strata CLI is not executable: /data/llm/strata/bin/strata
```

最后一条在未安装 host 路径的当前机器上执行，证明缺失前置条件时拒绝启动，而不是静默使用默认卡/模型。

反向断言（应失败，实际失败）：

| 覆盖 | 结果 | 输出 |
| --- | --- | --- |
| `listen.address = "0.0.0.0"` | exit 1 | `the provider must listen on 127.0.0.1 only` |
| `gpus = [ "0" "1" ]` | exit 1 | `every entry must be a GPU UUID from nvidia-smi (GPU-…), not a numeric index` |
| `gpus = [ <3 张真实 UUID> ]` | exit 1 | `1-2 cards only. The plan caps the application at two GPUs (one Strata, one Python)…` |
| `listen.port = 15605` | exit 1 | `The option 'knoted.strata.listen.port' is read-only, but it's set multiple times` |
| `extraArgs = [ "--api-key=leak" ]` | exit 1 | `never pass an API key on the command line; use apiKeyFile` |
| `gpus = [ "GPU-6BC31459-…" ]`（大写） | exit 0 | 归一化为小写十六进制、`GPU-` 前缀保留，`CUDA_VISIBLE_DEVICES=GPU-6bc31459-…` |

`git diff --check` / `git diff --cached --check` 均为 exit 0。

限制：只做了 parse + NixOS 求值 + 断言 + preflight 小脚本构建，**没有**构建或激活整个系统、没有修改
`/etc/nixos`；host 仍需在自己的 flake 中实际构建并启用。

## 8. host 部署实测记录（引自 host 证据）

本节全部引自 host 子任务证据 `/data/knoted-dev/tasks/knoted-t06-t16/evidence/t12-host.md`
（`/etc/nixos` 提交 `ae00563a3eb85e482b080049450e2b5103fffc85`，未 push）。除明确标注“本轮复跑”的项外，
本节内容本轮未复跑；小节号前缀 `host §` 指该证据文件。

### 8.1 实现与服务单元

- 承载程序是**上游 Strata `0.2.13.1`**：`strata --version` →
  `strata 0.2.13.1 (build 0.1.59.1; python 3.12.15 at …/terminal/0.1.59.1/python/bin/python3.12)`；
  Terminal bundle `0.1.59.1`，引擎组件 `0.1.59 linux-x86_64-cuda-cuda13`（`build_id=712fec6b4297+cuda13.0`）。
  **不是 llama.cpp**：§3.2 的等价承载未启用（host 证据 §8 偏差 1）。
- 服务单元：`strata.service`（NixOS 模块 `features.server.strata`，`/etc/nixos/features/server/strata.nix`），
  外加 `knoted-api-key.service`（oneshot，`Before=strata.service`，重建 `/run` 凭据）。
- `strata-serve` wrapper 的脱敏等价命令（真实 argv 无密钥）：

  ```sh
  STRATA_API_KEY="$(cat "$CREDENTIALS_DIRECTORY/strata-api-key")"; export STRATA_API_KEY
  exec /data/llm/strata/bin/strata serve \
    --profile /data/llm/strata/config/iq3_s.json \
    --host 127.0.0.1 --port 15604 --offline
  ```

  `--profile` 与 `--config` 在本机 CLI 里是同一参数的别名（`strata serve --help`），所以 app 模块写的
  `--config` 同样有效，不是偏差。
- 引擎子进程（`strata serve` 派生，`systemctl status`/`nvidia-smi` 可见）：

  ```sh
  …/components/engine/0.1.59-linux-x86_64-cuda-cuda13/strata --serve \
    --pack /data/llm/strata/packs/iq3_s \
    --native …/models/IQ3_S/…-00001-of-00002.gguf \
    --ple-gguf …/models/IQ3_S/…-00002-of-00002.gguf \
    --expert-profile …/terminal/0.1.59.1/data/expert-profile.bin \
    --expert-cache auto --prefill auto --spec 4 --max-context 32768 --kv int8 --ple-io ram
  ```

### 8.2 NixOS 专属启动依赖：`LD_LIBRARY_PATH=/run/opengl-driver/lib`

首次启动的真实报错（原样）：

```text
strata generate: native embedding: cannot pin 322 MiB: CUDA driver version is insufficient for
CUDA runtime version, nor place it in VRAM.
```

根因不是驱动旧（引擎要求 `min_driver.nvidia=580.65`，本机 `595.112.07`），而是 **NixOS 上 `libcuda.so.1`
不在搜索路径**：nix-ld 只加 `/run/current-system/sw/share/nix-ld/lib`，驱动库在 `/run/opengl-driver/lib`，
引擎 dlopen 失败。`LD_DEBUG=libs` 对照（host §2 第 3 步）：

```console
$ LD_DEBUG=libs strata --help | grep -A4 "find library=libcuda.so.1"
find library=libcuda.so.1 [0]; searching
  trying file=/run/current-system/sw/share/nix-ld/lib/libcuda.so.1
  …（无命中）
$ LD_LIBRARY_PATH=/run/opengl-driver/lib LD_DEBUG=libs strata --help | grep -A2 "find library=libcuda.so.1"
find library=libcuda.so.1 [0]; searching
 search path=/run/opengl-driver/lib:/run/current-system/sw/share/nix-ld/lib (LD_LIBRARY_PATH)
  trying file=/run/opengl-driver/lib/libcuda.so.1
 calling init: /run/opengl-driver/lib/libcuda.so.1
```

该变量因此是**启动硬依赖**，不是优化项；`nix/strata-client.nix` 目前没有设置它（§9 偏差 8）。
单元里另有两处 NixOS 专属环境：`XDG_DATA_HOME=/data/llm/strata/share`（引擎组件实际所在目录）与
`LoadCredential=strata-api-key:/run/knoted/strata-api-key`。

### 8.3 端点、GPU 与认证

```console
$ systemctl status strata.service | head -8
     Active: active (running)
    Process: … ExecStartPre=…-strata-preflight (code=exited, status=0/SUCCESS)
    Process: … ExecStartPost=…-strata-healthcheck (code=exited, status=0/SUCCESS)
   Main PID: 1770078 (python3.12)
     Memory: 49.5G (peak: 49.5G)          # 84 GB 模型主要在 RAM，CPU 算专家（单卡 24 GB）

$ sudo curl -sS -H @/run/knoted/strata-auth-header http://127.0.0.1:15604/v1/models
{"object": "list", "data": [{"id": "qwen3.8-flash-next-iq3_s", …, "meta": {"n_ctx": 32768}, …}]}

$ curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:15604/v1/models   # 不带密钥
401

$ nvidia-smi --query-gpu=index,uuid,memory.used --format=csv
index, uuid, memory.used [MiB]
0, GPU-6bc31459-053f-60ef-0615-3aec9593c9e7, 23695      # Strata：唯一使用中的卡
1, GPU-9e2b6db5-acef-fa85-a28a-7214d1f4c6c0, 1          # 留给 Knoted 的 Python 服务
2, GPU-bf8d2065-6d30-4a83-d9a7-efb41ab26cef, 1          # 不使用

$ nvidia-smi --query-compute-apps=pid,process_name,gpu_uuid,used_memory --format=csv
pid, process_name, gpu_uuid, used_gpu_memory [MiB]
1744156, …/engine/0.1.59-linux-x86_64-cuda-cuda13/strata, GPU-6bc31459-053f-60ef-0615-3aec9593c9e7, 23694 MiB
```

GPU 分配与 §3.1 计划一致（单卡 Strata，第二张闲置给 Python，第三张不用）；本轮复验的 GPU 行输出与上表相同，
只 pid/占用尾数随重启变化（本轮 `23695 MiB` / `23592 MiB`）。

### 8.4 流式调用的真实时间点（host §5，本轮未复跑）

探针 `/tmp/strata_probe.py`（读凭据文件，密钥不进 argv），短 prompt，`max_tokens=64`：

```text
[first sse event] +4.0 ms
[first reasoning_content delta] +501.9 ms 'We'
[first content delta] +819.0 ms '2'
[finish_reason=stop] +939.8 ms usage={'prompt_tokens': 64, 'completion_tokens': 40, 'total_tokens': 104, 'prompt_tokens_details': {'cached_tokens': 0}}
[DONE] +939.9 ms after 40 events
[total] 940 ms, events=40, [DONE]=True, answer_chars=11
[answer] '2 + 2 is 4.'
```

另一次短调用（`max_tokens=24`）把预算全用在思考通道：`finish_reason=length`、`answer_chars=0`、
`[DONE]` 正常到达 —— 与“`delta.reasoning_content` 不算答案”的判断一致（§2.3）。

含义（给 T62）：首字节很快（+4 ms 是服务端 SSE 头，不是首 token），首 token 约 +0.5 s（思考通道先出），
正文首字符约 +0.8 s。客户端不能把“连上”或“收到 reasoning”当作已开始输出正文。

### 8.5 断连取消实测（host §5，本轮未复跑）

客户端读 1.0 s 后主动关连接（`max_tokens=512` 长任务）：

```text
[first sse event] +5.5 ms
[first reasoning_content delta] +524.1 ms 'We'
[client] closing the connection after 1.0s (events so far: 49, [DONE] seen: False)
[total] 1017 ms, events=49, [DONE]=False, answer_chars=0
[after disconnect] activity = {'requests': 3, 'in_flight': 0, 'last_request_at': 1794675124}
[follow-up request after cancel] 364 ms, [DONE]=True
```

引擎日志（host §5，`journalctl -u strata.service`；**本轮未复跑**，先前的 engine log 已随重启轮转）：

```text
[strata] done: 50 tokens in 1 s (96.0 tok/s) (disconnect, cancel=True), expert cache 94.3% hit
[strata] done: 24 tokens in 1 s (71.6 tok/s) (length, cancel=False)
[strata] done: 40 tokens in 1 s (89.2 tok/s) (stop, cancel=False)
```

即：断连后 `in_flight=0`、引擎自判 `cancel=True`、紧接的请求 364 ms 正常完成（若 512 token 仍在生成，
会排在同一槽位后面）。§2.6 的取消契约（不自动重发、不把部分输出标完成、只在确认停止后归还槽位）在真实引擎上成立。

### 8.6 停机语义与 `/run` 凭据重建（host §6，本轮未复跑）

长任务进行中 `systemctl stop strata.service`：引擎随 cgroup 退出（`nvidia-smi --query-compute-apps`
为空）、`port 15604 free`、`is-active=inactive`。客户端在中断时收到的是**错误块 + `[DONE]`**（原样）：

```text
data: {"error": {"type": "server_error", "message": "the engine stopped unexpectedly (exit code -15); the next request restarts it"}}
data: [DONE]
```

→ **对 T61/T62 的硬要求：只看 `[DONE]` 不能判定成功，必须先检查有没有 `error` 块。** 这一条已写进 §3 表。

`/run` 是 tmpfs，因此：

```console
$ sudo rm -f /run/knoted/strata-api-key && sudo systemctl restart knoted-api-key.service
$ sudo ls -l /run/knoted/
-r--r----- 1 root   knoted 64 strata-api-key
$ sudo systemctl restart strata.service   # ExecStartPre / ExecStartPost 均 0/SUCCESS
```

持久副本在 `/var/lib/knoted/strata-api-key`（`root:root 0400`），契约路径由 `knoted-api-key.service`
每次开机重建；轮换 key 需要重启 `knoted-api-key` + `strata` + 客户端。

### 8.7 凭据传递：没有 `--api-key-file`

`strata serve` 的 CLI 没有 api-key 参数（只有 `launch` 有 `--api-key/--api-key-env/--new-key`），
因此 key 只能由 wrapper 从 systemd `LoadCredential` 读入并导出到 `STRATA_API_KEY` 环境变量。
实际密钥串对全部 `/proc/*/cmdline` 扫描命中 0（host §8.2）：

```console
$ sudo sh -c 'KEY=$(cat /var/lib/knoted/strata-api-key); for f in /proc/[0-9]*/cmdline; do
    tr "\0" " " < "$f" 2>/dev/null | grep -qF "$KEY" && echo "KEY FOUND in $f"; done; echo scan-done'
scan-done                    # 没有任何 KEY FOUND
```

run config 里也删掉了 `api_key` 字段，密钥唯一权威是凭据文件（+ systemd credential）。

### 8.8 版本、generation 与回退（host §7）

```console
$ sudo nix-env --list-generations --profile /nix/var/nix/profiles/system | tail -7
 292   2026-11-13 15:38:17
 293   2026-11-14 14:16:51
 294   2026-11-14 15:22:54      ← 本次改动前（不含 strata）
 295   2026-11-15 00:40:47
 296   2026-11-15 00:42:24
 297   2026-11-15 00:45:02
 298   2026-11-15 00:50:28   (current)
```

本轮复验确认 generation 仍为 298（`ae00563` 对应的 system closure
`/nix/store/9fc4hk17mbfiq152z8r1ll5n0p0agy6b-nixos-system-wks-lab-26.11.20261109.5e6fbb3`），服务
`ActiveState=active`、`NRestarts=0`。回退路径（未执行，已确认存在）：

- 回退一步（297，仍带 strata）：`sudo nixos-rebuild switch --flake .#wks-lab --rollback`；
- 回到改动前（294，彻底没有 strata）：
  `sudo nix-env --profile /nix/var/nix/profiles/system --set /nix/var/nix/profiles/system-294-link`
  然后 `/nix/var/nix/profiles/system/bin/switch-to-configuration switch`。
- 两种方式都不动 `/data/llm/strata`（模型/引擎/packs 保留），只是不再有 unit。

## 9. 与 app 契约的偏差（10 条，含人工裁决项）

本节的“偏差”指 **host 实际部署与本文/app 契约（§3、§4）之间的差异**。没有一条被静默改写成契约；
需要裁决的保持原状并在此标注。别名、端口、上下文、GPU 分配与取消契约**一致**：`/v1/models` 的
`id`/响应 `model` 均为 `qwen3.8-flash-next-iq3_s`，监听 `127.0.0.1:15604`，`n_ctx=max-context=32768`，
只用 `GPU-6bc31459-…` 一张卡。

| # | 偏差 | 影响与后果 | 裁决 |
| --- | --- | --- | --- |
| 1 | **承载来源**：host 没有 `imports` `nix/strata-client.nix`，而由 `/etc/nixos/features/server/strata.nix` 自己声明 `strata.service`，跑同一套上游 CLI + 引擎 | 本模块**不是唯一部署来源**，当前定位是“契约 + preflight 参考”；两边同时启用会各自起一个 `strata serve` 抢 `127.0.0.1:15604`，第二个必然失败。模块的 `knoted-strata.service`、tmpfiles、preflight 在本机**未生效** | **需人工裁决**。统一的唯一安全做法：host 在 `features/server/strata.nix` 改为 `imports = [ <knoted 模块路径> ]` 并删除本地重复 unit（或反过来让 Knoted 模块只导出契约常量、不建 unit）。本轮**未统一**，不声称已统一 |
| 2 | **服务单元名**：host 是 `strata.service`；app 契约 §4.7/§7 写 `knoted-strata.service` | 按契约写 `systemctl status knoted-strata` 会直接失败；T61/T62 的 readiness/启动依赖若按契约引用单元名会落空；运维脚本需要知道真实名字 | **需人工裁决**（改 host 单元名或改契约）。本文不改契约文字，只在此登记 |
| 3 | **凭据传递**：`strata serve` 没有 `--api-key-file`（只有 `launch` 有 `--api-key/--api-key-env/--new-key`）；key 只能 systemd `LoadCredential` → wrapper 导出 `STRATA_API_KEY` 环境变量 | 与模块 wrapper 做法一致（模块也是 LoadCredential→env），但与 §3.2 llama.cpp 映射里的 `--api-key-file` 不同。环境变量对同 cgroup/同用户的进程可经 `/proc/<pid>/environ` 读取，比 argv 好但不等于零暴露；实测 `/proc/*/cmdline` 命中 0 | **需人工裁决**（接受 env 传递，或改引擎侧其它凭据机制）；不改契约 |
| 4 | **停机/取消语义**：`systemctl stop` 时在途请求收到 `{"error": …}` + `[DONE]`；引擎日志 `(disconnect, cancel=True)` | 客户端若只按 `[DONE]` 判成功，会把“引擎被杀”当正常完成并写入不完整答案；现有契约 §3 只写了 `[DONE]` | **需人工裁决并回写契约**：`[DONE]` ≠ 成功，必须先检查 `error` 块。本文 §3 已补该行，T61/T62 必须实现 |
| 5 | **投机解码关闭**（`drafting=off`，run config 无 `--mtp`）：`launch` 默认把 MTP 草稿层指向旧安装 `/wrd/opt/Strata-data/mtp/rt`（6.5 GB，2026-10 遗留目录，本任务范围外） | 吞吐下降（约 −20~30% 量级，**未系统测量**）；正确性/别名/端点/上下文不受影响；§5 的 MTP drafter 行因此记为“不适用” | 建议计划确认（是否接受无投机解码），非契约冲突 |
| 6 | **并发 = 1**：`/v1/status` 的 `concurrency.serving=1`（`--drafting off` 下 `launch` 的 fit 结果） | Knoted 单请求串行使用无影响；并发请求会排队（`--max-queue` 默认），T61/T62 不能假设并行吞吐；要并发需在 run config/preset 显式 `--parallel` | 建议计划确认（若计划要求并发） |
| 7 | **版本固定方式**：上游 release installer 不按 commit 固定；实际 Terminal `0.1.59.1` + engine `0.1.59`，产品号 `0.2.13.1` | “按 commit 复现”不成立，只能按版本号 + 摘要复核；engine/tokenizer 的 sha256 未记录（release 做了 checksum/signature 校验） | 记录即可；要 commit 级固定需源码构建（未做） |
| 8 | **NixOS 运行前提**：必须 `LD_LIBRARY_PATH=/run/opengl-driver/lib`、`XDG_DATA_HOME=/data/llm/strata/share`，`XDG_*`/`STRATA_DATA_DIR` 指到 `/data/llm/strata`；`nixos-rebuild switch` 会因 unit 启动失败返回 4 | `nix/strata-client.nix` **没有**设置 `LD_LIBRARY_PATH`/`XDG_DATA_HOME`/`STRATA_DATA_DIR`；host 若要 `imports` 本模块（偏差 1 的统一路径），必须先补这些，否则引擎报“CUDA driver version is insufficient”而驱动并不旧 | 记录 + 统一时必须补；不阻塞当前部署 |
| 9 | **`/run` 凭据需开机重建**：host 加持久副本 `/var/lib/knoted/strata-api-key` + `knoted-api-key.service` 每次开机重建 `/run/knoted/strata-api-key`（`root:knoted 0440`，`Before=strata.service`） | 契约把 `/run/knoted/strata-api-key` 当静态路径；重建顺序或权限不对则 preflight 拒绝启动、客户端 401；轮换 key 需重启 `knoted-api-key` + `strata` + 客户端 | 记录（运维前提），当前部署已满足 |
| 10 | **验证便利文件**：`/run/knoted/strata-auth-header`（`Authorization: Bearer …`，0600）是 host 手工一次性创建，不在 Nix 声明里 | 重启后消失，需要时按 §4 第 6 条重建；不得为了“可复现”在 Nix 里再存一份密钥副本 | 记录；不要把它当成已声明的部署产物 |

## 10. 未执行 / 限制

- **单卡性能未测**：吞吐、首 token 延迟分布、长上下文都只有一次性观察（§8.4），不是性能测试；
  `drafting=off` 的 −20~30% 只是量级估计，未测量。
- **只验证了 Strata 一条承载路径**：§3.2 的 llama.cpp 等价映射仍未安装/未运行，不能把本文的 Strata 证据
  移作 llama.cpp 证据。
- **host 未使用 app 模块**（§9 偏差 1）：模块的 preflight/tmpfiles/单元在本机未生效，§7 的求值与断言只是
  “模块本身可用”，不是“本机由模块部署”。
- **并发/排队行为未测**：`concurrency.serving=1`，没有真实多请求排队、队列超时、`/cancel` 的端到端验证。
- **客户端断连后的服务端重连/续传未测**：只验证了“断连取消 + 后续新请求正常”，没有验证客户端重连后
  会话状态与部分输出落库（属 T61/T62）。
- **流式时间点、断连取消、停机语义由 host 证据 §5/§6 提供，本轮未复跑**：原因是本轮复验只做只读端点与
  模型摘要检查；再做长生成会改动 `activity` 计数与 GPU 状态，不影响结论但会造成证据与 host 采集时点
  混淆。当前引擎日志已随重启轮转，`cancel=True` 行只在 host 证据里可查。
- **mock 预检（§2）与 Nix 模块求值（§7）保持原样**：它们是 app 子任务的实际输出，端口 19095 的 mock 证据
  不写入任何系统配置，仍不能替代真实端点验证。
- **本轮没有修改 `nix/strata-client.nix`**：偏差 1/2/3/8 需要的是契约/统一决定，不是单方面改模块；待人工
  裁决后再改，避免“先改契约再补证据”。
- 未 push、未做任何 GitHub 操作；本文只做本地 docs 提交。
