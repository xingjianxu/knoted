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
