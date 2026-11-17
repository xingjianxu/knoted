# T39 事项页面和编辑实施证据

- Issue：[#120](https://github.com/xingjianxu/knoted/issues/120)。用户直接委派，主 Agent 协调串行实施和独立审查。
- 基线：`b496d89`；实施工作区 `/wrd/prj/knoted/.worktrees/knoted-t39` 与分支 `feat/t39` 已于交付后安全清理。
- 状态：done（PR [#123](https://github.com/xingjianxu/knoted/pull/123)，主线 `bbdef36`，dev `bbdef36`；主线验证、实例验收和任务工作区清理完成）。
- Brief：`/wrd/prj/knoted/.worktrees/knoted-t39-t50-plan/.superpowers/sdd/2026-11-17-t39-t50/task-39-brief.md`。
- 实施报告：`/data/knoted-dev/tasks/t39-t50/reports/t39-implementation.md`。

## 实现与公开接口

- `features/tasks/TaskForm`：RHF/Zod 组合 mtone 的 Input、Textarea、Select、FormField、Button。支持标题/说明、四种状态、四种优先级、可空所属项目/父事项、开始/截止 DateValue。
- `TaskList` 展示事项及项目、状态、归档；`SubtaskList` 读取当前父事项的所有归档状态子事项，支持游标分页及单层子事项创建。子事项没有下一层创建入口。
- `/tasks` 与 `/tasks/$taskId` 沿用项目的扁平认证路由结构；导航注册真实入口；登录返回地址仅允许合法事项 UUID 深链。
- 使用生成客户端现有 Task/TaskCreate/TaskUpdate DTO 和 queryKeys，无新增依赖、OpenAPI、迁移或存储契约。
- 项目和父事项选项遍历现有游标分页；创建子事项继承父项目，父项目变更后失效所有事项/项目查询，子列表与详情重新读取。
- PATCH/DELETE/归档/取消归档发送当前 `expected_version`；父事项或项目不可用/归档/删除时详情只读。自身归档可在祖先可写后取消，删除需要确认并可取消。
- 409 保留当前表单草稿、显示当前版本、提供 JSON 副本和复制按钮；明确刷新服务端版本后才能重试。刷新失败维持禁止重试，草稿仍可复制。详情按 ID 挂载，切换事项不串草稿。
- Rust 路由把现有 `DomainError::InvalidField` 附到 `ApiErrorBody.field_errors`。既有 T35 HTTP 测试验证后端独立拒绝非法上海时间，并返回 `start/due` 字段；不改契约。

## mtone 0.4.0 能力核查

实际消费精确锁定的 `@bixlab/mtone@0.4.0`；公开类型 `dist/index.d.ts` 的 Input 为原生 `React.InputHTMLAttributes<HTMLInputElement>`，支持 `type=date/time`、`step`、禁用、ref；Select 和 FormField 支持受控值及字段错误。纯日期和精确时间由两个公开 Input 组合，时区与 IANA 唯一时间映射是 Knoted 业务规则，属于应用适配，无通用组件缺口，不创建上游需求。

DateValue 原样保留 `{date,time,timezone:"Asia/Shanghai"}`。前端使用 Intl 的 IANA 上海规则枚举相邻日偏移，仅接受唯一映射，拒绝 1991 春季缺失和秋季重复时间；纯日期截止按下一日零点比较，不固定 UTC+8，不把业务日期写成 UTC 午夜。后端仍是最终校验权威。

## 实际验证

日志目录：`/data/knoted-dev/tasks/t39/evidence/`。全部命令在本任务 Worktree 执行；Rust 使用 `TMPDIR=/data/knoted-dev/tasks/t39/tmp`、`CARGO_TARGET_DIR=/data/knoted-dev/tasks/t39/build/cargo`。

| 命令 | 退出码 | 实际结果 | 日志 |
| --- | --- | --- | --- |
| `pnpm install --frozen-lockfile` | 0 | 锁文件未修改 | `baseline-install.log` |
| `pnpm --filter @knoted/web test`（基线） | 0 | 13 文件 / 205 用例通过 | `baseline-web-test.log` |
| `pnpm --filter @knoted/web exec vitest run tests/tasks/t39.test.tsx`（RED） | 1 | 6 用例因事项页面未实现失败 | `t39-red.log` |
| `cargo test -p knoted-api --test t35 t35_real_iana_invalid_local_time_http`（RED） | 101 | 1 用例因缺少 field_errors 失败 | `rust-red.log` |
| 深链安全返回场景（RED） | 1 | `/tasks/$taskId` 被回退为 `/` | `deep-link-red.log` |
| `pnpm --filter @knoted/web exec vitest run tests/tasks/t39.test.tsx` | 0 | 15 用例通过，含 t39_1/2/3 | `t39.log` |
| `pnpm --filter @knoted/web test` | 0 | 14 文件 / 220 用例通过 | `web-test.log` |
| `pnpm --filter @knoted/web typecheck` | 0 | 浏览器及 Node TS 检查通过 | `typecheck.log` |
| `pnpm --filter @knoted/web lint` | 0 | 无错误或警告 | `lint.log` |
| `pnpm --filter @knoted/web build --outDir /data/knoted-dev/tasks/t39/build/web` | 0 | TS + Vite 构建通过 | `build.log` |
| `cargo fmt --check` | 0 | 格式通过 | `cargo-fmt.log` |
| `cargo test -p knoted-api --test t35` | 0 | 9 用例通过 | `rust-t35.log` |
| `cargo test -p knoted-api` | 0 | 157 用例通过（既有 doctest 忽略 1 个） | `rust-api-tests.log` |
| `cargo clippy -p knoted-api --all-targets -- -D warnings` | 0 | 静态检查通过 | `clippy.log` |

测试场景除卡片三项外，还覆盖：独立/项目/子事项创建、清空日期及父关系、刷新失败继续保护冲突草稿、删除取消不发请求、归档/取消归档、删除成功关闭详情、父和项目归档只读、安全深链及纯日期/历史偏移/非法日期时间边界。网络通过内存 HTTP 边界替身，页面、mtone、RHF/Zod、查询和生成客户端均为真实实现；后端日期验证使用真实 HTTP 路由及隔离 PostgreSQL schema。

## 实施子 Agent 交接时的限制

以下是集成前的历史记录，最终交付见“主 Agent 集成、实例与收尾”。

- Web 测试出现与基线一致的 jsdom `Window.scrollTo` 未实现提示；构建提示已有大块资源及外部 outDir 不自动清空，不影响退出码。未修改依赖或掩盖警告。
- 本地消费测试不等于真实浏览器/HTTPS dev 验收。dev 运行 SHA、部署和主线全门禁由主 Agent 完成并回写。
- 未 push、未创建 PR、未合并；未修改 README 主状态、其他业务模块、host 或生产配置。Worktree 和分支保留供独立审查。

## 独立审查修正 I1（round 1）

审查指出：详情查询普通刷新会拿到新 version，而旧表单仍保留旧完整字段，保存时可能用新 version 静默覆盖远端修改。已用两个回归先观察有效失败：dirty 草稿经普通列表刷新后未产生预期409；pristine 表单未同步远端字段（`fix1-red.log`，17用例中2失败）。

修正后 TaskForm 将草稿与 CAS 基础版本绑定，普通查询刷新不提升 dirty 草稿基础版本；未修改的表单随服务端同步。只有保存成功（以 PATCH 返回的 DTO 为新基础）或明确成功完成冲突刷新，才推进保留草稿基础版本。保存回写详情缓存，避免成功响应被旧缓存回刷。回归覆盖列表刷新和创建其他事项触发失效两条路径，断言原版号提交产生409且远端说明/日期未覆盖，并覆盖 pristine 同步及连续两次保存版号推进。

| 命令 | 退出码 | 实际结果 | 日志 |
| --- | --- | --- | --- |
| `pnpm --filter @knoted/web exec vitest run tests/tasks/t39.test.tsx`（RED） | 1 | 17用例中2失败，复现I1及pristine不同步 | `fix1-red.log` |
| `pnpm --filter @knoted/web exec vitest run tests/tasks/t39.test.tsx` | 0 | 18/18通过 | `fix1-t39.log` |
| `pnpm --filter @knoted/web test` | 0 | 14文件 / 223用例通过 | `fix1-web-test.log` |
| `pnpm --filter @knoted/web typecheck` | 0 | 通过 | `fix1-typecheck.log` |
| `pnpm --filter @knoted/web lint` | 0 | 无错误或警告 | `fix1-lint.log` |
| `pnpm --filter @knoted/web build --outDir /data/knoted-dev/tasks/t39/build/web` | 0 | 通过，保留原有大块资源/outDir提示 | `fix1-build.log` |

Rust 文件未再变化，本轮未重跑 Rust；上一轮实测结果保留。该修复交接时待主 Agent 限定复审与集成；最终复审及部署结果见下节。

## 主 Agent 集成、实例与收尾

- PR [#123](https://github.com/xingjianxu/knoted/pull/123) 普通 merge，主线 `bbdef3674d7b5cae1afd75e6da03a223669e6390`；包含实施 `a8e1d74` 与 I1 修复 `38816e8`。Issue #120 由关闭关系自动关闭。
- 独立初审发现 I1：Query 普通刷新更新版本而旧草稿仍在，可能静默覆盖远端字段；已用有效 RED 重现，绑定草稿/baseVersion，限定复审 I1 ADDR / Spec PASS / Quality APPROVE，无剩余重要问题。
- 主 Agent 独立分支验证：Rust255 passed / 1既有ignored、Web最终223 passed；fmt/clippy/typecheck/lint/build全部exit0。I1只改Web，Rust源码差异检查为空。
- 主线复跑实际结果（`/data/knoted-dev/tasks/t39/logs/main-*.log`）：cargo fmt --check、cargo test --workspace --locked（255 passed / 0 failed / 1 ignored）、cargo clippy --locked --all-targets -- -D warnings、Web typecheck/lint/test（14文件223 passed）/build全部exit0。
- 固定产物 `/data/knoted-dev/runtime/bbdef3674d7b5cae1afd75e6da03a223669e6390` 从已提交主线archive构建，完整SHA256SUMS通过（`build-artifact.log`）。
- 本卡无迁移，schema保持11。部署前停本实例API/Worker并确认旧PID退出，一致备份 `/data/knoted-dev/service/backups/20261117T150212Z-t39-`（pg_dump/assets/摘要和pg_restore --list均校验）；随后原子切current并pcwrap down/up。HTTPS ready200，API/Worker真实可执行路径指向固定SHA，共享PG/Caddy/Strata PID前后一致（`deploy.log`、`runtime-{before,after}.json`）。
- 真实HTTPS Chromium，无route拦截：`live-browser.log` exit0，deep_link、invalid_date_front_and_backend、parent_project_inheritance、ordinary_refresh_cannot_bypass_cas、conflict_preserved_and_retry、independent_create、switch_no_leak均true，cleanup_ok=true、passkey_count_stable=true。先制造远端更新，再普通刷新列表，旧草稿保存实际409且远端说明保留；显式刷新后可成功重试。
- 夹具仅通过API创建/软删项目和事项，临时会话已撤销；最终读回活跃项目0、活跃事项0、未撤销验收会话0、Passkey1（前后不变）。未运行硬件Passkey验证。
- 任务工作区 `.worktrees/knoted-t39` 干净，分支为主线祖先后安全移除并 `git branch -d feat/t39`。仅本任务生成dist移到任务日志区，包缓存无需保留；未触碰他人 `feat-https-shared-caddy` 工作区。
- dev运行 `bbdef36`；此后的纯证据文档PR不改变运行产物。
