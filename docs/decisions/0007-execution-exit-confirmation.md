# ADR 0007: 计算退出证明与持久监督

- Date: 2026-11-18
- Status: Superseded
- Decision owner: 用户 Xingjian Xu
- Confirmation: 2026-11-18 本会话，主 Agent 明确询问是否确认新的 11 月 18 日补充 RFC 并按推荐方案继续；用户紧接该问题回复原文“通过，基于执行”。确认对象为 [RFC 推荐方案](../rfcs/2026-11-18-t47-execution-exit-confirmation.md)，包括内部退出证明字段、guardian 与私有持久凭证、无法确认时保留占用，以及 Rust 持久尝试关联/迟到请求处理。不包含生产、破坏性恢复或宿主权限变更。
- Related issues: T47 [#132](https://github.com/xingjianxu/knoted/issues/132)、T48 [#133](https://github.com/xingjianxu/knoted/issues/133)
- Related PRs: 方案 [#152](https://github.com/xingjianxu/knoted/pull/152)、T47 [#157](https://github.com/xingjianxu/knoted/pull/157)
- Supersedes: 无；补充 [ADR0006](0006-file-pipeline-and-aggregate-contracts.md)，其原决策继续有效
- Superseded by: [ADR 0014](0014-retire-file-index-rag.md)（计算执行服务）

## 背景与问题

T47 要求取消、截止和重启后确认计算退出再释放资源。直接 child 的 communicate/wait、HTTP 超时、interrupted 或任意终态，都不能独自证明整个计算后代树已回收。独立自有进程实验已复现 leader 退出后孙进程仍存活，并验证当前内核及锁定 Python 3.12 的 guardian/pidfd/subreaper 方向；这不是产品实现已经通过的证据。

## 决策内容

1. 内部 ExecutionRecord 增加可选 boolean `compute_exit_confirmed`，缺失默认 false；新 Python 响应明确返回。true 只允许 terminal，并表示该 ID 不会再启动计算且已启动的整个受监督后代树已回收。证明 false→true 可补齐且单调；不改写终态业务结果，不凭升级、PID 不存在或已发信号批量补 true。
2. 保留 ExecutionRequest、状态枚举、manifest 与旧 `ExecutionState::cancellation_confirmed()` 的 cancelled-only 含义。Rust 独立资源确认须核验完整已有 request echo、terminal 及退出证明；200 cancel 和 409 后的对应记录都不能绕过。允许后继尝试、释放资源、接收成功结果均依赖证明；成功发布另需 succeeded、完整产物摘要、deadline、当前 lease/来源/观察栅栏。
3. intelligence 每次执行创建轻量 guardian：模型加载前的启动屏障、subreaper、pidfd 计算组身份、唯一信号/wait 所有者。私有 SQLite/原子记录保存 launch intent、owner epoch/随机标记、READY/GO 与版本化退出凭证；先持久登记才允许计算。正常完成、取消、超时、异常与 API owner pipe EOF 共用收束路径；先停止 spawn，回收同组及被收养后代至 ECHILD，再 fsync/原子登记证明。不能回退到重启后不安全的数字 killpg。
4. state_dir 单实例锁先于恢复且不传给 guardian；继承 FD 有明确所有者。恢复先重建未确认 general/GPU 占用再接纳请求。guardian 死亡、启动窗口不确定、凭证丢失/损坏或内核无法结束时 degraded、保留占用并拒绝新计算，不以 finally、重建 semaphore 或找到锁归零。旧记录无证明保持 false。900 秒是触发收束的上限，不保证不可中断内核等待在该时刻消失。
5. Rust 在 POST 前、当前 lease CAS 内把每次 claim/attempt 的新 execution_id、完整不可变 request（含原 deadline）及协议修订追加到既有 file.extract payload。网络重发同 ID/原请求，真正重试新 ID；接管先收束所有前序尝试。GET404 不是退出证明：同 ID materialize/replay 后 cancel/观察，封住迟到 POST；首次 materialize 即使 deadline 已过也持久 failed 截止终态、不给 GO、退出证明 true，后续只能重放。冲突/超时/503/查无记录仍未确认。记录未有安全 GC 协议前不删除重建。
6. 稳定安全 ApiError.code 贯穿子进程与双端分类，不解析展示消息。损坏/不支持/非法编码/限额/超时为明确永久失败，临时 IO/依赖故障有限重试，未知错误保守处理；不联网下载或切换模型。正文 parser/options 身份与监督协议修订分离，不使备注/标签修改触发重算。

不新增第二套业务队列、公开执行状态、宿主 cgroup/权限或生产服务。PG 沿用 ADR0006 的成功表及 opaque jobs payload；私有监督记录不存正文/凭据。固定 SHA 解释器/依赖/源码运行，父/guardian/compute 均 `-I -B`，模型只读、当前 dev 显式 CPU。

## 备选方案

维持状态字段无法证明后代退出；持久数字 PID/PGID 有复用及信号竞态。宿主每执行 cgroup/常驻监督器改变权限和管理边界，本次不采用。选择明确证明字段与 intelligence 内部监督，范围限于已批准现有执行服务。

## 决策理由

将业务终态与资源退出证据分开，允许自然终态与取消竞态的正确判断，并封住重启、丢响应和迟到 POST 后重复计算窗口。失败时保留占用优先于无证据放行；不声称可以隔离任意恶意逃逸程序。

## 影响

- 正面：可验证资源释放与接管边界，后继任务复用同一执行协议和成功发布规则。
- 负面：guardian/私有账本与故障注入测试更复杂；少数不确定故障需要人工排查，不能保证所有崩溃自动恢复。
- 迁移与回退：增量保留旧账本/记录；先登记 OpenAPI/示例及双端类型测试。旧 Rust deny_unknown_fields 会拒绝新字段，两端必须同固定版本部署，新客户端对旧字段缺失安全拒绝放行。dev 停本实例写进程和 intelligence，核对 guardian 收束后一致备份 PG/assets/SQLite/私有凭证再切换；共享 PG/Caddy/Strata 与模型不动。回退使用理解新退出语义的兼容构建，不直接用旧逻辑重建占用。生产与破坏性恢复仍需另行确认。

## 验证与后续任务

T47 真实验证 leader 先退/孙进程存活、TERM 忽略/重复取消、各启动崩溃窗口、API SIGKILL/正常关闭、guardian 也死亡/凭证损坏不放行、不误杀 sentinel、迟到 POST/丢响应、lease 接管、终态与 cancel409、短 deadline、旧观察拒绝、离线中文 OCR 与既定准入边界，以及固定产物搬移/运行后摘要。T48 随后接入 Office 和双端测试；T49/T50 再完成真实应用验收。前置实验、ADR 或阶段 A 测试不替代这些验收。

## 状态历史

- 2026-11-18：RFC 经独立实验与方案审查，作为 PR #152 纳入讨论记录，尚未批准时仅允许独立准备。
- 2026-11-18：用户针对该补充 RFC 明确回复“通过，基于执行”，记录为 Accepted；不改写 ADR0006 历史正文。
