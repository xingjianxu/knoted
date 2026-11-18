# T48 Office 解析实施证据

Issue #133；基线 `5377c581cc7696cc416304b4ae1b758ee0c30f79`。
以下阶段 A 段落为当时的历史记录；整卡主线/dev 交付与清理记录在文末，早期阶段边界保留作历史证据。
阶段 A 当时仅纯解析库与安全测试，T47 未集成，执行注册、Python/Rust
双端执行状态/取消/摘要/过期结果和 dev 验收留在阶段 B；不以 parser 测试替代。

- [x] 读取任务卡、ADR 0001/0003/0006、只读研究与独立阶段 A brief。
- [x] 真实 DOCX/XLSX/PPTX fixture 首轮 RED：33 failed（模块不存在）。
- [x] 首轮 GREEN：33 passed；补充完整性回归 RED：2 failed / 42 passed。
- [x] 补充 GREEN：44 passed。
- [x] Python 完整 pytest + 冻结后全部 T48 / ruff / 格式检查（组合验证，快照区别见下）。
- [x] 阶段 A 安全提交与主 Agent 独立审查。
- [x] 阶段 B、主线集成、dev 验收与清理（最终记录见文末）。

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
`office-ooxml-body-v2-hidden-slides-included-cache-only` 策略身份。
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


## 阶段 A 审查修复轮 1

I1：inline/shared string 改按正文 `t` / rich `r/t` 读取，排除 `rPh` 与
phoneticPr；真实包 plain/rich × inline/shared 四例均先失败再通过。
I2：DOCX 正文不再遍历任意后代，限定允许的段落/run 容器与无正文的属性/锚点。
noBreakHyphen → U+2011，softHyphen → U+00AD，ptab → TAB；字体相关 sym、ruby、
pgNum、contentPart 与未知正文表达明确 unsupported_format，保留普通超链接正文。
输出策略身份升 v2，不让修复前后使用相同解析身份。预算与 B 边界不变。

日志前缀 `r1-i1-red`：4 failed / exit 1；`r1-i1-green`：4 passed / exit 0。
`r1-i2-red`：9 failed、1 passed / exit 1；`r1-i2-green`：10 passed / exit 0。
`r1-final`：60 passed、1 warning / exit 0；`r1-ruff`：All checks passed / exit 0；
`r1-format`：21 files already formatted / exit 0。原有 duplicate ZIP fixture 警告
按 M1 deferred 保存，未消音。既有模型相关源/测试无改动，按主 Agent 指示不重跑
完整模型 suite；早先 102 例运行与本轮 60 例是组合验证，不称为同一快照完整运行。

## Phase B 执行清单（已恢复）

上游主线 `6a53e980fb2c62fd308e3ab85555aca581a81b63`，本分支 merge 基线
`d6618a6beec49bba2f1b574a6a9f48c388b92480`；ADR0007 已接受，T47 已交付。
本阶段由同一实施者串行实现，父 Agent 独立审查、合并及 dev 验收。

- [x] 真实 Python execution Office dispatch/manifest 的 RED→GREEN。
- [x] Rust Office identity、共享有界 artifact 读取与定位校验的 RED→GREEN。
- [x] enqueue/Worker/成功表选择 Office，保留既有退出/租约/来源/deadline 栅栏。
- [x] Python HTTP 成功/错误/取消/短截止/重启、Rust echo/proof/摘要/过期与隔离 PG 管线。
- [x] 冻结代码后完整 Rust fmt/test/clippy、Python pytest/ruff，记录实际计数和 scope。
- [x] 提交、父审查与集成；本实施者不执行远程/dev/真实 SourceRoot 写入。

### Phase B 身份与实际链路

纯 parser policy 保持 `office-ooxml-body-v2-hidden-slides-included-cache-only`，
options 精确值为：

```text
office-ooxml-body-v2-hidden-slides-included-cache-only:66f5d3305d4cda1ad981b12e95d75f4c4f4efcc31e5ca45a8985ead92f65da8a
```

持久 job/file_extractions 的 parser_revision 为该 policy 加 `:docx`、`:xlsx`
或 `:pptx`；expected source format 由此选择，并同时由 Worker 与发布函数核验。
这避免同 source/content 改扩展名时误复用另一格式的成功或耗尽失败预算；备注/
标签不改变此身份。Python 只收到既有 options，不新增路径/格式公开 DTO。
`parse_office(staged=True)` 把受控 object_id 当不透明名字，包身份仍受原准入校验；
纯库默认模式仍拒绝扩展名与包类型不符。成功 artifact 的 parser 对象含原 policy、
options 和完整 budgets，Rust 逐项匹配；PDF/text 身份与监督协议修订不变。

