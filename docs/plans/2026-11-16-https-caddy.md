# HTTPS 接回宿主 Caddy 实施计划

> 使用 superpowers:executing-plans 串行实施；主 Agent 负责修改、验证与集成，末尾独立审查。

**Goal:** HTTPS 15601 交给共享 Caddy，删除 knoted-https-reload，pcwrap 保留开发测试应用进程。
**Architecture:** 经 features.server.caddy.virtualHosts 接入共享服务，useACMEHost 复用证书；静态根使用 /data/knoted-dev/runtime/current/share/knoted/web，/api/* 与 /health/* 回环代理到 15602。pcwrap 仅 prepare → migrate → API。
**Tech Stack:** NixOS / Caddy、Bash、process-compose / pcwrap、Node 场景测试。
**Spec:** 本会话用户明确指示 HTTPS 接回 Caddy，删除专用 reload 服务，pcwrap 不再包含 knoted-https；ADR 0005 记录该确认。

## 约束与方案

- 延续域名、TLS 证书、端口、SPA 路由、API 和数据库；不部署生产，不更改数据或其他应用。
- 共享 vhost 符合宿主唯一桥接规则；保留独立 pcwrap Caddy / 独立 systemd Caddy 均不满足用户指定归属。
- /etc/nixos 的用户 pi.nix 修改保留；候选构建原样包含其已运行配置以避免回退用户环境，但不提交该文件。
- 原生 worktree 工具不接受路径参数，不能满足固定目录约束；使用 Git 在两仓库主根 .worktrees 中隔离。本会话已有 feat-https-shared-caddy 工作区不触碰。
- Caddy 使用 current 静态根，应用更新不再需要 host pin 或 reload；旧独立实例的固定 X-Knoted-Revision 头移除，运行版本通过 current/share/knoted/REVISION 与 SHA256SUMS 核验，避免共享配置持有过时 SHA。

## Review Focus

- pcwrap 启动不受宿主 HTTPS 端口占用、证书或 Caddy CLI 的影响。
- /api/*、/health/* 保留完整路径，SPA 深链与资源能以 caddy 身份读取。
- 无显式 HTTP 回退；ACME reloadServices 只走共享 Caddy，无专用 reload 残留。
- 定向切换保持其他 vhost、PostgreSQL / Strata PID，API 与数据库保持原样。
- 重启 pcwrap 不恢复旧 HTTPS；boot 配置与运行配置均持久化，保留回退证据。

## 任务与验收

所有实现由主 Agent 串行执行；审查子 Agent 只读，无写文件和远程权限。

- [x] 现状与授权检查；基线 pnpm test:pc 21/21。
- [x] T1 pcwrap：先改场景测试验证缺证书/HTTPS 已占用仍可 prepare，再移除 HTTPS 进程、模板、启动与 reload 子命令、证书工具依赖，保留 artifact 与 API 端口检查。文件 tools/dev/pc、tests/pc、process-compose.yaml、pcwrap.config.json。
- [x] T2 Host：knoted.nix 删除续期选项/单元，添加经桥接的 HTTPS vhost，复用 ACME、current 静态根；更新相关注释。nix eval 验证服务/续期/vhost/firewall，构建候选并以 caddy 用户 validate。依赖 T1 边界。
- [x] T3 文档与决策：新增 ADR 0005，旧 ADR 只加替代关系；更新当前运行指南、架构和 AGENTS；历史证据不改写。
- [x] T4 验证与切换：保存完整命令输出；cargo fmt/test/clippy、pnpm test:pc/lint/typecheck/test，独立审查。候选只安装 boot、定向更新 Caddy 单元与 ACME postrun，停止旧 pcwrap HTTPS、reload 共享 Caddy，重载 pcwrap 三进程并验证 TLS、readiness、静态/深链/401、无 HTTP 回退、共享 PID。
- [ ] T5 提交与集成：每次提交前读 git-commit；Host 本地合并 main（无直推），Knoted PR 普通 merge，主线复验后仅清理本任务 Worktree/分支。

## 证据与状态

原始输出放 /data/knoted-dev/tasks/caddy-handoff/，公开摘要放 docs/plans/knoted-v1/evidence/https-caddy.md。无数据库迁移，回退恢复旧 Caddy 单元/配置与 pcwrap 配置快照，端口切换顺序相反。

进展：Issue #69 已建；T1 RED→GREEN 21/21。T2 纯求值中，T4 应用门禁执行中。

进展：Host eval：knotedUnits=[]、ACME reload=[caddy.service]；候选构建成功，dry-activate 仅列 caddy reload；应用七项门禁全部 exit 0。独立审查指出 ADR 索引空行导致表格断裂，已修正文档格式。

Host 已本地合入 main 7a7fba5，boot 安装和定向 reload 成功；TLS、静态/API、共享 PID 与其它 vhost 复核通过。系统单元目录为 store symlink，逐项修改失败后按完整内容比较结果原子更新整目录链接。独立审查两项非阻断意见均已修复，受限 PATH 用例 mutation RED→GREEN，pc 测试22/22。当时待完成：Knoted PR 合并、主检出三进程 down/up、主线复验及本任务清理（后续完成项见下段）。

PR #71 已合并至 a8643fc，主检出真实 down/up 后只有 prepare/migrate/knoted-api；停编排时静态站点仍200，启动后ready200。完整主线七项门禁全部 exit 0；最终文档PR和任务Worktree/分支清理后更新PR交付记录。

最终文档补充已由独立 Reviewer 复核，数字、SHA、链接与运行证据一致。T5 清理实际结果在最终PR合并后读回记录，避免提前声明完成。
