# T49 补充子任务：文件领域读取与元数据事务

- 状态：planned；属于 [T49](T49.md)，不单独宣称整卡完成。
- 依据：T49 既定 list/detail/PATCH/content 行为、[ADR 0006](../../../decisions/0006-file-pipeline-and-aggregate-contracts.md)。本子任务不新增公共字段、迁移、模块边界或依赖；将文件领域事务与 FD 读取放到已有 core files 模块，API 只作 HTTP 适配。
- 原因：API 生产依赖没有 SQLx/rustix/sha2；不能在路由复制数据库与文件安全实现，也不应为此扩大 API 依赖。按 T49 的额外业务模块规则拆出明确子任务，由同一个 T49 实施 Agent 在其任务分支串行完成。
- 隔离：主根 `.worktrees/knoted-t49` / `feat/t49`；主 Agent 负责契约、集成及 dev。等待 T48 时仅作独立准备，集成/运行验收仍遵循上游依赖。

## 文件范围与接口

允许新增 `crates/knoted-core/src/files/browse.rs`（如有必要将安全读取放在同目录独立业务文件）、在 `files/mod.rs` 注册、复用 `identity::FileSource`、增加 `crates/knoted-core/tests/t49.rs`。只允许为复用投影作必要的最小 visibility 调整，不重构其他实体/通用游标/身份算法。使用具体命名输入输出及 `AppContext`，不泄露数据库行或源根绝对路径到公开 DTO。

## 行为裁决

以下是主 Agent 对既定操作的实现选择，不冒充新增用户需求：

1. list 包含 present/missing，稳定 UUID 升序；沿用默认 50、范围 1..100、作用域绑定的 opaque cursor，返回 limit+1 判定 next_cursor。detail 可读取 missing 的历史元数据。根失联不改变已发布元数据。
2. PATCH 只改备注和标签，正 expected_version CAS；缺省/显式 null 备注三态、原文不 trim、不新增长度限制，拒绝 PostgreSQL 无法存储的 NUL。tag_ids 缺省不变、空数组清空活跃集合、重复 ID 视为集合；新增成员必须为活跃标签，否则 422 并全部回滚。保留不在公开活跃集合内的软删标签历史成员。实际变更只递增一次 version；完全 no-op 不递增；不改 content_version、摘要、观察或成功提取。
3. missing 详情只读，PATCH 返回 409/current_version；不存在返回 404。沿用关系准入→来源→按 UUID 标签锁，若需要根锁则必须先根后来源；不引入 source→root。relink 仍复用 T45 的独立事务与目标 DTO，不改变其语义。
4. content 从登记的当前批准根 FD 安全相对读取，逐段 NOFOLLOW，叶子 NONBLOCK 且必须普通文件。核验根身份、路径、捕获观察；64KiB 分块、100MiB 硬上限、SHA、同 FD 前后元数据、当前路径仍指向该 FD；最终 root→source 短事务再检查 present/fresh、content_version/SHA/完整 observation 与捕获关系、根/扫描状态，随后返回完整已验证字节。不能伪造 capture Lease，不用历史 staging 替代当前根读取。
5. content 的元数据-only 并发可返回最终 version ETag；内容/路径/观察变化拒绝。确认 missing/未登记 ID 为 404；数据库仍 present 但根离线、扫描未收敛、观察失效或读取变化等为现契约 503，不偷偷新增 409/413/Range/304。此方案最多缓冲 100MiB/请求，非性能承诺，也不声称阻止外部文件系统在最终校验后继续写入。
6. 200 保持 application/octet-stream，metadata version ETag，安全 attachment basename/UTF-8 filename*、精确 Content-Length、private/no-store、nosniff。禁止将原路径、MIME 或控制字符直接注入响应头。

## 测试与交付

- [ ] 真实 PG 验证 CAS、备注三态、标签联合更新/no-op/回滚、generic tag 写后旧 CAS、missing、分页及游标作用域。
- [ ] 真实临时根/FD 验证路径穿越、symlink/FIFO/非普通文件、根身份、同路径替换、观察失效、读中变化、metadata-only 与最终 ETag；确定阶段的 PG/FD 屏障优先于 sleep。
- [ ] 自建并 RAII 持有本任务 tmp，释放事务/任务并显式 `db.drop().await` 后断言，避免已有 #138/#153；不修改通用测试 helper。
- [ ] T49 API 和 Web 测试消费真实操作，运行 Rust fmt/test/clippy 及相应 Web/API 门禁，留存真实输出。库测试不冒充整卡 dev。
- [ ] 同一实施者安全提交；主 Agent 独立审查、整卡上游依赖检查、PR/主线/dev 验收后才完成 T49。
