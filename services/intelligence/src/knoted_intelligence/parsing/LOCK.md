# 解析锁版清单（T10）

本文件是 `knoted_intelligence.parsing` 的唯一事实来源：`models.py` 中的版本、
revision、artifact 相对路径、size、sha256 与 `OPTIONS_REVISION` 都在这里记录了
host 实测依据。**改任何一个值都必须同时更新本节与 `OPTIONS_REVISION`，并按新证据
重跑 `tests/test_t10.py`；不允许在运行时静默改选型。**

## 1. 路径推导

| 项 | 值 |
| --- | --- |
| 模型根 | `KNOTED_INTELLIGENCE_MODELS_DIR`，默认 `/data/knoted-dev/service/models` |
| OCR 模型目录 | `<模型根>/ocr` |
| Docling artifacts 根 | `<模型根>/ocr/docling`（即 `DOCLING_ARTIFACTS_PATH`） |
| 机器可读清单 | `/data/knoted-dev/service/models/ocr/MANIFEST.json` |
| 复现/校验脚本 | `/data/knoted-dev/service/models/ocr/fetch-models.sh`（幂等；sha256/size 不符即失败，不覆盖） |
| 解析时下载 | **禁止**。`parsing/offline.py` 强制 `HF_HUB_OFFLINE=1`、`HF_DATASETS_OFFLINE=1`、`TRANSFORMERS_OFFLINE=1`、`HF_HUB_DISABLE_TELEMETRY=1`；Docling 始终收到显式 `artifacts_path`，缺文件直接 `models_missing`，不会退化为在线下载 |

代码里没有任何 `/home/...`、任务临时目录或 `/nix/store` 路径；Nix store 只出现在
fixture 的生成脚本里作为字体示例。

## 2. 组件版本（host 实测，精确锁定）

| 组件 | 版本 |
| --- | --- |
| `docling` | `2.150.1` |
| `docling-ibm-models` | `4.0.4` |
| `rapidocr` | `3.10.0` |
| `onnxruntime` | `1.32.0`（docling 的基础依赖不含它，必须显式安装；本卡为 CPU 轮子，GPU 不在验收内） |
| `torch` | `2.15.0` |
| `transformers` | `5.23.0` |

## 3. 模型 repo 与 revision

| 角色 | 上游 repo @ revision |
| --- | --- |
| layout（transformers 引擎，Docling 默认） | `docling-project/docling-layout-heron` @ `8f39ad3c0b4c58e9c2d2c84a38465abf757272d8` |
| layout（onnxruntime 变体，本卡未启用） | `docling-project/docling-layout-heron-onnx` @ `40bde044036bb181c130ddf6c51792187268748f` |
| TableFormer（`v2.3.0` tag，**fast** 模式） | `docling-project/docling-models` @ `fc0f2d45e2218ea24bce5045f58a389aed16dc23` |
| RapidOCR det/cls/rec + 字典 | `RapidAI/RapidOCR` @ tag `v3.10.0` |

## 4. 必需 artifact（相对 `<模型根>/ocr/docling`）

运行时只校验**存在性与 size**（便宜）；sha256 由 `fetch-models.sh` 校验，值记录如下。

| 文件 | size (B) | sha256 |
| --- | ---: | --- |
| `docling-project--docling-layout-heron/config.json` | 3,268 | `fdea30805ce2f5666b147fca941dcdd27ad468e27d6ed21902207d3da056a97d` |
| `docling-project--docling-layout-heron/model.safetensors` | 171,658,996 | `00333a43451945aaf89db8ca9c0a17e75d1537c17db60fdb91aa95f4c7929e0c` |
| `docling-project--docling-layout-heron/preprocessor_config.json` | 444 | `cd38cd59999e7a95d68e487fbe5132df3d4e5c32a0836add57e6126ba0c4eaf1` |
| `docling-project--docling-models/model_artifacts/tableformer/fast/tableformer_fast.safetensors` | 145,453,276 | `3119563aab5a7c96fda4d621119b63fd8806272b86c30936d15507616422f718` |
| `docling-project--docling-models/model_artifacts/tableformer/fast/tm_config.json` | 7,060 | `dca6762508dddfae6d57d6cb4ef822c6000119dff0f3b6489db7413118c2622a` |
| `RapidOcr/PP-OCRv6_det_small.onnx` | 9,929,594 | `090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f` |
| `RapidOcr/ch_ppocr_mobile_v2.0_cls_mobile.onnx` | 585,532 | `e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c` |
| `RapidOcr/PP-OCRv6_rec_small.onnx` | 21,234,383 | `6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884` |
| `RapidOcr/ppocrv6_dict.txt` | 74,947 | `b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d` |

