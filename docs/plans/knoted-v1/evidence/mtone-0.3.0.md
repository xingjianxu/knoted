# mtone 0.3.0 升级与消费复验

## 范围与来源

- 用户直接委派：将 `@bixlab/mtone` 更新到 0.3.0，并更新相关代码和文档。
- 关联 [Issue #70](https://github.com/xingjianxu/knoted/issues/70)、[实施计划](../../2026-11-16-mtone-0.3.0.md)、[ADR 0002](../../../decisions/0002-mtone-ui-ownership.md)。本次不是架构变更。
- 初始基线 `a2e0e36`；任务分支 `chore/mtone-0.3.0`，Worktree `/wrd/prj/knoted/.worktrees/mtone-0.3.0`。只改本任务路径，不操作其他任务 Worktree。
- 原始输出归档 `/data/knoted-dev/tasks/mtone-0.3.0/`，下文日志名相对该目录。历史 T17—T20 的 0.2.0 安装/验收记录不改写。

## 发布包亲查

实际依赖是正式 npm **`@bixlab/mtone@0.3.0`**，`pnpm add --save-exact` 生成 lock，`pnpm list --filter @knoted/web @bixlab/mtone --depth 0` 确认实际安装 0.3.0。检查副本使用官方 registry 的 `npm pack`，仅只读核查，不是安装来源。

- 元数据：`npm-metadata.json`；发布包 `package-inspection/package/`；公开类型差异 `types.diff`。
- 官方 [元数据](https://registry.npmjs.org/@bixlab%2fmtone/0.3.0)、[README](https://unpkg.com/@bixlab/mtone@0.3.0/README.md)、[类型](https://unpkg.com/@bixlab/mtone@0.3.0/dist/index.d.ts)、[CSS](https://unpkg.com/@bixlab/mtone@0.3.0/dist/styles.css)、[llms.txt](https://unpkg.com/@bixlab/mtone@0.3.0/llms.txt)。检查副本与 lock 的 tarball integrity 相同：`sha512-Oe67c7IwpyVTOYMKr9H7tuJTaGw1NPABDh90+3u49UQ0a6S1s6PBTAzURx6V24AYAKaLs4uldwh3e71Ulb4Knw==`。
- ESM；React/ReactDOM peers 仍 `^19.2.0`，应用维持 `19.3.1`；根导出与 `styles.css` 导出兼容，新增 `llms.txt`。
- 公开声明新增 Form 托管字段、Dialog、布局等能力；仅核查与现有消费相关部分，不宣称新组件全部通过 Knoted 业务验收。
- 新增传递依赖 `@rc-component/table@1.12.0`、`react-resizable-panels@4.24.0` 及其依赖；lock 没有无关升级。未新增 Knoted 对 Base UI/表格底层库的直接依赖或私有导入。
- pnpm 自动为 `react-resizable-panels@4.24.0` 添加版本限定 release-age 例外；亲查其已发布元数据、MIT license、React peers 与安装 manifest（没有安装期 lifecycle 脚本），官方发布时间 `2026-11-15T18:32:34.562Z`。仅保留该精确传递版本与用户指定 `@bixlab/mtone@0.3.0` 例外，移除旧 0.2.0 例外，不全局放宽等待期。

## 真实差异、适配与 RED/GREEN

| 阶段 | 实际结果 | 日志 |
|---|---|---|
| 0.2.0 基线 frozen install / Vitest | exit 0；6 files / 54 tests passed | `baseline-install.log`、`baseline-test.log` |
| 精确版本断言改为 0.3.0，尚未升级依赖 | exit 1；期望 0.3.0、实际 0.2.0，53 passed / 1 failed；这是结构 RED，不是业务行为 RED | `red-version.log` |
| 正式 npm 升级后首轮 Vitest | exit 1；53 passed / 1 failed：手动 FormField 清错后没有 aria-invalid 属性，旧测试期待字符串 false | 初始结果见会话；后续成功运行覆盖了 `green-test.log`，不冒称该文件仍含首轮失败 |
| 调整语义断言后 Vitest | exit 0；54 passed | `green-test.log` |
| 首轮默认浏览器环境 | exit 1；15 个用例均未能启动，因为默认缓存缺少 revision 1254；不是产品行为 RED | `playwright.log` |
| 显式 Nix 动态库与现有 Chromium 后 | exit 1；14 passed / 1 failed：外部点击未关闭 Dialog | `playwright-nix.log` |
| 公开 Dialog 适配后 | exit 0；15 passed | `playwright-green.log` |

根因与处理：

1. **FormField**：0.3.0 发布实现会在手动模式下保留子控件已有 `aria-invalid`；无显式错误且子控件未提供该属性时，清错后为缺省，而非 `"false"`。两者都表示有效。Vitest 用 `toBeValid()`，浏览器确认不再为 `"true"`，仍验证错误时 true、可访问描述和恢复。不用私有补丁强制旧 DOM 输出。
2. **Dialog**：公开类型 `DialogProps.modal` 默认 true，发布实现设置 `disablePointerDismissal: modal`；旧 T18 场景依赖外部点击关闭。将夹具从已弃用 `Modal` 改为推荐 `Dialog`，显式 `modal={false}`，继续验证焦点陷阱、Esc/关闭/完成/外部点击回焦及触发点卸载；不是删测试绕过默认值变化。
3. 生产现有 Provider、主题/CSS、认证页、外壳与游标 reducer 无需为此次升级改动；没有顺带迁移 RHF 或采用新增通用控件。

## 本地检查

| 命令 | 退出码 | 实际结果 / 日志 |
|---|---|---|
| `pnpm install --frozen-lockfile` | 0 | lock 可复现；`frozen-install.log` |
| `pnpm lint` | 0 | 两个 workspace package；`lint.log` |
| `pnpm typecheck` | 0 | web 与 api-client；`typecheck.log` |
| `pnpm test` | 0 | 6 files / 54 tests；`test.log` |
| `pnpm build` | 0 | Vite 生产构建成功；`build.log`；存在 >500 kB chunk 警告，未调整阈值掩盖 |
| `pnpm exec playwright test --workers=3` | 0 | T17—T19 共 15 tests；`playwright-green.log`；启动器已知 ENOENT 噪声见下文 |
| `cargo fmt --check` | 0 | `cargo-fmt.log` |
| `cargo test --locked` | 0 | 共 125 passed，1 ignored doctest；`cargo-test.log` |
| `cargo clippy --locked --all-targets -- -D warnings` | 0 | `cargo-clippy.log` |
| `pnpm exec eslint tests/e2e playwright.config.ts` | 0 | `final-e2e-lint.log` |
| Markdown 本地链接 / 安装包对比 / 构建字体扫描 | 0 | 62 links、0 missing；实际安装文件与官方检查副本一致；HTML/CSS 无远程 import/Google Fonts；`docs-and-artifacts.log` |
| `git diff --check` | 0 | `final-diff-check.log` |

Chromium 使用已存在的 `/data/knoted-dev/tasks/t17/worker/browsers` revision 1254（只读使用，不移动/修改其他任务成果）；从现有 NixOS 系统闭包解析 `lib/`，仅给当前测试进程配置 `LD_LIBRARY_PATH`，路径列表保存在本任务 `browser-library-path.txt`。不修改主机配置，不额外安装浏览器或系统库。各浏览器用例继续断言页面错误/外部请求为空，包含真实 CSS、系统字体、Portal tokens/层级、通知和危险操作取消等消费行为。

### 已知限制与独立问题

- Playwright fixture 展开生产 Vite 配置，继承 TanStack 路由生成插件；启动时按仓库根查 `src/routes`，输出 ENOENT。15 个组件测试实际通过，但启动日志并非无错误。查重后单独记录 [Issue #73](https://github.com/xingjianxu/knoted/issues/73)；这两份配置本次未改，不把路由夹具修复扩大进依赖升级。
- 构建仍有大 chunk 警告；没有为消除警告改拆包或调整阈值。
- 未验证新增组件的全部业务场景、真实硬件 Passkey、生产部署或线上实例；未声称本轮覆盖。
- 当前消费均可用，无已证实 mtone 通用能力缺口；无需新建上游需求、不修改上游、不发布 npm。

## 审查与集成

独立只读 Reviewer `review-mtone-0.3.0` 已检查全部 12 个跟踪文件差异及两个新文档、发布/安装文件一致性、62 个本地链接与原始验证日志，结论 **APPROVED，Critical / Important / Minor 均无**。主 Agent 核实了 Form 手动模式、Dialog 公开外部关闭参数和所有实际测试结果。并行 Caddy 改动、未消费组件全场景、完整传递依赖安全审计与生产部署不属于此次审查背书；集成新主线后需再次验证，不以本轮结论替代。

最终复跑及合并信息记录在关联 PR；只有本地门禁通过、PR 普通 merge、主线验证后，才完成任务与 Worktree/已合并分支清理。仓库没有 GitHub CI，不以远程 checks 为空当作通过证据。
