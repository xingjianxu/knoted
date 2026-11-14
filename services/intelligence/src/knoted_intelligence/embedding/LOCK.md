# BGE-M3 dense 嵌入锁版清单（T11）

本文件与 `bge.py` 的常量是同一份锁的两个视图：模型 revision、逐文件 size/sha256、
tokenizer revision、推理依赖版本、`model_revision` 配置键、GPU UUID 选择方式和
`OPTIONS_REVISION` 都在这里记录 host 实测依据。**改任何一个值都必须同时更新 `bge.py`、
`bge.py` 中的 `OPTIONS_REVISION` 和本文件，并重跑 `tests/test_t11.py`；运行时不得换模型**
（换模型 = 新的 embedding_set，旧向量空间不可混用）。

## 1. 路径与配置

| 项 | 值 |
| --- | --- |
| 模型根 | `KNOTED_INTELLIGENCE_MODELS_DIR`，默认 `/data/knoted-dev/service/models` |
| 快照目录 | `<模型根>/bge-m3`（`SentenceTransformer(<dir>)` 直接加载，无 `.git`、无 ONNX） |
| 机器可读清单 | `<快照>/MANIFEST.json`，`schema: knoted-bge-m3-models/1` |
| 校验/重建脚本 | `<快照>/fetch-model.sh`（`--check` 只校验；sha256/size 不符即失败，不覆盖） |
| 运行时下载 | **禁止**。`bge.py` 在导入 `sentence_transformers`/`torch` 前设置 `HF_HUB_OFFLINE=1`、`HF_DATASETS_OFFLINE=1`、`TRANSFORMERS_OFFLINE=1`、`HF_HUB_DISABLE_TELEMETRY=1`，并显式传入本地路径；缺文件即 `models_missing`，不回退 hub（快照里也没有 `pytorch_model.bin`） |
| 模型 revision 配置键 | `model_revision`，固定为 `BAAI/bge-m3@5617a9f61b028005a4858fdac845db406aefb181`；本地快照没有 `.git`，该 revision 只能来自本文件/`MANIFEST.json`/`bge.MODEL_REVISION`，三者由 `bge.manifest()` 交叉校验，不一致即 `manifest_mismatch` |
| tokenizer revision | 与模型同一 commit：`5617a9f61b028005a4858fdac845db406aefb181`（tokenizer 文件在本快照内，不二次下载） |
| GPU 选择 | `KNOTED_EMBED_GPU_UUID` 必须是 `nvidia-smi -L` 的 UUID（形如 `GPU-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`），由 `bge.py` 在导入 torch 前复制到 `CUDA_VISIBLE_DEVICES`；**不设默认 0/1**，未设置时用 CPU，非法 UUID 或配置了 GPU 但 torch 看不到卡都是明确错误（`invalid_gpu_uuid` / `cuda_unavailable`），不静默降级 |
| NixOS CUDA 前置 | `LD_LIBRARY_PATH=/run/opengl-driver/lib`（否则 torch 看不到卡；host 证据 §6/§7） |
| 共享 GPU 槽 | `KNOTED_INTELLIGENCE_GPU_SLOTS`，默认 `1`。`kind=parse` 与 `kind=embed` 都在子进程启动前获取该槽，OCR 与嵌入不会同时驻留；`max_slots` 仍限制全部并发，`gpu_slots` 只限制 GPU 类执行 |

部署主机上 Python 侧（OCR + 嵌入串行）使用 `GPU-9e2b6db5-acef-fa85-a28a-7214d1f4c6c0`
（index 1）；index 0 的 `GPU-6bc31459-…` 由 Strata 独占。UUID 由 host 侧
`features.server.knoted.embedGpuUuid` 声明，Knoted 代码不硬编码 index。

## 2. 模型与 tokenizer（`BAAI/bge-m3` @ `5617a9f61b028005a4858fdac845db406aefb181`）

