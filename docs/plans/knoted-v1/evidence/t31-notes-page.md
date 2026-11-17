# T31：真实笔记树、编辑会话与只读历史页面

- 状态：implemented；待主 Agent 独立审查、PR 集成、主线门禁和真实 HTTPS 消费验收。本文不宣称已合并或已部署。
- Issue：[#89](https://github.com/xingjianxu/knoted/issues/89)。基线 `752a09357434f6095e57671071d0c783e71fbb47`（T30 PR #101 已合入）；工作区 `.worktrees/knoted-t31`，分支 `feat/t31`。
- 执行依据：[T31](../tasks/T31.md)、[ADR 0002](../../../decisions/0002-mtone-ui-ownership.md)、T30 的最终会话接口及主 Agent 已采纳的接线预检。未修改后端、OpenAPI、依赖或锁文件。

## 页面与数据所有权

注册 `/notes` 与 `/notes/$noteId`，认证父壳的“笔记”入口可用，未实现导航项仍禁用。`safeReturnTo` 只接受已登记的严格本地路径与 UUID 详情，继续拒绝外站、编码绕过、反斜杠、认证路径及额外后缀。路由生成文件由现有插件生成。

目录使用唯一应用 API/QueryClient，按真实全局 GET `/notes` 的 `next_cursor` 读取所有页，按 UUID 去重，再按真实 parent_id、position、id 构造公开 mtone Tree。不存在 `has_children`、total、伪根、lazy branch 或未登记 query。重复游标、孤儿或不可达父子关系成为目录 Query 错误，提供从首页重读；错误不会卸载当前本地编辑器。创建、移动、删除、恢复成功后重新完整读取目录。标题只额外消费版本不旧于目录项的已确认 detail 缓存，未保存标题不进入树，也不从 detail 改动目录父节点或位置。

Notes 父布局保留 T30 draftStore 与首次初始化 DTO。Query 清空、失效或后台较新 Note 都不会卸载已初始化编辑器、替换草稿或偷偷推进 CAS。标题和文档属于同一个 draft；NoteEditor 的 key 只有 noteId:resetEpoch。页面挂载真实 T28 菜单、T29 图片/代码控件及 T32 标签/资料关联。历史预览使用独立 NoteEditor，真正设置 `editable=false`，不挂可写工具栏、菜单或上传控件；快照列表直接消费已有正文与游标，无额外单快照接口。

## 命令与离开协议

- 当前笔记 MOVE 使用 `runStructuralMutation`：先保存/历史，取得 flush 后实际版本，再发 MOVE；仅自身相同内容确认推进基础，保留编辑器实例。失败保留原树、选择和草稿。
- 历史恢复整个请求置于 `restoreFromServer` 回调屏障，CAS 取最新基础。成功同时替换 title/document/base 并递增 epoch；409/503 保留原稿，不提前装载预览。
- DELETE 先 flush，再同步读取确认版本并发请求；204 后保存独立 receipt，恢复版本为删除请求版本 + 1。导航到 `/notes` 仅一次绕过已成功删除会话的第二次 flush。旧会话结束；恢复取返回 Note 新开会话。原父恢复 409 时明确提供 `{restore_parent:{parent_id:null}}` 的根目录恢复。未调用尚未实现的 `/trash`。
- Notes 只有一个 mounted `useBlocker`，统一处理树选择、Sidebar、前进后退及普通离开；等待保存/历史时实际路由与选择不变，失败留稿。浏览器关闭仍仅为 T30 best-effort，未宣称异步卸载可靠送达。
- 退出先安全离开 Notes，同步读回 router 实际 pathname 后才 POST logout；失败不撤会话、不清缓存。锁定 Router 的阻止导航 Promise 可能保持 pending，因此 Notes 的最小 React Context 只通知现有 Shell“已阻止”，使本次退出 reject 并恢复可重试状态；没有第二套 guard/store/transport，每次 reject 通知在 finally 清理。

## 401 与 T30 最小接线补充

Notes 生命周期在同一公开 api.raw 上注册/卸载 response middleware，发现 401 即暂停本 workspace 写入。已有本地会话保留 title/document/editor，展示复制、新标签页登录和明确重试入口；服务器 Query 仍按既有策略清除。首载无会话（包括 `/notes` index 的树 401）沿既有流程去登录。

T30 仅增加可选 `assertCanWrite(noteId)`，在实际 PATCH、history POST、copy POST、own MOVE 与 history restore callback 发包前重新检查；已在途确认仍正常处理。DELETE 在 flush 后、实际请求前也重新检查。写入不确定标志 `unresolvedWrite` 继续独立于 dirty；未实际发包的 guard 拒绝不会制造一次未确认写入。显式重新认证只校验 session 并重试原 CAS，不采纳新的 Query base。已确认删除通过 `draftStore.forget` 结束旧会话，恢复必须重新初始化。

## 实际验证

全部命令在本任务 Worktree 执行，使用 `TMPDIR=/data/knoted-dev/tasks/t31/tmp XDG_CACHE_HOME=/data/knoted-dev/tasks/t31/cache`。原始输出存于 `/data/knoted-dev/tasks/t31/`。

| 命令 | 实际结果 | 日志 |
| --- | --- | --- |
| `pnpm install --frozen-lockfile --store-dir /data/knoted-dev/tasks/t31/pnpm-store` | exit 0，无锁文件变化 | `install.log` |
| 基线 `pnpm --dir apps/web run test` | exit 0，11 文件 / 158 tests | `baseline-test.log` |
| `pnpm --dir apps/web exec vitest run tests/tasks/t31.test.tsx` | exit 0，31/31 | `focused-final5.log` |
| `pnpm --dir apps/web run lint` | exit 0 | `lint-final5.log` |
| `pnpm --dir apps/web run typecheck` | exit 0，两个 tsc 配置 | `typecheck-final5.log` |
| `pnpm --dir apps/web run test` | exit 0，12 文件 / 189 tests | `test-final4.log` |
| `pnpm --dir apps/web run build --outDir /data/knoted-dev/tasks/t31/dist --emptyOutDir` | exit 0，3257 modules | `build-final5.log` |

有效行为 RED：未注册 Notes 页面时 5/5 失败（`red-initial.log`）；目录孤儿结果与被阻止的退出挂起（`red-expanded.log`）；等待 history 时关联 401 后仍发 DELETE（`red-dispatch-guard.log`）；保存后目录标题不更新（`red-tree-title.log`）。这些修复后的完整场景均包含于最终 31/31 和 189/189。另有移除实际 PATCH guard 的反向回归核验 `red-patch-pause-probe.log`：明确观察错误发出 PATCH，恢复 guard 后最终通过。测试只有网络层 fixture，不替换真实 Router、QueryClient、ApiClient、Tree、HistoryPanel 或 NoteEditor；浏览器 back 用公开 browser history，未误用绕过 POP blocker 的 memory history 来冒充浏览器验证。

31 个本卡场景覆盖至少 51 笔记跨页真实分支、失败重读、MOVE 503/409 不漂移、fresh CAS 与 DOM 稳定、只读预览和历史游标、恢复成功/失败、父子删除门槛及删除后根恢复新会话、mounted 导航/Sidebar/back、退出阻止后重试、首载 401、已有会话 401、在途 PATCH/history 后的停写、明确重新认证原 CAS 重试、缓存移除与后台版本提示、真实业务控件接线及确认标题更新。原 T20 鉴权/安全回跳、T27—T30 和 T32 回归保留；仅把 T20 的“笔记仍禁用”改为已开放事实。

## 限制与后续

- Rust 未变；Rust 必需门禁、独立审查、远程操作、固定 SHA 主线/HTTPS 验收与清理由主 Agent 执行。当前未执行 dev 写入或部署。
- Vite 保留大 chunk 提示：共享编辑依赖约 629 kB、Notes lazy chunk 约 860 kB；构建成功，不在本卡扩展性能拆包任务。
- jsdom 没有真实布局/系统 IME；既有编辑器回归与本卡 DOM 只读/实例断言不替代主 Agent 的真实浏览器消费证据。菜单和图片交互仍须在最终 HTTPS 页面验收。
- mtone 精确发布版本为 0.3.0，核查并消费公开 Tree、Dialog、ConfirmDialog、Select、Button、Input 等能力；属于业务组合，无已证实通用能力缺口，未改上游或开重复需求。
