# T47 实施记录

基线 `5377c581cc7696cc416304b4ae1b758ee0c30f79`，Issue #132，隔离分支 `feat/t47`。
Phase A 代码与实际构建 revision：`7ef1524edb5c0247a8c95583b04215d4a5549693`。

以下为 Phase A 历史记录。2026-11-18 用户已批准退出证明补充 RFC，Phase B 按 ADR0007 实施，当前进展见文末。

- [x] 核对任务、ADR 0001/0003/0006 与三个运行准备报告。
- [x] 专用白名单入口；14 个定向用例含真实 500/501 页 PDF、40M 像素边界、100MiB 拒绝与中文 OCR，exit 0。
- [x] 从固定 SHA 构建 Python 产物，整体搬移后 `-I -B` 父子导入验证通过。
- [x] Python 完整 pytest 72 passed；ruff check/format、pc 34 passed、shell 语法均 exit 0。
- [x] 无计算健康检查：临时 loopback 实例 health=ok，active_executions=0；退出后清单不变。
- [x] 代码安全提交；保留任务分支与 Worktree。
- [ ] 主 Agent 独立审查与 Phase B 后续集成。

产物位于 `/data/knoted-dev/tasks/t47/build/artifact-relocated`，原始绝对路径已整体搬走。
共 38,239 个普通文件、6,422,035,511 字节、468 个 ELF；无 symlink、无 pyc，运行后
全量文件集合和摘要与 `SHA256SUMS` 精确一致。解释器 Python 3.12.15，实际 executable
SHA256 `9b6fd06e487935c92477cb47cabbda8d814cda44823a5159eddf791178afe7d0`；
uv.lock SHA256 `3f66a80110a7ea18390eb216556331600b66f4578d4f74739d197970fad53cbe`。
解释器/stdlib/Docling/Torch/ONNX Runtime/应用路径在搬移后的产物内，未依赖开发 venv 或 home 源码。

产物仍依赖宿主 loader/glibc/libstdc++/zlib；独立 ldd 在部分可选模块及依赖预加载路径显示
未解析项，实际 CPU 导入已通过，不承诺全部标准库、GPU/MPI 模块或跨主机移植。
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
