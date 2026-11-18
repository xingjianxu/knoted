# T51/T52 OnlyOffice 只读预览 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 从文件详情打开固定版本的原版式 Office 只读预览，退出后不再授权取源。

**Architecture:** T51 用 T49 稳定校验读取和单进程带界副本实现预览授权，签名浏览器 DocEditor 配置并以短时 token 服务 Document Server。T52 复用现有文件详情 Dialog 和构建时信任来源，不改共享代理或其它服务。

**Tech Stack:** Rust 2024/Axum/Tokio/SQLx、OpenAPI + pnpm 客户端、React/Vite/Vitest、mtone 0.4.0、现有 OnlyOffice Docs、pcwrap 固定 dev 产物。

**Spec:** [ADR 0008](../decisions/0008-office-preview-authorization.md)、[RFC](../rfcs/2026-11-18-t51-t52-office-preview.md)、[T51](knoted-v1/tasks/T51.md)、[T52](knoted-v1/tasks/T52.md)。

## Global Constraints

- T51 [#179](https://github.com/xingjianxu/knoted/issues/179) → T52 [#180](https://github.com/xingjianxu/knoted/issues/180) 串行；已核实上游 `32cbb6f`、`8ffb240`、`2c89043` 在基线 `a303192`。主 Agent 串行控制共享文件/审查/集成；写入委派另建合规 Worktree，不充当独立审查者。
- DOCX/XLSX/PPTX；最多两份、每份最多 50 MiB、最长 120 秒；原件与普通下载不改变，不迁移数据库。不读候选用户路径，不接受用户给的目标 URL，不提供编辑、回调保存或跨重启恢复。
- 只配置受信 HTTPS origin/运行凭据路径；不输出密钥、临时 URL/JWT/会话等至日志、测试工件、PR。禁止改共享 Caddy / OnlyOffice、生产、其他应用与来源写权限。
- 任务工作区分别 `.worktrees/knoted-t51`、`.worktrees/knoted-t52`，分支 `feat/t51`、`feat/t52`；TEMP/target/cache/evidence 只在 `/data/knoted-dev/tasks/t51`、`t52`。锁文件/生成入口不随意升级。

## Review Focus

- 来源大于 50 MiB、并发创建 3 个及反复取源：在读入前拒绝超限，不能用撤销/过期释放尚在交付副本的准入；失败和丢弃响应最终清理。
- 创建后原件替换或会话撤销：不读取替换后的字节；已过期/撤销会话不得取源。取源和撤销竞争只允许已获准交付，禁止新的请求复活授权。
- 相同文件在两个会话或两个组件实例打开：不串撤、签名缓存 key 不复用、晚到成功只撤自己的预览。
- 配置地址变更/错误、第三方加载失败：构建和运行信任来源必须相等、未配置为关闭，不跟随异源脚本替代；同一来源失败可重试。
- OnlyOffice 只读功能与现有 PDF Blob 同时可用：真实 Chromium 测 XLSX 表切换、禁止编辑、非信任来源被 CSP 拦截、普通预览无回归；不以模拟 SDK 冒充真实验收。

---

### Task 1: T51 — 后端授权与客户端契约

**Files:** `api/openapi.yaml`、`api/examples/check.mjs`、`packages/api-client/src/generated/schema.d.ts`；`crates/knoted-core/src/files/{office,download}.rs`；`crates/knoted-api/src/routes/{office,mod}.rs`、必要 core config/API 启动接线、`Cargo*.toml`/`Cargo.lock`（仅所需既有版本）、`tools/dev/pc/{service.sh,knoted.toml.tmpl}`（仅可选配置接线）；`crates/knoted-api/tests/t51.rs`、必要的 pc 配置测试。

**Interfaces:**
- `POST /api/v1/files/{fileId}/office-preview`，会话+CSRF，JSON `{expected_content_version: integer >= 1}`；201 `OfficePreview {preview_id: UUID, expires_at: date-time, document_server_origin: uri, config: OfficeEditorConfig}`，409 选定已非可用版本，422 不支持/超限，429 无准入，503 未配置/依赖不可用。配置是 `documentType`（word/cell/slide）、`document`（fileType/key/title/url/permissions）、`editorConfig`（mode/view 及只读选项）、`token`，与原生 SDK 命名一致。
- `DELETE /api/v1/files/{fileId}/office-preview?preview_id={uuid}`，会话+CSRF，仅撤对应文件和创建会话的预览，204 无正文；不存在、已撤/过期用幂等 204，不影响别的授权。
- `GET /api/v1/office-preview-sources/{previewId}?token={opaque}` 给 Document Server；不使用浏览器 cookie，对未知、失效、错误 token 统一 404。成功原始只读副本，按格式 MIME、`Cache-Control: private, no-store` 和 `X-Content-Type-Options: nosniff`。令牌为安全随机的 32 字节 base64url 无填充，只保存哈希；不准任意路径、Range 部分读取或重定向。别将带查询的 URL 放入请求 span、错误体、证据、代理日志。
- `[office]` 启用条件为配置规范的 `document_server_origin` 和 `jwt_secret_file`，缺省关闭；Knoted 对外 origin 沿用经验证的 `auth.origin`。JWT 使用现有底层密码库 HS256；文档 key 每预览唯一、写权限均 false、`editorConfig.mode=view`、禁插件/宏、无 callbackUrl。域层以 `AppContext`/命名参数验证创建、撤销和交付；读取复用 T49，经事务重核选定 `content_version`，在文件分配前应用预览尺寸限额，不影响 T49 下载。

- [ ] **登记契约**：先更新 OpenAPI，并运行 `pnpm api:check` 和 `pnpm api:generate`；确认生成接口符合上文，内部 intelligence schema 不动。
- [ ] **先写入口测试**：`t51_1` 用 `FixedClock` 验证创建后 119 秒可取、120 秒与撤销后 404；`t51_2` 核对未知/篡改 token、跨实例 ID/目录与浏览器会话范围；`t51_3` 原件改后取源字节仍等于创建时，另开旧版本不成功；`t51_4` 两份（含并发/未消费的响应）可准入，第三份 429，释放后可重试；`t51_5` 检查 CSRF、未登录、已撤会话、缺配置、错误格式/超限和签名配置的权限/无回调/TTL，不以弱宏判断只读。
- [ ] **观察 RED**：`cargo test --workspace t51_`，记录端点尚未实现的断言；不可把 PG 不可用、跳过或 0 用例当 RED。
- [ ] **实现**：预留两份内存预算（准入许可随备份或响应所有权释放）；通过已有 FD 安全读取构造缓存条目；不持锁跨 `await` 的会话查询，交付前重核最新授权（撤销、过期、句柄）及会话有效性而不触碰 idle 时间。以过期回收和快速清理形成界限。路由映射统一错误、限制响应缓存；OnlyOffice 未配置不影响其余 API。dev 通过 pcwrap 可选变量配置 origin/凭据绝对路径，凭据不复制到产物。
- [ ] **验证和提交**：专项 T51 和既有 T49，真实运行 `cargo fmt --check`、`cargo test --workspace`、`cargo clippy --all-targets -- -D warnings`、必要 `pnpm api:check`/client lint/typecheck、`pnpm test:pc`；独立 reviewer 后重复受影响检查。用有效 `git-commit`，默认普通 merge，主线验证、固定产物与真实 HTTPS 取源后保留证据并清理。仅可验收后端，不误记 T52 已通过。

### Task 2: T52 — 网页查看与精确来源

**Files:** `apps/web/src/features/files/preview/OfficePreview.tsx`；现有 `FileDetail.tsx`/`preview/inspect.ts`/`preview/FilePreview.tsx` 仅预览分流与资源所有权接线；`apps/web/vite.config.ts` 或单独 Vite 业务配置模块；`apps/web/tests/tasks/t52.test.tsx`，必要固定产物和真实验收脚本接线、任务证据文档。

**Interfaces:** 消费上一任务生成 DTO；`OfficePreview({api, file})` 置于既有受控 Dialog 的 Office 分支。上传或编辑接口不引入。发布构建使用 `KNOTED_OFFICE_ORIGIN` 注入唯一已校验的 HTTPS origin 与可执行的 `script-src 'self' <origin>; frame-src 'self' blob: <origin>; object-src 'none'` 应用 HTML CSP；脚本使用同一配置检查签发结果。必须支持端口的精确匹配；默认禁用 Office，不放行任何额外 URL。一条精确兼容例外若确需，先用实际浏览器定位，不能加任意 inline/eval 或改变共享服务设置。

- [ ] **先写失败测试**：`t52_1` 用假的 DocsAPI 核对 signed `cell` 配置与 viewer 事件，另用真 Chromium 验收工作表切换；`t52_2` 关掉/切换时销毁实例，晚到成功仅 DELETE 该 ID，不创建实例；`t52_3` 测精确协议/主机/端口、恶意或空的 origin、静态 CSP 无通配符并保留 PDF Blob；`t52_4` 脚本/Viewer 初始化失败可重试，循环无重复加载或串撤，401 关闭并沿用登录处理。
- [ ] **观察 RED**：`pnpm --filter @knoted/web exec vitest run tests/tasks/t52.test.tsx`，记录预期失败（不以工具缺失为证）。
- [ ] **实现**：同一生命周期最多拥有一个 SDK 实例和已取得预览 ID；关闭/切换必须同步禁止事件/新渲染、销毁实例、发出幂等撤销。不要仅靠取消 POST 假定服务端未创建；晚到回执仍辨认其所有者并撤销，丢失回执由服务端 TTL 保底。不加载服务端任意 URL，拼接受信 origin 的固定 `/web-apps/apps/api/documents/api.js`，不变更签名配置，仅追加 SDK 的事件/布局；忙/错误/重试复用 mtone。另行可用原件下载。
- [ ] **验证和提交**：专项 T52、T50 回归及 Web 完整 lint/test/typecheck/build，实际启用和未启用构建分别检查 HTML 与 origin；独立审查。真实 Chrome 验收两工作表有可辨内容的 XLSX、DOCX/PPTX 只读、普通 PDF、缓存/撤销与 CSP；保留退出码和实际截图/断言（脱敏）。主线 merge 后部署固定 SHA，只更新 Knoted dev，核对运行 SHA、共享服务不变和自有样本清理，回填任务卡、证据、README 后清理各自 Worktree/已合并分支。