共享 artifact.rs 从原 PDF 路径抽出同一有界读取：受控 output root/regular
file/NOFOLLOW、完整长度和 SHA256、读取前后观察值、manifest metadata 都保留。
Office 另验 source_format、页语义、bbox=null、ordinal、正文长度、truncated=false
及结构位置语法；PPTX 不套 PDF 500 页上限（601 页 ordinal fixture 通过）。
只有完整成功写既有 0015 file_extractions，execution_id 来自真实持久 attempt。
取消/截止/重启继续复用 T47 guardian 与不可变 request、退出证明及最终 PG clock。

真实私有管线：扫描→稳定采集→file.extract→loopback Python→guardian/compute→
manifest→Rust验证→PG成功表。三个实际库生成的 Office 文档成功，损坏和假扩展名
永久失败；再次扫描/只改 metadata 无重开。真实 rename 保持 inode/source_id/
content_version：假 DOCX 命名的 XLSX 改回 .xlsx 可成功；成功 DOCX 改成 .xlsx
创建新格式尝试并失败，不以旧成功跳过。Python HTTP 另覆盖空白/隐藏页 ordinal、
错误预算码、取消后 PID 不存活、短 deadline、正常停机后重启持久终态/新 ID 重试，
sentinel 保持存活。Rust HTTP 对真实 manifest 故意改 SHA/大小/metadata/产物字节
均被拒绝；隔离 PG 的锁等待跨 deadline 回滚成功表与 outbox，旧观察拒绝。

### Phase B 日志

仍位于 `/data/knoted-dev/tasks/t48/logs/`，每条命令含 `.command/.log/.exit`。
`b-dispatch-red` 3 failed；`b-dispatch-green` 3 passed；`b-opaque-red` 3 failed
（明确 object_id 不能作为原扩展名），其后全部定向通过。
`b-rust-red`/`b-rust-red2` 中 3 格式 decoder 为预期失败，另有测试误解既有
ExecutionRecord 字段/verify_fresh 语义，按真实 API 修正 fixture，未改协议。
`b-enqueue-red` 为实际采集后未创建 Office job；`b-rust-core-green` 5 passed。
`b-format-binding-red` 实际错误命名也写成功（4 条而非 3 条）；format selector
加入后不再发布。`b-rename` 实际完整管线与双向扩展改名 1 passed / 68.27 秒。
`b-python-early` 5 passed；之前 `b-http` 1 failed/1 passed 的失败是测试把既有
失败结果 HTTP422 误写成409，已修 fixture，未改 HTTP 状态。
`b-t48-python` 为 uv run 任务卡入口 **65 passed / 无 warnings / exit 0**。
恶意 duplicate ZIP fixture 用局部 pytest.warns 断言预期警告，未全局消音。
最终完整门禁结果将在下段记录；API/pc/Web 无运行变更，不重复其独立工具链门禁。


### Phase B 最终门禁与交接边界

冻结 Office/Rust 产品代码后：`b-final-rust` **393 passed / 1 既有 ignored
API doctest / exit 0**；`b-final-t48-rust` **10 passed / exit 0**；
`b-final-fmt`、`b-final-clippy`、`b-final-ruff`、`b-final-format` 均 exit 0。
全 Python 首轮 `b-final-python` 为 **156 passed / 1 failed / 9 warnings /
exit 1 / 271.87 秒**。唯一失败是既有 T47 guardian fixture 只等待 marker.exists
后读取 PID，命中创建到写入间窗口而读到空字符串（ValueError，line101）。
这是旧 fixture 的发布竞态，不标作 Office 产品 RED，也没有盲目重跑消失后隐去。

主 Agent 明确授权只修该 fixture：孙进程先写 `.tmp` 并关闭，再 os.replace
原子发布；真实孙进程/owner EOF/回收/sentinel 断言保持不变，不改 guardian 或
ledger 产品。修后 `b-guardian-fixture-green` **8 passed / exit 0**（两参数均
覆盖）；`b-final-ruff2`、`b-final-format2` 均 exit 0。主 Agent 接手修正后完整
Python 独立 gate，本实施者按明确指令不再重复模型全套；不得将组合结果宣称
最终 HEAD 已全套 Python 157 passed。首轮 9 warnings 均既有 #156 依赖弃用。

