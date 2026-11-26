# ADR 0012: 笔记编辑器直接消费 npm `@bixlab/mtone-blockeditor`，替代直接依赖 BlockNote

- Date: 2026-11-25
- Status: Accepted
- Decision owner: 项目维护者（本次会话用户）
- Confirmation: 2026-11-25 当前会话中，用户首先明确要求「升级依赖mtone到最新版。安装@bixlab/mtone-blockeditor，使用这个控件，代替本项目中直接使用的blocknote。@bixlab/mtone@0.15.2 @bixlab/mtone-blockeditor@0.2.0」（已通过 [#286](https://github.com/xingjianxu/knoted/issues/286) / [PR #287](https://github.com/xingjianxu/knoted/pull/287) 实施并合入 `564c821`）；随后针对 ADR 0002 是否需同步更新的问题，确认采用「不改写 ADR 0002 历史正文，新增 ADR 0012 部分替代 ADR 0002 第 4 条并维护双向链接」方案，回复「好的，按你推荐的流程完成」。
- Related issues: [#286](https://github.com/xingjianxu/knoted/issues/286)、[#288](https://github.com/xingjianxu/knoted/issues/288)
- Related PRs: [#287](https://github.com/xingjianxu/knoted/pull/287)（编辑器迁移实现）；本 ADR PR 见 [#288](https://github.com/xingjianxu/knoted/issues/288)
- Supersedes: 部分替代 [ADR 0002](0002-mtone-ui-ownership.md) 第 4 条（BlockNote 编辑器核心与自建 mtone 组合工具栏/菜单边界）；ADR 0002 其余条款继续有效
- Superseded by: [ADR 0013](0013-note-table-persistence.md) 部分替代普通表格的持久化限制

## 背景与问题

[ADR 0002](0002-mtone-ui-ownership.md)（2026-11-16）确立了「通用 UI 由 npm `@bixlab/mtone` 提供，不在 Knoted 建通用库」的归属边界，其第 4 条在当时上游尚未发布独立编辑器包的前提下约定：「BlockNote 只保留编辑核心，关闭默认 UI，使用 mtone 组合工具栏/菜单；编辑器状态与选区适配仍在 Knoted」。

随之，Knoted 在 `apps/web` 直接依赖 `@blocknote/core` 与 `@blocknote/react`，并在 `apps/web/src/features/notes/editor/` 维护了自定义 `schema.tsx`、`formatting-toolbar.tsx`、`menus/*` 与 `blocks/*` 等约 400+ 行通用块编辑器 UI 封装。

2026-11-25，mtone 上游已发布独立官方控件包 `@bixlab/mtone-blockeditor@0.2.0`（配套 `@bixlab/mtone@0.15.2`），内置基于 mtone 的选区格式栏、斜杠菜单、块侧边操作菜单、代码块 Shiki 高亮与语言选择器、图片编辑/上传表单与只读呈现。继续在 Knoted 维护第二套直接对接 BlockNote 的编辑器 UI 会带来重复建设、样式与交互分叉，以及升级 BlockNote/mtone 时的双倍维护成本。

## 决策内容

1. **直接消费精确锁定的 npm `@bixlab/mtone-blockeditor`**：`apps/web` 直接依赖精确锁定的 `@bixlab/mtone-blockeditor`（当前为 `0.2.0`，与 `@bixlab/mtone@0.15.2` 配套）并消费其公开 `BlockEditor` 控件、公开类型（`BlockEditorBlock`、`BlockEditorInput`、`BlockEditorProps`）及 `@bixlab/mtone-blockeditor/styles.css`（置于应用 CSS 的 `mtone` 层）。
2. **移除对 `@blocknote/*` 的直接依赖与自建编辑器通用 UI**：`apps/web/package.json`、`apps/web/src/` 与 `apps/web/tests/` 不再直接依赖或导入 `@blocknote/core` / `@blocknote/react`，不再在 Knoted 维护自定义 BlockNote schema、格式工具栏、斜杠菜单、块操作菜单或代码块/图片通用控件（`@blocknote/*` 仅作为 `@bixlab/mtone-blockeditor` 的上游内部传递依赖存在）。
3. **Knoted 保留业务契约与数据边界**：
   - `apps/web/src/features/notes/editor/document-adapter.ts` 保留 `NoteDocument` v1 双向转换与安全校验（7 类持久化块白名单、5 种行内标记及 `BlockEditor` 默认 `code` 互斥标记的无损兼容、严格 ASCII URI 拼写、5000 块 / 8 层嵌套限制、图片 `asset_id` ↔ 同源 `/api/v1/assets/{id}/content` 双向映射）。
   - `apps/web/src/features/notes/editor/image-upload.ts` 通过 `BlockEditor` 的 `uploadImage` 回调对接 `POST /api/v1/assets`（`Idempotency-Key`、`state === "ready"` 校验、拒绝已回收资产、在 `documentKey` 切换/卸载/只读态切换时中止在途上传）。
   - `apps/web/src/features/notes/editor/note-editor.tsx`、`NotesPage.tsx` 与 `HistoryPanel.tsx` 保留草稿隔离、自动保存/冲突恢复、历史只读预览、导航拦截及对后端暂不支持块类型/外部图片地址/非法链接的中文告警拦截（不将不合法 DTO 传入自动保存或后端）。
   - 不把后端 DTO、业务接口、权限或持久化逻辑送入 `@bixlab/mtone-blockeditor`。
4. **沿用既有升级与缺口治理规则**：仅使用 npm 官方源正式发布版本作为正式依赖，禁止私有路径导入、复制源码、patch 或未发布 tarball 交付；通用编辑器能力缺口按 [UI 开发指南](../development/ui-mtone.md) 与 [mtone-component-request](../../.agents/skills/mtone-component-request/SKILL.md) 向上游交接。

## 备选方案

| 方案 | 收益 | 成本与结论 |
|---|---|---|
| 继续在 Knoted 直接依赖 `@blocknote/*` 并维护自建工具栏/菜单 | 完全控制 BlockNote schema 与 `onBeforeChange` 事务拦截 | 重复维护通用编辑器 UI，无法直接复用上游持续演进的格式化、Shiki 高亮、菜单与无障碍能力；违背 UI 归属方向，不采用 |
| 在采用 `@bixlab/mtone-blockeditor` 的同时扩张后端 `NoteDocument` v1 schema | 直接持久化表格、引用、提示块等扩展块类型 | 属于跨模块存储与 API 契约变更（影响数据库校验、纯文本提取、全文/向量索引与 OpenAPI），超出当前依赖迁移范围；暂不纳入本次决策，后续有明确产品需求时再单独立项评估 |
| 消费 `@bixlab/mtone-blockeditor` 并在 Knoted 保持 `NoteDocument` v1 边界适配 | 统一通用编辑器 UI 归属、移除直接 `@blocknote/*` 依赖，同时保持后端 v1 契约零破坏与零迁移成本 | 上游编辑器支持的扩展块及外部图片 URL 需由前端适配器拦截告警，且 `code` 标记需在加载/保存边界做无损兼容；综合收益最高，采用 |

## 决策理由

- 与 [ADR 0002](0002-mtone-ui-ownership.md) 的核心原则一致：通用交互控件归 mtone 生态维护，业务契约与数据边界归 Knoted。
- `@bixlab/mtone-blockeditor@0.2.0` 已在 [#286](https://github.com/xingjianxu/knoted/issues/286) / [PR #287](https://github.com/xingjianxu/knoted/pull/287) 中通过完整本地门禁（451 项 Vitest、124 项真实 Chromium Playwright E2E）及开发实例部署验收，事实证明其公开 API（`documentKey`、`initialContent`、`onChange`、`readOnly`、`locale`、`uploadImage`）可满足 Knoted 当前笔记读写、图片资产上传与只读历史预览需求。
- 保持后端 `crates/knoted-core/src/notes/document.rs` 契约不变，避免了无必要的存储迁移与跨模块接口风险。

## 影响

- **正面**：彻底移除 Knoted 对 `@blocknote/core` / `@blocknote/react` 的直接依赖与约 400+ 行自建编辑器控件代码；编辑器外观、键盘/无障碍交互与 Shiki 代码高亮统一跟随 `@bixlab/mtone-blockeditor` 演进。
- **负面与边界约束**：
  - `@bixlab/mtone-blockeditor@0.2.0` 不暴露底层 `editor` 实例与 `onBeforeChange`，非法链接、外部图片地址及后端 v1 暂不支持的块类型（表格、引用、分割线、折叠块、提示块、视频、音频、附件、4–6 级标题）需在 `onChange` 的 `fromEditorBlocks` 阶段拦截并以中文告警提示，不污染已保存草稿。
  - `@bixlab/mtone-blockeditor@0.2.0` 使用 BlockNote 默认的互斥 `code` 标记（`excludes: "_"`），`document-adapter.ts` 需在加载时做兼容归一化并通过 sidecar 比较保留未编辑块的原始多标记数据。
  - 前端构建产物新增 `shiki` 语法按需分块（本地静态产物，无外部 CDN 或远程字体请求）。
- **迁移与回退**：实现迁移已随 [PR #287](https://github.com/xingjianxu/knoted/pull/287) 完成，无数据库 schema 或 API 迁移。若未来上游新版本出现消费回归，可回退至上一个已验证的 npm `@bixlab/mtone-blockeditor` 精确版本；不自动退回在 Knoted 自建 BlockNote UI。

## 验证与后续任务

- 已完成实现与验收证据：[#286](https://github.com/xingjianxu/knoted/issues/286)、[PR #287](https://github.com/xingjianxu/knoted/pull/287)（合并 SHA `564c8211591ccd053e36cce30cf3df4a586861ab`，已上线 dev 实例 `https://wlab.132140.xyz:15601/`）、[任务卡 #286](../tasks/286.md) 与 [UI 开发指南](../development/ui-mtone.md)。
- 后续若计划在后端持久化表格、引用、提示块等扩展块类型，或要求上游提供块白名单/只读约束能力，需分别走独立的 API/存储架构评估或 [mtone-component-request](../../.agents/skills/mtone-component-request/SKILL.md) 流程。

## 状态历史

- 2026-11-25：维护者在会话中确认采用 `@bixlab/mtone-blockeditor@0.2.0` 替代直接使用的 BlockNote（[#286](https://github.com/xingjianxu/knoted/issues/286) / [PR #287](https://github.com/xingjianxu/knoted/pull/287)）并确认以 `ADR 0012` 部分替代 `ADR 0002` 第 4 条，直接记录为 Accepted。
