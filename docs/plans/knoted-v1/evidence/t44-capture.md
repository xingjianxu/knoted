# T44 稳定副本与版本实施

Issue #129；实施基线 c7d3ef5fe8ec817731c680314b71246a1eee4487；全卡审查基线 348003a53307cd416207d5fd0e899204ceda34a0。
独立 worktree `.worktrees/knoted-t44` / `feat/t44`。父 Agent 负责审查、集成和 dev 更新。

- [x] 阅读 brief、T44、Spec、ADR0006、T43 最新交接与安全研究。
- [x] RED/GREEN：稳定采集、流式限额、安全路径、取消、发布竞争及 Worker 运行链。
- [x] 完整 Rust 格式、测试与 Clippy 门禁。
- [x] 安全提交、报告交接（提交 SHA 见外部交接报告）。
- [ ] 父 Agent 独立审查、集成与 dev 验收。

实现决策：当前完整扫描授权采集；不完整扫描仅失效，不推断 missing/move。每块同步读取/写入后 await yield，不派生 blocking task；Future 丢弃同步关闭并清理未发布对象。闭合对象进入 DB commit 之前进入保留状态，避免取消或提交响应丢失时删除可能已被引用对象。

## 持久化及后续消费

迁移 `0013_t44_captures.sql` 只扩展来源内部状态：`input_object_id`（`<files.staging_root>/input/<ObjectId>`）、`captured_observation`、`captured_observation_version`、`captured_scan_id`。对象先写随机 `.part`，64KiB 缓冲逐块 SHA-256/100MiB 限额；fsync、关闭、FD 相对无覆盖重命名并 fsync 目录后才发布来源。

两次 stat 间隔固定2秒，复制前后与当前根/相对路径重新打开的 metadata 必须一致（device/inode、opaque handle、size、mtime、ctime、nlink）。源和 staging 的所有路径段均 NOFOLLOW；源叶 NONBLOCK 并 fstat 限定普通文件。配置 staging_root 必须已存在，模块仅通过安全 FD 创建 input 子目录；未启用不打开目录。运行期不以启动时 alias 检查替代 FD 验证。

扫描完成事务写既有 `jobs` 的 `file.capture`，payload 为 scan_id/relative_path；真实共享 Dispatcher 领取并处理，未启用/旧代/取消拒绝，超限为明确 PermanentFailure，其余变化/IO 可重试。无第二套 queue/attempt 表，无 parser、extraction、move/relink 或索引实现。

发布锁 jobs/root/source，复核当前完整扫描、根 identity/generation/paused、完整观察及 observation_version、present；等待行锁后和提交前重查真实租约期限。首次 registry/source 同事务；强 incarnation 相符且路径未变才继承来源，不按 hash/inode 猜测跨路径移动。无 opaque handle 时不以同一扫描作替代证明，不能证明 incarnation 则保守新来源。已变观察由 T43 先增 version/observation_version 并失效，稳定摘要变化再增 content_version；重复相同观察不重复增语义版本。

T46/T47 必须同时校验 present、content_fresh、当前 observation_version 与 captured_observation_version、完整 captured_observation 和内容版本。只能消费已关闭 input 对象；不能直接读原根，不能用相同摘要合并身份。已发布/曾被引用对象不因新采集盲删；commit 已开始但响应未知也保守保留，后续回收必须证明无 source/jobs 引用。本卡不承诺任意外部并发写的原子快照，也不实现该回收器。

## RED 与回归

原始命令输出和相邻 `.exit` 在 `/data/knoted-dev/tasks/t44/logs/`。先有稳定成功用例，再由占位错误得到真实断言失败；入队和 Dispatcher 分别先断言实际 DB/运行结果；staging 叶替换及 disabled 场景暴露实际遗漏后补最小修复。

| 命令 | 结果 | 输出 |
| --- | --- | --- |
| `cargo test -p knoted-core --test t44`（初始占位） | exit101，1失败：稳定输入 Changed | `red-stable.log` |
| `cargo test -p knoted-core --test t44` | exit0，1通过 | `green-stable.log` |
| `cargo test -p knoted-core --test t44 t44_scan_atomically` | exit101，期望队列1实为0 | `red-enqueue.log` |
| `cargo test -p knoted-worker --test t44_runner` | exit101，实际 dispatcher 未成功 | `red-dispatcher-behavior.log` |
| `cargo test -p knoted-core --test t44 t44_staging_leaf` | exit101，替换叶仍被发布；修复后exit0 | `red-staging-leaf.log` / `green-staging-leaf.log` |
| `cargo test -p knoted-core --test t44 t44_disabled` | exit101，disabled仍可发布；最终回归通过 | `red-disabled.log` |

