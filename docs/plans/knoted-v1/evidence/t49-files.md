> 阶段 A/B 历史记录保留；整卡主线/dev交付与清理见文末最终记录。

# T49 文件浏览、备注标签与下载：阶段 A 与 B

阶段 A 历史状态：**NEEDS_CONTEXT**。以下 A 记录保留当时的验证范围；上游等待已由末尾阶段 B 记录取代。T49 自身的主线集成及真实 dev 全格式验收仍由主 Agent 完成，不能据此标记整卡 DONE。

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


## 阶段 B：默认认证连接及能力文案收口

- 恢复基线：`3c2f34dd59c482f17840c21a088eb2c113d84067`（父 Agent 普通 merge 已验证的 T48 主线 `c936e0a9b8b4cfc9c8cdee9749fb038f3648cbf3`，保留 A）。T47/ADR0007 与 T48 整卡完成为父 Agent 提供并验证的输入，不再等待旧 RFC；本实施者未访问 dev 或真实 SourceRoot。
- 当前 whole A+B 审查基线暂为上述 `c936e0a`，父 Agent 在独立门禁结束及纯状态文档同步后记录最终基线。
- 仅修改 Dashboard 实际能力文案及对应 t41 断言，新增 t49 默认连接回归；未改变 A 的 Router/QueryClient/ApiClient、文件业务或 Rust 字节。概览列明文本、PDF、PNG、JPEG、WebP、DOCX、XLSX、PPTX 已支持，索引仍未启用，搜索/RAG 未开放。
- 新增 5 个真实 loopback HTTP 场景：详情页后台列表 401、详情 401、下载 401、列表页 401、主动退出登录。测试不注入 bundle，不 mock fetch/认证回调/QueryClient 模块；仅给 Node 的相对 Request URL 提供 loopback origin 适配。使用产品默认 Router→QueryClient→ApiClient，验证安全 return-to、缓存及脏草稿卸载、待响应下载连接被中止、迟到字节不生成 Blob URL、重新进入不复用旧草稿。服务、连接和查询均在 finally 关闭。此为真实 HTTP 连接回归，不是 dev Passkey 或浏览器下载运行验收。
- 文案 RED：`b-dashboard-red.log/.exit=1`，1 failed/6 passed；改文案后与 T49 共 26 passed（T49 19、T41 7）。认证行为原已正确，新回归首次即绿，不伪称新增产品修复的 RED。

以下日志均在原 logs 目录，各 `.exit=0`：

| 命令 | 日志 | 结果 |
|---|---|---|
| `cargo test --workspace t49_` | `b-rust-targeted` | 12 passed，计数脚本结果为同名 `.count.json` |
| `cargo fmt --check` | `b-fmt` | 通过 |
| `cargo clippy --all-targets -- -D warnings` | `b-clippy` | 通过 |
| `pnpm --filter @knoted/web test tests/tasks/t49.test.tsx tests/tasks/t41.test.tsx` | `b-web-targeted` | 26 passed，2 files |
| `pnpm typecheck` | `b-typecheck` | 通过 |
| `pnpm lint` | `b-lint` | 通过 |
| `pnpm api:check` | `b-api-check` | 83 operations，29 examples，14 invalid rejected |

B 定向测试保留 15 次 jsdom scrollTo 提示（新增默认路由往返增加调用，#143），不是测试失败。没有 Python 改动或重复模型测试；只为 Rust 测试配置父授权的解释器路径，未安装或修改其环境。全量 Rust/Web/API verify/build 由父 Agent 对冻结分支独立执行，不重复将历史 A 全量结果当作 B 组合门禁。

代码与测试已冻结：`57d31c4ffd6fc85b287619adf6383ac352103f05f78dc8be2ad136fd68b1b881`，19 个 whole A+B 源码/测试文件，逐文件摘要位于 `/data/knoted-dev/tasks/t49/phase-b-frozen-source-manifest.json`。其后仅写证据并安全提交。真实 HTTPS/API/浏览器/全格式业务验收、独立审查、PR/主线/部署及清理由父 Agent 继续；本分支保留待集成。

