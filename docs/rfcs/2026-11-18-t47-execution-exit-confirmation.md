# RFC: T47 计算进程退出确认与崩溃恢复

- Date: 2026-11-18
- Status: Resolved
- Owner: 用户决策，主 Agent 整理
- Related issues / ADRs: [T47 #132](https://github.com/xingjianxu/knoted/issues/132)、[T48 #133](https://github.com/xingjianxu/knoted/issues/133)、[ADR0006](../decisions/0006-file-pipeline-and-aggregate-contracts.md)
- Decision record: [ADR0007](../decisions/0007-execution-exit-confirmation.md) Accepted。用户于 2026-11-18 本会话针对该 RFC 确认问题回复“通过，基于执行”；按推荐方案实施。原 ADR0006 继续有效。

## 问题、目标与非目标

T47 要求取消、超时和重启时确认计算进程退出后再释放资源；Rust 租约失效也不能让旧计算与新尝试无约束重叠。现有 ExecutionRecord 只有状态，没有整个计算后代树退出的证据。`interrupted` 只表示不可继续本次计算，HTTP 超时只表示没有收到答复；`communicate()` 仅回收直接子进程。

已完成的独立短时实验实测：直接 child 结束且 communicate 返回后，关闭 stdio 的孙进程仍可存活；独立 guardian/subreaper 在模拟 API 被 SIGKILL 后，可以从私有管道 EOF 触发取消、回收两个后代至 ECHILD，再退出。实验全部自有进程已清理，不曾操作真实应用。完整事实、候选与边界见 `/data/knoted-dev/tasks/t39-t50/reports/t47-crash-lifecycle-readonly.md`，实验输出 `/data/knoted-dev/tasks/t39-t50/research/t47-lifecycle/probe-result.json`。这不是产品生命周期已经通过的证据。

目标是在现有 jobs、execution_id、SQLite 与 HTTP 路由上完成可校验退出确认，保持单 GPU 槽及 900 秒执行上限。源目录继续只读，CPU 为当前 dev 选择，模型与 Strata 不变。不新增第二套业务队列、不改公开执行状态枚举、不配置宿主 cgroup/权限、不做生产部署；不能声称任何不可中断内核等待都能在900秒内消失。

## 现状和备选方案

| 方案 | 接口与运维影响 | 评价 |
|---|---|---|
| 维持状态字段，并将 interrupted 或任意 terminal 当退出确认 | 无 DTO 改动，但旧实现与崩溃窗口没有后代退出证明 | 不能满足取消/资源安全要求 |
| 增加明确退出确认字段，intelligence 内部 guardian 持有计算组身份并回收后代 | 一个内部 DTO 补充；私有账本增量与轻量进程；不改宿主管理边界 | 推荐，可单独验证、失败时保留占用 |
| 宿主提供每执行独立 cgroup/外部常驻监督器 | 涉及主机配置、权限及新监督边界 | 将来可评估，本次范围不采用 |

只依赖 root PID/starttime 后调用数字 killpg 仍有检查与信号之间的竞争；pidfd 是当前内核的身份引用，但其整数不能跨服务重启持久化。只拿到 flock 或发现旧 PID 不存在，也不能代替整个计算树已退出的证明。

## 推荐方案

### 1. 内部 ExecutionRecord 增加 `compute_exit_confirmed`

`api/intelligence.openapi.yaml` 的 ExecutionRecord 增加可选 boolean 字段 `compute_exit_confirmed`，默认 false。Rust 反序列化缺失字段视为 false，Python 对新响应明确返回该值。它表示该 execution **不会再启动计算，且其已启动的整个受监督计算后代树已确认回收**；仅记录 PID、已发送信号、leader 已退出、监督线程结束、HTTP 超时或 interrupted 状态都不能设 true。

同一 execution 已持久的 true 不反转，也不复活；旧 false 不能因为升级或 PID 不存在批量补 true。字段只允许与 terminal 状态一起为 true。queued/running/cancel_requested 必须 false；尚未启动就取消时，先以不可再启动的终态/启动栅栏证明后可 true。succeeded/failed/cancelled/interrupted 的 true 均须来自本协议的完整退出凭证或明确的从未启动证明。终态业务结果不重写；退出证据可以从未确认补齐为确认，不得把一个终态改成另一个终态来伪造取消。

Rust 原 `ExecutionState::cancellation_confirmed()` 继续只认 cancelled，含义不变。新增独立资源确认判断，同时要求匹配 execution/request echo、terminal 状态及 compute_exit_confirmed=true。取消 HTTP 返回200也必须检查证明；409后GET对应记录并验证证明，不能只信旧状态 helper。它用于允许新尝试、确认 jobs 取消后的资源释放和接收成功产物；不能用它把 succeeded 说成“计算被取消”。成功发布还须 succeeded、完整manifest/摘要及来源/租约栅栏。interrupted 为可重试终态，新 execution_id 也必须等前序资源确认后才启动。

该字段不暴露 PID、绝对路径、进程组或秘密，不改变 ExecutionRequest、kind、状态枚举或结果 manifest 结构。老 Rust 客户端有 deny_unknown_fields，会拒绝新字段，因此 Rust/Python 必须同版本部署；新客户端面对旧服务字段缺失会安全拒绝放行，不能宣称可混合版本无损运行。

### 2. 单一内部监督所有者与持久恢复

每个 execution 在 intelligence 包内部创建轻量 guardian；先设置 subreaper 和启动屏障，再加载计算模块。guardian 是唯一发 TERM/KILL 和 wait 的所有者，持有新建计算组的 pidfd；当前 Linux 支持 PIDFD_SIGNAL_PROCESS_GROUP，能力不足则拒绝启动，不回退到不安全的持久数字 killpg。

服务在 SQLite/私有原子记录中增量保存 launch intent、owner epoch/随机标记、READY/GO 阶段、诊断身份与版本化退出凭证。先持久登记再允许 compute，计算 bootstrap 在 guardian 取得内核身份前不得加载模型；父子关闭无关 pipe 副本，避免假活连接。API 正常退出或 SIGKILL 后，健康 guardian 从 owner 管道 EOF 触发收束；同组及被收养后代都须回收至 ECHILD，再 fsync/原子登记凭证。guardian 专属一个 execution，无无关子进程、无第二 waiter，保留可 wait 的 SIGCHLD；先停止新 spawn 再清理至 ECHILD。服务锁 FD 不传给 guardian，owner pipe 各端的继承/关闭有明确所有者。正常完成、取消、超时、spawn 后异常、结果登记失败均走同一收束路径。

SQLite open 不再仅凭旧状态自行清空资源。state_dir 单实例锁先于恢复；重启读取旧执行与凭证后恢复 general/GPU 占用。guardian 健康时自动等待清理；guardian 也死亡、启动窗口无完整证据、凭证损坏/丢失或内核拒绝结束时，health=degraded，保留不确定占用并拒绝新计算。不能以 finally、重新建 semaphore、拿到锁或找不到 PID 归零。此选择优先避免旧/新计算重叠，代价是少数无法证明的故障需要人工排查，不能保证所有崩溃都自动恢复。

内部记录不保存文档正文或凭据，也不成为业务数据库。Python 私有 schema/记录原子性和重复写 CAS 必须测试；不追溯把旧版本终态标成已确认。初次激活须读回旧实例执行状态，未知旧记录保守处理。关闭服务必须停止 admission，并在明确收束或保持不确定占用后结束；不伪称限时 join 就已退出。

### 3. Rust 持久 attempt 关联与迟到请求

file.extract 的 jobs payload 保存每次 claim/attempt 对应的 execution_id、完整不可变 ExecutionRequest 与协议 revision，追加写必须在当前 jobs lease CAS 事务内完成，并先于 POST；不需要新的业务队列。自动/人工新尝试都使用新 ID；一次网络重发使用原 ID、原 deadline，不借重放延期。

接管时先收束前序 attempt，不能只查新 attempt 或因 GET404 就认为旧 POST 永远不会到达。已有路由下使用同 ID/原请求 materialize/replay 后 cancel，再观察退出确认；迟到原 POST 只能重放既有记录，不再产生新计算。同 ID 比较完整不可变请求（包括原 deadline），记录在没有安全回收协议前不删除或重建。若首次 materialize 时 deadline 已过，仍持久登记绑定原请求的 failed/截止终态（可返回现有201 ExecutionRecord），不给 GO，compute_exit_confirmed=true；重复提交仍只能重放该终态，不能因过期拒绝而留下未来可启动的空洞。原请求若已抢先创建记录则按该记录收束。任何超时、503、参数冲突或仍查不到记录都继续未确认，不能开启后继。这可能短暂启动一个原本未送达的旧请求，必须受同一资源预算和取消栅栏约束，并用延迟 POST 真实测试验证。

旧 lease 不能登记新关联或发布；取消清理只能操作它实际登记的 execution，不碰新 attempt。发布仍复核 lease、源 content_version、present/fresh 与 captured/current observation；compute_exit_confirmed 不是来源新鲜度证明。

### 4. 保留稳定失败分类

复用现有 ApiError.code（已是非空 string，客户端按 code 分支），将安全机器码而非展示 message 用于失败分类。损坏、不支持、非法编码、输入/像素/页数超限、执行超时为明确永久失败；模型/临时 IO 依赖不可用按既有有限重试。未知错误保守分类且最多既有4次尝试，不自动下载或切换模型。新增 Office 安全预算的具体内部实现由 T48 在现有限额和受限产物原则内评估，不能在此伪造已定参数。

## 影响与迁移

- 新字段属于 Rust/Python 跨模块接口补充；已由本次用户独立确认，见 ADR0007。原 ADR0006 的确认本身不包含该退出证明。
- Python 账本只增量补列/私有记录，旧数据保留；PG 沿用已批准 file_extractions 的 execution_id 与 opaque jobs payload，不改变业务身份或正文版本。
- 先补 OpenAPI/正反示例/客户端类型及双端测试，再实现 guardian/attempt/解析处理。两端从同一固定 SHA 产物启动，父子使用固定解释器/锁定依赖，禁止从 worktree 或可编辑环境运行持续服务。
- dev 停本实例写进程及新增 intelligence 进程，保留 PG/assets 与 SQLite/私有账本一致备份，验证后切换；停止边界须等待/核对 guardian，不能只检查 API/Worker PID。共享服务与模型不动。
- 回退保留新记录，使用理解新退出语义的兼容构建；不得直接切旧 Python 并把旧逻辑重建 semaphore 当作安全恢复。生产和破坏性恢复仍需单独授权。

## 验证与开放问题

用户已确认整个推荐方案，包含 compute_exit_confirmed 字段及 guardian 无法确认时保留占用的取舍。以 ADR0007 补充 ADR0006，不改写其原决策。

实施验收至少包括：leader先退而孙进程存活、重复取消/TERM忽略、spawn前后各崩溃窗口、API SIGKILL/正常重启、guardian也死亡及损坏凭证时不放行、不误杀无关sentinel、迟到POST与丢响应、租约接管、自然终态与取消409竞态、短化deadline验证900秒上限动作、旧观察结果拒绝、真实离线中文OCR、500页/40M/100MiB边界、固定产物搬移及运行后摘要不变。不得用 mock 或 interrupted 状态替代实际子进程退出取证。

guardian 早期实现、具体 SQLite 私有字段布局及可检验凭证格式仍由 T47实施并经独立审查确定；如果需要新增另一个公共字段、状态或宿主权限，再回到架构确认。上述实验仅支持方向，完整启动窗口、锁定3.12解释器能力、真实process-compose停止边界尚未验证。

## 评审记录

- 2026-11-18：只读 research_t47_crash_lifecycle 完成独立 owned-process 实验及候选报告；主 Agent 起草本 RFC，未实施协议字段或产品 guardian。
- 2026-11-18：独立 review_t47_exit_rfc 给出 PASS，可交用户审阅；补强了旧helper调用点、guardian/ECHILD前提、完整请求与deadline幂等绑定、证明单调及legacy边界。报告 `/data/knoted-dev/tasks/t39-t50/reviews/t47-exit-rfc-review.md`，不是人工批准。
- 2026-11-18 人工决定：用户针对本补充 RFC 明确回复“通过，基于执行”，推荐方案获批；见 ADR0007。此记录不是实施或运行验收完成。
