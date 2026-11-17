# T39 事项页面和编辑实施证据

- Issue：[#120](https://github.com/xingjianxu/knoted/issues/120)。用户直接委派，主 Agent 协调串行实施和独立审查。
- 基线：`b496d89`；工作区 `/wrd/prj/knoted/.worktrees/knoted-t39`，分支 `feat/t39`。
- 状态：本卡实施和本地验证完成，**待独立审查、主 Agent 集成与 dev 部署验收**；不声称已完成生命周期。
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

## 限制与交接

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

Rust 文件未再变化，本轮未重跑 Rust；上一轮实测结果保留。待主 Agent 限定复审与集成，未新增部署声明。
