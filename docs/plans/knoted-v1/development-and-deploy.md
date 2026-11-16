# 开发、测试部署与恢复

## 管理边界

测试实例的**应用**由本仓库的 `@bixlab/pcwrap` + process-compose 管理（[ADR 0003](../../decisions/0003-test-instance-process-management.md)）：配置渲染、迁移、API、独立 HTTPS 入口都在 `process-compose.yaml` 里，控制端口 `127.0.0.1:15600`。**外部依赖**（PostgreSQL 角色/库、Strata、机器级模型共享根、ACME 证书、防火墙）仍由 `/etc/nixos` 管理。**生产实例**将来才由 NixOS 运行，本次不部署。

日常入口：`pnpm pc up -D`、`process list`、`process restart <name>`、`process logs <name>`、`down`。应用改动不需要再改 `/etc/nixos` 的 pin；只有外部依赖或生产启用才产生 host 任务。真实切换已随 PR #64 完成（运行 `4a546b2`），结果与回退入口记在 [证据](evidence/pcwrap-dev-service.md)。

远程GitHub写默认授权；生产变更、仓库级设置与非 Knoted 服务重启仍按根 AGENTS.md 的确认边界。没有性能测试。所有代码编辑在主仓库 `.worktrees/<task>`。

## 路径与入口

| 用途 | 路径/值 |
|---|---|
| 外部测试入口 | https://wlab.132140.xyz:15601/ |
| RAG只读根 | /data/vaultsyncd-dev/dev-server/vaults/knoted-test |
| Agent样本唯一写区 | 上述根/agent-fixtures/<task-id>/ |
| 固定产物运行根 | /data/knoted-dev/runtime/\<full-sha\>/（`bin/knoted-api`、`share/knoted/{web,migrations}`），`current` 指针原子切换 |
| 稳定测试实例 | /data/knoted-dev/service/{config,state,assets,staging,logs,backups,bin}（属主宿主用户） |
| 编排控制端口 | 127.0.0.1:15600（pcwrap；15601 为 HTTPS、15602 为 API） |
| 临时任务目录 | /data/knoted-dev/tasks/<task-id>/{tmp,cache,build,evidence} |
| 隔离恢复演练 | /data/knoted-dev/tasks/<task-id>/restore |
| Strata独立安装 | /data/llm/strata |
| Strata共享服务 | `strata.service`（`User=strata`，127.0.0.1:15604）+ `strata-api-key.service`（oneshot）；客户端组 `strata-clients` |
| Strata运行凭据 | `/run/strata/api-key`、`/run/strata/auth-header`（root:strata-clients 0440；持久副本 `/var/lib/strata/api-key`） |
| OCR/嵌入模型 | 共享根 /data/llm/docling 与 /data/llm/bge-m3（按 revision 分目录，由环境变量指向） |
| 现有证书 | /var/lib/acme/wlab.132140.xyz/fullchain.pem 和 key.pem |

不复制私钥进仓库、模型目录、日志或Nix store；只配置受控读取。Nix store及/run运行凭据是声明式部署例外，临时脚本和构建/浏览器缓存不得散落/tmp或用户家目录。每任务配置TMPDIR、CARGO_TARGET_DIR、UV_CACHE_DIR、PLAYWRIGHT_BROWSERS_PATH及包管理缓存到任务区；测试实例持久状态不得指向任务Worktree。不清理其他任务目录。

## 服务约定

编排有四个进程：`prepare`（校验固定产物、渲染 `knoted.toml`/`Caddyfile`、准备目录、校验证书、检查端口、安装 ACME reload 入口）、`migrate`（一次性 `sqlx migrate run`）、`knoted-api`（127.0.0.1:15602）、`knoted-https`（独立 Caddy，HTTPS 15601，静态网页与 /api 同源，`X-Knoted-Revision`）。`knoted-api` 依赖 `migrate` 成功，`knoted-https` 依赖 API healthy。

PostgreSQL 使用独立库 `knoted_dev` 与专用角色，复用本机 PG 监听；角色/库 provisioning 由 `/etc/nixos` 的 `knoted-db` oneshot 完成。Python 15603、Strata 15604、Meilisearch 15605 端口保持预留，暂不纳入编排。端口启动前检查，占用时失败并报告，不杀其他进程或自动漂移。生产端口不在本计划内。

重启语义：`process restart knoted-api` 不重跑迁移；改模板、编排或环境变量后需 `down` 再 `up`（`prepare` 才会重跑）。

