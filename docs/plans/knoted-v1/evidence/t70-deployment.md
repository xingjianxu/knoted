# T70 生产模块与主机验收

应用模块通过 [PR #232](https://github.com/xingjianxu/knoted/pull/232) 合入 `097cd5b4b749633f2ef0cc4ea513e9b7eb3017f8`。生产主机固定该主线中的模块提交 `eccca479a3bff2813d69d73f839c10ba5504dad4`。实际使用的应用产物为 `5308378799a768ded33d26d1e08c37473f30eca5`，NAR 摘要 `sha256-d5qMCEaESJScdjwaSHQVr/nCKrcfD5kMYVcERS0A5+Y=`。[Issue #230](https://github.com/xingjianxu/knoted/issues/230)

## 应用验证

| 实际运行 | 结果 |
| --- | --- |
| `cargo fmt --check`、`cargo clippy --locked --workspace --all-targets -- -D warnings` | 均退出 0 |
| `cargo test --locked --workspace` | 534 项通过，0 失败，1 项既有忽略 |
| `nix-build … nix/test-module.nix` | 实际 NixOS KVM，约 33 秒。两处端口冷启冲突在迁移前停止并保留原监听者，非回环连接失败。Python 单元不能读取业务凭据、来源目录或访问 PostgreSQL。正常停机和实际 ASGI MainPID 异常退出均有 guardian 退出证明，证明缺失报告 degraded |
| `nix-build … nix/test-production-vm.nix` | 实际 NixOS KVM，约 31 秒。Python 停止后通过真实 HTTPS 注册虚拟 Passkey、创建笔记、上传图片和补存快照。VM 关机再启动后以同一虚拟 Passkey 登录，笔记和图片摘要相符。测试证书 reload 后的 TLS 序列号从 01 变为 02 |
| `ruff check`、`nix-instantiate --parse`、`git diff --check` | 退出 0，独立审查完成 |

API 在迁移结束与 Rust 重新绑定端口之间有短暂交接。若此时被抢占，API 拒绝启动，已占用端口的进程保留。Python 回环端口由 systemd socket 持续管理。

## 主机接入

维护者在会话内明确授权 wks-lab 的独立生产实例及域名 `kn.bixlab.cn`。新建空库 `knoted_prod`，由宿主用户 `xingjian` 管理并装载 pgvector 0.8.7。本机 PostgreSQL 认证沿用 [ADR 0004](../../../decisions/0004-database-owned-by-host-user.md) 接受的回环 trust。生产数据根为 `/data/knoted-prod`，独立资产、staging、Python 状态和 Meilisearch。应用账号与 Meilisearch 账号分别限定对应目录。模型按共享只读路径消费。生产固定 GPU UUID `GPU-bf8d2065-6d30-4a83-d9a7-efb41ab26cef`。

完整系统候选的 dry-activate 显示会影响 DAE、Polkit、Caddy 和 Home Manager。实际使用独立生产单元、固定闭包 GC root 和共享 Caddy 的定向热加载。构建保留原运行代与 profile。测试实例继续使用 pcwrap。

首轮在定向启动后实测域名的 HTTP 挑战路径返回 200，但 Let's Encrypt 从外网连接 80 超时。第二轮切换已有权限的 Cloudflare DNS-01 后，lego 的本机递归解析器对挑战名 SOA 返回 SERVFAIL。两轮实际回退均恢复原 Caddy 配置和共享进程，生产库、运行目录及专用账号保留以便继续。随后以专域递归解析替代故障解析路径。

本地记录在 `/data/knoted-dev/tasks/t70/evidence/`。包括 Rust 门禁 `t70-branch-rust.log`、两组 KVM `t69-module-final.log` 与 `t70-branch-browser-vm.log`、系统 dry-activate 和两次签发失败及回退记录。

## 生产运行与本机域名验收

- 宿主提交为 `781aba696d6827aee89b4cc85cada494e3a76770`，已快进至本机宿主主线。完整构建输出为 `/nix/store/rsk2ffrh16v8kncl6y6smbsdw7cy1872-nixos-system-wks-lab-26.11.20261109.5e6fbb3`，通过生产 GC root 保留。原启动 profile、运行代和用户未提交文件均比对一致。
- 最终 DNS-01 仅为新域名选用经实际 acme UID 验证的 `223.5.5.5:53` 查找区域，发布 TXT 后等待 60 秒，由 Let's Encrypt 完成独立验证。正式证书 SAN 为 `kn.bixlab.cn`，证书链和主机名校验通过。续期 timer 活动，证书更新由共享 Caddy 热加载。
- 在 wks-lab 访问 `https://kn.bixlab.cn` 和 readiness 均为 HTTP 200，未登录会话查询为 401。API、Worker、Python 的 /proc 可执行文件匹配固定产物。15702、15703、15705 均只监听 127.0.0.1。库的迁移版本为 27，生产账号保持首次注册状态。
- 生产内部接口以合成输入完成一次真实 BGE-M3 GPU 执行。仅配置预定 GPU 可见，返回设备为 `cuda`、1024 个有限归一化值。产物字节和摘要一致，guardian 出具退出证明。校验脚本最初把设备字符串写成 `cuda:0`，改按 BGE 应用实际使用的 `cuda` 复核同一执行。
- 共享 Caddy、PostgreSQL、Strata、OnlyOffice、DAE、Polkit 的 PID 前后一致，dev readiness 保持 200。独立域名的 Caddy 配置热加载与撤销均已实际执行。
- 维护者确认生产入口为 `kn.bixlab.cn` 直接映射到 wks-lab 本机。复验确认 A 记录 `115.24.115.250` 属于本机 `ens5f0`，请求该域名的 `/health/ready` 返回 HTTP 200 和 `{"status":"ready"}`，TLS 校验结果为 0，续期定时器和生产服务均为 active。

早期外部 443 探测的超时记录见证据目录的 `external-*-result.json`。维护者随后确认以本机域名映射和 HTTPS 作为验收范围。

操作和定向回退步骤在宿主仓库 `docs/knoted-production.md`。部署前后与签发输出位于同一 T70 evidence 目录。本机域名复验记录为 `host-local-acceptance.log`。GPU 成功复核输出为 `host-real-gpu-verified.log`。