## 独立审查修复 R1：仅提交变化字段

独立审查 I1 发现：页面把纯备注修改也作为备注+标签双字段替换，标签同版本软删后会携带未编辑的失效 ID 而 422；纯标签修改又会把合法的原备注空串改写为 null。修复基线为 `1f1501e629dee2176ca2f906fab385689d4d2e42`。

`FileDetail` 现在分别比较备注和标签与当前草稿基线，只将有变化的字段加入同一次 PATCH，继续使用一个 expected_version。用户明确接受当前服务端版本后重新比较保留草稿，实际双字段变化仍原子提交。不会静默删除尚未加载名称的标签 ID，也不改变 missing、401、请求 owner、下载 ETag 或缓存身份边界。

新增 5 个真实组件行为用例覆盖：dirty 备注期间同版本软删标签仍可保存且保留草稿；原备注空串的纯标签修改不发 note；两字段实际变化单次 CAS；显式 rebase 后两字段变化或备注已相同而只改标签。原 dirty CAS 用例改为断言未改标签不提交，并继续断言完整草稿保留原标签 ID。HTTP fixture 按既有可选字段语义更新行，并模拟软删标签校验；没有修改 Rust/API。

- `r1-red.log/.exit=1`：首轮 6 failed/18 passed，其中 2 项是测试误把 MultiSelect 输入点击当成展开操作，属于测试操作错误，不计产品 RED；随后改用公开标签移除按钮。
- `r1-product-red.log/.exit=1`：修正测试操作后 5 failed/19 passed；明确出现标签失效 422、空串变 null 及多发未改字段。
- `r1-green.log/.exit=0`：产品修复后 24 passed；`r1-typecheck`、`r1-lint` 的 `.exit` 均 0。原有 scrollTo 提示保持记录。
- 冻结 sourcehash：`b3e08dc4c3014ce65a4ed1838ecac38a122ee48c628f8074c423add3f819e5dd`，逐文件摘要为 `/data/knoted-dev/tasks/t49/r1-frozen-source-manifest.json`。主 Agent 对冻结分支独立跑完整 Web 并定向复审 I1；本轮 Rust/API/Python 未改，沿用其修前完整组合证据，不重复模型或全量 Rust。整卡集成及实际运行验收仍待主 Agent 完成。

## 最终主线与 dev 交付（2026-11-18）