三个实际成功 manifest 在日志目录 `b-office-manifests.json`，均从私有测试
SQLite 只读导出并重新校验 artifact SHA/size；`b-frozen-code-sha256.txt` 保留
Office/Rust 冻结哈希，最终带 fixture 修正的文件清单由完整报告记录。
本实施者未运行 main/dev/发布/真实 SourceRoot；待父独立 review、完整 Python
复验、PR 合入、固定产物及 dev 验收，不宣称整卡生命周期完成。


### 独立审查修复第 2 轮（I1 / I2 / I3）

基于 `586c856`，主 Agent 此前已独立确认完整 Python 157 passed / 9 既有
warnings、Rust 392 顶层 + 1 嵌套子进程 passed / 1 ignored，静态门禁通过。
本轮仅修三项 Important，完整审查与逐项回复在外部实施报告。

- I1：锁定 openpyxl 生成的无 `is` 空 inlineStr 当作空单元格，保留相邻
  正文；`v` / `f` 冲突结构拒绝，避免缓存缺失早退绕过校验。
- I2：在 ZipFile / ZipInfo 全量构造前，以已有有界输入快照遍历实际中央
  目录固定头，跳过变长字段，第 4097 条即 office_budget_exceeded。
  伪造 EOCD 少量声明不能绕过。4096 条含 extra/comment 的真实 DOCX 仍成功。
  ZIP64 中央目录、多磁盘为 unsupported_format；目录长度/计数不一致拒绝。
  不支持自解压前缀或尾部垃圾；不改变 100 MiB、4096 或其他既有预算。
- I3：真实 HTTP helper 保存一次选定的解释器供服务和 Office fixture 共用。
  子进程清除两个可选 env，临时 repo 的默认 .venv 目录链接指向任务管理环境，
  修前真实 NotPresent panic，修后真实三格式成功；临时目录由 TempDir 回收。
- 补卡片要求的 Rust cancelled + compute_exit_confirmed true/false 回归。
  guardian 产品及测试本轮未改；旧 PDF 命名/OCR 注释 Minor 留待后续。

日志根 `/data/knoted-dev/tasks/t48/logs/`；每个 label 有 `.command/.log/.exit`：
`r2-python-red` 5 failed / 1 passed；`r2-python-green` 6 passed；
`r2-default-red` exit101（预期默认环境故障），`r2-default-green` 1 顶层 +
1 子进程 passed；`r2-office-final` **76 passed / 无 warnings / exit0**；
`r2-rust-final`（cargo test --workspace --locked t48_）**12 顶层 + 1 子进程
passed / exit0**，含真实 Worker 管线与发布锁跨截止测试；
`r2-fmt-final`、`r2-clippy-final`、`r2-ruff-final`、`r2-format-final` 全 exit0。
代码/测试冻结后仅更新本证据与报告；父将独立完整 Python 复验同一快照。
本轮不重复完整 Rust/Python，无远程、部署、SourceRoot 或模型宿主变更。


### 独立审查修复第 3 轮（仅测试 I4）

轮 2 I1/I2/I3 已复审解决；主 Agent 在 cf96 已验证完整 Rust 394 顶层 +
2 嵌套 passed / 1 ignored，Python 168 passed / 9 既有 warnings /249.13 秒。
收到门禁结束通知后，本轮只修改 `crates/knoted-core/tests/t48_http.rs`。

新增默认环境回归原先把 KNOTED_TEST_PYTHON 当绝对路径剥父目录，又与服务的
环境变量优先级不一致。冻结二进制在 PATH-only 与冲突 UV 两种环境下均真实
exit101（`r3-path-red` / `r3-priority-red`）。现在服务与回归共享选择函数，
保持 KNOTED > UV > 默认 .venv，由所选解释器只读报告 sys.prefix 作为临时
默认环境链接目标。无手建环境或 site-packages 修改。

`r3-http-final` 完整 HTTP 模块 **3 顶层 + 7 嵌套 passed / exit0 /37.76 秒**：
默认 case、PATH 命令名、UV-only、KNOTED 与无效 UV 冲突均经过真实三格式
HTTP 子测试。`r3-fmt-final`、`r3-clippy-final` 与 diffcheck 全 exit0。
所有命令/原始输出/退出码仍在 `/data/knoted-dev/tasks/t48/logs/`。
Python 全部源/测试和 Rust 产品字节未改，不重复模型或完整 Rust/Worker 管线。
最终提交与完整差异哈希见外部实施报告；继续冻结交父独立复验与 scoped 审查。

## 最终主线与 dev 验收（2026-11-18）