证书来源参考receiptd；不是复用其Next/Bun启动器。缺证书/过期/名称不符拒绝HTTPS入口启动；ACME续期reload测试入口。API readiness只检查阶段所需依赖；模型不可用时基本笔记业务仍可用并显示能力不可用。

专用服务用户knoted，源目录仅读取权限，assets/state/staging仅其可写。Python只见staging及模型，不给源目录和PG凭据。OnlyOffice使用当前服务地址作受信任依赖，JWT通过运行凭据引用，不输出内容。保持其已有外链限制和其他应用配置。

## /etc/nixos 交接

测试实例的应用更新**不再**产生 host 任务。host 仓库现在只负责：

- 实例目录与属主（`/data/knoted-dev/service` 归宿主用户）、机器级模型共享根（ADR 0001）；
- PostgreSQL 角色/库 provisioning（`knoted-db` oneshot）、Strata 与共享服务；
- ACME 证书、防火墙 15601、运行期可执行文件（`caddy`、`sqlx-cli`、`openssl`）；
- ACME 续期后的 `knoted-https-reload.service` 只触发实例安装的 reload 入口；
- 将来的生产实例（`features.server.knoted.app.enable`，本次关闭）。

需要改 host 时（外部依赖变化、生产启用）仍按 host 仓库规则建独立任务工作区：

1. 先读当前 flake、模块与主机名，不假定 Knoted 的路径规则可覆盖主机仓库。
2. `nix eval` 对应选项，构建系统候选，比较非 Knoted 单元差异。
3. 做 boot 候选安装与定向运行变更，避免会隐式重启共享 PostgreSQL/Strata 的全机 `switch`。
4. 记录主机提交 SHA；不得自动推送 host 仓库。

T10/T11/T12 的历史 app/host 拆分规则仅适用于已完成的那些外部依赖接入；`tools/dev/build-artifact.sh` 的产物构建仍在 app 侧。

## 部署门槛

每个改变运行行为且依赖齐备的任务，在审查集成后更新测试实例；迁移不兼容前端时功能开关默认关闭，保持当前功能可用。纯文档、未接入的库及测试任务注明无需部署。实例运行固定提交产物，不能从马上清理的任务工作树长期开服务——编排与运行根都在稳定位置（`/wrd/prj/knoted` 主检出 + `/data/knoted-dev/runtime/<sha>`）。

顺序：记录运行版本→相关测试→提交/审查→本地集成→主检出构建固定产物→`prepare` 校验→必要备份→迁移→切换 `current` 并 `process restart`（迁移则 `down` 后 `up`）→HTTPS/readiness/登录/本任务功能验证→保存 SHA 证据。不可逆迁移另需明确决策；没有授权时保留待部署，不声称完成。数据库回滚不能仅切旧二进制假装恢复。

## T68之前的迁移备份

T21起若后续任务需要迁移，而统一admin backup尚未实现，先用运维维护窗口完成最小一致备份：`pnpm pc process stop knoted-api`（必要时连同 Python 执行进程），确认无应用写入，再对knoted_dev执行pg_dump，并复制本实例assets及生成摘要清单到service/backups/<timestamp>/。命令须明确数据库和实例路径，不能停止共享PostgreSQL或其他应用。新空实例无历史数据时记录“空库，无旧数据”；有数据但无法备份则不迁移。T68完成后统一替换为管理命令。模型依赖和只读源目录不参与这个备份。

## 一致备份和恢复

管理命令规划为 `knoted-api admin backup --output <dir>` 和 `admin restore --input <dir> --target <isolated-config>`。backup进入维护模式拒绝写入、停止Worker领取和发布，等待/取消运行任务并确认安静，随后PG dump+assets清单/摘要，最后恢复服务。失败保持可诊断清单、不标完成；退出路径解除本次维护锁。

备份包括笔记历史、回收站、聊天引用与依赖、全部仍被引用图片、迁移版本和非密配置摘要。不备份源目录，索引可重建，密钥另按主机安全配置管理。恢复仅向新数据库与隔离资产目录，校验清单后迁移/重建全文和向量，撤销sessions/challenges/bootstrap凭据，使用现有Passkey重新登录或管理命令恢复。禁止默认覆盖正在运行实例。

## 回滚及健康

system回滚与数据回滚分开记录；新增迁移尽量兼容旧读取，部署前说明旧版本是否可启动。测试只做单场景功能和失败恢复，不测吞吐/延迟/规模容量。日志使用request_id/job_id/source_id，不打印原文、模型prompt、Cookie、JWT或私钥。
