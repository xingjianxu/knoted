# T39–T50 最终审查修复证据

基线 `2c89043a0d10a53568aa206e31f2c651a3294706`。定向验证使用本任务独立 TestDb schema、临时源目录、暂存目录和 loopback intelligence 服务。工作区、日志和缓存信息见下。

## 修复与边界

- **I1 稳定文件补扫**。跨代仍完整读取新原件并校验来源、根、观察、租约及暂存关闭状态。持锁的同一来源具备一致强身份，且新采集的 SHA-256 和大小匹配时，复读旧闭合对象以校验完整内容。提交前复核旧物元数据，继续引用原 ObjectId，推进采集观察，并调用既有提取入队以判断缓存。旧物缺失或已损坏时采用本次新对象。事务失败清理本次新对象并保留旧引用。曾发布的历史对象仍由原引用拥有，同扫描收据仍按原条件重放。
- **I2 重关联根准入**。持根锁对照当前 `files.enabled` 和配置路径，然后打开根并校验实际身份。关闭或路径变更返回既有 409 冲突。关闭仍配置 A、改用 B 时 A 保持完好的两种夹具，均核对双方完整来源行、备注、版本、标签和关系。正常夹具显式启用 A。
- **I3 DOCX 表格正文**。表和行仅消费已支持正文成员，并允许格式属性及空范围标记。合法的 `w:customXml` 包行或包单元格返回 `unsupported_format`。DOCX 的持久解析身份与工件身份为 `office-ooxml-body-v3:docx`，旧成功保留历史并由新补扫按新身份重新排队。XLSX、PPTX 和 Office 预算选项沿用原身份。Worker 将旧 DOCX 作业转入既有取消流程，退出未确认时保留租约；同一已登记 execution ID 仍可补做提交栅栏。新执行和结果发布使用新身份。 [表子节点规范](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.table?view=openxml-3.0.1) [行子节点规范](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.tablerow?view=openxml-3.0.1)

## 定向回归

定向执行的原始命令、stdout/stderr 与退出码保存在 `/data/knoted-dev/tasks/t39-t50-final/logs/`，文件通常以 `.cmd`、`.out`、`.exit` 区分。旧 DOCX 作业的接管回归在解析身份切换后、接管逻辑加入前取得 RED。

| 场景 | RED | GREEN |
| --- | --- | --- |
| I1 两轮后续完整补扫、对象身份和数量稳定、提取观察推进 | `rust-i1-i2-red` 退出 101，1 失败，前后 ObjectId 不同 | `rust-i1-i2-final` 退出 0，T44 32 通过和 1 个嵌套通过，覆盖注入入队事务失败、损坏/缺失旧物、handle 冲突 |
| I2 停用或换根，原根完好 | `rust-i2-red` 退出 101，2 失败，两种情况实际返回成功 | `rust-i1-i2-final` 退出 0，T45 23 通过。API 正常夹具启用已登记根后，`rust-i2-api-fixture-green` 退出 0，1 个真实 HTTP 路由用例通过 |
| I3 DOCX 表格完整性和格式身份 | `python-i3-red` 退出 1，两种正文包装未抛错，DOCX 工件返回旧身份。3 失败，3 正常对照通过 | `python-i3-green` 退出 0，T48 79 通过；最终布局调整后 `python-i3-regression-final` 退出 0，7 个定向用例通过 |
| I3 工件校验和旧缓存 | `rust-i3-artifact-red`、`rust-i3-cache-red` 各退出 101，各 1 失败。Rust 接收旧 DOCX 工件，旧成功抑制新任务 | `rust-i3-policy-final` 定向复验 2 通过。组合输出含 T48 工件 6 通过，持久发布 4 通过，真实 HTTP 3 通过及 7 个嵌套通过 |
| I3 旧 DOCX 作业接管 | `rust-i3-retired-red-retest` 退出 101，1 失败。未经退出核验走了 Rust 文本任务终态 | `rust-i3-retired-green` 退出 0，1 通过，退出未确认时状态为 `cancel_requested` 并保留租约。Worker 组合输出 2 通过，另验证已登记的真实执行退出确认后收束，账本仍只有原 execution ID |

`rust-i3-retired-red` 首次尝试因 Worker 测试缺少直接 `serde_json` 依赖而编译失败，使用现有 JSON 值接口修正夹具后另存有效 RED。主 Agent 首次 Rust 总门禁发现 API 的成功夹具仍使用默认停用配置，退出 101。修正仅涉及该测试，定向复验保留原鉴权、CSRF、ETag 与版本冲突断言。I1/I2 第一次 GREEN 验证出现嵌套校验缓冲导致的调试栈溢出，将 64 KiB 校验缓冲移到堆上后重跑通过。初次 Ruff 和 Clippy 分别指出一行长度和两处测试切片构造，修正后的检查退出 0。Rust Office 组合的所有测试二进制均输出通过，日志包装脚本在测试进行期间因缓存路径更新而读到不一致的脚本，最终退出 2，未记录 Cargo 退出码。该次组合原始输出存为 `rust-i3-green.out`，独立的定向回归另有原始退出码。

