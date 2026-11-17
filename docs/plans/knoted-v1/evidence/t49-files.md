# T49 文件浏览、备注标签与下载：阶段 A

状态：**NEEDS_CONTEXT**。本证据只涵盖隔离分支的阶段 A 实施及本地验证；T47/T48 整条执行链、独立审查、主线集成、真实 dev 全格式验收均未完成，不能据此标记 T49 DONE。

- Issue：[#134](https://github.com/xingjianxu/knoted/issues/134)，由主 Agent 维护。
- 基线：`ef04fa6ec4892135fb36955bdb1cf616bdce879a`；任务分支 `feat/t49`。
- 依据：[T49](../tasks/T49.md)、[领域补充](../tasks/T49-core.md)、[ADR 0002](../../../decisions/0002-mtone-ui-ownership.md)、[ADR 0006](../../../decisions/0006-file-pipeline-and-aggregate-contracts.md)。
- 详细报告：`/data/knoted-dev/tasks/t39-t50/reports/t49-implementation.md`。
- 所有日志和对应退出码：`/data/knoted-dev/tasks/t49/logs/<name>.log` / `<name>.exit`。

## 已实施的既定边界

Core `files::browse` 提供 UUID 升序、作用域游标的列表和包含 missing 的详情；联合备注/标签 PATCH 使用关系准入→来源→UUID 排序标签锁，元数据 CAS、备注三态、不 trim、不限制长度但拒绝 NUL；去重后的活跃标签集按事务替换，保留软删历史成员。无效标签整体回滚，完全 no-op 不增版本；实际修改仅增一次 metadata version，不改源摘要、内容版本或成功提取。

Core `files::download` 从当前登记根逐段 FD/NOFOLLOW 打开，只读普通文件且叶子 NONBLOCK。验证捕获观察、大小和 SHA，64KiB 分块且完整 payload 至多 100MiB，读后重查 FD/当前路径；最终 root→source 短事务复核根、扫描、来源及完整观察关系。元数据-only 并发返回最终版本 ETag；present 但依赖不就绪或变化为 503，确认 missing/未登记为 404。响应是经过校验的完整字节，并非流式下载或文件系统快照。

API 保持既有 OpenAPI，无新字段/迁移/依赖。`GET /files`、`GET/PATCH /files/{id}`、`GET /files/{id}/content` 均鉴权，PATCH 有 CSRF；content 为 octet-stream、metadata ETag、安全 attachment/UTF-8 filename*、精确长度、private/no-store 和 nosniff。手动 relink 继续调用既有 T45 操作。

Web 提供 `/files` 与详情深链、导航和 files 401 安全返回；复用游标 reducer/queryKeys。备注与标签形成同一基础版本草稿，保存成功先确认，再独立处理列表刷新；409 保留草稿，明确刷新/重试。每次详情挂载拥有 AbortController，A→B→A 的旧请求不能改变新轮次 UI。下载比较 metadata ETag、通过已鉴权 ApiClient 取 Blob，使用后撤销 URL。missing 普通编辑/下载只读，仍显示历史元数据和标签 ID；手动选择并确认 present 目标后 relink，目标缓存按返回 ID 保存。未开放 generic links 编辑、检索、RAG、Office 在线预览；Dashboard 阶段文案及 indexing_enabled 未改。

## UI 归属

亲自检查锁定的 npm `@bixlab/mtone@0.4.0` package exports、`dist/index.d.ts`，并完成真实组件消费测试。使用公开 List/ListItem、FormField、Input、Textarea、MultiSelect、Button、Alert、Empty、Spinner、ConfirmDialog；只增加文件业务状态和布局。无需新增通用能力，无上游 Issue、源码改动或依赖替换。

## 验证矩阵

- `t49_1`：匿名 list/detail/content 拒绝；authenticated GET 无 CSRF 可读，PATCH 缺 CSRF 拒绝。
- `t49_2`：真实临时根/FD；腐坏持久化路径的穿越/绝对/点段、父/叶 symlink、FIFO、目录、物理丢失、根身份失配、同路径替换、100MiB+1 sparse 文件拒绝。private cfg(test) 首块同步点覆盖读中覆写/截断/追加/rename/symlink；PG 最终根锁屏障覆盖 metadata-only 成功、新 scan_token、观察失效及最终 FS 变化拒绝。
- `t49_3`：真实 PG 验证备注原文/null/省略、联合标签 CAS/no-op/重复/坏标签回滚/软删历史、generic 标签写后旧 CAS、并发 CAS 一胜一冲突；源内容及成功 extraction 行不变。分页包含 missing、稳定排序与末页 null。
- Web：dirty CAS、未知标签 ID、保存后刷新失败、A→B→A 迟到成功、迟到详情、迟到下载、Blob URL 撤销、ETag 不符、401 清除本地草稿、missing、游标加载禁用、目标 relink 缓存/导航、鉴权深链、大小写同 UUID 拒绝、同版本软删标签投影更新。
- 测试临时根由 fixture 在 `/data/knoted-dev/tasks/t49/tmp` 创建并 RAII 持有；确定阶段测试先释放事务并 await 工作，再显式 `db.drop().await`，最后断言。没有修改公共 TestDb helper。

所有下列日志均位于上述 logs 目录，配套 `.exit` 均为 `0`：

| 命令 | 日志名 | 实际结果 |
|---|---|---|
| `cargo fmt --check` | `fmt` | 通过 |
| `cargo test` | `rust-all-final` | 顶层 381 passed / 0 failed / 1 ignored；额外嵌套 child 1 passed |
| `cargo test --workspace t49_` | `rust-targeted-final` | 12 passed / 0 failed |
| `cargo clippy --all-targets -- -D warnings` | `clippy` | 通过 |
| `pnpm typecheck` | `typecheck-final3` | Web 与 api-client 通过 |
| `pnpm lint` | `lint-final3` | 通过 |
| `pnpm test` | `web-all-final3` | Web 18 文件、262 passed |
| `pnpm --filter @knoted/web test tests/tasks/t49.test.tsx` | `web-targeted-final2` | 14 passed |
| `pnpm build` | `build-final3` | 通过；保留 bundle-size warning |
| `pnpm api:check` | `api-check` | 83 operations、24 examples，12 invalid rejected |
| `pnpm api:verify` | `api-verify` | 通过，生成客户端无 diff |

Rust 计数使用主 Agent 的 `count-rust.py` 区分顶层和嵌套子测试，结果保存在同名 `.count.json`。Web 全量有 4 次 jsdom scrollTo 提示（既有 3 次及本卡深链新增 1 次），未宣称无噪声。本记录不把网络 mock 或临时 FD 测试当成真实 dev 管线验收。

## 阶段 B 待办

待 T47/T48 真实上游集成及明确批准的退出协议落地，再由主 Agent 安排独立审查、全格式源扫描→采集→解析→浏览/下载的运行验收和 T49 最终主线/dev/文案/状态回填。当前不部署、不触碰真实批准源根、不写远程、不清理工作区。