PR [#159](https://github.com/xingjianxu/knoted/pull/159) 普通 merge 至
`c936e0a9b8b4cfc9c8cdee9749fb038f3648cbf3`，dev 同 SHA、schema15。
Issue #133 和继承的 PID fixture 竞态 #166 已由关闭关系自动关闭。
最终修复 R2 `cf96b97`、R3 `1e00917` 及独立定向复审均已完成，
SpecPASS / QualityAPPROVED，I1–I4 全部 ADDRESSED，无 Critical/Important。
旧 PDF 测试名/OCR 注释属于非阻断措辞 Minor；依赖警告继续 #156。

父最终完整产品快照 `cf96b97` 实际门禁：

| 命令/检查 | 实际结果 | 原始证据 |
|---|---|---|
| `cargo test --workspace --locked` | 394顶层 + 2嵌套通过，1既有ignored | `logs/parent-r2-rust.log/.exit` |
| `uv run --locked --group dev pytest -q` | 168 passed，9 warnings，249.13秒 | `logs/parent-r2-python.log/.exit` |
| Office专项 | 76 passed，无warning | `logs/r2-office-final.log/.exit` |
| 最后R3仅HTTP测试助手，父独立HTTP模块 | 3顶层 + 7嵌套通过，37.78秒 | `logs/parent-r3-http.log/.exit` |
| fmt / clippy `-D warnings` / ruff / format | 全exit0，28 Python文件格式符合 | `logs/parent-r2-*`、`parent-r3-*` |
| 主线实际Python导入与 `cargo test --workspace --locked t48_` | 导入主线源码，13顶层 + 7嵌套通过 | `logs/main-import.log`、`main-t48-rust.log/.exit` |
| 主线fmt/clippy/固定联合产物 | 全exit0，38317文件完整清单 | `evidence/main-verification.json` |

R3之后所有Python源/测试和Rust产品字节对已完整验证的cf96不变；
仅Rust HTTP测试助手/证据变化，其独立覆盖如上，不把组合称为另一遍完整suite。
主线树与最终审查HEAD/18文件摘要一致，uv环境已切到主线，未从已清理WT运行。
API/pc/Web行为及依赖锁未改，沿用T47已验证的同字节门禁，不重复无影响模型/平台审计。
T47的产物封装/搬移/CPU/宿主动态库边界仍适用，不宣称跨机器运行闭包。

固定产物清单 SHA256：
`a643efdafbf04b7b9c662f907794a04c62f083362babc44064005b986fc0cf1b`。
部署前已依流程停止自有三进程、确认私有监督clean，并完成PG/assets/staging和
原生SQLite一致性备份：`/data/knoted-dev/service/backups/20261118T023806Z-t48-/`。
实际三进程均从c936e0a固定产物运行，HTTPS/内部健康、CPU/只读模型、loopback、
共享服务身份不变；不重做T47已验证且字节未变的打包器搬移/ELF审计。

真实源目录只使用本次UUID下五份样本，完整初始→unchanged→DOCX正文v2→cleanup：

- 初始generation85：DOCX中文段落和表格、XLSX原工作表/A1/缓存42且隐藏标记不出现、PPTX空白第一页/正文第二页/隐藏第三页均真实提取。DOCX/XLSX页码NULL、所有bboxNULL，artifact SHA/size/格式及compute_exit_confirmed校验通过。
- 损坏DOCX和旧二进制XLSX各永久失败一次，无成功行；generation86未变扫描保持三成功/两失败，不重新获取预算。
- generation87 DOCX正文v2产生第四成功，content_version推进、旧提取行保持；其他格式不重算。
- generation88五源全部missing/fresh=false，自有样本删除；三成功来源共四历史提取、两失败终态保持。

运行日志 `logs/live-acceptance.log/.exit=0` 和八个步骤输出均保存；
`evidence/runtime-{initial,unchanged,changed,cleaned}.json` 保留独立实际检查。
最后只读审计 `evidence/final-runtime-audit.json`：17个持久Python执行全部
request echo/退出证明有效（9成功、5失败、1取消、2中断，含T47历史），
健康active0、文件任务active0、staging无.part，完整38317文件运行后仍匹配清单。
全局来源0present/21missing、成功提取13、预期失败工作7、既有Passkey1。
正常移除本任务WT与已合并本地/远程分支；11个忽略文件均pytest/pyc可重建缓存，
未知忽略文件0，其他任务WT保留，见 `evidence/worktree-cleanup.json`。

上述本地原始证据均位于 `/data/knoted-dev/tasks/t48/`，不含会话凭据或私有owner令牌。
本文支持范围仍为受界ZIP32 OOXML正文子集；预算与特殊容器可能拒绝合法文档，
缓存可陈旧，未执行公式或Office。文件浏览/预览由T49/T50继续，检索/RAG未开放。
