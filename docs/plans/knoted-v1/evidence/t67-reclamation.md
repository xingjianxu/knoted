# T67 图片与暂存对象回收验收

实施 [PR #222](https://github.com/xingjianxu/knoted/pull/222) 合入 `e051558`。首轮 dev 发现宿主挂载表带有网络命名空间文件，Worker 保留待回收文件。修复 [PR #224](https://github.com/xingjianxu/knoted/pull/224) 跳过这类非目录条目，选定目录仍按 FD 的 mount ID 校验。修复合入 `e51532fb56e2c374125ce9de265f43ca5b24d57c`，功能及修复 Issue 由相应 PR 自动关闭。

## 门禁及独立审查

| 命令或场景 | 实际结果 |
|---|---|
| `cargo fmt --check` | exit 0，主线复跑通过 |
| `cargo test --locked --workspace` | 实施版 506 通过，修复版 507 通过、0 失败、1 既有忽略，包含真实 OCR 和 Office |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 两版均 exit 0 |
| `cargo test --locked -p knoted-core --lib --test t67 t67_` | 修复主线 10 项通过，包含 3 项挂载用例和 7 项回收用例 |
| `cargo test --locked -p knoted-core --test t26` | 实施主线 12 项通过 |
| `pnpm lint`、`pnpm typecheck`、`pnpm test`、`pnpm build` | exit 0，23 个文件共 442 项通过，保留既有构建体积提示 |
| `pnpm --filter @knoted/web exec vitest run tests/tasks/t31.test.tsx` | 项目主线 36 项通过，其中回收后的重放用例 1 项 |
| 私有 mount namespace 探针及真实宿主映射 | bind mount 跨越返回 EXDEV，修复版解析宿主挂载表后确认源目录与资产根分离 |
| `git diff --check` | exit 0 |

独立审查复核两阶段回收、引用锁、幂等重放、已证实终态的命名工件、挂载别名及源目录隔离，并独立核实修复前后命名空间回归。图片回收以数据库登记授权；终态满 24 小时且已有退出证明的暂存对象按受控名字清理。未知资产、原始采集输入及缺少退出证明的对象保留。

## 固定产物与 dev

切换前停止本实例 API 和 Worker，并为 PostgreSQL 和资产做一致备份，备份摘要校验通过。首次使用固定版本 `e051558` 时合成孤立图片保留。第二次使用同一合成样本，切换与放行前重新生成一致备份。修复固定产物 `e51532f` 校验条目 38,343 个，`SHA256SUMS` 摘要为 `7dd614188131b42d710e564989461fd19e9b0cc0ff203da48bcd65f0a0f06f29`。

在 Chromium 认证上下文通过 HTTPS 15601 调用上传、笔记和快照 API，建立两份合成图片及一项仅历史引用。在仅有合成样本符合条件的窗口，Worker 启动后回收无引用图片，文件不存在且鉴权读取返回 404。历史图片文件仍在，鉴权读取返回 200，历史快照引用可读。合成笔记移入回收站，临时验收会话撤销。复验期间共享 Caddy、PostgreSQL、OnlyOffice、Strata 的进程 PID 一致。暂存回收的边界由专项自动化用例覆盖，本轮 dev 实测的是图片回收。

本地证据见 `/data/knoted-dev/tasks/t67/evidence/`，包括 `head-rust.log`、`netns-rust.log`、`netns-mainline.log`、`netns-host-probe.log`、`netns-dev-release.log`、`web-suite.log`、`dev-acceptance.log`（首次失败）和 `netns-dev-acceptance.log`（复验通过）。
