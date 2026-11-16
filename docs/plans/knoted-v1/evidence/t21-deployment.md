# T21 最小 HTTPS 测试部署证据

状态：app/host实现、部署验收与主线集成验证通过；最后文档分支随收尾PR合入后清理，读回结果记PR。最终运行版本见下方“真实部署发现与最终方案”。

用户直接委派完成 T20—T21；Issue [#51](https://github.com/xingjianxu/knoted/issues/51)，计划 [实施清单](../../2026-11-16-t21-implementation.md)。T20 为 #50 / PR #52 / 集成e8d2c4f / 实现d68f213，前端54用例（T20 18）；本卡不添加业务接口或迁移。

## app 产物契约

`tools/dev/build-artifact.sh <full-sha> <new-output>` 使用 Git archive 隔离源码，Rust --locked release、pnpm frozen/build；输出 bin/knoted-api、share/knoted/web、migrations、REVISION及SHA256SUMS。源码、缓存及日志在 /data/knoted-dev/tasks/t21；稳定服务不引用 Worktree。nix/package.nix 校验完整规范清单、拒绝符号链接和特殊文件、检查迁移和网页/API后复制；Nix保留ELF中的store库引用。该入口是本机已验证构建产物的封装，不声称Nix从源码重建所有前端/Rust依赖。产物来源由固定源码构建过程取证，清单用于传递过程完整性校验。

services.knoted 提供 enable/package/configFile/user/group 和仅API的 knoted-api 单元；host负责账号、PG、迁移、证书与代理。模块 ProtectSystem=strict/ProtectHome/NoNewPrivileges，配置为绝对运行路径，机密不入store。

## 已运行的验证与审查

完整输出：/data/knoted-dev/tasks/t21/evidence/。

- build-input-validation.log：短SHA、未知完整SHA拒绝，输出目录未创建。bash -n exit0；Nix package/module parse exit0；module-eval.json实际求值API单元。
- package-valid.log / package-tampered.log：完整清单接受，内容篡改拒绝。
- 独立 reviewer /root/review 发现 Important：仅sha256sum -c会接受遗漏项；package-incomplete-red.log 亲自复现接受遗漏清单。修复为重新计算全部文件并cmp后，package-incomplete-green.log拒绝、package-complete-green.log接受，exit符合预期。没有未处理Minor。
- baseline-fmt.log / baseline-rust-test.log / baseline-clippy.log：Rust fmt/test/clippy exit0，85 passed（1既有ignored）；最终主线门禁另附。

## host 交接与规则解释

独立 /root/t21_host 只改 /etc/nixos 的专用 Worktree，原主机 main 8832361，用户未提交 features/ai/pi.nix 保留、不进入本任务提交；首次候选排除该文件后，dry发现将撤销当前运行的Superpowers插件并重启Home Manager。主Agent独立diff确认唯一home-files差异为.pi/agent/settings.json，因此后续候选原样读取用户现有pi.nix，以保持已运行环境；集成仍只提交本任务文件，原工作区用户diff保持原样。端口15601/15602原空闲，现有ACME证书覆盖 wlab.132140.xyz且有效至2027-01-11。专用账号knoted已存在；新PG角色/库均knoted_dev，沿用现有本机认证，不扩大PG权限。模型目录属主/权限保持ADR0001。

初案采用共享Caddy桥接，独立审查指出全局证书guard会阻碍其他站点启动/reload，故改为独立 knoted-https.service；不修改 services.caddy.virtualHosts 或绕开其 features.server.caddy.virtualHosts 桥接。主机AGENTS该桥接规则约束共享NixOS Caddy的vhost option，独立进程使用自己的Caddyfile，没有写上述option。独立单元使用既有caddy身份读取现有证书，仅15601，auto_https off，admin为私有Unix socket，运行数据/配置均独立/run目录；证书校验只约束本服务。此解释如不成立，代价是主机规则需补充独立测试listener用法；当前方案避免测试证书使生产站点停机。

证书validator已用真实fixture验证有效/过期/错误域名/缺失四项；实际启动、续期reload、HTTPS与运行SHA待固定包后验收。

最终app提交前门禁：t21-precommit-gates.log，frozen install/api/typecheck/lint/test/build/fmt/Rust test/clippy/diff全部exit0，前端54 passed、Rust85 passed（1既有ignored）。

真实固定产物构建：artifact-build.log exit0；完整SHA 5a699bf521530771ab826bf1604ef48f2a9ce7f8。输入artifact /nix/store/hah9gxqg3qdgsmkslpg1f60d0jv40jsq-artifact-5a699bf，NAR hash sha256-ZdGe5dMhYxEZugFMaYGLKZLuopmpqtiNWR0NlmgZodU=；包 /nix/store/n55vmc7s1lrm57pzdy9azh408xjc46jv-knoted-5a699bf52153。API --help与real-package-integrity.log完整摘要检查通过；nix-store --query --references确认保留glibc/gcc运行库。


## 真实部署发现与最终方案

1. 初次全机test虽普通dry未显示无关restart，实际停止postgresql-setup，沿postgresql.target Requires与postgresql.service PartOf传播重启PG及其应用；local-fs.target的X-StopOnReconfiguration还重启Strata。相关服务已恢复active。普通dry默认隐藏target，STC_DISPLAY_ALL_UNITS=1及反向依赖检查才完整显示；不把此轮说成“无无关重启”。证据在host implicit-restarts.log / dry-display-all-targets.log。后续取消共享PGensure声明和setup依赖，使用专用knoted-db；不修改共享服务策略，也不再全机test/switch。
2. tmpfiles跨属主路径拒绝创建配置，导致API502；用仅管理固定service子树的knoted-prepare替代，拒绝符号链接目录和覆盖既有真实配置，API依赖其成功。父目录和共享模型归属不变，后续ready200。
3. 真实浏览器发现既有GET /api/v1/auth/session404，T15原测试仅自建guard Router漏掉实际HTTP适配器。独立Issue #54由PR [#55](https://github.com/xingjianxu/knoted/pull/55)修复，master e2b3dab；新增真实build_router7例RED→GREEN，Rust92 passed/1既有ignored，独立审查和主Agent复跑通过。无API/schema更改。

最终app固定源码 **f35a69a4ec0686ab08ae448bd4bf33d96ecce942**（T21合并补修后的提交）。artifact-final-build.log为锁文件构建，source store `/nix/store/whhabrjz43yki10ypgz44128qk6vn9lp-artifact-session-http`，NAR `sha256-s0++IOkkExrCo9+QTD+cesHEVasmQBRa/w3fI85sHjs=`；显式GCroot `/nix/var/nix/gcroots/knoted-t21-artifact-f35a69a`。运行包 `/nix/store/gwz1f7pb6fqam460rnydhv67z6pva7r7-knoted-f35a69a4ec06`。早期5a699bf仅是发现缺陷前的构建，不是最终运行版本。

最终host候选与boot profile为 `/nix/store/v9jyba8b24gglhiw1h5xwqhman9zyh14-nixos-system-wks-lab-26.11.20261109.5e6fbb3`，boot动作exit0建立持久GCroot和下次启动配置。为避免已证实的隐式生产重启，运行期仅原子加载精确单元覆盖树 `/run/knoted-system-units-v9jyba8b24gglhiw1h5xwqhman9zyh14`：仅替换knoted-db/prepare/migrate/api/https，606个非Knoted条目的内容/链接/模式保持一致。运行system元数据仍指向先前zavsc...测试代，但5个实际单元逐字等于boot候选；没有声称做了最终全机switch或重启。

主Agent独立cmp五个单元、diff既有服务pre/post PID+active（空差异）、/proc/1219453/exe指向上述最终包。主AgentHTTPS ready=200 / {"status":"ready"}，X-Knoted-Revision=f35a69a完整SHA；明文15601只返回400“Client sent an HTTP request to an HTTPS server.”。对应输出 https-final-ready-* / plain-http-final-*。t21_1失效证书拒绝及无HTTP回退、t21_2 readiness/登录、t21_3产物/SHA均已实际验证。

## 真实浏览器认证与首次人工使用

`real-https-browser-final.log`，Chromium通过真实HTTPS+真实API+虚拟WebAuthn：保护页401转登录；bootstrap/options200、finish201；退出204且session401；Passkey真实签名login200并返回/settings/security；凭据/会话列表200；撤销当前会话204且session401。令牌只在内存，未写日志、URL或仓库。单元测试不替代此验证。

当前测试库有名为 `T21 virtual test Passkey` 的验收凭据，无存活浏览器会话；虚拟认证器已关闭。首次人工使用通过本机签发恢复令牌，再在[/recovery](https://wlab.132140.xyz:15601/recovery)注册自己的实体Passkey，旧测试凭据/会话按已实现恢复语义撤销：

```bash
sudo -u knoted /nix/store/gwz1f7pb6fqam460rnydhv67z6pva7r7-knoted-f35a69a4ec06/bin/knoted-api admin token --purpose recovery --config /data/knoted-dev/service/config/knoted.toml
```

命令输出是一次性秘密，仅供操作者本机使用，不粘贴Issue/日志。真实硬件Passkey和整机重新启动未人工验证；最终配置已构建、boot安装、实际五单元与候选一致，现有业务PIDs保持稳定。


## 最终审查与回退验证

最终独立host审查仅1项Important：定向激活异常回退原先只恢复运行单元，没有恢复预先更新的boot profile。已补真实异常分支的finally恢复旧profile并执行旧system的boot动作，分别记录runtime/boot恢复结果，不将失败误报成功；主Agent独立运行两项mock流程通过（正常双恢复、boot profile失败如实记录），日志host-rollback-independent.log。未在成功部署上人为触发故障。无未处理Minor。

独立TLS实际续期reload exit0且自身PID不变，共享Caddy PID不变；host最终报告含四场景证书validator结果和20文件manifest。host执行记录与精确回退入口：/data/knoted-dev/tasks/t21-host/evidence/report.md（定向helper会同时恢复runtime入口和previous-boot-system；手工回退仅boot，不执行已知有隐式传播的全机test/switch）。

最终host提交：`33450eec0feadb608d63c3932bb5810635d90c15`，已ff集成 `/etc/nixos` main（未push），仅四个本任务文件；用户Pi diff SHA256仍为a18658e2d381c776fa48506e241692ff3039a9dbdbb9e42e66135b5a93ca17c4。候选/运行状态未因提交而改变；主机任务工作区在主线核验后清理，其他任务worktree保留。

## 主线与清理核对

PR [#53](https://github.com/xingjianxu/knoted/pull/53)普通merge至master `d623582ec073ba12f1d3351da60553bf028e659a`，运行源f35a69a是其祖先。主Agent t21-mainline-gates.log frozen install/api/typecheck/lint/test/build/fmt/Rust test/clippy/diff全部exit0；前端54、Rust92（1既有ignored）。既有Chromium15通过，实际HTTPS虚拟Passkey全流程通过。

Issue #50/#51/#54均由PR关闭，Projects Done；host main33450ee，主线pure eval仍精确对应已部署boot候选，13项服务active及公网ready200，host任务worktree/分支已清理，其他vaultsync任务worktree和原Pi改动保留。Knoted T20与HTTP补修工作区/分支已清理；最后docs/t20-t21-close文档分支在收尾PR合入后清理，最终主线SHA、worktree list、git status和ready读回写PR核对项，不为记录清理再创建循环文档提交。
