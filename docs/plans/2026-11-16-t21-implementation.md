# T21 最小 HTTPS 测试部署实施计划

> 执行：superpowers:executing-plans。按任务卡串行；T20 集成后才构建部署。

**Goal:** 将固定提交的 T20 网页和现有认证 API 部署到 HTTPS 15601。
**Architecture:** 应用侧从固定 Git SHA 导出源码，锁文件构建产物，由 Nix 包验证校验清单并封装；应用模块只提供 API 服务。独立 host Agent 配置专用 PG、稳定目录、证书和 HTTPS 单元，避免改变其他应用。
**Tech Stack:** Rust、pnpm/Vite、NixOS/systemd、Caddy、PostgreSQL。
**Spec:** [T21](knoted-v1/tasks/T21.md)、[部署约定](knoted-v1/development-and-deploy.md)。

## 约束与分工

主 Agent 仅修改 Knoted 的 nix/package.nix、nix/module.nix、tools/dev 与任务文档，在主根 .worktrees/knoted-t21 / feat/t21。host Agent 仅修改 /etc/nixos 独立 Worktree；输入固定 app SHA/产物，输出独立 host 提交、运行证据及回退路径。不启用未实现的 Worker/Python 业务，不改共享模型归属，不记录凭据。无新存储或业务接口。

## Review Focus

- 构建来源必须是已提交完整 SHA，不能混入工作树文件。
- 包内网页、API 与 REVISION 必须来自同一源码。
- 证书失效或域名错误时不启动公网监听，不回退 HTTP。
- URL 深链接刷新返回 SPA，API 未授权请求保持 401，不被静态 fallback 掩盖。
- host 用户改动、其他库和其他运行服务保持原状。

## 检查清单

- [x] Task 1：T20 PR 集成并独立验收；核实前端入口与 API 路径。
- [x] Task 2：tools/dev/build-artifact.sh 接收 revision/output，强制完整 SHA，git archive 隔离源码，锁定依赖构建，输出 bin/knoted-api、share/knoted/{web,migrations,REVISION} 和 SHA256SUMS；对错误 SHA/脏工作区输入做场景检查。
- [x] Task 3：nix/package.nix 接收 artifact/revision，校验 REVISION 和 SHA256SUMS 后复制产物；nix/module.nix 暴露 services.knoted.enable/package/configFile/user/group，仅声明 knoted-api 单元。实际 nix eval/build 验证。
- [x] Task 4：运行前端全量门禁与 Rust fmt/test/clippy，保留命令输出；读取 git-commit Skill 后提交；requesting-code-review 独立审查，核实修复。
- [x] Task 5：host 领取固定提交与产物；专用PG角色/库、稳定目录、证书检查、HTTPS15601、API15602。候选 build、完整dry（含target与依赖传播）检查、仅自身单元定向加载与健康检查、boot持久化；不再整体test/switch，保留原system和unit入口可回退，不触碰用户改动。
- [x] Task 6：t21_1 证书失效不退HTTP、t21_2 HTTPS readiness/登录、t21_3运行SHA/产物一致；真实浏览器虚拟Passkey流程另记硬件验证限制。
- [ ] Task 7：PR普通merge，主线检查、任务卡/证据更新、仅清理本任务已合并Worktree/分支。

## 当前进度

2026-11-16：计划建立，基线6ee4380；T20实施中。host只读预检：端口空闲、证书有效、账号存在，PG专用库待建立。原生worktree工具不提供路径参数，无法满足项目固定路径，使用Git fallback。无独立人工决策点；生产变更或不可逆操作停止对应动作。

2026-11-16：T20 PR #52普通merge至e8d2c4f。app准备独立审查1项Important已RED→GREEN修复；t21-precommit门禁全部exit0。构建与部署仍按Task5等待固定app提交。

2026-11-16最终验收：T15遗漏HTTP接口由Issue #54 / PR #55补修，T21固定f35a69a构建，真实HTTPS虚拟Passkey全通过，前端54/Rust92/既有Chromium15。host因共享setup/local-fs隐式传播，取消整体test/switch，采用专用db/prepare +仅五个自身单元定向加载与boot持久化；606非自身条目及既有PID不变。早期实际影响、回退、用户Pi配置保留与实体凭据恢复均写部署证据，未隐藏。Task7仍待PR集成/主线验证/清理。