初次编译分别发现测试 FilesConfig.staging_root 应为 Option、worker 未依赖 tempfile；在运行行为测试前修正夹具，未加依赖。初次 clippy 2处 tuple type_complexity，改为命名 FromRow 结构。上述失败输出均保留，编译失败不计作 RED。

回归另含 copy 中增长/取消/drop、真实子进程 RLIMIT_FSIZE 引发写入 IO 失败、源/子目录/staging/叶替换、旧租约及等待source锁后过期、源观察 CAS、partial/旧扫描、同路径替换、注入相同inode而handle不同、双版本及旧对象保留、精确100MiB不截断、生产 WorkerRunner heartbeat/request_cancel 清理。

补充自检：`cargo test -p knoted-core --test t44 t44_same_scan_handle` 先 exit101（持久 handle 冲突被 same-scan fallback 错误继承），移除 fallback 后 exit0；`red-handle-fallback.log` / `green-handle-fallback.log`。研究要求强身份优先，同一扫描不是持续 FD 证明。旧行 capture 字段均 NULL 时不能作为 T46 已闭合输入消费。

## 同观察重放裁定

主 Agent 裁定：同 scan_id 不能替代 incarnation 证明；同观察幂等应复用“已发布闭合结果”，不得重新认领当前文件身份。实现只读 receipt 重放：锁定并复核 lease/root/current complete scan/source；source 仍 present/content_fresh，观察及 observation_version 匹配，捕获列（含持久 handle）与 captured_observation 一致；随后通过安全 FD 流式验证既有 input 的 SHA/size、读前后 metadata/当前 staging 路径，重查 lease/root，回滚只读事务并返回原对象。不会新建来源/对象或刷新 freshness/版本/时间，也不声称当前原件 incarnation。不同扫描、缺失/失效、持久 handle 冲突或无合法 receipt 仍须强身份证明；不合格旧对象保留，新的稳定捕获可以修复来源当前 input 指针。

`cargo test -p knoted-core --test t44 t44_no_handle_same`：真实 RED exit101（Changed），完整一致的无 handle 夹具在 `red-receipt-consistent.log`；GREEN 见 `green-receipt.log`（26通过，含 receipt 与早先回归）。最终另补不同 scan 的无 handle 来源与 staging symlink，不以重放绕过身份限制。没有新增公有确认字段或 attempt 表。

## 最终验证与交接

状态：implementation-ready，待主 Agent 独立审查、合并及 dev 验收。未改依赖/锁/API/README，未迁移 dev、初始化实际根、push/PR/merge 或修改共享服务。

全部 Rust 命令的环境为 `CARGO_TARGET_DIR=/data/knoted-dev/tasks/t39-t50/build/rust-target TMPDIR=/data/knoted-dev/tasks/t44/tmp`。

| 精确命令 | 最终结果 | 原始输出/退出码 |
| --- | --- | --- |
| `cargo test --workspace t44_` | exit0；core28 + worker2 = 30顶层通过，另IO子进程1次通过 | `t44-final.log` / `.exit` |
| `cargo test --workspace --locked` | exit0；323顶层通过、0失败、1既有doc忽略；另IO子进程1次通过 | `cargo-test-final.log` / `.exit` |
| `cargo fmt --check` | exit0，无输出 | `fmt-final.log` / `.exit` |
| `cargo clippy --locked --all-targets -- -D warnings` | exit0 | `clippy-receipt.log` / `.exit` |
| `git diff --check` | exit0，无输出 | `diff-final.log` / `.exit` |

实际输出摘录：

```text
test result: ok. 28 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.20s
```

逐target统计保存在 `logs/target-counts.json` 及完整交接报告。IO用例在隔离子进程临时收紧自身 RLIMIT_FSIZE 后恢复，产生额外 `1 passed` 行；不能将日志简单求和324当作顶层测试数。两个早先全量GREEN也保留为 `cargo-test-first-full.log` / `cargo-test-before-receipt.log`；最终以 receipt 实现后的上述输出为准。

未修改 Web/pc/脚本，未在本卡重复运行这些不受影响工具链。API/生成客户端双版本预登记由父 Agent 串行完成，不归本提交验证声明。无性能测试；复制变更拒绝并最终收敛不等于任意外部并发写下的原子快照。

完整报告：`/data/knoted-dev/tasks/t39-t50/reports/t44-implementation.md`；任务包 `task-44-report.md` 指向该报告。主 Agent 承担后续独立 Spec/Quality 审查、schema13备份迁移、固定SHA构建与实际根专属样本验收。