快照 = 13 个文件 / `2,295,354,503` B。上游只发布 `pytorch_model.bin`；本地
`model.safetensors` 由 host 无损转换（391 tensors、fp32、逐 tensor 摘要一致），
`fetch-model.sh` 用纯标准库重算 tensor 摘要。

| 文件（相对 `<快照>/`） | size (B) | sha256 |
| --- | ---: | --- |
| `1_Pooling/config.json` | 191 | `e54c164a07274f2eb45bb724f54a79d1efcc90c41573887cd9a29aeee0597352` |
| `README.md` | 15,822 | `0b81ccf9134e5874d620a86e6905062ea999e779c34eb1a7e65eaeb7fe00e450` |
| `colbert_linear.pt` | 2,100,674 | `19bfbae397c2b7524158c919d0e9b19393c5639d098f0a66932c91ed8f5f9abb` |
| `config.json` | 687 | `26159e7ad065073448460117eb24b7a4572f6f4e78eadff65dc0a11c052449fa` |
| `config_sentence_transformers.json` | 123 | `1eef72430e7194a1e59680e635aed81ffa083f05668dbc5bb1c56c04c0999c38` |
| `model.safetensors` | 2,271,064,520 | `5632ea99fd6c2dc9c1d0837660b581ccbff68b0eca17f95d86373e16385d2f69` |
| `modules.json` | 349 | `84e40c8e006c9b1d6c122e02cba9b02458120b5fb0c87b746c41e0207cf642cf` |
| `sentence_bert_config.json` | 54 | `eb9b44b13c0f52a3b3685c3b1cbdea1ba8b04bea123b98f61610048940776eb1` |
| `sentencepiece.bpe.model` | 5,069,051 | `cfc8146abe2a0488e9e2a0c56de7952f7c11ab059eca145a0a727afce0db2865` |
| `sparse_linear.pt` | 3,516 | `45c93804d2142b8f6d7ec6914ae23a1eee9c6a1d27d83d908a20d2afb3595ad9` |
| `special_tokens_map.json` | 964 | `8c785abebea9ae3257b61681b4e6fd8365ceafde980c21970d001e834cf10835` |
| `tokenizer.json` | 17,098,108 | `21106b6d7dab2952c1d496fb21d5dc9db75c28ed361a05f5020bbba27810dd08` |
| `tokenizer_config.json` | 444 | `a62b2b6784f990259fddef5f16388693a8043be4f69179e6a5257eeb3f9abac4` |

运行时**必需**的是 dense 路径加载的 10 个文件（`bge.REQUIRED_FILES`）；每次进程加载模型前
逐文件校验存在性、size **与 sha256**（`artifact_problems()`，约 2.3 GiB，一次进程一次）：`config.json`、`config_sentence_transformers.json`、`sentence_bert_config.json`、
`modules.json`、`1_Pooling/config.json`、`model.safetensors`、`tokenizer.json`、
`tokenizer_config.json`、`special_tokens_map.json`、`sentencepiece.bpe.model`。

`README.md`、`sparse_linear.pt`、`colbert_linear.pt` 计入快照清单（sha256 固定，由
`MANIFEST.json`/本文件/常量三方交叉核对，运行时不读取、不哈希），但**不是运行必需**：
BGE-M3 的 sparse 与 ColBERT 头在本卡不使用也不加载，只有 dense
（`normalize_embeddings=True`）路径。启用其它头属于新契约，必须新 revision 与新
`OPTIONS_REVISION`。

## 3. 推理依赖（与 `pyproject.toml` / `uv.lock` 一致）

| 组件 | 版本 |
| --- | --- |
| `sentence-transformers` | `6.3.0` |
| `transformers` | `5.23.0` |
| `torch` | `2.15.0`（PyPI/TUNA wheel 实际为 `2.15.0+cu132`，CUDA 13.2） |
| `safetensors` | `0.8.0` |
| Python | `3.12` |

