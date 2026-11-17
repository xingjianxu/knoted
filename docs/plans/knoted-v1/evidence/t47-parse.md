# T47 Phase A 实施记录

基线 `5377c581cc7696cc416304b4ae1b758ee0c30f79`，Issue #132，隔离分支 `feat/t47`。
代码与实际构建 revision：`7ef1524edb5c0247a8c95583b04215d4a5549693`。

本阶段仅固定 Python 产物和 PDF/图片准入。新退出证明 RFC 尚待确认；未修改执行账本、Rust 生命周期或启用测试实例计算进程。

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

`t47_2`、900 秒/取消/退出证明、Rust 双端协议、完整 Rust/Web/Python 联合产物、真实 compute
禁写产物、主线集成和 dev 验收均未完成。当前未完成整卡，未预填 main/dev SHA。

原始日志与逐项 exit：`/data/knoted-dev/tasks/t47/logs/`。完整报告：
`/data/knoted-dev/tasks/t39-t50/reports/t47-implementation.md`。
