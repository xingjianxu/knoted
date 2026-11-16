# T21 最小 HTTPS 测试部署证据

状态：app 实现就绪，待固定提交构建、host 激活与最终集成；不能据此称部署完成。

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

独立 /root/t21_host 只改 /etc/nixos 的专用 Worktree，原主机 main 8832361，用户未提交 features/ai/pi.nix 保留且不进入候选。端口15601/15602原空闲，现有ACME证书覆盖 wlab.132140.xyz且有效至2027-01-11。专用账号knoted已存在；新PG角色/库均knoted_dev，沿用现有本机认证，不扩大PG权限。模型目录属主/权限保持ADR0001。

初案采用共享Caddy桥接，独立审查指出全局证书guard会阻碍其他站点启动/reload，故改为独立 knoted-https.service；不修改 services.caddy.virtualHosts 或绕开其 features.server.caddy.virtualHosts 桥接。主机AGENTS该桥接规则约束共享NixOS Caddy的vhost option，独立进程使用自己的Caddyfile，没有写上述option。独立单元使用既有caddy身份读取现有证书，仅15601，auto_https off，admin为私有Unix socket，运行数据/配置均独立/run目录；证书校验只约束本服务。此解释如不成立，代价是主机规则需补充独立测试listener用法；当前方案避免测试证书使生产站点停机。

证书validator已用真实fixture验证有效/过期/错误域名/缺失四项；实际启动、续期reload、HTTPS与运行SHA待固定包后验收。

最终app提交前门禁：t21-precommit-gates.log，frozen install/api/typecheck/lint/test/build/fmt/Rust test/clippy/diff全部exit0，前端54 passed、Rust85 passed（1既有ignored）。