上表是运行时**必需**的 9 个文件（`models.REQUIRED_ARTIFACTS`）。host 目录里另有
`docling-project--docling-layout-heron-onnx/*` 与 `tableformer/accurate/*`（合计 14 个），
本卡未启用，不参与存在性校验。`RapidOcr/*.pth`（torch 后端）已从 host 删除，因此不存在
“悄悄回退 torch”的路径。

## 5. 解析行为（全部固定）

| 项 | 值 |
| --- | --- |
| OCR 引擎 | `rapidocr`，后端 `onnxruntime` |
| OCR 语言 | `lang=["ch"]`（PP-OCRv6 识别器中英共用；fixture 的中英混排是证据） |
| OCR 模型规模 | `small` |
| OCR 模式 | `OcrMode.FULL_PAGE`（无文本层的图片/扫描页整页识别） |
| 版面模型 | layout-heron transformers 引擎（本地 `model.safetensors`） |
| 页面栅格比例 | `DOCLING_IMAGES_SCALE=1.0`（72 dpi）；PDF 像素检查按此比例计算 |
| 表格结构 | TableFormer **fast**（`TableFormerMode.FAST`，显式设置；docling 2.150.1 的 pipeline 默认已改为 accurate，本卡不用） |
| enrichment | **全部未启用**：不做图片分类、图片描述、代码/公式增强、chart 抽取。因此**不使用任何 tokenizer**（无 VLM/code-formula 模型），也不加载额外权重；这不是遗漏，是锁定的组件范围 |
| 输出（进程内） | `ParsedDocument.title` + `heading`/`paragraph`/`table` 块（含 `page` 与 `bbox`；纯文本用 `structure=line:N`）+ `full_text` |
| 输出（artifact） | 只保留**一份**文本：`blocks[].text`；不再复制 `full_text`，避免 100 MiB 输入产出约 2 倍体积 |

`bbox` 坐标系：Docling 页面坐标，**原点在左下角**、y 向上，单位是 PDF 点（1/72 英寸；
图片按其栅格坐标）。前端/下游若用左上角原点，绘制前必须翻转 y。

`OPTIONS_REVISION`（worker 提交 `kind=parse` 时必须使用该字符串）：

```text
parse-v1+docling-2.150.1+layout-8f39ad3c0b4c58e9c2d2c84a38465abf757272d8+tableformer-fc0f2d45e2218ea24bce5045f58a389aed16dc23-fast+rapidocr-3.10.0-v3.10.0+ocr-onnxruntime-small-ch+onnxruntime-1.32.0
```

## 6. 准入上限（与 `models.py` 一致）

| 限制 | 值 | 检查时机 |
| --- | --- | --- |
| 文件大小 | 100 MiB | 读取前 `stat()`；超限 `input_too_large` |
| 图片像素 | 40M | 只读图片头（PIL 惰性打开）；超限 `image_too_large`，不解码像素 |
| PDF 页数 | 500 | pdfium 打开后取页数；超限 `too_many_pages` |
| PDF 单页像素 | 40M | 打开后逐页读 MediaBox（pt，按 `DOCLING_IMAGES_SCALE=1.0`）并做同一像素检查；超限 `image_too_large`（消息含 `pdf page N`）。小文件的大 MediaBox 无法绕过上限 |
| 解析产物 | 128 MiB | 子进程序列化 artifact 后按字节检查；超限 `output_too_large`，不写半成品 |

超过上限是明确终态，**不发布截断结果**。900 秒执行上限由执行协议/worker 租约负责，
不在解析器内重复计时。纯文本（UTF-8/UTF-8 BOM/UTF-16 BOM）直接读取；其他编码或无法
识别的二进制输入按 `unsupported_format` 失败，不静默乱码。DOCX/XLSX/PPTX 的提取
（含各自的定位与上限）由 T46/T47/T48 承接，本卡按 `unsupported_format` 明确失败。

## 7. 失败时报告什么

所有解析失败都是 `ParseError`，`code` 取下列之一：

`input_missing`、`empty_input`、`input_too_large`、`image_too_large`、
`too_many_pages`、`unsupported_format`、`corrupt_input`、`models_missing`、
`output_too_large`。

