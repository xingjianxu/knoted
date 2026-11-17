# T27 FR-1 剪贴板输入保护

范围：T27—T35 最终审查唯一发现 FR-1；产品基线 `40e9c3db70ee481337f6588472f6e60cf7bba872`，Issue #84。仅修复编辑器业务输入边界，父协调 Agent 负责 PR、集成、主线及 dev 验证、清理。

## 方针与实施检查清单

- [x] 完整读取最终审查、九卡计划、根规则与 ADR 0002，确认错误来源。
- [x] 真实 NoteEditor 和 NotesPage 回归测试先观察 RED。
- [x] 提交前拒绝不可序列化的整次事务，显示明确提示；不改变原文/选区，不静默丢链接或文字。
- [x] 真实 Chromium 1254 原生剪贴板 + Ctrl+V 验证非法输入拒绝、后续输入可保存。
- [x] NotesPage 真实 Router/客户端/编辑器证明复制、导航 flush、A→B→A 不漏稿。
- [x] focused 回归及 pnpm typecheck/lint/test/build 留存原始输出。
- [x] 自查完成，修复与回归作为一个提交交付父 Agent；独立 scoped re-review 和集成由父负责。

Ruling: 最终使用 BlockNote 0.57.0 已发布公开 `editor.onBeforeChange` 的 `tr.doc` 实际候选树，遍历链接 marks 并复用同一 `safeHref` 校验器；失败 `return false` 拒绝整次事务。仅导出原应用 helper，校验逻辑和 strict URI / NoteDocument 契约未变，无依赖/API/存储/跨模块接口变更。`tr.docChanged` 区分实际内容变更和 selection-only；删除内容成功后清提示。代价是非法粘贴仍需用户修正重试，剪贴板不修改，不自动退化链接。

首次提交 `a4399dd` 使用 `getChanges()` 转换候选 Block DTO，这在新块临时无 ID、UniqueID append 尚未运行时会抛错；集成验收发现后，本波继续纠正为上述原始候选树校验。禁止把正常无 ID 新块笼统 catch 后放行；URI 校验覆盖新节点，同样保留原文/选区/后续草稿。

