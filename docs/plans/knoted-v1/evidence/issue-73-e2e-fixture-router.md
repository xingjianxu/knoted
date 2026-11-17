# Issue #73：组件测试夹具隔离路由生成

关联任务：[#73](https://github.com/xingjianxu/knoted/issues/73)。2026-11-17 用户直接委派立即修复；基线 `ccf1fa6`，分支 `fix/73-e2e-fixture-router`，Worktree 为主仓库 `.worktrees/issue-73-e2e-fixture-router`。

## 范围与执行清单

采用独立 fixture 配置，通过 `apps/web/vite.components.ts` 共享 React/Tailwind 插件工厂。生产配置继续先加载 TanStack 路由插件；fixture 不再导入生产配置。保留现有 root、端口和 React alias，补充共享配置的 lint 覆盖。显式使用 `.ts` 导入，两个既有 `noEmit` TypeScript 配置配套启用 `allowImportingTsExtensions`，避免新增 Vite native-loader 兼容性警告。不改依赖、业务、路由产物、公共 UI、API/存储或主机配置，遵循 ADR 0002，无需新 ADR。

- [x] 读取任务、规则、配置及历史证据，创建独立 Worktree 并领取 Issue。
- [x] 复现：15 个 Chromium 用例通过，但启动报告仓库根 `src/routes` 的 ENOENT；日志无 ENOENT 断言失败。
- [x] 提取组件插件工厂，移除 fixture 对生产配置的继承。
- [x] 验证无该启动错误、无路由产物变化，运行本地门禁并保留原始输出。
- [x] `requesting-code-review` 独立只读审查，核实反馈并消除本次引入的配置导入警告。

提交、创建含 `Closes #73` 的 PR、普通合并、主线验证和清理属于后续交付步骤；最终状态和主线 SHA 维护于关联 PR 的「合并后核对」，本文件记录提交前证据。

## 验证证据

本任务日志：`/data/knoted-dev/tasks/issue-73/`。浏览器只读复用现有 T17 Chromium 1254；动态库路径只作用于测试进程，未修改系统配置。基线日志 `baseline-playwright.log`：`15 passed (5.8s)`，同时报告 `ENOENT: ... scandir '.../src/routes'`。

| 实际命令 / 检查 | 结果 | 原始日志 |
|---|---|---|
| `pnpm install --frozen-lockfile` | exit 0，锁文件未变 | `final2-install.log` |
| `pnpm lint` | exit 0，包含共享配置 | `final2-lint.log` |
| `pnpm typecheck` | exit 0，包含两个 TS 配置 | `final2-typecheck.log` |
| `pnpm test` | exit 0，12 files / 199 passed | `final2-test.log` |
| `pnpm build` | exit 0，保留原有大 chunk 提示 | `final2-build.log` |
| `pnpm exec eslint tests/e2e playwright.config.ts` | exit 0 | `final2-e2e-lint.log` |
| `pnpm exec playwright test --workers=3` | exit 0，15 passed (5.6s) | `final2-playwright.log` |
| 启动日志与路由副作用检查 | ENOENT / WebServer Error 均为 0，11 份生产路由及生成文件 SHA-256 前后一致，仓库根 `src/routes` 未创建 | `final2-routes.json` |
| `cargo fmt --check` | exit 0 | `branch-cargo-fmt.log` |
| `cargo test --locked` | exit 0，229 passed / 0 failed / 1 ignored doctest | `branch-cargo-test.log` |
| `cargo clippy --locked --all-targets -- -D warnings` | exit 0 | `branch-cargo-clippy.log` |
| `git diff --check` | exit 0 | 本次会话实际输出 |

最终执行摘要为 `final2-web-results.json` 和 `branch-rust-results.json`；测试、构建均无新增的 native-loader 警告。未为配置小修新增实现镜像测试，以真实启动日志 RED→GREEN、路由产物副作用检查和完整既有套件验收。保留 Node 环境的 NO_COLOR/FORCE_COLOR 提示及已有 Vite 大 chunk 提示，不称所有日志无警告。

独立 Reviewer `review_issue_73` 核对代码、两个 TS 配置的传递覆盖及真实日志/路由哈希；最终结论 Ready to merge，Critical / Important / Minor 均无遗留，Declined to judge 为空。主 Agent 独立复核所有退出码、差异和反馈；未以 Reviewer 口头结论替代验证。
