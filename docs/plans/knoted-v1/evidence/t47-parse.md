# T47 实施记录

**完成：** PR [#157](https://github.com/xingjianxu/knoted/pull/157) 普通 merge，主线与 dev `0f6552936f3ae26fdb3fa184251a550036bb188b`，schema 15。产品冻结/修复树 `9566bb2` 与合并树完全一致，已完成独立门禁、全卡审查及 R1 复审、主线验证、固定联合产物、dev 验收和仅本卡 Worktree/已合并分支清理。T48—T50 与协调 Worktree 保留，整个阶段尚未完成。

## 最终验证

| 实际范围 | 结果 | 原始证据（`/data/knoted-dev/tasks/t47/logs/`） |
|---|---|---|
| 父最终完整 Rust workspace | 382 顶层 passed + 1 额外子进程 passed；0 failed；1 既有文档示例 ignored | `parent-r1-cargo-test.log`；fmt/clippy 均0 |
| 父最终完整 Python | 92 passed, 9 warnings in 167.01s；ruff check/format0 | `parent-r1-pytest.log`、`parent-r1-ruff-*`；9条均既有依赖弃用 #156，未过滤/升级 |
| API / pc | 83 operations, 29 examples (14 invalid rejected)；verify0；pc36passed、shell0 | `parent-api-*`、`parent-pc-*`；这些字节在R1未变 |
| 独立审查 | 全卡发现4项Important，R1全部ADDR，新增Critical/Important0、未关闭0 | 外部 `reviews/t47-b-{review,r1-review}.md` |
| 主线集成 | Rust t47_13、Python T47 34/4既有warnings、fmt/clippy、pc36/API均0；真实Python导入来自主检出，51个产品/契约/配置/测试摘要不变 | `main-verify-runner.log`、`main-*` |
| 联合产物 | 官方builder0；38,316普通文件、6,455,727,726 bytes、470 ELF、0links/pyc；封写和整体搬移后真实OCR5blocks、3进程同固定exe/-I-B、prooftrue/停后CLIclean；运行后全清单精确一致 | `main-artifact-*`、`parent-main-elf-dependencies.log` |

最终联合产物位于 `/data/knoted-dev/runtime/0f6552936f3ae26fdb3fa184251a550036bb188b`，清单本身 SHA256 `aa20b0572ea32d2faf4876974613d5df23e35627ea7885e06e1e1bd2208681c7`。搬移验证时原路径不存在；验证后恢复完整SHA目录再激活。解释器、依赖/源码封装与下面历史产物严格区分，不把 b27c388 的证明冒充最终修复树。

## 真实 dev 与清理

停止本实例旧 API/Worker并确认退出、关闭旧编排后，一致备份 PG/assets/staging：`/data/knoted-dev/service/backups/20261118T010551Z-t47-`。e10旧版本没有Python产物、有效state路径/15603 listener/受管进程均不存在，首次启用缺库态已单独确认；没有伪造旧SQLite备份。之后原子切换 current 并启动同SHA三进程，核对固定exe/argv、CPU与两个只读模型根、loopback监听、HTTPS/内部健康、迁移成功和共享 PG/Caddy/Strata MainPID不变。

- 七个自有来源实际自动 scan→capture→Python→成功表：PDF/PNG/JPEG/WebP四种成功且中文关键词/页/bbox有效；损坏PDF、501页、40M+1像素明确永久失败，没有成功行或部分结果。
- unchanged扫描不新增提取或重试Failed；自有PNG变更到content_version2，新增成功并保留旧历史。清理后七来源均missing/freshfalse；本卡成功提取5行、预期失败3项保持稳定。
- 活动计算期间实际 `pc process restart knoted-intelligence`：原执行cancelled/prooftrue，receipt原因owner_eof，后代消失，同原请求重放稳定；API/Worker原starttime及共享服务保持。
- 活动计算期间通过pidfd仅SIGKILL已验证的当前intelligence owner：自动重启后原执行interrupted/prooftrue、receipt原因owner_eof，后代消失，同原请求重放稳定。两种模式均验证完整回显后才移除自有staging输入。
- 最终11个Python执行均重新通过GET验证回显/退出证明：成功5、边界失败3、取消1、owner崩溃中断2。第二个中断来自下述首次harness断连诊断，历史保留。health=ok/active0，staging无自有direct输入、无.part；全实例0present/16missing来源、9成功提取、5预期失败提取job（T46既有2+本卡3），活跃文件job0，原Passkey仍1。
- 实际运行后38,316文件摘要再次一致；正常移除仅 `.worktrees/knoted-t47` / `feat/t47`，无未知忽略成果（只依赖及Python/lint缓存）。其他Worktree保留，原始日志/报告/历史产物在 `/data` 留存。

## 诊断与适用边界

初次部署脚本在HTTP已ready、Worker仍等process-compose健康依赖的短窗口提前检查，返回1；实际Worker随后正常启动。修正等待三个进程和prepare/migrate完成后，仅恢复读回核验而未重复部署，结果0。首次SIGKILL已正确触发owner EOF及恢复，但harness遗漏直接ConnectionResetError重试而提前退出；保留失败记录，核对原记录/receipt/后代后只清理自有输入，新增聚焦重试检查，并用新自有执行完整复测0。最终只读统计初次写错Passkey表名，按0003改为passkey_credentials后复核0；均为验收脚本问题，没有改产品来隐藏失败。

私有部署与生命周期脚本有独立静态审查；修复有效配置绑定、关闭/备份边界、优化模式显式门禁、动作前owner/listener/GO身份、echo后清理及因果receipt采集后无阻断项。源目录仅增删Agent自有UUID样本，未操作原件/共享服务/模型/生产。500页边界由真实PDF准入测试覆盖，短deadline实际收束覆盖超时机制，未假称在dev做完整500页OCR或等待满900秒。宿主loader/glibc/C++/zlib依赖仍存在；未验证GPU/MPI/_crypt或跨机器闭包；Office/浏览/预览和索引/RAG不在本卡完成范围。

## 阶段A/B历史与修复过程

下面内容是各阶段当时交接快照，其中“待父验证/集成”已由上面的最终记录结清。

基线 `5377c581cc7696cc416304b4ae1b758ee0c30f79`，Issue #132，隔离分支 `feat/t47`。
Phase A 代码与实际构建 revision：`7ef1524edb5c0247a8c95583b04215d4a5549693`。

以下为 Phase A 历史记录。2026-11-18 用户已批准退出证明补充 RFC，Phase B 按 ADR0007 实施，当前进展见文末。

- [x] 核对任务、ADR 0001/0003/0006 与三个运行准备报告。
- [x] 专用白名单入口；14 个定向用例含真实 500/501 页 PDF、40M 像素边界、100MiB 拒绝与中文 OCR，exit 0。
- [x] 从固定 SHA 构建 Python 产物，整体搬移后 `-I -B` 父子导入验证通过。
- [x] Python 完整 pytest 72 passed/8 warnings；ruff check/format、pc 34 passed、shell 语法均 exit 0。
- [x] 无计算健康检查：临时 loopback 实例 health=ok，active_executions=0；退出后清单不变。
- [x] 代码安全提交；保留任务分支与 Worktree。
- [ ] 主 Agent 独立审查与 Phase B 后续集成。

产物位于 `/data/knoted-dev/tasks/t47/build/artifact-relocated`，原始绝对路径已整体搬走。
共 38,239 个普通文件、6,422,035,511 字节、468 个 ELF；无 symlink、无 pyc，运行后
全量文件集合和摘要与 `SHA256SUMS` 精确一致。解释器 Python 3.12.15，实际 executable
SHA256 `9b6fd06e487935c92477cb47cabbda8d814cda44823a5159eddf791178afe7d0`；
uv.lock SHA256 `3f66a80110a7ea18390eb216556331600b66f4578d4f74739d197970fad53cbe`。
解释器/stdlib/Docling/Torch/ONNX Runtime/应用路径在搬移后的产物内，未依赖开发 venv 或 home 源码。

产物仍依赖宿主 loader/glibc/libstdc++/zlib；逐 ELF ldd 有191个文件显示未解析项，不能统称为 optional；实际 CPU 所需 C++/zlib 已通过导入与 maps 确认。GPU/MPI/_crypt 等未运行分支和跨主机移植没有可用性承诺。
无计算健康 harness 初次错误要求退出码 0；核实锁定 Uvicorn 在正常 shutdown 后重发
SIGTERM 后，改为验证 -15、shutdown 完成日志及 PID 消失，复测 exit 0。未修改产品退出协议。

Phase A 时 `t47_2`、退出证明、Rust 双端协议和真实 compute 尚未实施；Phase B 的本地证据如下。
主线集成、联合产物、dev 和清理仍由主 Agent 验收，本文不预填 main/dev SHA。

原始日志与逐项 exit：`/data/knoted-dev/tasks/t47/logs/`。完整报告：
`/data/knoted-dev/tasks/t39-t50/reports/t47-implementation.md`。

## Phase B 本地交接

源码冻结提交 `b27c38888c223f43cb9c5eda089f60e34ef435ac`。实现覆盖严格双端退出证明、
单实例锁、GO 前持久登记、owner EOF、guardian 全后代树回收及私有 receipt、Rust
持久 attempt/迟到 POST、PDF/图片真实解析、产物校验和既有来源/lease 发布栅栏。
intelligence 与 Worker 的固定产物编排及 SIGTERM 关闭已接入；本代理未操作 dev。

- 最终 T47 Python 定向 30 项通过：真实中文 OCR/500与501页/像素限额/deadline，
  READY/GO guardian 崩溃、逃组后代、receipt 写入失败、API SIGKILL/重启、冷备等。
- Rust 真实 HTTP 3项通过，包含丢 POST 响应、原请求/原 deadline 重放、过期首次
  materialize、cancel409后的资源证明；真实 scan→capture→OCR→成功表和 unchanged
  不重试通过。实际 Worker 二进制在 compute 启动后收到 SIGTERM，exit0且原 execution
  退出证明 true。
- 完整 Rust fmt/workspace test/clippy 均 exit0：381passed，1个既有文档示例 ignored。
  此后仅增加 output_too_large 永久分类及对应单测，真实旧分类 RED→GREEN、最终
  fmt/clippy均exit0；父独立全门禁将覆盖该最终快照。
- 首轮完整 Python 为81passed/4failed，失败是旧泛错误码/文本输入预期与T47明确
  机器码/白名单不符。最小更新后4项兼容定向全部通过；完整轮次早于后续取消竞态
  修复，不冒称它覆盖最终字节。新竞态定向RED→GREEN，父独立完整pytest待完成。
- 最终 ruff check/format、pc36项、API check/verify、shell语法均exit0。
- 私有停机CLI、state/receipt与冷备规则见 [intelligence README](../../../../services/intelligence/README.md)。
  无证明/缺库/legacy/损坏均非0；不执行恢复或写原库，不以PID消失代替凭证。

固定 Python 产物已从 b27c388 真实构建，移除全部文件/目录写权限后，在原位置和整体
搬移后的第二绝对目录各完成真实中文OCR。API/guardian/compute实际exe均在各自固定
产物根，-I-B，succeeded/prooftrue，退出后CLI clean0；搬移后父子sys.prefix/stdlib/
模型依赖导入路径全部在产物内。两轮后完整文件集合和摘要均与清单精确一致。

产物 `/data/knoted-dev/tasks/t47/build/artifact-b-relocated`，38,241普通文件、
6,422,057,839bytes、468ELF、0links/0pyc；SHA256SUMS文件SHA256
`ca3d7d55a7dcf6c16bb93777ced5196d8a14f4b255d2a979acf269d447369ebb`。
解释器/锁SHA同上，宿主ELF运行库限制仍适用，不宣称完整系统闭包。
构建、两个实际compute、搬移后父子导入、两次完整清单验证的原始log及.exit均为0，
文件前缀 `b-artifact-*`，逐ELF readelf为 `b-elf-dependencies.log`。

独立审查、最终父门禁、主线/dev/清理尚未由实施者执行，整卡仍为待集成交接状态。

## R1 审查修复

独立审查指出四项Important，已逐项定向RED→GREEN：Python发布事务最后重新检查原
请求deadline（root/source锁等待跨截止均回滚成功与outbox）；guardian退出后重读
receipt以消除提交观察竞态；运行期只调和未解决占用且请求记录仍独立核验；execution
与owner/hash/queued监督登记同一事务。真实进程中断的两个初始登记窗口只留下全无或全有。

相关Python回归58passed/4既有弃用warnings；T46/T47 publication6passed，真实Worker
pipeline及机器码单测通过，fmt/clippy/ruff均exit0。完整命令和逐项失败/修复证据已追加
外部implementation报告。父将对修复后SHA跑完整门禁并作scoped复审。上述b27固定产物
保留为历史证据，最终联合产物与真实dev仍待父执行，不将旧产物计作R1字节验证。