消息**以**锁版组合摘要**开头**（`[locked: …] code: detail`），因此错误本身就是选型报告，
不会变成“换一个模型试试”，也不会被日志/记录的头截断砍掉版本与 revision：

```text
[locked: docling 2.150.1, docling-ibm-models 4.0.4,
 layout docling-project/docling-layout-heron@8f39ad3c0b4c58e9c2d2c84a38465abf757272d8,
 tableformer docling-project/docling-models@fc0f2d45e2218ea24bce5045f58a389aed16dc23 (fast),
 rapidocr 3.10.0 (RapidAI/RapidOCR@v3.10.0; onnxruntime/small/ch),
 onnxruntime 1.32.0, torch 2.15.0, transformers 5.23.0]
corrupt_input: docling could not parse the input (UnidentifiedImageError)
```

`ParseError.public_message` 是唯一允许进入执行记录 `error.message` 的文本（子进程只复制
带 `public_message` 的异常）；docling/PIL 的原始异常文本可能含缓存路径，只写服务端日志。
执行记录的拼接上限是 2000 字符，但摘要位于消息开头，截断不会影响锁版信息。

模型缺失示例（消息只列相对文件名，不泄露部署绝对路径；`models_missing` 正文可能上
千字符，整段保留到 `error.message`）：

```text
[locked: ...]
models_missing: the locked OCR artifacts are incomplete
(RapidOcr/PP-OCRv6_rec_small.onnx: missing); provision them with
.../models/ocr/fetch-models.sh as described in parsing/LOCK.md
```

安装/修复：`/data/knoted-dev/service/models/ocr/fetch-models.sh`（无参数验证现有目录，
传目标目录则按固定 revision 下载并逐文件校验）。换机器必须重跑，模型不进 git，也不进
Nix store。

## 8. 验证记录

- `tests/test_t10.py::test_t10_1_offline_parse_does_not_download`：解析 fixture 前
  monkeypatch `socket.socket.connect` 与 `socket.create_connection` 使其抛错；解析成功
  且 `attempted == []`，同时断言四个离线环境变量与 `DOCLING_ARTIFACTS_PATH`。
- `t10_2`：中文关键词「知识库/合同/笔记/检索/摘要/中文识别」全部保留，英文 `docling`、
  `rapidocr` 与日期 `2026-11-15` 保留；同一 fixture 经执行协议跑完整子进程，产物含
  `parser.options_revision` 与请求的 `options_revision` 两个值。
- `t10_3`：截断 PNG 与二进制垃圾分别得到 `corrupt_input` / `unsupported_format`；直接
  调用与经 `/internal/v1/executions` 的执行记录都以完整锁版摘要开头，记录为终态 `failed`。
  `t10_3b` 把模型目录指向空目录，经执行协议验证上千字符的 `models_missing` 摘要不会被
  记录拼接截断。
- `t10_4`：100 MiB（真实入口 + 稀疏文件）、40M 像素（62 字节合成 PNG 头）、PDF 超大
  MediaBox（≈0.6 KB 的合成 PDF，`pdf page 1`）、500 页边界；不生成大文件。
- `t10_4b`：128 MiB 产物上限；用 32 字节的合成上限走真实 `_compute` 路径得到
  `output_too_large`。
- `t10_1b`：在独立子进程中阻断 socket 并解析 fixture，验证执行子进程口径的离线环境。
- host 证据（安装、离线 `unshare -rn` 转换、逐 artifact 加载）：见 `/etc/nixos`
  `features/server/knoted.nix` 与 T10-host 交付的部署证据。

## 9. 已知边界

- 本卡只验证 CPU 路径；`onnxruntime` 为 CPU 轮子，GPU OCR（index 1）留给部署任务。
- DOCX/XLSX/PPTX 不在本卡范围（按 `unsupported_format` 明确失败），由 T46/T47/T48 承接。
- `options_revision` 的兼容策略：服务端**接受并记录**请求值，同时在
  `parser.options_revision` 写入锁定值；下游必须比较两者，不一致时按新 revision 重新
  解析，不能把旧产物当成当前解析结果。
- 版面 onnx 变体已下载但未启用；启用需要改组件选择并产生新的 `OPTIONS_REVISION`。
- 不改 T09 的账本/状态机语义；`kind=embed` 仍是 T09 占位，等 T11。
