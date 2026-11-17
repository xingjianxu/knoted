# mtone 0.4.0 升级与消费复验

## 范围与来源

- 用户直接委派：更新前端依赖库到 `@bixlab/mtone@0.4.0`；关联 [Issue #112](https://github.com/xingjianxu/knoted/issues/112)、[ADR 0002](../../../decisions/0002-mtone-ui-ownership.md)。不是架构变更，不涉及 API/存储或生产部署。
- 基线 `7c9f119`；分支 `chore/mtone-0.4.0`；Worktree `/wrd/prj/knoted/.worktrees/mtone-0.4.0`。本任务仅升级精确依赖、必要的笔记树类型适配及现行文档，历史升级与任务验收记录不改写。
- 原始输出在 `/data/knoted-dev/tasks/mtone-0.4.0/`；下文文件名相对该目录。执行步骤为核查发布包 → 精确升级与必要兼容适配 → 本地验证 → 独立审查/PR → 主线验证与仅本任务清理。

## 发布包核查

实际安装来自正式 npm 发布版本；官方 registry 的 `npm pack` 副本只用于只读检查，不以本地 tarball/link 作为交付依赖。默认 registry 元数据及官方元数据分别在 `npm-metadata.json`、`npm-official.json`，检查副本在 `package-inspection/package/`，公开类型差异在 `types.diff`。

- [官方元数据](https://registry.npmjs.org/@bixlab%2fmtone/0.4.0)、[README](https://unpkg.com/@bixlab/mtone@0.4.0/README.md)、[类型](https://unpkg.com/@bixlab/mtone@0.4.0/dist/index.d.ts)、[CSS](https://unpkg.com/@bixlab/mtone@0.4.0/dist/styles.css)、[llms.txt](https://unpkg.com/@bixlab/mtone@0.4.0/llms.txt)。实际安装的 manifest、README、llms.txt、JS、类型和 CSS 均逐字节等于官方检查副本。
- lock 与官方 tarball integrity 一致：`sha512-nITB6pBKhhZ07WHyWglxuhp3BqggBvvZDOCnDqxbtPEKZXfhDk343AgnHecNZvGEFSuS7uV6IGSfG2j4q7MuTQ==`。
- ESM、根导出、styles.css/llms.txt 导出不变；React/ReactDOM peers 仍 `^19.2.0`，应用仍为 `19.3.1`。直接依赖的名称和约束未变。
- Tree 支持更广的键类型与功能；Tabs、Avatar、Divider 等声明扩展，并新增多个组件。不因为发布新增能力而扩大本任务的业务采用范围。
- 包管理器曾重解析 Base UI 的 Babel runtime（7.29.14 → 8.1.2）；两版均满足已安装 manifest 的 `^7.29.7 || ^8.0.0`，本任务恢复原解析以避免无关升级，后续 frozen install 验证通过。最终 lock 仅变更 mtone。
- `minimumReleaseAgeExclude` 只将 mtone 例外从 0.3.0 换为精确 0.4.0，保留原 `react-resizable-panels@4.24.0`；未全局放宽供应链策略。

## 兼容适配与实际失败

1. **版本约束结构 RED**：先更新既有精确版本断言，运行 T17 用例，得到 4 passed / 1 failed，期望 0.4.0、实际 0.3.0（`red-version.log`）。这是结构断言，不冒称业务行为 RED。
2. 正式升级后 Vitest 已有 199 项通过，但首轮 `pnpm typecheck` 失败：`queries.ts(23,88): TS2345`，`TreeKey` 不能赋给字符串（`branch-typecheck.log`）。新 `TreeItem.id` 为 `string | number`；原 `noteTree` 返回通用 `TreeItem[]`，导致目录可达性校验失去字符串 UUID 保证。
3. 在应用内定义递归 `NoteTreeItem extends TreeItem`，将 `id` 收窄为 `string`，并在 `noteTree` 构建及遍历中使用。没有 `as` 强转、字符串转换、业务 ID 改变或通用组件包装；只有类型层变更。
4. 随后类型检查通过，199 项 Vitest 与 15 项真实 Chromium 测试全部通过。生产笔记树继续用原公开 `items/value/expanded` 接口；T31 用例覆盖完整分页目录、真实分支展开、选择导航和编辑/移动/恢复等既有行为。

## 本地检查

执行脚本 `verify.sh` 将逐条命令输出与退出码存档；Rust 日志为 `branch-*`，最终前端日志为 `green-*`。

| 命令 | 退出码 | 实际结果 / 日志 |
| --- | --- | --- |
| 基线 frozen install / `pnpm test` | 0 | 12 files / 199 tests；`baseline-install.log`、`baseline-test.log` |
| `pnpm install --frozen-lockfile` | 0 | 精确锁版可复现；`green-frozen-install.log` |
| `pnpm lint` | 0 | web 与 api-client；`green-lint.log` |
| `pnpm typecheck` | 0 | web（含测试）与 api-client；`green-typecheck.log` |
| `pnpm test` | 0 | 12 files / 199 tests passed；`green-test.log` |
| `pnpm build` | 0 | 生产构建成功；`green-build.log` |
| `pnpm exec playwright test --workers=3` | 0 | T17—T19 共 15 tests passed；`green-playwright.log` |
| `pnpm exec eslint tests/e2e playwright.config.ts` | 0 | `green-e2e-lint.log` |
| `cargo fmt --check` | 0 | `branch-cargo-fmt.log` |
| `cargo test --locked` | 0 | 236 passed，1 ignored doctest；`branch-cargo-test.log` |
| `cargo clippy --locked --all-targets -- -D warnings` | 0 | `branch-cargo-clippy.log` |
| `git diff --check` | 0 | `green-diff-check.log` |
| Markdown 本地链接 / 发布包与构建样式 | 0 | 65 links / 0 missing；6 个包文件相同，3 个构建 HTML/CSS 无远程 import/Google Fonts；`docs-and-artifacts.log` |

Chromium 只读使用现有 revision 1254 缓存 `/data/knoted-dev/tasks/t17/worker/browsers`；从本机 NixOS closure 解析现有 lib 目录，仅给测试进程配置 `LD_LIBRARY_PATH`（本任务 `browser-library-path.txt`）。未安装系统工具、修改主机配置或其他任务成果。Rust 使用本任务 `rust-target/`、`tmp/`，不占用其他任务输出。

浏览器覆盖真实样式/系统字体、Portal 主题/层级、键盘/焦点、表单、菜单/通知、确认取消/失败重试与 Tree/游标。CSS 没有远程 import 或 Google Fonts 请求；实际浏览器测试也断言外部请求与页面错误为空。

## 限制与集成状态

- 生产构建仍有 >500 kB chunk 警告；未改阈值掩盖或顺带拆包。
- 未验证新增组件全部业务场景、硬件 Passkey、生产部署或运行中实例升级；不以本轮测试替代这些验收。
- 没有新证实的 mtone 通用能力缺口，无需上游需求交接、不改上游、不发布 npm。
- 独立只读 Reviewer `review-mtone-0.4.0` 已检查全部 10 个任务文件、发布包/lock 一致性、公开 Tree 实现及验证日志；Critical / Important / Minor 均无，结论可合并。主 Agent 核实上述范围与证据，并在 `precommit-*` 日志中重新执行全套门禁，结果仍全部通过。
- 独立审查不覆盖未消费的新组件/Tree 高级模式、硬件 Passkey、生产部署或运行实例升级；未独立认证 npm 签名/provenance（本轮仅核对官方 integrity 与文件一致性），不将这些未知项宣称为通过。
- PR 合并、最终主线 SHA、主线复验与工作区清理结果在本任务 PR 中记录；本文件只记已执行的分支验证，不提前宣称后续阶段完成。仓库没有 GitHub CI，远程 checks 为空不代表检查通过。
