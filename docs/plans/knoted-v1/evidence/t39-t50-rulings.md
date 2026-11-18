# T39—T50 过程裁决与代价

按会话账本的记录顺序归档，供整阶段最终审查核对。这里保留了等待期准备选择与之后主线/dev 验收的历史；不构成新的 Accepted ADR，不替代 [T47 补充退出契约](../../../rfcs/2026-11-18-t47-execution-exit-confirmation.md) 的独立人工确认。完整任务状态以 [索引](../README.md) 为准。

| 顺序 | 裁决与依据 | 代价及适用边界 | 证据 |
|---|---|---|---|
| 1 | 现有详细任务卡与用户直接实施委派已授权按指定 subagent 方法工作，不再要求一次通用计划批准；新架构仍单独确认。 | 若范围判断错误，需要返工未集成成果；不能借原授权批准新的跨模块/存储契约。 | [实施计划](../../2026-11-17-t39-t50.md) |
| 2 | 原生 Worktree 工具不能指定目录，使用 Git fallback 满足主根 `.worktrees/<task>` 硬约束。 | 需要手工核验与清理任务 Worktree，不能回退到主仓开发或删除其他任务。 | [Worktree 规则](../../../development/git-worktree.md) |
| 3 | 内核 opaque handle 是 ADR0006 下的可选内部观察证据；同根、当前完整扫描、完整 handle、摘要、nlink 和唯一性共同支持普通跨扫描改名。 | 不支持或证据不足时保守建立新来源/手动关联，历史失踪来源不能靠启发式复活；不承诺克隆/回滚的绝对身份保证。 | [ADR0006](../../../decisions/0006-file-pipeline-and-aggregate-contracts.md)、[T45](t45-identity.md) |
| 4 | 成功表 `file_extractions` 随真实 Rust 文本发布在 T46 建立，不等 T47；T47 再接真实 Python 尝试关联。 | 后续卡必须复用既有成功表与唯一身份，Rust 文本 execution_id 为 NULL，不能伪造 Python ID。 | [T46](t46-text.md) |
| 5 | 同扫描元数据不能证明文件 incarnation，不能覆盖 handle 冲突；同观察幂等仅能重放已验证的闭合采集 receipt。 | 必须满足同扫描/路径/捕获元数据/持久身份、当前根/租约/观察/新鲜度及闭合对象 SHA/大小；不新增身份写入或版本递增。其他情况仍需强证明或新来源，可能降低复用率。 | [T44](t44-capture.md) |
| 6 | T46 使用明确的文本/代码/配置扩展与有限无扩展名白名单；其他格式只采集，PDF/图片和 Office 分别留给后续卡。操作观察参与补排身份，成功缓存仍只依赖内容/解析器/选项。 | 未列扩展暂不解析；不能用二进制文本失败或假成功代替格式支持。元数据修改不使成功提取失效。 | [T46](t46-text.md) |
| 7 | T46 修复只抑制同语义 Failed 工作的新预算，不盲目抑制 queued/retry/running；保留观察栅栏与 Cancelled 后的新捕获补排。 | 新操作观察可能取消/重排工作，不承诺跨任意观察全局只有四次；T47 接管设计需继续考虑此代价，不能放松已批准发布栅栏。 | [T46](t46-text.md) |
| 8 | 等待 T47 新契约确认期间，串行准备下游可独立测试的部分；整卡依赖、合入与运行验收顺序仍为 T47→T48→T49→T50。 | 上游集成后可能需要适配准备成果；不部署未经验证的组合，不将等待当批准或准备当整卡完成。 | [条件计划](../../2026-11-17-t39-t50.md) |
| 9 | T49 单列既有 core files 领域补充子任务，API 保持薄适配。当前根 FD 完整有界读取、摘要及最终 root/source/观察校验后响应；missing 404，仍 present 但不可用 503，元数据-only 并发返回最终版本 ETag。 | 最多 100MiB 输入载荷需完整缓冲，实际内存另有开销且首字节延迟较高；扫描期间可能保守拒绝。不是流式/性能承诺，也没有新增公开参数或状态。 | [T49 core](../tasks/T49-core.md)、[PR #161](https://github.com/xingjianxu/knoted/pull/161) |
| 10 | T48 用有界 defusedxml OOXML 正文解析，复用 ParsedDocument/Block，不建立第二套 DTO；不可靠文字表达明确拒绝。 | 相比完整 Office 渲染只支持文档化正文子集，合法复杂文档可能拒绝，需要后续能力工作；不能靠遗漏文字伪装完整成功。 | [PR #159](https://github.com/xingjianxu/knoted/pull/159) |
| 11 | PPTX 包括隐藏与空白幻灯片，保留原始 1-based 序号；DOCX/XLSX 无可靠页码时为 NULL，Office bbox 均不虚构。 | 隐藏幻灯片正文可能被提取；不是放映可见性或页面渲染结果承诺。 | [PR #159](https://github.com/xingjianxu/knoted/pull/159) |
| 12 | Office 内部预算绑定选项身份：4096 parts、单 part 32MiB、展开总量 128MiB、压缩比 200、XML 50 万节点/深度 64、XML 及输出文本各 16MiB、实际 cells/blocks 各 10 万。原 100MiB/900秒边界保持。 | 大型、高压缩比或复杂合法 Office 可能被拒绝；超限是明确失败，不返回截断内容，也不冒充原 Spec 已给出的新限额。 | [PR #159](https://github.com/xingjianxu/knoted/pull/159) |
| 13 | XLSX 仅输出已存公式缓存并标注来源；无缓存不求值、不变零；保守拒绝外部/结构引用公式，DOCX/PPTX 不支持的正文表达拒绝。 | 缓存仍可能陈旧，一些合法公式工作簿不支持；不是 Office 数值/日期显示格式，不执行公式/宏/外链。 | [PR #159](https://github.com/xingjianxu/knoted/pull/159) |
| 14 | T50 首版支持 PDF、静态 PNG/JPEG/WebP 和严格转义文本；APNG/动画 WebP/未知容器拒绝预览但保留普通下载。 | 合法动画只能下载；不能先相信小画布，再让大帧绕过解码前 40M 像素守卫。此项现已通过完整 T50 验收。 | [PR #162](https://github.com/xingjianxu/knoted/pull/162)、[预览限额](../indexing-and-rag.md) |
| 15 | 预览绑定所选来源、内容版本、元数据 ETag/version、摘要和格式；ETag 不符时保守刷新/重试，不新增版本参数。 | 元数据-only 修改可能触发额外的 50MiB 有界取回，不应误称正文改变或重新解析。此项现已通过完整 T50 验收。 | [PR #162](https://github.com/xingjianxu/knoted/pull/162)、[双版本契约](../../../decisions/0006-file-pipeline-and-aggregate-contracts.md) |
| 16 | T50 文件选择镜像 Rust ASCII 小写与 Path 扩展语义，隐藏的仅扩展名 `.txt` 和 Unicode 大小写折叠变体不能扩大白名单。 | 这些名称保持仅下载；不将前端通用 Unicode lower-case 当作后端选择规则。 | [PR #162](https://github.com/xingjianxu/knoted/pull/162) |
| 17 | T47 Worker 的真实停机需要启用既有 Tokio signal feature；主 Agent 离线仅补必需的 signal-hook-registry 1.4.8 传递锁项，不升级既有包，不另写手工信号处理库。此项窄化替代初期锁字节不变的准备假设。 | 增加一项传递依赖并需完整 Rust 复验；不扩展公开接口、宿主权限或其他依赖治理范围。 | [T47 B 计划](../../2026-11-18-t47-execution.md)、[ADR0007](../../../decisions/0007-execution-exit-confirmation.md) |
| 18 | Office 持久 parser_revision 使用既有 policy 加 `:docx/:xlsx/:pptx` 绑定原扩展格式，选项/预算摘要保持；暂存名不推断格式。 | 同内容改扩展可能新增语义提取，避免错误复用旧成功或耗尽失败预算；备注/标签及同格式元数据仍复用。没有新增公开或存储字段。 | [T48](t48-office.md) |
| 19 | 在 stdlib 分配完整 ZipInfo 前按实际中央目录条目计数；本正文子集仅接受连续 ZIP32 目录，明确拒绝 ZIP64/多盘/自解压前缀/尾垃圾。 | 一些合法小型 Office 的特殊容器封装可能不支持；仍保留100MiB/4096与既有机器错误码，不引入另一个归档引擎或依赖。 | [T48](t48-office.md) |
| 20 | 最终review排除项逐项接受范围边界：后续索引/RAG/历史GC/生产另做；不承诺任意外部FS全局快照/弱身份猜测；完整Office排版/备注媒体/公式求值/其他ZIP、跨浏览器PDF/GPU/MPI/跨机器/进程逃逸均不作已验证能力；性能无本阶段SLA，#172延迟保留。最终文档/主线/dev/清理仍必须完成；Strata重启原因保持未知。 | 合法复杂输入及其他环境需要后续能力/验证，性能可能需要后续工作；不能用范围边界跳过最终交付证据。I1预防新增重复物仍在范围内。 | [最终修复证据](t39-t50-final-fixes.md)、[阶段交付](t39-t50-integration.md) |
| 21 | 更新DOCX解析身份时，旧身份仅识别并收束全部持久尝试（含当前lease），取得退出证明后取消，不登记新的执行身份/尝试且不发布旧结果；新扫描使用新DOCX身份，XLSX/PPTX/选项预算不变。 | 旧排队DOCX任务取消后须由新扫描补排；不能把旧任务误路由Rust文本而遗留运行计算，也不复活旧预算/结果。无新公开字段或存储契约。 | [最终修复证据](t39-t50-final-fixes.md)、[ADR0007](../../../decisions/0007-execution-exit-confirmation.md) |
| 22 | 澄清裁决21：沿已确认ADR0007，收束旧持久execution_id在404时仍可重放其原POST建立提交栅栏；先前“禁止新POST”的委派措辞过宽，限制的是新执行身份/新尝试及旧结果发布。 | 原ID若尚未创建计算，原POST重放可能短暂启动旧计算，必须随即按原协议取消并取得退出证明；不能以退休版本为由跳过监督，也不另改HTTP契约。 | [最终修复证据](t39-t50-final-fixes.md)、[ADR0007](../../../decisions/0007-execution-exit-confirmation.md) |

完整任务与分支组合审查及全部deferred分诊已完成；三项Important已统一修复并通过唯一复审、主线及dev验收。本裁决档案不代替实际验证，协调区最后收尾与证据见[阶段交付](t39-t50-integration.md)。