`t11_3` 用 `importlib.metadata.version` 读取实际安装版本并与这些常量比较，因此 `uv.lock`
漂移会让测试失败，而不是悄悄换掉推理栈。

## 4. 嵌入契约（全部固定）

| 项 | 值 |
| --- | --- |
| 模型 | `BAAI/bge-m3`，dense 头 |
| 维度 | `hidden_size = 1024`（`bge.HIDDEN_SIZE`） |
| dtype | `float32`（`bge.OUTPUT_DTYPE`） |
| 归一化 | `normalize_embeddings=True`，每行 L2 范数 `1.0 ± 1e-4`（`NORMALIZATION_TOLERANCE`），实测偏差 ~1.2e-7 |
| 最大序列 | `max_seq_length = 8192` token（上游 `max_position_embeddings=8194` = 8192 + 2 个 XLMRoberta 偏移位） |
| 单次批上限 | `MAX_BATCH_SIZE = 64` 条文本（超出 `batch_too_large`，不排入无界队列） |
| 输入对象上限 | `MAX_INPUT_BYTES = 8 MiB`（超出 `input_too_large`，解码前拒绝） |
| 空批次 | `embed_dense([]) == []`，不加载模型 |
| 空白文本 | 空串/纯空白 → `empty_text`（不产生无意义向量） |
| 超长文本 | token 数 > 8192 → `text_too_long`；**不静默截断**（截断后的向量代表不同内容，却会复用同一 revision） |

`OPTIONS_REVISION`（worker 提交 `kind=embed` 时应使用该字符串）：

```text
embed-v1+bge-m3-5617a9f61b028005a4858fdac845db406aefb181-dense-1024
```

## 5. `kind=embed` 的执行产物

`bge.py` 只负责 `embed_dense(texts)`；执行协议由 T09 的账本/状态机承载，`_compute`
把结果写成 JSON artifact：

```json
{
  "kind": "embed",
  "source_version": 1,
  "input_sha256": "…",
  "options_revision": "embed-v1+…",
  "model": "BAAI/bge-m3",
  "revision": "5617a9f61b028005a4858fdac845db406aefb181",
  "dims": 1024,
  "normalized": true,
  "dtype": "float32",
  "device": "cuda",
  "lock": {"model_revision": "…", "tokenizer_revision": "…", "hidden_size": 1024, "…": "…"},
  "text_count": 4,
  "vectors": [[…1024 个 float…], "…"]
}
```

`lock` 是 `bge.lock_summary()` 的完整快照（repo/revision/tokenizer/维度/dtype/归一化/
max_seq_length/runtime 版本/`gpu_uuid_env`/`options_revision`），消费方必须比较它和当前
锁，不能把旧 revision 的向量当新空间。

**输入对象格式（T11 临时约定）**：UTF-8 文本（可带 BOM），**每个非空行是一条文本**，
行首尾空白去掉，空行仅作分隔；超过 64 行或 8 MiB、非 UTF-8、没有非空行分别是
`batch_too_large` / `input_too_large` / `invalid_input` / `empty_input`。分块与批处理契约
由 T58 的 `embedding/service.py` 最终确定；本卡只保证单批 dense 语义，不把该行格式当作
长期 API。

## 6. 失败码（`EmbeddingError.code`，稳定）

`models_missing`、`manifest_mismatch`、`invalid_gpu_uuid`、`cuda_unavailable`、
`model_load_failed`、`invalid_input`、`empty_input`、`empty_text`、`batch_too_large`、
`input_too_large`、`text_too_long`、`invalid_output`、`invalid_dimension`、
`output_count_mismatch`、`non_finite_output`、`not_normalized`。

消息以 `[locked: …] code: detail` 开头（锁摘要在前，截断也不会丢 revision/版本），
`EmbeddingError.public_message` 是唯一允许进入执行记录 `error.message` 的文本；加载/导入
异常的原始文本（可能含路径）只写服务端日志。

