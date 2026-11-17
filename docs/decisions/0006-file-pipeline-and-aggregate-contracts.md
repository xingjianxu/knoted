# ADR 0006: 文件处理链与概览回收站契约

- Date: 2026-11-17
- Status: Accepted
- Decision owner: 用户 Xingjian Xu
- Confirmation: 2026-11-17 本会话，主 Agent 提供 RFC 路径与具体 DTO、双版本、根/扫描/来源/提取存储方案，用户明确回复“确认，按 RFC 实施”。确认原文通过 request_user_input_async 的 questionItemId `["request_user_input_async","call_ee4d628ff26e4cc5b1ec5c74daf92ff8",0]` 读回。确认范围即 RFC 推荐方案，不包含生产或破坏性操作。
- Related issues: T39 [#120](https://github.com/xingjianxu/knoted/issues/120)，后续 T41—T50 按卡关联。
- Related PRs: 初始方案 [#121](https://github.com/xingjianxu/knoted/pull/121)。
- Supersedes: 无
- Superseded by: 无

## 背景与问题

T39—T50 已获实施授权，但初始 DashboardSummary/TrashItem 未表达卡片所需聚合及恢复条件；文件源尚未落库，单一 content_version 无法同时表达元数据 CAS 与解析新鲜度。跨 scanner/capture/worker/API/Web 的共同契约需要明确决策。

讨论见 [RFC](../rfcs/2026-11-17-t39-t50-contracts.md)，获确认方案已随 PR #121 纳入主线，原始审阅版本为 `c0aa09ff38d5220d5238ef45574803c21bae9c7b`。下列决策正文固定保存关键约束，后续替代使用新 ADR。

## 决策内容

### T41/T42 浏览器 DTO

保留 DashboardSummary 已有 counts、tasks_by_status、recent_tasks，新增：

- `recent_notes: Note[]`：当前未删除笔记，按更新时间降序，最多 10 条。
- `due_tasks: Task[]`：当前未删除、未归档且未终态、有截止日期的到期/逾期事项，复用 T35 日期计算；按截止边界与 ID 排序，最多 10 条。
- `job_failures: integer`：当前 failed 任务数量，不包含正文或错误堆栈。
- `indexing_enabled: boolean`：当前阶段 false；不伪装成索引失败，不提前实现 T53。

TrashItem 保留现有字段，新增 `version: integer`、`can_restore: boolean`、`reason: string|null`。version 对笔记映射 content_version，对项目/事项映射 version；reason 给出父删除、项目只读、关系/终态限制等用户可理解原因。GET /trash 增加可选 `entity_type` 筛选，仍返回 Page，无总数。可恢复性是读取时提示，执行仍调用已有领域 restore 并重新验证 CAS/约束。只列软删除对象，不混归档。

### 文件版本与公开 DTO

FileSource 新增 `version: integer`，作为 PATCH/relink 的 expected_version 与 ETag。`content_version` 只标识源内容；备注/标签变化只递增 version，不改变源摘要，也不让已提取正文失效。源内容变化递增两者；可见性/定位变化递增 version，并在发布/读取时复核当前状态及观察值。解析状态不加入 FileStatus，仍只有 present/missing。

### 增量持久化

主 Agent 串行分配迁移编号（当前已到 0011）。新增而不改写旧迁移：

- `file_roots`：固定单根记录，UUID、配置路径、device/inode、最后成功完整扫描代次/时间、暂停原因；首次显式登记，根身份变化只暂停，不自动接纳替换根。
- `file_scans`：UUID、root_id、递增 generation、开始/结束时间、complete 标志、错误分类。同根只有一个有效扫描领取令牌；通知合并、补扫及重试不得并行发布。完成事务复核领取令牌、当前 generation 和根身份，再按本代观察集 reconciliation；旧代、中断代和失效令牌不能判定缺失。
- `file_sources`：已有计划字段 id/root_id/relative_path/device/inode/size/mtime_ns/sha256/content_version/status，加 ctime_ns/nlink/version/note/mime/created_at/updated_at/last_seen_at/last_seen_scan_id。仅 present 行以部分唯一索引占用同根当前路径；missing 旧来源保留原定位和稳定身份，因此同路径新文件可拥有新 source。registry 和已有 file_tags/entity_links 继续维护稳定身份。
- `file_extractions`：仅保存成功完整产物：UUID、source_id、content_version、input_sha256、parser_revision、options_hash、成功执行/任务标识、结构 blocks JSONB、created_at；同一 source/version/parser/options 唯一。每次失败、取消与重试尝试保留在既有 jobs/Python execution 账本，不覆盖成功产物；新尝试使用新 execution_id。提取身份独立于元数据版本，旧执行不能更新新尝试或已发布结果。

首次发现但尚未取得稳定摘要的观察记录属于 file_scans 的待采集集合，不伪造 FileSource.sha256。首次稳定采集成功后登记可对外返回的 file_sources；已登记来源发生变化时先让旧提取因观察版本变化而不可用，再经稳定采集更新摘要并排队提取。单次采集失败不等于来源失踪。

任务租约、取消及重试沿用 jobs；执行计算沿用既有 Python execution_id、SQLite 账本和 staging。发布事务同时复核 lease_token、源内容版本、present 状态和采集观察，再写提取记录/outbox。不在 T43—T50 提前建立全文/向量索引。

### 配置与实例边界

新增显式 `files` 配置：enabled、root、staging_root、debounce_seconds=2、rescan_seconds=600；单根身份通过首次登记保存。intelligence 连接沿用 127.0.0.1:15603 的内部服务，配置只引用凭据路径，不保存凭据正文。

dev 根使用既有部署文档指定的 `/data/vaultsyncd-dev/dev-server/vaults/knoted-test`；Agent 只在其 `agent-fixtures/tNN/` 写样本。应用对源目录只读，Python 仅能看到 staging 和模型。源根失联/身份变化/扫描失败时保留现有来源，显示暂停原因，不删除或全量标失踪。未配置时能力关闭，不扫描其他目录。

## 影响与迁移

新增 DTO 字段由 OpenAPI 先登记再生成客户端；同步更新示例与检查。尚未实现的聚合/文件端点没有现有线上消费者，但现有客户端编译仍需通过。存储增量创建，不删除旧表/数据，不改现有领域锁与恢复语义。

dev 迁移前停本实例写进程并备份 PG/assets，校验备份后迁移、构建固定 SHA、pcwrap 更新；不重启共享 PostgreSQL/Caddy/Strata。旧二进制若拒绝新 schema，回退需兼容构建或隔离恢复备份，不直接切旧 current 冒充数据回滚。

## 备选方案与决策理由

保留现状不能满足 T41/T42 验收；前端拼装聚合缺少一致读取，文件使用单版本会让备注/标签变更误伤已提取正文。选择保持现有领域服务、HTTP 路径和 Python 协议，增量补字段与表，避免引入第二套身份、执行或恢复规则。

## 影响

- 正面：元数据 CAS 与内容新鲜度明确分离；扫描代次、移动身份与成功产物发布拥有一致边界；概览和回收站可表达卡片要求。
- 负面：新增版本/观察状态、增量迁移和并发回归测试；客户端必须与 OpenAPI 同步生成。
- 迁移与回退：如上，开发实例先停写备份；不破坏旧数据，不将切换旧二进制冒充数据库回滚。未来不可逆迁移仍须单独确认。

## 验证与后续任务

按 T41—T50 逐卡 TDD、双端协议验证及真实 dev 验收；额外固定同路径替换保留历史引用、扫描代次拒绝、重试成功与旧执行迟到、未稳定摘要不伪造等场景。本文确认方案，不声称代码已实现或测试通过。

## 状态历史

- 2026-11-17：主 Agent 起草 RFC，两个研究 subagent核查现状，独立审查提出三项存储并发风险，修正并复审通过。
- 2026-11-17：用户明确回复“确认，按 RFC 实施”，记录为 Accepted；不替代 ADR0001—0005。
