# T30 自动保存与冲突草稿证据

- Issue：[＃88](https://github.com/xingjianxu/knoted/issues/88)。基线：`7bf185dd5f1e300390ff208c78a40fef90104308`（已含 T25、T29、T34）。
- 分支/工作区：`feat/t30` / 主根 `.worktrees/knoted-t30`。本卡仅交付独立编辑会话与业务对话框；页面与路由接线属于 T31。
- 状态：实施与作者自审；主 Agent 独立审查、Rust 门禁、PR、主线验证与集成待完成。未部署，不宣称 T31 页面验收。
- 临时产物、依赖 store、缓存、日志均在 `/data/knoted-dev/tasks/t30/`；冻结锁文件安装，无新依赖、HTTP 或迁移。

## 行为与消费协议

`createNoteDraftStore({ api, queryClient })` 由同一认证编辑外壳通过稳定 React state 创建一次，不能放在随笔记切换卸载的 Editor 内，也不是模块单例。`useNoteAutosave({ initialNote, draftStore })` 返回：

| 接口 | 约定 |
| --- | --- |
| `draft` / `changeDraft(patch)` | 当前内存草稿；编辑器仅传 `{ document }`、标题仅传 `{ title }`。局部更新同步合并，传入 DTO 深复制。 |
| `baseVersion` / `status` / `dirty` / `error` | 逐笔记固定 CAS 基础、保存状态与错误。正常输入停止 1000ms 保存；同笔记只有一个 PATCH，在途新编辑合并为最新稿，旧确认不清新稿。409 和网络/HTTP 失败暂停自动保存；显式 `retrySave()` 不采纳错误里的 currentVersion，409 仍需冲突处理。 |
| `serverUpdated` | 后台 Query 更新只提示；即使 clean 也不自动改草稿、基础版本或编辑器输入。Query 仅接收确认的服务器 Note，已有更高版本缓存不回退。 |
| `initialDocument` / `resetEpoch` | `<NoteEditor key={noteId + ':' + resetEpoch} document={initialDocument} ... />`。key 不含版本/状态；保存和 Query 更新不重建编辑器。 |
| `flush(): Promise<Snapshot>` | 绑定原笔记会话，取消 debounce、串行排空保存，再无 body POST snapshots；同时调用共用 barrier。失败拒绝，正常应用导航必须 mounted 时 await 成功；保存已成功而历史失败时重试仅 POST snapshots。快照版本不等于自身确认基础或期间再编辑时拒绝，绝不声称 dirty 离开成功。 |
| `loadServer(): Promise<Note>` | 暂停自动写、等待已有写/结构操作，再以真实 GET/no-store 加载。加载期间有新编辑则拒绝；成功整体替换草稿/确认值/base 并递增 resetEpoch。 |
| `restoreFromServer(command): Promise<Note>` | T31 历史恢复必须用此覆盖整个恢复请求：先 flush 保存和历史，保持暂停 PATCH，再 `command(noteId, baseVersion)`。等待旧写后才调用恢复；响应期间若有新编辑，保留新稿/旧 base、拒绝并标 conflict，不能装入恢复版本或自动重试。成功原子安装草稿/base/新 resetEpoch。网络或 409 保稿暂停。 |
| `resetFromServer(note): Promise<Note>` | 将已确认的服务器 Note 原子安装为同一笔记的新编辑 epoch；等待已有写，拒绝期间的新编辑。适用于已确认的外部整体重载；历史恢复使用上面的 callback barrier，不能先独立发恢复请求再仅升级 base。 |
| `runStructuralMutation(command): Promise<Note>` | 自己的 MOVE 专用：`await flush` → 暂停新 PATCH → `command(noteId, baseVersion)` → 校验返回 id/title/document 等于确认稿 → 只更新 base/服务器缓存，不变 resetEpoch 或当前草稿。期间编辑保留为 dirty，结束后按新 base 串行保存；失败保留旧 base 并暂停。并发不同命令拒绝 busy，不能把未执行的命令当成功。不能用于加载不同历史内容。 |
| `saveCopy(title?): Promise<Note>` | 捕获调用时本地草稿，独立根 `parent_id:null` POST；默认标题含后缀按 Unicode 代码点限 500。成功返回新 Note 并更新其服务器缓存，原稿/冲突保留，默认不跳转。 |
| `copyDraft(): Promise<void>` | 用户动作中复制本地 `{ title, document }` JSON；失败拒绝，成功或失败均不清草稿。 |

`ConflictDialog` 直接组合锁定 npm `@bixlab/mtone@0.3.0` 公开 Dialog/Input/Button/Alert。动作由 props 注入并捕获失败，pending 时禁重复操作；关闭/Esc 仅隐藏对话框，不能解除 store 的 conflict。T31 切笔记时可按 noteId key 对话框，以独立保留副本标题输入。

浏览器 `beforeunload` 对 dirty 同步提示，`pagehide` best-effort 尝试 flush；不保证页面退出后的请求交付。cleanup 仅同步解除订阅/事件/计时器，原 store 可结算所属在途请求。无 localStorage、sendBeacon 或异步 cleanup。

## 验证

运行前缀统一为 `TMPDIR=/data/knoted-dev/tasks/t30/tmp XDG_CACHE_HOME=/data/knoted-dev/tasks/t30/cache`，在本任务 worktree 执行。日志目录同上，报告补全实际命令、退出码与提交 SHA。

- RED：`pnpm --dir apps/web exec vitest run tests/tasks/t30.test.tsx`，缺模块 exit 1；`red.log`。首轮两个 controller 用例 GREEN 后扩充真实客户端边界、错误、flush、切笔记、重载与真实组件测试。
- MOVE RED：`red-structural.log`（缺方法）、`red-concurrent-move-identity.log`（并发命令被错误共用），对应 GREEN 已观察。
- 自审 RED：`red-self-review.log`，复现最终 Query invalidation 期间新编辑仍让 flush 成功，以及 GET 重载在途显式 retry 发并行 PATCH。修复后 `green-self-review.log`：38/38，exit 0。
- 最终专属测试 `focused-final.log`：44/44，exit 0；完整 Web 门禁与最后日志见交付报告；本卡无 Rust 改动，Rust 全门禁由主 Agent 在审查/集成阶段独立运行。
- 真实 `createApiClient` 注入 fetch，检查 PATCH/POST 路径、请求体、same-origin credentials、CSRF、中间件抛出的 ApiRequestError；Deferred 构造真实串行竞态，不制造同 note 两个非法在途 PATCH。
- 真 NoteEditor/BlockNote 验证普通保存、Query、结构版本确认保留实例与选区；显式 reset 才重建。合成 composition 仅回归，不声称 OS IME 验收。
- 真实 mtone Dialog/Input/Button/Alert 验证失败反馈、busy、Esc/重开保留冲突。所有 fake timers / stub globals 恢复；未增加 timeout。

- 按 T31 交接补充恢复 callback barrier，`red-restore.log` 观察缺方法失败，`green-restore.log` 观察旧 PATCH → snapshot → restore 串行成功及期间新编辑拒绝的 GREEN。

最终 Web 门禁（全部 exit 0）：

```text
pnpm --dir apps/web exec vitest run tests/tasks/t30.test.tsx
Test Files  1 passed (1)
Tests  44 passed (44)
pnpm --dir apps/web run lint
$ eslint src tests vite.config.ts
pnpm --dir apps/web run typecheck
$ tsc --noEmit && tsc --noEmit -p tsconfig.node.json
pnpm --dir apps/web run test
Test Files  11 passed (11)
Tests  153 passed (153)
pnpm --dir apps/web run build
✓ 3134 modules transformed.
✓ built in 771ms
```

完整原始输出：`focused-final.log`、`lint-final.log`、`typecheck-final.log`、`test-final.log`、`build-final.log`。构建保留既有 618.94 kB chunk 提示，无配置放宽。

## 自审结论与限制

自审完成并修复上述可复现竞态，以及 JSON 字段顺序误判（`red-json-order.log` → `focused-final.log`）；JSONB/传输对象顺序不应视为正文变化；父 Agent 的独立审查未由作者自审替代。新保存模块尚未由生产页面导入，build 验证类型与现有应用构建，完整页面消费与真实 HTTPS 导航验收由 T31 承接。内存草稿在认证编辑外壳存活期间保留，不承诺浏览器重开后恢复。
