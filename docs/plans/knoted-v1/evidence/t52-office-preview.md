# T52 OnlyOffice 只读网页预览交付证据

[PR #183](https://github.com/xingjianxu/knoted/pull/183) 普通 merge 至 `4e722319dc850ee7627c94bb7d7919cdf0020e97`；同一固定产物在 dev 启用。已读回 [Issue #180](https://github.com/xingjianxu/knoted/issues/180) Closed、Projects Done。后端独立验收见 [T51](t51-office-authorization.md)。

## 实现和独立评审

基线 `dadbef7`，实现 `8ef48fb`，隔离修复 `82e9ddd`，整合候选 `c3ef2b9`。首轮独立评审指出 CSP 无法表达 IPv6、同主机不同端口会自动发送会话 Cookie、静默启动需要有界重试；按用户确认的 [ADR 0010](../../../decisions/0010-office-preview-credentialless-host.md) 修复全部问题，独立复审 Spec PASS / Quality APPROVED，整体验证复核 APPROVED。主线与整合树相同，Web 与独立测试头 `82e9ddd` 相同。

父页使用现有 mtone Dialog 和 T51 的会话/CSRF 客户端；只有可信同源 `credentialless` Knoted 页面通过逐次通道接收签名配置并加载官方 DocsAPI。关闭、切换、失败、45 秒启动超时销毁宿主并撤本次授权；迟到结果只撤自己的 ID。未改 Rust/契约/锁文件/迁移、原件或普通下载，不修改共享 OnlyOffice、Caddy、宿主或其他消费者。用的是官方查看器，取消其原生匿名名称提示不妨碍预览。不支持 `credentialless` 的浏览器只有原件下载；不把无凭据隔离称为不可信供应商脚本的沙箱，撤销不能收回已发出的字节。

## 本地门禁

实际日志和退出码在 `/data/knoted-dev/tasks/t52/evidence/`：

| 命令/检查 | 结果 |
| --- | --- |
| `pnpm --filter @knoted/web exec vitest run tests/tasks/t52.test.tsx` | 27/27；首轮 RED 24 项，隔离轮 RED 18 项 |
| 同上 `tests/tasks/t49.test.tsx tests/tasks/t50.test.tsx` | 141/141 |
| `pnpm --filter @knoted/web test` | 候选和主线归档均 20 文件/416 通过 |
| `pnpm --filter @knoted/web lint` / `typecheck` | 候选和主线均 exit 0 |
| `pnpm --filter @knoted/web build`（unset 和精确 origin 两种） | 均 exit 0；发布 HTML 的 CSP 在 module 前，父页保留 `blob:` PDF，宿主页精确 HTTPS script/frame origin，禁止通配符、inline/eval；禁用构建无外部 SDK 路径/来源 |
| 后端门禁 | 本卡未改，沿用 T51 的实际验证，不计复跑 |

已保留既有 jsdom `scrollTo` 提示和 Vite 大 chunk 警告。候选测试早期先后记录 Knoted 固定产物目录 `0700` 导致既有 Caddy 403（校正自有产物权限即可）、Playwright 拦截 API 显示 Cookie 筛选前值（两端回环 canary 与 CDP 实际发包一致，证明误判）、截图变化不足以证明切表（补真实受信点击、active 标签和内容）。不将这些轮次当通过证据。

## 固定部署和配置复现

启用版通常可从主根构建：`KNOTED_OFFICE_ORIGIN=https://wlab.132140.xyz:15402 KNOTED_BUILD_ROOT=/data/knoted-dev/tasks/t52 bash tools/dev/build-artifact.sh "$full_commit_sha" "$new_output_path"`。Vite 继承该环境变量；不设置即有意构建禁用版。必须使用同一个规范小写 HTTPS DNS/IPv4 origin（无路径、查询、片段、IPv6 或显式默认端口），并与 Knoted API 的 `KNOTED_OFFICE_ORIGIN` 完全相同。Knoted 运行时另使用 `KNOTED_OFFICE_JWT_SECRET_FILE` 指向已授权的绝对凭据文件路径，不复制到产物或Web构建。未配置或不匹配时关闭 Office 功能，并保留正常下载。

本次实际命令 `bash /data/knoted-dev/tasks/t52/tmp/build-web-release.sh 4e722319dc850ee7627c94bb7d7919cdf0020e97 enabled`，从该 SHA 的 Git 归档执行冻结安装和显式配置 origin 的 Web 构建，复用与该 SHA 实际源字节不变的 T51 Rust、Python 与迁移。固定包 38,323 项全部校验，清单 SHA-256 `2957e12055dfd6754f91932988d272581938740f0b66a0fd214e65802332292c`。候选和主线产物的 Web 字节一致。`run-dev-acceptance.sh` 在验收失败时恢复禁用基线；主线实测成功后才保留启用配置。API、Worker 和 intelligence 执行路径均为该 SHA，API/intelligence ready，Caddy、OnlyOffice 两单元和 PostgreSQL PID 前后不变。

## 真实 HTTPS 主线验收

- `main-r1-browser.json` / `.exit`，Chromium `158.0.8127.18`、现有共享 OnlyOffice 10.0.0：三个 Office 类型实际渲染，使用签名只读模式。真实点击 `预算表`→`实际表`，active/selected 标签与 A1 `BUDGET_SHEET_A`→`ACTUAL_SHEET_B`、B1 `2026`→`2027` 同时变化；已独立看图。PDF 仍为原生 viewer、两页数据与截图确认。
- 宿主 `window.credentialless === true`、无 Knoted Cookie。已测 20 条真实 vendor wire 请求（含 4 SDK + 4 viewer frame）；0 条附带 Knoted Cookie。父页阻止 vendor 脚本和 frame、宿主阻止同主机错误端口，0 个 CSP 探针外发。只保存统计和自有合成夹具截图，不记 JWT、URL 查询或 Cookie/header 值。
- 各格式关闭后自身 DELETE 204、宿主移除并回焦；模拟一次 **浏览器侧** SDK 请求中断，真实应用清理该 ID 并显示 Retry，重试用新 ID 成功打开官方 viewer；撤销测试会话后，普通下载返回 401，登录生命周期清除弹层。没有改动共享服务来注入故障。会话撤销、浏览器关闭、自有夹具/目录删除全部成功，回读 0 个本任务来源仍 present。
- 主线脱敏截图目录 `/data/knoted-dev/tasks/t52/evidence/browser-89f22346-c43e-4018-8628-47a58e4a4b07/`，含 `xlsx-budget.png`、`xlsx-actual.png`、`word.png`、`slide.png`、`pdf.png`。证据仅涵盖当前 stock 环境和实测支持隔离的 Chromium；未来兼容性失败时关闭 Knoted Office 配置，而不是放宽共享服务/代理。
