# T23 笔记 CRUD 与版本 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成 T23 的笔记 CRUD、版本 CAS、事务 outbox 与测试实例交付。

**Architecture:** 沿用 T03 的 HTTP DTO 与 T22 文档校验，service 持有事务，repository 集中 SQL。新增 0005 notes 迁移，笔记实体注册、更新和后台索引通知原子提交；handler 尚未实现时仅保留 outbox，避免 Worker 消费未知 kind。HTTP 复用会话/CSRF与统一错误，内容版本只由服务端维护。

**Tech Stack:** Rust 2024、SQLx/PostgreSQL、Axum，现有依赖，不加 UI。

**Spec:** [T23](knoted-v1/tasks/T23.md)、[数据/API](knoted-v1/data-and-api.md)、[OpenAPI](../../api/openapi.yaml)、[部署约定](knoted-v1/development-and-deploy.md)。

## Global Constraints

- 仅 T23，单实施者串行；不实现 T24 移动/恢复、T25 快照或索引 handler。
- 所有修改在主根 .worktrees/knoted-t23 / feat/t23；临时输出 /data/knoted-dev/tasks/t23。
- title 最多500字符；文档 format_version=1、2MiB、5000块、8层，复用 T22。
- version 从1开始；语义内容变化与软删除递增；SQL 同时匹配 ID/version/可写状态，404/409区分。
- list limit 默认50、最大100；(position,id) 键集、base64url版本化游标完整验证并绑定 parent 过滤条件。
- DELETE 按 T03 使用 expected_version query。新增迁移不修改已应用迁移；不改变已确认契约/ADR。
- 本地 fmt/test/clippy 是唯一合并门禁；默认远程授权允许 Issue、PR、merge；生产/不可逆操作仍需人工确认。

## Review Focus

- 并发父删除/子创建与同级追加：事务互斥确保父可见、同级位置连续。
- stale/no-op 写：过期版本409且返回current_version；相同内容不增加语义版本或outbox。
- 文档/HTTP非法输入：未知字段、null、大小及非法cursor均拒绝，错误不泄露正文。
- 数据库中途失败：注册/内容/version/outbox一并回滚，无幽灵事件。
- 鉴权和可见性：匿名401、跨站写403、deleted普通详情/list不可见。

### Task 1: 笔记持久化、服务与 HTTP 接线

**Files:** 新建 core notes/{repository,service}.rs、api routes/notes.rs、api tests/t23.rs、migrations/0005_t23_notes.sql；仅修改 notes/mod.rs、routes/mod.rs 注册模块。文档更新本计划、任务卡和证据。

**Interfaces:** 消费 AppContext、notes::document::validate_document、既有会话/CSRF；产出异步 create(&AppContext,CreateNote)->Result<Note,DomainError>、get(GetNote)、list(ListNotes)->NotePage、update(UpdateNote)、delete(DeleteNote)->()，统一命名输入；Note 字段遵循 T03。

**Executor/Dependencies:** 主Agent为唯一实施者；依赖 T15 32cbb6f、T22 8d4a472，基线 4ef04e3 已包含。不可并行修改。独立只读 reviewer 在最终提交后审查。

- [x] 写真实 PostgreSQL+HTTP 场景 t23_1/2/3，以及 CRUD/校验/游标/鉴权/父子并发回归；先运行并观察缺失路由造成的真实断言失败。
- [x] 新增 notes 表与注册、事务锁/CAS/outbox SQL；实现 service 校验与命名 DTO，HTTP 仅适配输入输出。
- [x] 跑聚焦测试直到通过；全量 cargo fmt --check、cargo test --workspace --locked、cargo clippy --locked --all-targets -- -D warnings，保留输出；diff/敏感信息检查。
- [x] 实际读取 git-commit Skill并提交；最终独立审查，重要反馈用RED→GREEN修复。

### Task 2: 集成、测试实例与收尾

**Files:** 本任务卡、evidence/t23.md；部署需要时仅授权独立 host 任务修改 /etc/nixos，遵守其规则。

**Interfaces:** 消费 Task 1 已审查提交；产出 PR merge、主线验证、运行SHA与HTTPS功能证据。主Agent负责集成和最终验证；host agent如需写文件各用独立工作区。

- [x] PR 用模板关联 #60 并粘贴门禁输出；普通 merge 后复验主线。
- [x] 按既有部署机制固定SHA构建，先一致备份再应用兼容新增迁移；HTTPS readiness/登录/本任务CRUD验证。无备份权限时保留待部署。
- [ ] 记录真实验收，Issue/Projects Done；归档日志，并在最后收尾PR合入后清理本任务已合并分支/worktree（清理读回记PR与最终报告，不为该记录循环创建文档PR）。

## Progress / Decisions

- 2026-11-16：已查重，Issue #60；独立Worktree已建立；基线110 passed、1既有ignored，exit0。无原生TODO，使用本清单。
- 既有任务卡/契约与用户明确完成指令已授权实施；常规设计沿用契约，不重复要求批准。候选实现为事务 SQL 或内存状态；选择 SQL，因为 CAS/outbox 必须同事务持久化，内存实现无法满足验收。
- 根规则仅重大架构/不可逆边界须人工确认，覆盖技能通用审批停顿。此计划不新增跨模块契约。

- 实际实现15场景；首轮9个路由缺失断言失败；创建时间精度回归与失败回滚释放锁已修复；缺失cursor过滤字段200→400先RED后GREEN。
- 同级结构写使用按notes表OID隔离的事务advisory锁；内容写只锁单行。无op写保留版本/outbox。未接入索引handler时outbox保持未发布，不创建会被当前worker拒绝的job。

- 独立审查发现NUL落库503的重要输入问题，已真实RED→GREEN修复；最终15聚焦/125全量pass（1既有ignored），fmt/clippy0；前端54pass/typecheck/lint/api检查0。

- PR #61已合入b0409b6，主线125/54及全部门禁通过；固定7dc601a已完成一致备份后dev更新，HTTPS CRUD/CAS/outbox/软删除与会话/CSRF/logout通过；606非Knoted条目及10项共享记录不变。详情见T23证据。
