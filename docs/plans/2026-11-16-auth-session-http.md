# 会话 HTTP 适配器缺失修复

任务依据：用户 T20-T21 委派；T21 真实 HTTPS 发现 GET /api/v1/auth/session 为404，关联 Issue #54。基线 e8d2c4f。

根因：T15 只交付 core sessions 与保护中间件，自建 Router 验证，T14/T16 实际装配的 auth Router 未提供契约中四个会话 HTTP 入口。修复不新增存储/API/权限策略，仅兑现已有 OpenAPI。

实施 Agent 在 .worktrees/knoted-auth-http / fix/auth-session-http，主 Agent 独立审查、集成与部署。原生 Worktree 工具无路径参数，不能满足主根 .worktrees 固定位置，故使用 Git fallback。输入为现有 sessions service、AuthConfig、session/CSRF guards；输出为 routes/sessions.rs、生产路由注册、必要 CSRF 恢复原语及 session_http.rs 真实入口测试。

- [x] 读根规则、T15/T16与OpenAPI；systematic-debugging确认生产路由遗漏，查重创建Issue。
- [x] 真实 build_router 回归先RED：匿名401而非404、当前DTO、CSRF恢复、退出、分页、撤销/不存在、Origin/CSRF负向。
- [x] 新增HTTP适配器：reuse core服务，DTO不泄漏hash，limits1..100，版本化base64url cursor，缺会话401，缺资源404。
- [x] GET session复用有效CSRF cookie；缺失/无效才轮换当前session digest并补Secure cookie，不强迫GET提供CSRF。成功退出/撤销当前会话清两枚Cookie。
- [x] cargo fmt --check、cargo test --locked、cargo clippy --locked --all-targets -- -D warnings，日志保留 /data/knoted-dev/tasks/auth-session-http/evidence。
- [ ] 读取git-commit并检查范围/敏感信息后提交，主Agent独立review/PR/merge/部署与主线验证。

边界与限制：缺失CSRF cookie的并发恢复可能各自轮换hash，仅最终token有效；常规携有效cookie的GET不轮换，不破坏并行请求。客户端遇到竞争导致403可重新GET恢复。无需数据库迁移；不改其他业务或mtone。


## 实际验证与交付证据

工作区未改前端、API契约或迁移。真实生产路由测试 `crates/knoted-api/tests/session_http.rs` 首次可运行时6例全部以404失败（`red-runtime.log`）；装配适配器后6例全绿，再补101个会话/limit100分页边界，共7例通过。早期测试夹具编译错误已在观察行为RED前修正，不把编译失败当行为证据。

最终本地门禁（完整输出 `/data/knoted-dev/tasks/auth-session-http/evidence/final-*.log`）：

```text
cargo fmt --check                                      exit 0
cargo test --locked                                    exit 0
  session_http: 7 passed
  workspace: 92 passed, 0 failed, 1 existing ignored
cargo clippy --locked --all-targets -- -D warnings      exit 0
git diff --check                                       exit 0
```

适配器始终按当前user_id限定查询/撤销/恢复，DTO显式选择公开字段，不序列化hash或session bearer。会话GET/列表与mutation响应设置Cache-Control:no-store；csrf恢复不改session bearer或其他设备；清cookie保持Secure/HttpOnly/SameSite属性。默认50/最大100均按core约定，满页额外探测1行以避免core limit钳制导致第101行丢失。缺失/无效CSRF的并发恢复限制见上文；常规已有有效cookie不更新hash。

Issue [#54](https://github.com/xingjianxu/knoted/issues/54) / Projects In Progress。实现完成、独立审查与集成/部署待主Agent验收，不提前关闭Issue。
