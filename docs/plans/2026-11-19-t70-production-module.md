# T70 NixOS 模块与生产主机交接

[T70](knoted-v1/tasks/T70.md) 依赖 T21 的固定产物和 T69 的隔离恢复。模块沿用 [ADR 0003](../decisions/0003-test-instance-process-management.md)、[ADR 0004](../decisions/0004-database-owned-by-host-user.md)、[ADR 0007](../decisions/0007-execution-exit-confirmation.md) 与 [ADR 0011](../decisions/0011-consistent-backup-and-isolated-restore.md)。实现和验证分别在 Knoted 应用工作树与隔离 NixOS 虚拟机进行。生产主机接入单独授权。

## 依赖和分工

- 主 Agent 将已验收的 T69 修订构建为固定产物，复核包、启动/停止与验收结果。
- 应用实施者使用独立工作树，范围为 `nix/`、`tools/dev/` 及 T70 验证记录。宿主的数据库创建、Caddy 和 ACME 由现有管理流程负责。已授权的 host 工作使用其仓库单独的工作树。
- dev 通过 pcwrap 保持原控制实例和路径。虚拟机使用独立数据库、实例目录、会话、Meilisearch 服务和测试 HTTPS。模型以只读挂载方式接入。

### Task 1: 固定应用包和生产模块

- [x] 验证完整 `SHA256SUMS`、`REVISION`、Python 构建记录、API、Worker、Web、迁移和 Python 入口及依赖工件。
- [x] 使用同一份本机 TOML 对 API、Worker 及迁移预检。由宿主预建 PostgreSQL，字段中的凭据留在 Nix store 外。常驻单元重启无需再次迁移。
- [x] 显式配置独立运行根、指定服务账号、回环监听和模型路径。资产、暂存与私有 Python 账本分别设置读写权限，源目录只读。
- [x] Worker/业务 API 独立于外部索引和推理就绪。Worker 负责任务停止，在正常停机时先于 Python；Python 进程组异常退出由 [ADR 0007](../decisions/0007-execution-exit-confirmation.md) 的 guardian 收束。以虚拟机证据确认 systemd 停机和主进程异常退出两条路径。

### Task 2: 隔离虚拟机验收

- [x] 验证内部端口从虚拟机外部不可达，HTTPS 正常。验证占用端口阻止对应启动而保留原进程，验证测试证书更新。
- [x] 关闭虚拟机内 Python、索引或模型，确认核心笔记在运行中与冷启动后可用。使用合成作业核对正常停止、主进程异常退出和 guardian 证明缺失。
- [x] 保留虚拟机磁盘、RP ID 和 origin 重启，使用同一 Passkey 登录，验证数据、图片及状态仍在。记录精确可执行文件来源和校验。

### Task 3: 生产主机交接

- [x] 明确主机、独立数据库/路径、HTTPS origin 和 RP ID、专用回环地址与索引实例、凭据位置、GPU UUID、依赖包、发布 SHA 和回退方式。
- [x] 获得规则 12 下单独的实际启用授权，核对存量主机变更和端口，保留已运行系统代。生产库是新建空库，无生产前代备份。
- [x] 通过主机仓库独立审核及本机功能验收，记录主机提交和实际激活闭包。T71 使用已核验的固定应用。
- [ ] 验证生产域名的独立公网路径。

生产接入经维护者明确授权，域名为 `kn.bixlab.cn`。候选沿用 ADR 0004 的本机 PostgreSQL 认证，业务使用独立新数据库。KVM 固定产物为 `5308378799a768ded33d26d1e08c37473f30eca5`。

实际在 wks-lab 定向启用，完整系统代保持原位。主机运行、正式 HTTPS 及公网 443 的实测范围见 [T70 验收证据](knoted-v1/evidence/t70-deployment.md)。
