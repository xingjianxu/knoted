# T72 最终测试部署与交接

## 固定版本

| 实例 | 入口 | 应用产物 | 编排 |
| --- | --- | --- | --- |
| dev | `https://wlab.132140.xyz:15601/` | `dbc5788fb64dcf7babf94ed3b70ce0088a4b72e7` | pcwrap、process-compose 与共享 Caddy |
| 生产 | `https://kn.bixlab.cn/` | `5308378799a768ded33d26d1e08c37473f30eca5` | wks-lab 定向 NixOS 单元与共享 Caddy，宿主提交 `781aba696d6827aee89b4cc85cada494e3a76770` |

dev 发布来自 [T71 PR #235](https://github.com/xingjianxu/knoted/pull/235) 的合并提交，实际执行 `KNOTED_BUILD_ROOT=/data/knoted-dev/tasks/t72 CARGO_BUILD_JOBS=4 bash tools/dev/build-artifact.sh "$revision" "/data/knoted-dev/runtime/$revision"`。发布脚本从提交归档构建 API、Worker、Web 和锁定 Python 3.12 依赖。完整核验 38,345 个文件，`SHA256SUMS` 的 SHA-256 为 `c97e0d65ae18deba5f4dce40b67760cbc9773bf3d84b01a6c0e8203d0423b93b`。Rust、Web、Python 和迁移源与前一发布之间无变更。Python 的 `BUILD.json` 和 `REVISION` 均与固定提交一致。

## 验证与切换

| 实际执行 | 结果 |
| --- | --- |
| Rust fmt、test 和 clippy | 主线 534 通过、0 失败、1 项既有文档示例忽略 |
| `pnpm test:pc` | 41 通过，0 失败、0 跳过 |
| `pnpm api:check`、`pnpm api:verify` | 退出 0。84 个接口、29 个样例，其中 14 个异常样例正确拒绝。生成客户端无变更 |
| T71 主线 Chromium | 1 通过，0 失败、0 跳过。独立 PostgreSQL、Meilisearch、Python、BGE-M3 与 HTTPS 使用本次完整固定产物 |
| dev HTTPS 和业务复核 | 一致备份后切换，API、Worker 和 Python 的实际二进制匹配固定产物。现场 HTTPS 验证会话、笔记与全文，账号、Passkey 登记、源根和资产根摘要一致 |

Rust 首轮复用缓存时，OCR 用例仍读取已清理的工作树路径。确认旧路径内嵌于测试二进制后，清理本任务的 workspace 编译产物，从主线重编 OCR 用例并重新运行全量门禁。

使用前代 `5308378` 执行 T68 一致性备份，含 11 个登记图片。归档清单摘要为 `ddda41003bca7821423e7aebb4074ddcc3bc41039d1f8759df58b51b86f83182`。源根与资产根通过路径及文件系统身份摘要比对，账号与 Passkey 登记记录用稳定 ID 摘要比对。dev 测试会话只在脚本内存中使用，切换前后通过实际 HTTPS 鉴权。合成笔记验证正文摘要和实际全文命中，结束后进入回收站，测试会话撤销。真实 WebAuthn 重新登录在独立 Chromium 虚拟认证器中实跑。共享服务与独立生产进程在 dev 切换时 PID 保持一致。

当前发布与前代 schema 均为 `0027`。紧急运行回退保留前代产物，经一致备份、pcwrap 停止、原子切换 `current` 与重启复验。需要恢复数据时按 [ADR 0011](../../../decisions/0011-consistent-backup-and-isolated-restore.md) 验证归档并在隔离目标恢复。

## 生产与访问范围

生产使用独立空库 `knoted_prod`、专用状态根及 Meilisearch。维持已批准的独立运行版本。正式证书、续期 timer、本机 HTTPS、三处回环监听及 CUDA BGE-M3 样本已实测。公网探测连接 443 超时，当前仍保留维护者配置的 A 记录，公网路径由 T70 跟踪。[T70 验收](t70-deployment.md)

T69 早期一次测试曾使用 dev 的默认 Meilisearch。交接只读核对在同一小时内有 14 个 T54/T62 测试前缀索引，逐个归属尚未证实，保留现状。后续门禁使用隔离 Meilisearch。

本轮在授权范围内使用 GitHub Issue、Projects、非默认分支和普通 PR 合并。在维护者授权后定向启用 wks-lab，DNS-01 使用现有权限生成临时挑战记录。测试实例回退、主机仓库和网络边界各自保留记录。生产保持待首次注册，账号登记由维护者完成。浏览器验收的实体认证器以及 Linux IBus/X11 之外的输入法平台仍在现有验证范围之外。[T71 矩阵](t71-acceptance-matrix.md)

实际日志位于 `/data/knoted-dev/tasks/t72/evidence/`。包括 `t72-build-artifact.log`、`t72-mainline-rust-gate.log`、`t72-mainline-t71-browser.log`、`dev-rollout.log`、`dev-https-acceptance.log` 和切换前后身份摘要。
