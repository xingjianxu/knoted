# T66 回答转存笔记验收

[PR #217](https://github.com/xingjianxu/knoted/pull/217) 普通合并至 `f796437cccad3be3e9f5513262cbe8fb0984caf7`。主线树与审查和测试过的实现提交 `1bb8c85` 一致。[Issue #216](https://github.com/xingjianxu/knoted/issues/216) 由合并自动关闭。

## 本地与主线

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | exit 0，主线复跑通过 |
| `cargo test --locked --workspace` | exit 0，497 通过、0 失败、1 个既有 doctest 忽略，包含真实 PostgreSQL、OCR 和 Office |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo test --locked -p knoted-api --test t23 --test t62 --test t64 --test t66` | 主线 32 项通过，其中 T66 5 项 |
| `pnpm lint`、`pnpm typecheck`、`pnpm test`、`pnpm build` | exit 0，23 个文件共 441 项通过，构建保留既有分包体积提示 |
| `pnpm --filter @knoted/web exec vitest run tests/tasks/t65.test.tsx` | 主线 12 项通过，其中转存 2 项 |
| `pnpm api:check`、`pnpm api:verify`、`git diff --check` | exit 0，84 个操作和 29 份契约样例，生成客户端可复现 |

首次 Rust 全量运行缺少 Worktree 的 Python venv。复跑显式使用已安装的测试 venv 和固定模型目录。独立审查复核鉴权、事务与锁顺序、引用快照及笔记编辑器读回。对应回归复现并修复了目录读取失败阻断根目录转存、同键变化时标题校验早于冲突检查两个问题。

## 固定版本与 dev

部署 `f796437`，复用源码一致的已校验 Python 运行产物。发布包 38,343 个校验条目，`SHA256SUMS` 摘要为 `ac84a71d79fc84f5064d3b210d4c056175f3247ca89419b5468428af8366d7ff`。API 与 Worker 停写后对 dev 数据库及资产生成一致备份并验证校验清单，迁移 0025 随固定发版应用。共享 Caddy、PostgreSQL、OnlyOffice 和 Strata 进程保持运行。

真实 Chromium 在 HTTPS 15601 下使用临时会话与受限合成资料范围。使用回环模型完成持久 SSE 生成，经聊天页面转存回答，验证正文及引用快照、搜索索引、普通笔记编辑。删除合成出处和会话后读取转存笔记，正文和引用保持一致。合成笔记及标签随后软删除，临时会话授权撤销，恢复常规 dev 配置。第一轮夹具沿用了 T65 的标题匹配条件，修正合成标题后重跑通过。

本地运行证据位于 `/data/knoted-dev/tasks/t66/evidence/`，包括 `rust-workspace.log`、`mainline-rust.log`、`web-suite.log`、`mainline-web.log`、`dev-release.log`、`dev-acceptance.log`。