## 7. 验证记录

- `t11_1`：真实模型、真实 GPU/CPU 路径；4 条中英样本 → `[4][1024]` float32，
  每行 L2 范数 `1.0 ± 1e-4`；同一文本两次逐元素差 ≤ `1e-5`；相近中文句余弦大于
  无关英文句（只做语义冒烟，不断言具体数值）。
- `t11_1b`：经 T09 执行协议跑完整子进程（`kind=embed`），artifact 的 `revision`、
  `lock.model_revision`、`options_revision`、`device`、`vectors` 与直接调用一致，
  artifact sha256/size 由测试重算。
- `t11_2`：向 `validate_embeddings` 注入**伪造输出**（NaN/±Inf/1023/1025 维、
  行数不符、未归一化、bool/字符串元素、float64）分别得到稳定错误码；空批次不加载模型；
  空白文本在加载前拒绝；8193 token 用 fake tokenizer 触发 `text_too_long` 且
  `encode` 从未被调用；8192 token 边界通过。模型本身没有被伪造。
- `t11_3`：`LOCK.md`、`MANIFEST.json`、`bge.MODEL_REVISION`、`lock_summary()` 与
  实际加载契约（`hidden_size`/`max_seq_length`/`device`）一致；13 个文件的 size/sha256
  三方一致，10 个必需文件的磁盘摘要由 `artifact_problems()` 真实重算；安装的 4 个库版本
  等于锁定值；UUID 环境变量的映射被实际检查。
- `t11_3b`：把必需文件软链到临时目录后篡改同尺寸的 `config.json`，
  `artifact_problems()` 返回 `sha256 ... does not match the lock`，证明运行期摘要校验有效。
- `t11_6`：经执行协议提交空对象、65 行、8 MiB+1（稀疏文件）与非 UTF-8 的 embed 输入，
  均在不加载模型的情况下得到终态 `failed`，消息含 `empty_input`/`batch_too_large`/
  `input_too_large`/`invalid_input`。
- `t11_5`：`max_slots=2` 下同时提交一个 parse 与一个 embed，两者在账本里的
  `started_at..finished_at` 区间不重叠（`gpu_slots` 默认 1）；两个请求都在对方结束前
  被接受，因此重叠是可观测的而不是理论上的。修复前该用例实测重叠（同机复跑）。
- `t11_4`：新解释器在最顶部封锁 `socket.socket.connect`/`socket.create_connection`，
  `HF_HOME`/`SENTENCE_TRANSFORMERS_HOME` 指向空目录，仍从本地快照加载并输出
  2×1024 归一化向量；事后 hub cache 目录为空（没有偷偷回填）。
- 命令：`cd services/intelligence && uv run --group dev pytest tests/test_t09.py
  tests/test_t10.py tests/test_t11.py -q` 与 `uv run --group dev ruff check src tests`。

## 8. 已知边界

- 不做性能/吞吐/显存/召回率基准；t11 只验证可运行性与契约。
- 不做 sparse / ColBERT；快照里的两个头未加载。
- 输入行格式是 T11 临时约定，T58 负责 `ChunkSet → EmbeddingSet` 的最终批处理契约。
- 模型不参与备份/迁移、不进 Nix store；换机器或删目录后重跑 `fetch-model.sh --check`。
- 本卡不改 T09 的账本/状态机语义，只把 `kind=embed` 的占位计算替换为真实 dense 模型。
- OCR 设备由 `KNOTED_INTELLIGENCE_DOCLING_DEVICE` 显式控制（默认 `cpu`，见
  `parsing/LOCK.md`）；嵌入仍按 `KNOTED_EMBED_GPU_UUID` 选卡，两者共用 `gpu_slots`。
- `CUDA_VISIBLE_DEVICES` 由本模块按 UUID 覆盖设置；同一进程内没有第二张卡的并发嵌入，
  GPU 槽位串行由调度器/T58 决定。
