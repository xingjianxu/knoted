# T42 统一回收站实施证据

状态：implementation-ready，待主 Agent 独立审查、集成与 dev 验收；不是完整生命周期完成。

- Issue：[#127](https://github.com/xingjianxu/knoted/issues/127)。
- 基线：`770263e`；任务分支 `feat/t42`；Worktree `/wrd/prj/knoted/.worktrees/knoted-t42`。
- 输入：[T42](../tasks/T42.md)、[ADR 0006](../../../decisions/0006-file-pipeline-and-aggregate-contracts.md)、主 Agent brief 与 `/data/knoted-dev/tasks/t39-t50/reports/t42-readonly-research.md`。
- 计划：依次完成只读聚合/资格、HTTP 注册、Web 恢复动作与真实路由、回归与门禁，交主 Agent 集成。复用 T24/T37 restore，消费 T41 基线及主 Agent 已生成的契约；不改锁文件、迁移或共享契约。

## 验收清单

- [x] API 真实 RED：缺失 `/trash` 返回 404 而期望 200，2 项失败；日志 `api-red.log`；当时 tail 包装 shell 退出 0，未独立捕获 cargo 子进程退出码，日志明确 2 failed，不把包装退出码算 GREEN。
- [x] Web 真实 router RED：访问 `/trash` 无回收站标题，1 项失败；日志 `web-red.log`；当时 tail 包装 shell 退出 0，未独立捕获 vitest 子进程退出码，日志明确 1 failed，不把包装退出码算 GREEN。追加行测试的首次 import 缺失是收集错误，不把它计为行为 RED。
- [x] Core SQL 只列实际实体表软删除行；排序 `deleted_at DESC, entity_type ASC, id ASC`，完整 tuple seek + limit+1；游标绑定 version/scope/order/filter（含无筛选）。
- [x] `REPEATABLE READ READ ONLY` 一致快照；笔记父链谓词只增加读取模式，命令保留原锁；事项复用关系与完整终态谓词。无 restore 探测、无 GET 写锁、无迁移。
- [x] 30 天只提示清理资格；40 天软删除对象仍能恢复；只归档对象与 registry-only 身份排除。
- [x] 前端类型筛选/前后翻页/加载期禁用，逐行 `type:id` 状态；笔记显式根恢复，确认/取消；各领域 CAS restore 路由。
- [x] 写成功立即移除全部 trash 缓存中的本行，再刷新；其它行保持挂载/位置选择/错误。读取失败与写失败分开反馈。
- [x] 真实 `/trash` 路由、启用侧栏、safeReturnTo 注册；本卡 router 测试 stub scrollTo，避免新增 jsdom 噪声。
- [ ] 主 Agent 独立 spec/quality 审查、PR 合入主线、主线门禁、dev 浏览器验收及清理。

## 验证

日志目录：`/data/knoted-dev/tasks/t42/logs/`。Rust 共享 target：`/data/knoted-dev/tasks/t39-t50/build/rust-target`；TestDb 默认只访问 `knoted_test` 隔离 schema，无 dev 数据修改。

| 命令 | 结果 | 日志 |
| --- | --- | --- |
| `cargo test -p knoted-core -p knoted-api --test t42` | 0；core 3、API 3，全部通过 | `rust-t42-final.log` |
| `cd apps/web && pnpm exec vitest run tests/tasks/t42.test.tsx` | 0；6/6，通过且无 scrollTo 噪声 | `web-green.log` |
| `cargo fmt --check` | 0 | `rust-fmt.log` |
| `cargo test --locked` | 0；267 passed、0 failed、1 ignored | `rust-test-final.log` |
| `cargo clippy --locked --all-targets -- -D warnings` | 0 | `rust-clippy-final.log` |
| `cd apps/web && pnpm test` | 0；17 文件、248/248 | `web-test.log` |
| `cd apps/web && pnpm lint` | 0 | `web-lint.log` |
| `cd apps/web && pnpm typecheck` | 0 | `web-typecheck.log` |
| `cd apps/web && pnpm build` | 0；既有编辑器大 chunk 警告 | `web-build.log` |
| `pnpm api:check` | 0；83 operations、20 examples，10 invalid rejected | `api-check.log` |

中间发现：初版复用 note `find_any` 自带 `FOR UPDATE`，只读事务实际报 25006（`api-debug.log`）；修为独立只读查询且保留写命令锁，现已通过持有 notes 写锁时读取的 PostgreSQL 验证。首轮 core/full suite 有测试夹具误用 registry 列名 `entity_id/deleted_at`，按实际 schema 修正为 `id`；第二轮 full `cargo test` 267 passed、0 failed、1 ignored。最终门禁以 `*-final.log` 与对应 `.exit` 为准。

全 Web suite 仍有其他既有 router 测试的 3 条 jsdom scrollTo 提示，本卡 6 项输出干净。全 Rust 1 项 ignored 为既有 auth middleware 文档示例。未执行 dev 部署；实施 Agent 按授权不 push/PR/merge、不修改运行实例、不清理其它任务。

## UI 消费与 dev 验收入口

实际锁定 `@bixlab/mtone@0.4.0`；已核查发布包公开声明，组合 List/ListItem、Select、Dialog、Button、Alert、Spinner、Empty 实现筛选、列表、确认与反馈，无通用能力缺口，无上游 Issue。

- 受保护 URL：`/trash`；main 名称“回收站”，heading“回收站”，侧栏 button“回收站”。
- 列表名“回收对象”；每行 region“回收对象 {title}”；类型筛选 combobox“对象类型”，选项“全部/笔记/项目/事项”。
- 笔记位置 combobox“恢复位置 {title}”，选项“原位置/根目录”；恢复 button“恢复 {title}”。
- 确认 dialog“恢复 {title}”；button“确认恢复/取消”；成功 status“已恢复 {title}”；刷新 button“刷新列表”。
- 翻页 button“上一页/下一页”；当前受限原因在对应 region 内显示。删除时间按上海时区展示。
