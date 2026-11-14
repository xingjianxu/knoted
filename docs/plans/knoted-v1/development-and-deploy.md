# 开发、测试部署与恢复

## 权限

本次仅文档；未来实现通过验证后部署指定测试服务。远程GitHub写、生产变更、无关服务重启均未授权。没有性能测试。所有代码编辑在主仓库 `.worktrees/<task>`。本机部署配置修改单列host任务，遵守/etc/nixos规则及重大权限变化的确认边界。

## 路径与入口

| 用途 | 路径/值 |
|---|---|
| 外部测试入口 | https://wlab.132140.xyz:15601/ |
| RAG只读根 | /data/vaultsyncd-dev/dev-server/vaults/knoted-test |
| Agent样本唯一写区 | 上述根/agent-fixtures/<task-id>/ |
| 稳定测试实例 | /data/knoted-dev/service/{config,state,assets,staging,logs,backups} |
| 临时任务目录 | /data/knoted-dev/tasks/<task-id>/{tmp,cache,build,evidence} |
| 隔离恢复演练 | /data/knoted-dev/tasks/<task-id>/restore |
| Strata独立安装 | /data/llm/strata |
| OCR/嵌入模型 | /data/knoted-dev/service/models |
| 现有证书 | /var/lib/acme/wlab.132140.xyz/fullchain.pem 和 key.pem |

不复制私钥进仓库、模型目录、日志或Nix store；只配置受控读取。Nix store及/run运行凭据是声明式部署例外，临时脚本和构建/浏览器缓存不得散落/tmp或用户家目录。每任务配置TMPDIR、CARGO_TARGET_DIR、UV_CACHE_DIR、PLAYWRIGHT_BROWSERS_PATH及包管理缓存到任务区；测试实例持久状态不得指向任务Worktree。不清理其他任务目录。

## 服务约定

公网Caddy HTTPS15601，静态网页和/api同源。内部默认API127.0.0.1:15602、Python127.0.0.1:15603、Strata127.0.0.1:15604、Meilisearch127.0.0.1:15605；PG独立数据库knoted_dev和专用角色，复用本机PG监听。端口启动前检查，占用时停止并报告，不杀其他进程或自动漂移。生产端口不在本计划内。

证书来源参考receiptd；不是复用其Next/Bun启动器。缺证书/过期/名称不符拒绝HTTPS入口启动；ACME续期reload测试入口。API readiness只检查阶段所需依赖；模型不可用时基本笔记业务仍可用并显示能力不可用。

专用服务用户knoted，源目录仅读取权限，assets/state/staging仅其可写。Python只见staging及模型，不给源目录和PG凭据。OnlyOffice使用当前服务地址作受信任依赖，JWT通过运行凭据引用，不输出内容。保持其已有外链限制和其他应用配置。

## /etc/nixos交接

每个host阶段是独立任务，不与Knoted应用提交混合：

1. Knoted提交包/模块及说明，记录SHA，相关构建和验证通过。
2. 在/etc/nixos按其AGENTS.md建立合规任务工作区；先读当前flake、模块和主机名，不假定Knoted的路径规则可覆盖主机仓库。
3. 仅新增Knoted feature/包、锁定引用与必要本机启用；模型服务安装依次Docling/BGE/Strata，分别验证和提交。
4. nix eval对应选项，构建系统候选，保存前代system引用；确认没有无关服务变更再激活测试配置。
5. 检查测试服务及既有依赖健康；失败恢复前代配置，不覆盖应用数据库。
6. 将主机提交SHA和应用SHA写入Knoted任务交接。不得自动推送任一仓库。

T10/T11/T12各自拆固定两个交付子任务：`Txx-app`只改Knoted客户端/fixture/验证报告；`Txx-host`只改/etc/nixos安装启用。每个子任务一个Agent，app先定义契约，host验证，再补app验收；不能以一个Agent任务同时改两仓库。T21/T70同样采用app/host交接。

## 部署门槛

T21建立持续测试实例。之后每个改变运行行为且依赖齐备的任务，在审查集成后更新dev；迁移不兼容前端时功能开关默认关闭，保持当前功能可用，不等待整个阶段才部署。纯文档、未接入的库及测试任务注明无需部署。部署使用固定提交构建产物，不能从马上清理的工作树长期开服务。

顺序：记录运行版本→相关测试→提交/审查→本地集成→固定构建→必要备份→迁移→更新实例→HTTPS/readiness/登录/本任务功能验证→保存SHA证据。不可逆迁移另需明确决策；没有授权时保留待部署，不声称完成。数据库回滚不能仅切旧二进制假装恢复。

## T68之前的迁移备份

T21起若后续任务需要迁移，而统一admin backup尚未实现，先用运维维护窗口完成最小一致备份：停止本测试实例API/Worker及其Python执行，确认无应用写入，再对knoted_dev执行pg_dump，并复制本实例assets及生成摘要清单到service/backups/<timestamp>/。命令须明确数据库和实例路径，不能停止共享PostgreSQL或其他应用。新空实例无历史数据时记录“空库，无旧数据”；有数据但无法备份则不迁移。T68完成后统一替换为管理命令。模型依赖和只读源目录不参与这个备份。

## 一致备份和恢复

管理命令规划为 `knoted-api admin backup --output <dir>` 和 `admin restore --input <dir> --target <isolated-config>`。backup进入维护模式拒绝写入、停止Worker领取和发布，等待/取消运行任务并确认安静，随后PG dump+assets清单/摘要，最后恢复服务。失败保持可诊断清单、不标完成；退出路径解除本次维护锁。

备份包括笔记历史、回收站、聊天引用与依赖、全部仍被引用图片、迁移版本和非密配置摘要。不备份源目录，索引可重建，密钥另按主机安全配置管理。恢复仅向新数据库与隔离资产目录，校验清单后迁移/重建全文和向量，撤销sessions/challenges/bootstrap凭据，使用现有Passkey重新登录或管理命令恢复。禁止默认覆盖正在运行实例。

## 回滚及健康

system回滚与数据回滚分开记录；新增迁移尽量兼容旧读取，部署前说明旧版本是否可启动。测试只做单场景功能和失败恢复，不测吞吐/延迟/规模容量。日志使用request_id/job_id/source_id，不打印原文、模型prompt、Cookie、JWT或私钥。
