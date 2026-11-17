# T38：项目页面

- 状态：completed（PR [#118](https://github.com/xingjianxu/knoted/pull/118) 普通 merge 至主线 `25b223094d2076bdf8855460a4289af451643a53`；主线门禁、固定产物部署与真实 HTTPS 浏览器验收均通过）。
- Issue：[#117](https://github.com/xingjianxu/knoted/issues/117)。基线 `83d0e30ebc3ba3a3b4f18ae8fac22a7b01e566b1`（T37 PR #115 已合入）；工作区 `.worktrees/knoted-t38`，分支 `feat/t38`。
- 执行依据：[T38](../tasks/T38.md)、[需求](../requirements.md)（R06/R10/R11）、[数据/API](../data-and-api.md)、[ADR 0002](../../../decisions/0002-mtone-ui-ownership.md)、[UI 指南](../../development/ui-mtone.md)、[Worktree 指南](../../development/git-worktree.md)。
- 直接依赖实际核对：T33 项目 API（PR [#96](https://github.com/xingjianxu/knoted/pull/96)）已在基线，`api/openapi.yaml` 与 `packages/api-client/src/generated/schema.d.ts` 含 `listProjects/createProject/getProject/updateProject/deleteProject/archiveProject/unarchiveProject/restoreProject`；T20 路由外壳（PR [#52](https://github.com/xingjianxu/knoted/pull/52)）提供 `_authenticated` 父布局与 `NavItem`。

## 范围与页面所有权

| 文件 | 职责 |
|---|---|
| `apps/web/src/features/projects/queries.ts` | `/projects` 列表（归档筛选 + 游标）、`/projects/{id}` 详情、归档前置条件所需的未完成事项读取（`/tasks` 分页合并 `todo`/`in_progress`） |
| `apps/web/src/features/projects/ProjectsPage.tsx` | 项目列表、创建表单、归档筛选、游标翻页、项目详情（修改、归档/取消归档、删除确认） |
| `apps/web/src/features/projects/ProjectAssociations.tsx` | 项目标签与资料关联的业务组合（复用公开 `/tags`、`/entities/project/{id}/tags`、`/links`） |
| `apps/web/src/routes/_authenticated.projects{,.index,.\$projectId}.tsx` | `/projects` 与 `/projects/{projectId}` 路由适配、UUID 校验 |
| `apps/web/src/routes/_authenticated.tsx` | 把 `项目` 从“待开放”改为真实导航入口 |
| `apps/web/tests/tasks/t38.test.tsx` | 本卡 6 条场景 |
| `apps/web/tests/tasks/t20.test.tsx` | 仅更新“未实现导航仍禁用”的占位断言（项目入口已开放） |
| `apps/web/src/routeTree.gen.ts` | 由现有 TanStack Router 插件按新路由生成，未手改 |

未修改 Rust、`api/openapi.yaml`、生成客户端、依赖或锁文件；无迁移。

页面只消费 `@knoted/api-client` 生成的契约与 `queryKeys`；路由适配层把 URL 中的 `projectId` 与 `onSelectProject` 传给页面，React props 不出现数据库类型或 SQL 细节。归档失败时服务端只返回 409（`version_conflict`，可带 `current_version`），不返回未完成事项清单，因此页面在 409 后用既有 `/tasks?project_id&status=` 分页读取 `todo` 与 `in_progress`，与后端 `has_tasks` 的 `status NOT IN ('done','cancelled')` 判定一致；清单为空时不展示“未完成事项”，只保留冲突说明。

## 实施中的裁决

- **路由文件形态**：任务卡写 `apps/web/src/routes/projects/`，仓库既有实际约定是扁平 `_authenticated.*.tsx`（笔记、凭据页均如此，插件 `routeTree.gen.ts` 也按此生成）。按仓库约定新建 `_authenticated.projects*.tsx`，不改目录结构。代价：若将来统一为目录路由，需要一次无行为变化的搬迁。
- **关联资料公共入口**：T38 允许目录只有 `features/projects/`，因此项目侧的标签/资料关联以 `ProjectAssociations` 在项目模块内组合公开端点，未把 `NoteAssociations` 改成通用组件（会越界修改 `features/notes/`）。代价：两处业务组合存在重复；笔记与项目在文案、归档只读和生命周期上确实不同，抽取共享组件留待单独任务评估。
- **T20 占位断言**：`项目` 入口从禁用变为真实链接，必须更新 T20 的“未实现导航仍禁用”断言，否则该回归与真实产品矛盾。仅改动该用例的导航名单，保留字体/严格 TS/零用例守卫与 `safeReturnTo` 白名单断言。
- **删除确认文案**：`ConfirmDialog` 明确写“项目中已归档的事项会保留，笔记和资料不会被删除”，避免用户以为会连带删除笔记；文案与 R11（项目删除前移出/删除事项）一致，实际门槛仍由服务端拒绝。
- **归档 409 的两种含义**：版本冲突与未完成事项都返回 409。页面同时展示冲突说明与（非空的）未完成事项列表，不猜测单一原因。

## 实际验证

全部命令在 `.worktrees/knoted-t38` 执行，原始日志在 `/data/knoted-dev/tasks/t38/`。

| 命令 | 实际结果 | 日志 |
|---|---|---|
| 基线 `pnpm --dir apps/web exec vitest run` | exit 0，12 文件 / 199 tests | 会话内基线记录 |
| `pnpm exec vitest run tests/tasks/t38.test.tsx`（首次 RED） | exit 1，`Failed to resolve import "…/features/projects/ProjectsPage"`，无测试收集 | 会话内记录 |
| `pnpm exec vitest run tests/tasks/t38.test.tsx` | exit 0，1 文件 / 6 tests | — |
| `pnpm exec vitest run`（全量） | exit 0，13 文件 / 205 tests | `t38-web-test.log` |
| `pnpm typecheck` | exit 0（`tsc --noEmit` 两个配置） | `t38-typecheck.log` |
| `pnpm lint` | exit 0 | `t38-lint.log` |
| `pnpm build` | exit 0，保留既有 >500 kB chunk 提示 | `t38-build.log` |
| `git diff --check` | exit 0 | — |

### 场景矩阵

| 场景 | 覆盖 |
|---|---|
| `t38_1` | 项目甲 → 项目乙 切换后：乙标签出现、甲标签消失、关联区域不含甲标签、详情表单标题为项目乙（不继承上一项目草稿） |
| `t38_2` | 归档 409 后展示“无法归档：仍有未完成事项”，列出 `todo`/`in_progress` 事项，`done` 事项不出现，归档按钮仍可重试 |
| `t38_3` | 删除确认对话框含“笔记…不会被删除”，确认后按 `expected_version` 发 DELETE，项目离开列表、详情关闭、其他项目保留 |
| 创建/修改/撤销 | 创建项目提交 `{title, description}` 并刷新列表；修改只提交变更字段与 `expected_version`；归档成功后出现“取消归档”，取消归档恢复可写与列表可见；归档/取消归档均失效项目与事项 Query |
| CAS 冲突 | 保存 409 时提示当前版本、保留表单输入 |
| 路由接线 | 真实 `createAppRouter` + memory history 渲染 `/projects`，导航到 `/projects/{id}` 后加载该项目关联 |

### TDD 与反向回归探针

产品在实现前完全不存在，首次运行是收集失败（模块不存在），不是行为断言 RED；因此另外做两次真实反向探针，证明测试确实约束行为，且都恢复为最终通过：

- 移除 `<ProjectDetail key={projectId}>` 的 key → `t38_1` 失败（详情表单仍是“项目甲”）：`probe-a-detail-key.log`，1 failed / 5 skipped。
- 归档 409 不再触发未完成事项读取 → `t38_2` 失败（找不到“无法归档：仍有未完成事项”）：`probe-b-unfinished.log`，1 failed / 5 skipped。
- 恢复后 6/6 通过。

## 限制与后续

- 本卡只做 jsdom 单元/集成测试；真实 Chromium + HTTPS 页面验收、固定 SHA 部署与回滚证据由主 Agent 在集成后回填，当前不声称已部署。
- 列表分页使用 `createCursorState/cursorReducer` 的上一页/下一页；加载期间禁用翻页，符合 T19 约定，未引入总数或稳定快照。
- `safeReturnTo`（`lib/router.tsx`）白名单仍只含 `/`、`/settings/security`、`/notes*`；`/projects` 深链在 401 后回落到 `/`。该文件不在本卡允许范围，未修改，列为后续接线事项而非本卡缺陷。
- 关联资料只支持选择已有标签、添加/移除标签与关联，不提供在项目页新建标签（笔记页负责标签创建）；如需，另开任务评估。
- 未做性能、Vitest 大 chunk 拆包、Rust/DB/OpenAPI 或 ADR 变更。

## 独立审查与修复 I1

独立审查（Spec PASS / Quality CHANGES REQUESTED，0 Critical / 1 Important / 7 Minor）在只读副本 `f4633f6` 上实际复跑：`t38.test.tsx` 6/6、全量 13 文件 / 205 tests、typecheck/lint/build 均 exit 0，并核对 `has_tasks` 判定与 CAS 字段。

- **Important I1：409 后未刷新 `loaded.version`，重试必然重复 409。** 保存、归档、取消归档、删除四个入口都只用缓存版本，失败后只显示文案而不刷新详情，提示“核对后重试”却无法成功。现加 `refreshAfterConflict`：收到 409 时失效 `projects.detail(projectId)` 与 `tasks.all()`；本地草稿按设计保留（详情草稿只在挂载时初始化一次），下次提交使用刷新后的 `expected_version`。
  - 有效 RED：`i1-red-conflict-retry.log` exit 1，`expected 7 to be 8`（重试仍用陈旧版本被拒）；GREEN：`i1-focused2.log` exit 0，6/6。
- 同时处理的 Minor：删除确认文案不再暗示“已归档事项会保留”（服务端只要仍有未删除事项即拒绝删除，且归档中的项目本身已禁用删除）；删除错误改用独立 `deleteError` 并在对话框关闭时清除，不再与归档错误混用或重复展示；`unfinished.error` 仅在已触发未完成事项读取时渲染；移除从未接线的 `assertCanWrite` 属性（T38 没有 401 停写机制，`readOnly` 才是实际守卫）；`ProjectAssociations` 补充与 `NoteAssociations` 的交叉说明；打开中的项目按钮加 `aria-current`。
- 新增覆盖：归档后断言 `保存修改` 与 `移除标签` 禁用，取消归档后恢复可用（对应此前未断言的部分）。
- 修复后全部门禁：`i1-test.log`（13 文件 / 205 tests）、`i1-typecheck.log`、`i1-lint.log`、`i1-build.log` 均 exit 0。
- 明确保留的 Minor：`ProjectAssociations` 的标签/关联增删与分页循环未单独建 jsdom 用例（与已测的 T32 笔记侧同源实现；真实 HTTPS 验收覆盖标签展示与关联增删）；与 `NoteAssociations` 的近似重复仍存在，待单独任务评估抽取。两项均记录为后续项，不影响本卡验收。

## 主 Agent 集成与实例验收

- 集成：PR [#118](https://github.com/xingjianxu/knoted/pull/118) 普通 merge → 主线 `25b223094d2076bdf8855460a4289af451643a53`（提交 `997e114`、`f4633f6`、`e4e41af` 均为其父链）；Issue #117 由 PR 关闭关系自动关闭。
- 主线验证（主检出，日志 `main-*.log`）：frozen `pnpm install` exit 0；`pnpm exec vitest run` exit 0（13 文件 / 205 tests）；`pnpm typecheck`、`pnpm lint`、`pnpm build` 均 exit 0；`cargo fmt --check` exit 0；`cargo test --workspace --locked` exit 0（255 passed / 0 failed / 1 既有 ignored）；`cargo clippy --locked --all-targets -- -D warnings` exit 0。Rust 未改动，跑全量门禁是为了确认主线整体仍绿。
- 固定产物：`/data/knoted-dev/runtime/25b223094d2076bdf8855460a4289af451643a53`，由该主线 SHA 的已提交源码 archive 构建 API/Worker/Web/migrations，`SHA256SUMS` 全量校验通过（`build-artifact.log`）。
- 部署前状态：运行 `1b37e55`，迁移 11，存活项目 0 / 事项 0。本卡无新迁移，schema 保持 11。
- 一致备份：`/data/knoted-dev/service/backups/20261117T141717Z-t38-`。`deploy.py` 先停 API/Worker 并确认原 PID 退出，再 `pg_dump --format=custom` + assets 拷贝 + 摘要清单，`sha256sum --check` 与 `pg_restore --list` 通过（`logs/deploy.log`）。
- 启用：原子切换 `current`，`pnpm pc down`/`up -D` 重载编排；HTTPS `/` 与 `/health/ready` 均 200；API/Worker `/proc/<pid>/exe` 实际指向固定产物；共享 PostgreSQL/Caddy/Strata `MainPID` 前后一致（`logs/runtime-{before,after}.json`）。
- 真实 HTTPS 浏览器验收（Chromium，无路由拦截；`logs/live-browser-final.log`）：`{"result":"passed","checks":{list_and_detail,edit_with_cas,archive_blocked_shows_unfinished,archive_success_readonly,unarchive,switch_no_leak,association_link,delete_confirm_scope,deep_link,passkey_count_stable:true},"cleanup_ok":true}`。覆盖：列表与详情、改标题走 CAS（PATCH 只带 `expected_version` 与变更字段）、未完成事项存在时归档 409 并列出真实事项、完成事项后归档成功且页面转只读、取消归档恢复、切换到另一项目不残留标签/关联/草稿、通过页面添加并移除资料关联、删除确认文案含“笔记…不会被删除”且删除后详情关闭、深链直接打开 `/projects/{id}`。
- 首次运行在 `setup` 阶段失败：任务 `TMPDIR=/data/knoted-dev/tasks/t38/tmp` 尚不存在，Playwright `mkdtemp` 报 ENOENT；单独复现确认同因（`browserType.launch: ENOENT … mkdtemp`），创建该目录后同一脚本与最终运行均通过。这不是产品缺陷，日志保留。
- 夹具回收：冒烟只通过 API 创建/软删项目、事项、笔记、标签并撤销临时会话，未执行硬件 Passkey ceremony；读回存活项目/事项/笔记/标签均为 0，未撤销冒烟会话 0，Passkey 数不变（`logs/final-fixture-readback.log`）。

## 收尾

- 收尾文档（本文件与任务卡、计划 README 状态）通过 `docs/t38-close` 分支的独立 PR 合入主线；任务工作区 `.worktrees/knoted-t38` 与已合并分支 `feat/t38` 在确认提交为主线祖先、工作区干净后安全移除。
- 保留的后续项（不阻断本卡完成）：`safeReturnTo` 未纳入 `/projects*`；`ProjectAssociations` 与 `NoteAssociations` 的近似重复及项目侧标签/关联增删的独立用例。
