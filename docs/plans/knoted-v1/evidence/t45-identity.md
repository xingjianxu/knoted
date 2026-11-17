# T45 实施记录

基线 0e95c3a；Issue #130 已只读查重。隔离 worktree knoted-t45 / feat/t45。
使用 executing-plans、TDD 与 using-git-worktrees；既有任务包为执行计划，父 Agent 负责独立审查和集成。

- [x] 阅读任务、Spec、ADR0002/0006、T43/T44交接和 controller rulings。
- [x] RED/GREEN：完整扫描唯一强身份移动、手动元数据事务、并发 CAS。
- [x] 完整 Rust 门禁与自审；提交与报告见外部交接记录。
- [ ] 主 Agent 独立审查、集成、dev 验收。

T44→T45：稳定捕获发布遵循 metadata3/content2；不按 same-scan 或 hash 替代 incarnation。
实现采用本代 missing 资格标记，历史 missing 不复活。备注非空（包括空字符串）与目标非 null 不相同时 409；不拼接。关系保留目标已有边，否则按旧边 UUID 升序迁移并递增关系 version。


## 实现与验收边界

迁移0014只增加本次完整扫描移动资格标记。扫描完成事务清除上一代资格，并标记原 present 且原路径已缺失的来源；稳定捕获在既有 root/scan/lease 锁下按同根 device/inode、完整 opaque handle、稳定摘要、两端 nlink=1 及唯一来源/观察判断。定位改变更新 metadata/observation 版本；不改变内容版本，不按摘要合并。

手动事务采用 tree → relation admission → root → UUID 文件行顺序。只检查旧来源 version；目标已有备注/标签/关联受行锁保护。根处于未登记、暂停或扫描中时409；另外使用 safe_fs 的 FD 相对路径访问在准入及提交前复核实际根 Identity，因此 DB 尚未暂停时的根失联/替换也409。目标可用性按当前已发布 present 行判断，不声称对任意外部文件变化提供原子快照。

标签包含已软删除标签的成员关系一起转移，绝不恢复标签，公开 DTO 仍只列 live 标签。所有入站/出站关联参与迁移，失效另一端仍保留；目标已有边保留 ID/version，其他边按旧 UUID 升序确定保留者并递增被改端点的 version。手动迁移不更新内容版本、摘要、捕获指针或历史 outbox 载体。

本次实现阶段不操作真实 SourceRoot/dev 服务，不 push/PR/merge；独立审查、集成和 dev 验收由主 Agent 执行。


## 实际验证

所有 Rust 命令使用 `CARGO_TARGET_DIR=/data/knoted-dev/tasks/t39-t50/build/rust-target`、`TMPDIR=/data/knoted-dev/tasks/t45/tmp`。原始日志与 `.exit` 在 `/data/knoted-dev/tasks/t45/logs/`。

| 命令 | 结果 | 日志 |
| --- | --- | --- |
| `cargo test -p knoted-core --test t45`，初始 | exit101；2失败（rename 新建ID、标签版本未推进） | red-initial.log |
| `cargo test -p knoted-core --test t45 t45_3`，占位 | exit101；1失败（InvalidField） | red-relink.log |
| `cargo test -p knoted-core --test t45 t45_relink_rechecks_root`，修复前 | exit101；1失败（根缺失仍成功） | red-root.log |
| `cargo test -p knoted-api --test t45`，注册前 | exit101；1失败（路由404） | red-http.log |
| `cargo test --workspace t45_` | exit0；core21 + api1 =22通过 | final-t45.log |
| `cargo test --workspace --locked` | exit0；346顶层通过，0失败，1既有doc忽略；另T44 IO子进程1次通过 | final-cargo-test.log |
| `cargo fmt --check` | exit0，无输出 | final-fmt.log |
| `cargo clippy --locked --all-targets -- -D warnings` | exit0 | final-clippy.log |

```text
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.44s
```

顶层/嵌套执行分开计数见 `workspace-counts.json`；不是简单相加347。并发用例调用真实 PostgreSQL 事务与 assign/removeTag、create/removeLink、reconcile_complete_scan；不以 mocks 代替。旧引用边界验证已存在的 source/capture/closed input/outbox，尚未实现的聊天引用不声称通过。完整交接报告：`/data/knoted-dev/tasks/t39-t50/reports/t45-implementation.md`。