## 环境和后续门禁

标准 `uv --project services/intelligence sync --locked` 退出 0，独立环境为 `/data/knoted-dev/tasks/t39-t50-final/build/python-env`。`python -I -B` 证实 `knoted_intelligence.app` 来自修复工作区。`cargo fmt --check`、定向 Clippy、Python Ruff 和 diff 检查均退出 0。冻结清单 `/data/knoted-dev/tasks/t39-t50-final/evidence/code-test-frozen.sha256` 保存每个产品及测试文件的 SHA-256。

本记录覆盖修复工作区的定向回归。整套门禁、独立复审、PR 和更新后开发实例验收由主 Agent 记录。

## 父完整门禁、唯一复审及最终交付

PR [#177](https://github.com/xingjianxu/knoted/pull/177) 普通merge，主线/dev **`e9da413e766ba98e34084ec219da4991f8d7a09d`**。Issue173自动CLOSED；产品Worktree/已合并本地分支已正常移除，远程分支读回不存在，4个忽略项仅pytest cache、未知0；其他任务不动。最终阶段汇总见 [交付记录](t39-t50-integration.md)。

- 父完整Rust：`cargo fmt --check`、`cargo test --workspace --locked`、`cargo clippy --locked --all-targets -- -D warnings` 全exit0，415顶层+8嵌套通过，1既有ignored。原API夹具失败101保留，改为显式启用根后重跑完整Rust；不是跳过失败用例。
- 父完整Python `uv run --locked --group dev pytest -q` 为171通过/9既有warning、370.68s，Ruff check0；原format1保留。最后`2e5968b`仅删测试一空行，前后AST等同、其余12冻结SHA一致，最终Ruff format28文件/check/7focused均0，无需再跑同产品模型。
- 最强模型的唯一scoped复审I1/I2/I3均ADDRESSED、Spec PASS/Quality APPROVED、new Critical/Important0；格式项追加最终读回后解除。报告`/data/knoted-dev/tasks/t39-t50/reviews/final-t39-t50-rereview.md`。父按git-commit正常纳入已审7MD，最后feature `78ba568`，主线tree精确且13产品/测试SHA逐项一致。
- 主线定向Rust9/Python3、fmt和官方fixed build全0；一次错过滤得到的main-relink 0tests不计覆盖，另跑真实两个disabled/换根负例。标准uv环境重新同步至rootmain，实际`app.__file__`已核对。Web/API/锁未改；新旧61个Web产物SHA完全相同，复用T50实际浏览器及PDF看图证据，不声称新SHA重跑浏览器。
- 固定38320文件manifest SHA256 **`95e309224c9dcc035c4697fc8b20445ab13d90723d0e66af0ebc8a305305e095`**。dev三进程同一完整SHA固定可执行文件、loopback、CPU/只读模型/health正确；一致备份`/data/knoted-dev/service/backups/20261118T045610Z-t39-t50-final-`含原生SQLite/PG/assets/staging及CLI清洁证明。未修改共享服务，本次部署前后PG/Caddy/Strata进程均保持。

真实四源完整复验`live-acceptance-r1` exit0：初代129→后续130→131→清理132。普通DOCX按v3完整发布Before/Table body/After，文本成功，两种customXml包装明确unsupported_format且无提取产物。两轮后续完整扫描保持四源id/content_version/version/observation_version/input ObjectId及成功、失败作业记录相同；整个staging/input闭合对象集合均**114件**，无多余保留、无.part，失败语义不重新启动。清理仅删除自有UUID源目录，四源变missing/fresh=false，历史输入仍在。

首次四源运行已通过正文/失败类型，但私有验收工具复用Btrfs目录FD时首次枚举未见新增4输入，严格集合断言失败。独立同FS探针原106+新增4仍枚举106，独立fresh目录流立即110，修后RED1→GREEN0；仅helper每次从pinned FD打开`.`独立枚举，严格断言不动，补差集日志后全四源重跑通过。更早的helper set与dict直接比较在live前被父离线fakeartifact探针查出并修正。原失败/成功finally/私有修正探针全部保留，不记为产品RED或跳过。

最后`final-runtime-audit` exit0：完整38320清单运行后精确，49个持久Python执行全部GET echo与退出证明有效（34成功/12预期输入失败/1取消/2中断）；health/filejobs active0、.part0，schema15、0present/75missing/成功提取57/预期failed jobs17/Passkey1。当前三shared PID与本次部署before相同；T50期间Strata曾更换PID的历史事实仍按原证据保留，原因未知，不扩大为整个会话都未变化的断言。原始证据根`/data/knoted-dev/tasks/t39-t50-final/`，核心JSON为`evidence/parent-final-verification.json`、`main-verification.json`、`live-acceptance.json`、`final-runtime-audit.json`、`worktree-cleanup.json`。
