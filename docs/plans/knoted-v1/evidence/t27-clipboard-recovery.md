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

Ruling: 使用 BlockNote 0.57.0 已发布公开 `editor.onBeforeChange` 的 `getChanges()` 后态，经原有 `fromEditorBlocks` 校验，失败则 `return false` 拒绝整次事务并提示“未应用此次编辑…原内容已保留…修正链接后重试；也可仅以纯文本粘贴链接名称文字”。`editor.document` 在此时仍是旧态，不能用于待提交校验。这个策略保持 strict URI 与 NoteDocument 契约，无存储/API/跨模块接口变更；代价是整个非法粘贴需用户修正后重试，剪贴板本身不修改，不自动退化链接。

上游依据：[事件与取消事务](https://www.blocknotejs.org/docs/reference/editor/events)、[粘贴处理](https://www.blocknotejs.org/docs/reference/editor/paste-handling)，并核实实际安装 0.57.0 的公开类型与事务过滤实现。所有输入走同一提交前边界，包含 paste、drop、命令、undo/redo；无需仅针对 HTML 剪贴板做脆弱的预解析或改动默认粘贴顺序。

## 验证证据

原始记录位于 `/data/knoted-dev/tasks/t27-t35/final-fix/`，完整命令与输出见 `/data/knoted-dev/tasks/t27-t35/final-fix-report.md`。本次新增 5 个 Web 回归，最终全量 **197 passed / 0 failed**；Rust 检查由父协调 Agent 执行，不把旧基线结果冒充本修复的新结果。

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
