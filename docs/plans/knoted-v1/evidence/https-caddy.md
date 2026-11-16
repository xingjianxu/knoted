# HTTPS 接回共享 Caddy 验证证据

- 日期：2026-11-16；Issue [#69](https://github.com/xingjianxu/knoted/issues/69)；[ADR 0005](../../../decisions/0005-https-owned-by-caddy.md)。
- 原始输出：`/data/knoted-dev/tasks/caddy-handoff/`（不包含证书私钥、环境密钥或用户会话）。
- 基线 `pnpm test:pc`：21/21；调整场景后 RED（编排、生成文件、证书缺失、HTTPS 端口占用）如预期失败；实施后 GREEN 21/21。
- 应用变更：pcwrap 仅 prepare/migrate/knoted-api；不渲染 Caddyfile、不安装 reload launcher、不读证书/检查 HTTPS 端口。保留独立证书检查工具及其测试。
- Host 变更：features.server.caddy.virtualHosts 共享入口，useACMEHost 复用证书；删除 knoted-https-reload 与 reloadAfterRenewal 选项，保留未来生产路径。

## 本地门禁与审查

| 实际命令 | 实际结果 | 原始输出 |
| --- | --- | --- |
| `cargo fmt --check` | exit 0 | fmt.log |
| `cargo test` | exit 0；125 passed、0 failed、1 ignored（既有） | rust-test.log |
| `cargo clippy --all-targets -- -D warnings` | exit 0 | clippy.log |
| `pnpm test:pc` | 22 tests / 22 pass / 0 fail | pc.log |
| `pnpm lint` | exit 0 | lint.log |
| `pnpm typecheck` | exit 0 | typecheck.log |
| `pnpm test` | 6 files / 54 tests passed | web-test.log |
| `bash -n tools/dev/pc/service.sh`、`git diff --check` | exit 0 | 主 Agent 实际执行 |

独立只读 Reviewer `/root/review_caddy`：无 Critical / Important。ADR 索引空行已修正；按反馈补充严格受限 PATH 成功用例（明确不含 caddy/openssl），短暂恢复旧 Caddy 依赖观察 RED，再还原并跑完整测试 GREEN 22/22（tls-path-red.log / pc.log）。

## Host 集成与切换

- Host 提交 `7a7fba5` 已本地快进合入 `/etc/nixos` main；未推送 host 远程。用户原有 pi.nix 修改原样保留且未进入提交；候选包含其现有内容以保持运行环境。
- `nix eval`：`knotedUnits=[]`，`reload=["caddy.service"]`；防火墙保留 15601，vhost 使用共享 Caddy bridge 和 ACME。
- 构建成功：`/nix/store/i6ji154ql7gg74mbgc6n11zsx9zf7yjl-nixos-system-wks-lab-26.11.20261109.5e6fbb3`。真实 caddy 用户 + 单元 EnvironmentFile 校验：`Valid configuration`、`status=0/SUCCESS`；既有其它 vhost 两条 header_up 冗余警告与本次无关。
- 候选相对之前 boot 配置：删除 knoted-https-reload，ACME order-renew postrun 与 Caddy reload trigger 改变；其它单元内容相同。Caddyfile diff 只增加 Knoted vhost。
- `dry-activate`：`would reload the following units: caddy.service`。没有全机 switch/test：仅安装 boot profile/bootloader，更新 `/etc/systemd/system` 与 Caddy 配置链接，daemon-reload，再定向 Caddy reload。
- 首次尝试逐个更新 systemd 单元链接遇到只读 store：查明 `/etc/systemd/system` 是上次部署留下的整目录 symlink。未停止旧 HTTPS、未 reload；改为在已逐文件比较后原子更新整目录链接，成功。未写入 nix store。
- `pnpm pc process stop knoted-https` 释放 15601，共享 Caddy reload 成功；旧生成的 Caddyfile、reload launcher/checker、HTTPS state 移入任务证据根的 retired-https，保留回退材料。
- `systemctl list-unit-files '*knoted*'`：`0 unit files listed`；`ss` 显示 15601 归现有 `caddy` PID 1706466，15600 与 15602 仍只监听回环。

## HTTPS 验收

真实证书校验请求（无 `-k`）：ready/live 200，未登录 session/notes 401，首页与 /recovery 200 且逐字匹配固定产物 index.html，JS 资源逐字匹配产物；明文 HTTP 15601 返回 400，无 HTTP 回退。

Caddy / PostgreSQL / Strata PID 分别为 1706466 / 1167036 / 1180032，与切换前相同。其它站点前后状态一致：receiptd 307、thesisd 307、OnlyOffice healthcheck 200。详见 runtime-checks.json、other-vhosts-{before,after}.json。

待应用 PR 合入后，从主检出 down/up，确认 pcwrap 中旧 HTTPS 进程定义也已移除；该步骤结果合入后追加。没有强制 ACME 发证/续期；续期目标已纯求值与单元 postrun 校对，并实测共享 Caddy reload。无数据库或生产实例变更。