上游依据：[事件与取消事务](https://www.blocknotejs.org/docs/reference/editor/events)、[粘贴处理](https://www.blocknotejs.org/docs/reference/editor/paste-handling)，并核实安装 0.57.0 的公开 `onBeforeChange({tr})`、`pmSchema` 类型及事务实现；不使用私有编辑器字段、私有 metadata、独立扩展或 patch。

## 首次提交验证证据（历史）

原始记录位于 `/data/knoted-dev/tasks/t27-t35/final-fix/`，完整命令与输出见 `/data/knoted-dev/tasks/t27-t35/final-fix-report.md`。本次新增 5 个 Web 回归，首次提交全量 **197 passed / 0 failed**（集成后的最终结果见下节）；Rust 检查由父协调 Agent 执行，不把旧基线结果冒充本修复的新结果。

| 验证 | 实际结果 | 原始输出 |
|---|---|---|
| 最终版本 covering tests 临时换回基线编辑器 | 3 failed，重复 fragment 粘贴出现 `链接地址无效`；危险 scheme 命令出现 `链接协议不受支持`，提示缺失 | `red-final-covering.log` |
| 删除反馈回归 | 旧 guard 在 delete-only 编辑后警告残留 RED；区分无变化/删除后 GREEN，selection-only 保留提示 | `red-feedback.log`、`green-feedback.log` |
| 同样的原生 ClipboardItem + Ctrl+V 基线 | 可见 `中文链接`，保存 JSON 不变，页面异常，拒绝提示超时 | `red-browser.log` |
| T27/T30/T31 focused | 109 passed / 0 failed | `green-t27-t30-t31.log` |
| 原生 NoteEditor 剪贴板 | 整次非法粘贴拒绝、JSON/原文不变；后续键盘输入序列化；显式 Ctrl+Shift+V 可保留链接文字为纯文本，零页面异常；若 text/plain 自身是非法完整 URL，默认自动链接仍被拒绝，提示明确只建议粘贴链接名称文字 | `green-browser.log` |
| 原生 NotesPage 完整链路 | 原文/DOM 选区保持，后续键盘输入可复制；有效编码 URL 原样保存；导航 PATCH + bodyless snapshot，A→B→A 重开包含新稿，零页面异常 | `green-notes-browser.log` |
| `pnpm typecheck` | exit 0 | `typecheck.log` |
| `pnpm lint` | exit 0 | `lint.log` |
| `pnpm test` | 12 files，197 passed，exit 0 | `test.log` |
| `pnpm build` | exit 0；保留既有 chunk-size warning | `build.log` |

组件回归使用真实 NoteEditor、真实 NotesPage/Router/API client，仅 HTTP 层是隔离 fixture。jsdom 只补齐缺失 ClipboardEvent/Range 平台能力，未 mock 编辑器或替换 serializer；最终 Chromium 原生剪贴板证明不依赖 jsdom 的 URL 行为。重复 fragment 实际输入是 `<p><a href="https://example.com/#part#two">中文链接</a></p>`，原生 NotesPage 额外带前后缀证明整次拒绝。有效输入是 `https://example.com/%E4%B8%AD#part%23two`，逐字保留 href。

T27 回归同时覆盖选区、原有文字、后续输入、有效 URL、undo/redo、危险 scheme 公开命令拒绝、selection-only 保留提示与 delete-only 清提示、非法完整纯文本 URL 的明确拒绝及名称文字恢复；原有适配器 strict URI 检查未改。T30/T31 原有 single-flight、CAS、unknown-write、正常保存后 DOM/selection、只读历史、A→B→A 草稿和 401 保护测试均在本次全套中通过。

边界：没有执行 dev/数据库/宿主配置/远程操作、没有变更后端/API/存储/依赖锁、没有重建通用 UI。真实 NotesPage 浏览器测试使用新增 `apps/web/tests/fixtures/t31-clipboard/` 和 Playwright HTTP fixture，不能冒充 dev HTTPS/真实后端验收。OS 人工中文 IME 和性能测试仍不在本次声明范围。构建 chunk 提示保留，不扩展本修复处理。

Ruling: 保持 BlockNote 默认 text/plain→Markdown/自动链接行为，不增加粘贴 hook 或改变 links 验证语义。原生探测证明完整的非法 URL 即使 Ctrl+Shift+V 也会被自动转链接并拒绝，因此把恢复提示收窄到“修正链接后重试；也可仅以纯文本粘贴链接名称文字”。这不是承诺任意 URL 可作为字面纯文本粘贴；无效输入持续整次拒绝，原文和后续有效输入仍受草稿保护。原始探索失败见 `plain-url-probe.log`，最终明确方针回归见 `green-browser.log` 与 T27 对应测试；不隐藏这个上游行为边界。

## 集成验收续修：临时无 ID 新块

产品基线 `8808c9f0573a2ba0027251afc7542598a6b04c39`；父 Agent 的真实 HTTPS 全笔记流程两次出现 `Node blockContainer does not have an ID`，尽管 14 个业务 flags 通过，验收仍失败。原生产 clipboard 链路通过不能覆盖这个正常 Enter 建块错误。本节是同一 FR-1 修复波的集成续修，不把第一次提交等同最终验收。

有效 RED：真实 mounted NoteEditor 的普通 Enter、新增两段 HTML paste 均在 `getChanges()` 读取临时无 ID 节点时抛错；2 tests failed / 2 unhandled errors。Chromium 1254 physical Enter 与原生多块 ClipboardItem + Ctrl+V 同样抛出完整同栈；codeBlock 实际选区通过 Control+End 确认后的 Shift+Enter 退出在基线已正常，保留为兼容覆盖，不冒充第三个失败。错误未由后端或 fixture serializer 产生。

最终 GREEN 与命令原始输出均在 `/data/knoted-dev/tasks/t27-t35/final-fix/integration-followup/`：

| 验证 | 结果 | 原始输出 |
|---|---|---|
| mounted NoteEditor Enter / 多块 paste RED | 2 failed / 2 errors，根因为候选临时 ID | `red-component.log` |
| native Enter / 多块 paste RED | 两项实际 pageerror；真实 code exit 兼容正常 | `red-browser.log` |
| T27 整文件 | 27 passed | `green-t27.log` |
| T27/T30/T31 | 111 passed | `focused.log` |
| typecheck / lint / build | 全部 exit 0；保留既有 chunk warning | `typecheck.log`、`lint.log`、`build.log` |
| 全量 Web | 12 files，199 passed / 0 failed | `test.log` |
| native 正常建块 + 非法新块 | Enter / code exit / 多块有效 paste 均真正新增可序列化块；非法第二新段整次拒绝，后续正文正常；全部零 pageerror | `green-browser.log` |
| 原 FR-1 native | 重复 fragment 拒绝、正文/选区、后续输入、label-only 纯文本恢复、完整非法 raw URL 明确拒绝；零 pageerror | `green-fr1-browser.log` |
| NotesPage native | 拒绝非法 paste 后实际键盘输入 + Enter 新段进入复制/保存/导航 flush/bodyless snapshot/A→B→A 保留；有效 URI 原样保存；零 pageerror | `green-notes-browser.log` |

本节新增 2 个持久回归，当前累计比最初基线 192 增加 7 个至 199。覆盖新块 ID 最终有效且唯一，以及非法链接出现在尚无 ID 的第二新段时也不能绕过保护。没有改后端 URI 或弱化任何旧断言。父负责后续 Rust 门禁、PR/主线/实际 HTTPS 完整流程重验与清理；本工作区不执行远程或 dev 操作。完整追加报告与提交回执仍在 `/data/knoted-dev/tasks/t27-t35/final-fix-report.md`。

## 主 Agent 最终真实 HTTPS 回执

PR106/107→代码主线/运行`b801d80e90cb7446be536c7bbc6f9ec4884c43d5`；父Agent主线fmt/test/clippy及Web typecheck/lint/test/build全exit0，229Rust/199Web/1既有ignored。三个实际生产HTTPS测试（无API拦截）全部exit0：

```json
[
  {
    "case": "clipboard",
    "revision": "b801d80e90cb7446be536c7bbc6f9ec4884c43d5",
    "exit": 0,
    "log": "/data/knoted-dev/tasks/t27-t35/evidence/final-clipboard-live.log"
  },
  {
    "case": "notes",
    "revision": "b801d80e90cb7446be536c7bbc6f9ec4884c43d5",
    "exit": 0,
    "log": "/data/knoted-dev/tasks/t27-t35/evidence/final-notes-live.log"
  },
  {
    "case": "uppercase",
    "revision": "b801d80e90cb7446be536c7bbc6f9ec4884c43d5",
    "exit": 0,
    "log": "/data/knoted-dev/tasks/t27-t35/evidence/final-uppercase-live.log"
  }
]
{"result":"passed","phase":"undo-redo","revision":"b801d80e90cb7446be536c7bbc6f9ec4884c43d5","clipboard":"native","checks":{"invalid_paste_rejected_with_feedback":true,"subsequent_typing_saved":true,"raw_invalid_plain_URL_explicitly_rejected":true,"plain_label_recovery_saved":true,"copy_contains_visible_content":true,"navigation_and_reopen_preserve_content":true,"valid_encoded_link_saved":true,"undo_redo_editor_stable":true,"no_pageerrors":true,"passkeys_unchanged":true},"cleanup_ok":true}
```

完整Notes输出`final-notes-live.log`包含page_errors=[]，uppercase输出`final-uppercase-live.log`为passed；所有ownfixture清理true，Passkey计数不变。current和API/Worker实际可执行路径均读回同SHA，迁移10，一致备份20261117T083355Z-t27-t35。修复worktree及两条已合并本地分支已安全清理，Issue84关闭/Projects Done。原生OS中文IME人工测试不在本回执内。
