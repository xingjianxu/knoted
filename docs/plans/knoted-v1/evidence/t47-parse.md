# T47 Phase A 实施记录

基线 `5377c581cc7696cc416304b4ae1b758ee0c30f79`，Issue #132，隔离分支 `feat/t47`。

本阶段仅固定 Python 产物和 PDF/图片准入。新退出证明 RFC 尚待确认；不修改执行账本、Rust 生命周期或启用测试实例计算进程。

- [x] 核对任务、ADR 0001/0003/0006 与三个运行准备报告。
- [ ] 专用白名单入口与真实页数/OCR/大小/像素测试。
- [ ] 固定 SHA Python 产物，搬移与父子导入验证。
- [ ] pc 完整性场景、Python 完整门禁。
- [ ] 安全提交与主 Agent 独立审查。

`t47_2`、900 秒/取消/退出证明、Rust 双端协议、主线集成和 dev 验收均属于后续 Phase B，当前未完成整卡。

原始日志：`/data/knoted-dev/tasks/t47/logs/`。完整报告：`/data/knoted-dev/tasks/t39-t50/reports/t47-implementation.md`。
