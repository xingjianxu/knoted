# T48 Office 解析阶段 A

Issue #133；基线 `5377c581cc7696cc416304b4ae1b758ee0c30f79`。
仅纯解析库与安全测试，整卡 **未完成**。T47 未集成，执行注册、Python/Rust
双端执行状态/取消/摘要/过期结果和 dev 验收留在阶段 B；不以 parser 测试替代。

- [x] 读取任务卡、ADR 0001/0003/0006、只读研究与独立阶段 A brief。
- [x] 真实 DOCX/XLSX/PPTX fixture 首轮 RED：33 failed（模块不存在）。
- [x] 首轮 GREEN：33 passed；补充完整性回归 RED：2 failed / 42 passed。
- [x] 补充 GREEN：44 passed。
- [x] Python 完整 pytest + 冻结后全部 T48 / ruff / 格式检查（组合验证，快照区别见下）。
- [ ] 安全提交与主 Agent 独立审查。
- [ ] 阶段 B、主线集成、dev 验收与清理。

## 实现选择与支持范围

使用锁定 `defusedxml 0.7.1` 直接读取受界 OOXML，复用既有 ParsedDocument /
ParsedBlock（仅内部 source_format Literal 扩展）。锁定 python-docx 1.2.0、
openpyxl 3.1.5、python-pptx 1.0.2 生成真实 fixtures，不增加或升级依赖。
直接 XML 读取避免 openpyxl 的稀疏维度矩形遍历，以及只读模式缺少隐藏行列信息；
代价是明确仅支持 transitional OOXML 正文文本子集，不承诺完整 Office 渲染。

DOCX 正文段落与表格按原顺序；横纵合并仅发出主单元格，嵌套表保留结构路径。
没有可靠渲染页码，因此 page/page_count/bbox 为 null。正文修订、移动修订、
altChunk、内容控件、文本框、域、脚注/尾注引用、替代内容拒绝；页眉页脚、样式、
图片及图像 OCR 不在正文提取范围。

XLSX 原 sheet 名与 A1 保留；只收 visible sheet / 非隐藏行列的实际稀疏 cell；
原合并区域附在主格上。不使用 dimension 生成矩形，无 worksheet 伪页码。
公式只发出已有缓存（标记 formula_cache），缺失缓存省略而非求值或变零，错误
缓存保留 Excel 错误文本；数值/日期为 OOXML 存储文本，不执行格式化或重新计算，
缓存新鲜度无保证。包含 `[`/`]`/`|` 的公式（含外部引用及结构化表引用）保守拒绝；
外部 workbook 关系拒绝。绘图、图表、批注与打印布局不在单元格提取范围。

PPTX 按原 1-based slide ordinal（含空白页和隐藏页，不重新编号），shape 按 XML
z-order，支持组递归、文本框与表格。不是视觉阅读顺序，不把 EMU 当 PDF bbox。
备注、母版/布局继承文字、图片、连线、动画不在 slide 自有文本提取范围；图表、
SmartArt/其他 graphic frame、AlternateContent、宏/嵌入对象拒绝。

只保留外部超链接的显示文本；不请求关系目标。不执行宏/公式/外链/Office/
嵌入对象。旧二进制、OLE 加密、宏启用、扩展名和包身份不符明确拒绝。

## 内部安全预算

以下是 controller implementation choices，不是用户新增规格。保留输入 100 MiB；
900 秒 deadline/cancel 必须由后续执行 supervisor 落实，纯函数不声称提供硬截止。

| 维度 | 当前内部上限 | 依据 |
| --- | --- | --- |
| ZIP members | 4096 | 限制目录与关系元数据开销 |
| 单 part 解压 | 32 MiB | 限制 XML/媒体单项分配 |
| 总解压 | 128 MiB | 避免小输入无限膨胀，不直接将输入限额当展开限额 |
| 单 part 压缩比 | 200 | 提前拒绝小压缩炸弹，正常 fixture 均满足 |
| XML 总节点 | 500,000 | 给所有 XML 树的对象开销设上界 |
| XML 深度 | 64 | 限制递归结构复杂度 |
| XML 文本 / 输出文本 UTF-8 | 各 16 MiB | 限制文本对象及结果，不发布截断 |
| 实际 cells | 100,000 | 与稀疏声明维度无关，隐藏格也计入 |
| 输出 blocks | 100,000 | 限制结构产物的对象数 |

