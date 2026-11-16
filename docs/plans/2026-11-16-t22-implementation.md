# T22 笔记 JSON 与提取规则 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现已授权 T22 的 JSON 校验及保留块定位的纯文本提取。
**Architecture:** core 内实现无 I/O 的文档边界，以命名输入接受格式版本及保存 JSON，严格反序列化为白名单类型，经限额与语义检查后才产生可提取的文档。现有 OpenAPI 是字段权威，T23 接入 HTTP/持久化，本卡不建立服务或改变存储契约。
**Tech Stack:** Rust 2024、现有 serde/serde_json/url/uuid/thiserror；无需新依赖。
**Spec:** [T22](knoted-v1/tasks/T22.md)、[数据/API](knoted-v1/data-and-api.md)、`api/openapi.yaml` NoteDocument/NoteBlock/InlineNode。

## Global Constraints

- `format_version=1`；未知版本返回 422 对应领域错误；超限 413；其他业务字段错误 422。
- JSON 上限 2MiB、5000 块（全部层级合计）、最大嵌套8层（顶层为1）；块 ID 文档内唯一，长度1—64字符。
- 块仅 heading(level1—3)、paragraph、bulletListItem、numberedListItem、checkListItem、image、codeBlock；inline仅text/link；marks仅bold/italic/underline/strike/code。拒绝未知字段，不自动丢弃数据。
- 图片只引用 UUID asset_id；链接只接受显式白名单的绝对 http/https/mailto（不是任意协议）；拒绝可执行HTML节点。text 内的HTML字符作为原文保留，任何渲染逃逸由消费者负责。
- 纯文本保留中文、代码空格/换行/标点，不 trim、不规范化；按前序遍历提取，保留原块 ID 与 children 索引路径。图片不展开 URL；仅其已有文本 content 参与提取。
- 所有写文件工作只在主根 `.worktrees/` 独立分支；不改 UI、路由、迁移、依赖、锁文件、上游项目或其他任务。
- 根 AGENTS.md、Accepted ADR 0001/0002 适用但不受本次变更影响。默认授权远程常规动作；不得修改仓库设置/标签定义/生产，不得绕过 hooks。

## Design and decisions

采用严格 serde 保存结构，而不是手写自由 JSON 遍历：前者可保证未知字段/枚举不被丢弃，后者更易漏验证。既有契约与用户直接委派构成实施依据，按项目工作流不重复索取常规方案许可。使用原始 JSON 字符串作为校验输入以精确验证 UTF-8 字节上限；反序列化错误仅暴露固定摘要与字段路径，禁止在错误中泄露正文。

输出为强类型 `ValidatedDocument`，外部不可任意构造，从而保证 extraction 只接收已验证文档；公开操作遵循 `fn operation(ctx: &AppContext, input: NamedInput) -> Result<NamedOutput, DomainError>`。DomainError 放本模块，不引入跨域错误框架。无数据库 I/O，AppContext 参数仅维持调用约定。

## Review Focus

- 多字节中文在2MiB边界按字节计数；块 ID 按字符限制。
- 第8/9层、总5000/5001块，深嵌套恶意输入不能栈溢出。
- 含大小写/控制字符/相对路径/javascript/data/file的链接须拒绝不安全协议；正常中英文绝对链接可用。
- 未知字段/HTML节点/非法 asset_id 不得静默丢失；错误不得泄露输入正文。
- 嵌套列表、空块和代码换行提取定位稳定，校验失败不改变原始输入，改正后可重新校验。

### Task 1: 文档校验与提取（单一实施 Agent）

**Files:** 新建 `crates/knoted-core/src/notes/document.rs`、`crates/knoted-core/src/notes/mod.rs`、`crates/knoted-core/tests/t22.rs`、`tests/fixtures/notes/*.json`；仅为注册修改 `crates/knoted-core/src/lib.rs`。
**Inputs:** `AppContext`、T03已合入NoteDocument schema；T05上下文/测试基础已在基线0718e66中。
**Produces:** `validate_document(&AppContext, ValidateDocument) -> Result<ValidatedDocument, DomainError>`，输入 `ValidateDocument { format_version: u32, document_json: String }`；`extract_plain_text(&AppContext, ExtractPlainText) -> Result<PlainTextDocument, DomainError>`，输入持有已验证文档；输出总文本及每块的 `block_id`、索引 `path`、`text`。输出类型可补充只读访问器。结构命名与 serde 字段遵循契约。
**Dependencies / parallel:** 单任务串行；实施 Agent 使用独立 `.worktrees/agent-t22-implementation` 与 `feat/t22-implementation`；主 Agent 持有 `.worktrees/knoted-t22` 与 `feat/t22`，只在实施提交后合入代码。

- [ ] 先写fixture及真实场景测试，建立仅签名/未实现的最小接口，观察断言失败，保存 RED 输出；不可只把编译错误作为行为RED。
- [ ] 实现白名单结构与严格反序列化；版本、总字节、块 ID、层级、块计数、heading level、image asset_id、inline与协议语义校验。
- [ ] 实现纯文本前序提取，块之间用一个换行分隔，块内content按序连接，不改变原文字节；记录所有块包括空块的定位。
- [ ] `t22_1`未知版本422、`t22_2`重复块ID、`t22_3`代码/中文原文保留，外加以上 Review Focus 的正反边界、未知字段和fixture回归；记录 GREEN。
- [ ] 运行 `cargo fmt --check`、`cargo test --workspace --locked t22_`、`cargo test --workspace --locked`、`cargo clippy --locked --all-targets -- -D warnings`。日志集中 `/data/knoted-dev/tasks/t22/logs/`，缓存与临时文件同任务目录。
- [ ] 实际重读 git-commit Skill、检查敏感信息与 diff 后按项目中文提交；写完整实现报告，禁止自行 push、开PR、合并或修改主 Agent 的计划文件。

### Integration checklist（主 Agent）

- [x] 分析任务、依赖与授权；Issue查重；独立 Worktree；记录基线0718e66。
- [x] 基线全工作区测试确认（exit 0，日志见本任务 logs/baseline-test.log）。
- [ ] Task 1 已派发并提交；读取实现并独立复跑验证。
- [ ] 请求独立审查，核实反馈并修复；完成分支最终审查。
- [ ] 更新T22状态与执行证据，纯库且尚无调用入口无需部署。
- [ ] 通过必需本地门禁、提交、push、按模板创建PR并普通merge；远程Issue由关闭关系处理。
- [ ] 主线快进、复跑必要验证，记录SHA，安全清理两个任务Worktree/已合并分支；读回Issue/Projects状态。

## Execution ledger

- 2026-11-16：原生 Worktree 工具仅接受ref且不支持强制路径，依据项目固定路径约束使用git worktree。原生TODO工具不可用，本文件清单为持久化进度。
- 基线：0718e66；T03实现1dfd5b8与T05实现7805c6d已包含主线（PR #4）；Issue #57。