PR [#161](https://github.com/xingjianxu/knoted/pull/161) 普通merge至
`8ffb2404d0fe44446ebe4020ad65499f5946b2a8`，dev同SHA、schema15；Issue #134自动关闭。
A876a9eb，B1f1501e，R1de4eef5仅修可选PATCH字段，最后普通merge已验证上游文档为dc935d6。
独立SpecPASS / QualityAPPROVED，唯一I1 ADDRESSED，无新增Critical/Important。
仅变化字段进入一次PATCH/CAS，纯备注不携带软删旧标签、纯标签不改写原空字符串备注，
显式rebase后重新比较，两字段真改仍原子提交。原compressed JSX/#143/Vite提示保留Minor。

| 实际门禁 | 结果 | 原始记录（任务49日志） |
|---|---|---|
| 父完整 `cargo test --workspace --locked` | 407顶层 +8嵌套通过，1既有ignored | `parent-b-final-cargo-test.log` |
| fmt / clippy / API check + verify | 全0，83operations/29examples/14invalid rejected | `parent-b-final-*` |
| R1父typecheck/lint/全Web/build | 全0，272 tests /18files | `parent-r1-*` |
| 修复覆盖 | 有效RED5失败/19通过→GREEN24通过 | `r1-product-red`、`r1-green` |
| 主线Rust/Web定向、fmt/clippy、实际主线Python导入 | 12 Rust、31 Web均通过，其余0 | `main-*` / `evidence/main-verification.json` |

R1仅Web，Rust/API/Python与父完整快照字节不变，未重复模型全套。
19路径最终源码hash：`b3e08dc4c3014ce65a4ed1838ecac38a122ee48c628f8074c423add3f819e5dd`，
父逐文件核验/主线树一致。父第一次命令漏传模型目录，T47pipeline报artifacts_unset/exit101，
原`parent-b-*`记录保留；补齐已批准只读模型根/CPU/emptyGPU后完整`parent-b-final`实际全0。
R1初轮两个MultiSelect测试操作错误也保留，未算作产品RED，未修改产品掩盖环境错误。

固定产物38320文件，清单SHA256
`4acdfaecd47820b6702642371aa548a823bd959cfa5a8aedda5186100f4747f2`。
consistent PG/assets/staging/native SQLite备份位于
`/data/knoted-dev/service/backups/20261118T031102Z-t49-/`。
三进程固定exe、HTTPS/internal健康、CPU/只读模型、loopback和共享服务身份不变；
不重做字节未变的T47打包器搬移/ELF审计，不作跨宿主运行闭包承诺。

真实验证使用独占UUID样本与内存会话，不拦截API、不改原件、不输出凭据：

- `live-api` exit0，12 checks全部true：匿名list/detail/content401、真实游标、备注标签联合CAS/no-op/invalid-tag回滚/旧版本409、源字段和路径注入拒绝、CSRF403、当前字节及安全headers、原件SHA不变、generic tags推进旧草稿栅栏、missing只读/下载404和明确relink目标元数据。自有标签/备注/会话/两源清理成功。
- `live-browser-r1` exit0，7 checks全部true：列表打开真实详情、草稿不自动提交、显式保存只改元数据、中文文件名原件88字节实际保存且SHA正确、missing编辑/下载禁用、确认relink并导航目标、撤销真实会话后content401→安全login redirect/草稿与详情卸载。全部cleanup flags为true。
- 第一轮`live-browser`在启动Chrome时失败，先创建的两自有源已missing；最小无会话/无DB探针复现长UUID TMP下ProcessSingleton Socket path too long，短任务tmp通过。仅修私有验收工具的Chromium TMPDIR，普通下载仍独占UUID目录；确认无活Chrome后清理两份已知私有缓存，所有自有会话已撤销。失败原log/exit1与`browser-launch-failure-cleanup.json`保留，非产品回归。
- `live-full-format` exit0：UTF8.txt、PDF、PNG/JPEG/WebP、DOCX/XLSX/PPTX八个真实fresh来源全部出现在list/detail；逐份实际鉴权下载字节/大小/SHA/metadata ETag/attachment/octet-stream/nosniff/private-no-store一致，逐格式匿名401。各有一次成功提取/一次成功工作；清理后八源missing/fresh=false、全局file jobs0、intelligence active0、无取消。JSON位于`evidence/full-format-6ff63a99-a06f-4209-be7a-a69374a4407e/`。

最后`final-runtime-audit` exit0：24个Python持久执行均GET echo及退出证明有效，
16成功/5既有输入失败/1取消/2中断（含T47/T48历史）；健康active0、staging无.part，
完整38320文件运行后仍精确，共享服务不变。全局0present/35missing、成功提取27、
既有预期failed jobs7、文件任务active0、Passkey1。
正常清理本任务WT/已合并本地与远程分支，27636个忽略文件均node_modules/dist等
可重建产物，未知0，其他任务WT保留；见`evidence/worktree-cleanup.json`。

以上原始JSON/log/.exit保存在`/data/knoted-dev/tasks/t49/`。下载完整缓冲最多100MiB，
扫描不稳定可保守503，不能承诺外部文件系统永不在最后校验后再改变；不提供Range/304。
T50预览限额是独立能力，未给普通原件下载加入50MiB限制。检索/RAG仍未开放。