`OfficeLimits` 常量通过 `options_revision()` 的 SHA256 绑定到
`office-ooxml-body-v1-hidden-slides-included-cache-only` 策略身份。
实际完整 64 位摘要由运行产物记录；预算改变身份也改变。
这些限额保守，可能拒绝有效大文档；后续调整须更换身份并重新验收。

ZIP 中央目录和实际读取均计数，不 extractall；全部 part 验 CRC、成员名与重复；
所有 XML 使用 defusedxml 禁止 DTD/entities/external，限制深度/节点/文本。
关系仅在包内解析，合法 `/xl/...` 是包绝对 URI，不是宿主文件路径；不存在目标、
越界、反斜杠、编码路径等拒绝。错误只含固定安全消息，不暴露路径或输入内容。
`corrupt_input`、`unsupported_format`、`input_too_large`、
`office_budget_exceeded` 是永久拒绝；`office_io_error` 独立保留 IO 故障，
未将 OSError 当损坏，MemoryError 不吞。执行端映射留 B，不从展示字符串解析。

## 验证记录

运行日志目录 `/data/knoted-dev/tasks/t48/logs/`，每次保留 `.command/.log/.exit`。
完整报告 `/data/knoted-dev/tasks/t39-t50/reports/t48-implementation.md`。
测试 temp/cache/state/staging 仅 `/data/knoted-dev/tasks/t48/`；共享模型只读，
Docling 显式 CPU，BGE GPU UUID 为空且 CUDA_VISIBLE_DEVICES 为空，复用已有锁定环境。

已运行 baseline：`tests/test_t09.py -q` → 24 passed / exit 0；
RED `red` → 33 failed / exit 1；`green2` → 33 passed / exit 0；
补充 `red2` → 2 failed / 42 passed / exit 1（缺 sheetData、DOCX drawing 文本遗漏）；
`green3` → 44 passed / exit 0。`red3` → 1 failed / 45 passed / exit 1
（sheet 名长度未设界），冻结后 `pure-final` 与 `uv-pure` 均 46 passed / exit 0。

完整 `full` 运行：**102 passed / 8 warnings / exit 0 / 171.81 秒**。
该运行启动时收集 **44 个 T48 用例**；运行期间仅 office.py/test_t48.py 增加
sheet 名长度边界与隐藏列预处理，其他既有 Python 源/测试未改（Literal 扩展在
该运行前已有）。因此采用该 102 例运行 + 冻结后全部 46 个 T48 用例的组合验证，
**不声称完整运行已收集后来新增的两例**，不重复无影响的模型运行。
冻结源码 SHA256 见日志目录 `frozen-sha256.txt`；源差异 `source-diff.log`。

`ruff-final` → All checks passed / exit 0；`format-final` → 21 files already
formatted / exit 0；`scope-check` → git diff --check / exit 0。README 证据链接已解析。
完整运行的 8 warnings：Starlette httpx 1、force_full_page_ocr 4、Docling 表格
图像字段 1、RapidOCR rec_font_path 1，以及恶意 ZIP fixture 重复成员 1。
既有弃用警告由主 Agent 关联 #156，不消音或升级锁版。

执行命令保存在各 `.command`：完整运行复用既有 `.venv/bin/python -m pytest -q`；
任务卡入口使用 `UV_PROJECT_ENVIRONMENT=<existing .venv> uv run --no-sync pytest
 tests/test_t48.py -q`，仅为禁止重装而增加 --no-sync。两者均显式指定任务内
pytest cache/basetemp。未运行 Rust t48_（没有真实 B 链，不能用空测试代替）。
