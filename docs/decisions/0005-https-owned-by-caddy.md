# ADR 0005: 测试 HTTPS 由宿主共享 Caddy 管理

- Date: 2026-11-16
- Status: Accepted
- Decision owner: xingjianxu（仓库 owner）
- Confirmation: 2026-11-16 会话中人工直接指示「Knoted 的 HTTPS 接回 caddy 管理，删除knoted-https-reload服务。pcwrap还用于管理开发测试的相关服务，但是不再包含knoted-https了」。
- Related issues: [#69](https://github.com/xingjianxu/knoted/issues/69)
- Related PRs: 见 Issue #69 的合并记录
- Supersedes: [ADR 0003](0003-test-instance-process-management.md) 中测试 HTTPS/Caddyfile/证书校验/专用续期 reload 归属；其余决策继续有效
- Superseded by: 无

## 背景与问题

ADR 0003 将测试 API 和独立 Caddy 一并放入 pcwrap，并在宿主保留 knoted-https-reload oneshot。这使已有共享 Caddy 与独立 Caddy 同时运行，证书续期跨越两个管理入口。用户明确要求 HTTPS 接回共享 Caddy，开发测试应用继续使用 pcwrap。

## 决策内容

1. HTTPS 15601 通过 `/etc/nixos` 的 `features.server.caddy.virtualHosts` 接入现有 `caddy.service`，使用 `useACMEHost` 复用 `wlab.132140.xyz` 证书；续期只 reload 共享 Caddy。
2. 删除测试 `knoted-https-reload.service`、实例安装的 reload launcher 和 pcwrap 的 `knoted-https` 进程。pcwrap 不再渲染 Caddyfile、读取证书、管理 HTTPS socket 或检查 HTTPS 端口。
3. pcwrap 保留 `prepare → migrate → knoted-api`，控制端口 127.0.0.1:15600，API 127.0.0.1:15602。保留固定产物与 API 端口检查，HTTPS 域名/端口只用于 API 的 Passkey origin 配置。
4. 共享 Caddy 提供静态 SPA，根为 `/data/knoted-dev/runtime/current/share/knoted/web`；`/api/*` 与 `/health/*` 保留原路径代理到 API。应用更新仍走固定产物校验与原子 current 切换，不需 host revision pin。
5. 原独立 Caddy 的 `X-Knoted-Revision` 固定头不再生成，避免共享配置缓存旧 SHA；运维以 current 指向的 `share/knoted/REVISION`、完整 `SHA256SUMS` 及 API 进程可执行路径核对版本。
6. 不给共享 Caddy 增加 Knoted 专属全局证书 guard；缺失/无效 TLS 配置由共享 Caddy 原生 validate/reload 拒绝，保留旧运行配置。独立证书检查脚本保留作运维工具，不参与 pcwrap 启动。

不涵盖：未来生产路径、不新增应用部署、数据库变更、其它服务重启、证书权限放宽。

## 备选方案

| 方案 | 收益 | 成本 / 风险 | 结论 |
| --- | --- | --- | --- |
| 保留 pcwrap 独立 Caddy 和专用续期服务 | TLS 故障独立 | 两套入口，与用户要求冲突 | 否 |
| 改回独立 knoted-https systemd 单元 | 生命周期归 host | 仍是独立 HTTPS 服务与重复续期链路 | 否 |
| 共享 Caddy vhost | 单一 HTTPS / ACME 管理，与宿主规则一致 | 共享 reload 需整份配置校验，Knoted 证书错误可能拒绝新配置 | 采纳 |

## 决策理由

用户直接指定归属。宿主已有同域名 ACME 与 useACMEHost 先例，共享服务已有证书权限与续期 reload，无需新守护进程。pcwrap 继续用于应用调试，API 停止时 HTTPS listener 与静态站点仍由 Caddy 提供，API 请求返回上游不可达。

## 影响

- 正面：删除专用进程与 reload 单元；API 启动不再依赖 HTTPS 工具、证书或 15601 空闲。
- 负面：共享配置错误会拒绝整次 Caddy reload；原版本响应头移除，需按产物与进程核验版本。
- 迁移：保存旧 Caddy 配置/单元和 pcwrap 配置；验证候选；停止旧 HTTPS 释放 15601，定向 reload 共享 Caddy；撤除专用 reload 单元与生成入口，安装 boot 配置，重载三进程编排。
- 回退：先 reload 旧共享 Caddy 配置以释放 15601，再恢复旧 pcwrap 配置与专用 HTTPS/reload 链路；恢复记录的 boot profile。不改数据库或降级 schema，无不可逆迁移。

## 验证与后续任务

实施与验证见 [计划](../plans/2026-11-16-https-caddy.md)，切换结果见 [证据](../plans/knoted-v1/evidence/https-caddy.md)。未来生产仍按独立任务重新确认，不启用现有 app.enable。

## 状态历史

- 2026-11-16：Accepted。人工直接指定 HTTPS 与 pcwrap 管理边界（Issue #69）。
