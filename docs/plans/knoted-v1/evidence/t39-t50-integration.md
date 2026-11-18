# T39—T50 阶段交付证据

状态：**产品交付完成**。十二张卡及最终三项补修已普通合并、独立审查和主线/dev验收；最新产品主线/dev为 `e9da413`（PR [#177](https://github.com/xingjianxu/knoted/pull/177)）。本收尾文档合入后执行末节的协调工作区清理，并以实际读回作为整任务最终交付。

## 授权与隔离

用户直接委派T39—T50，明确要求subagent及README `done（PR #N，主线 SHA，dev SHA；证据）`。跨模块与存储补充方案经人工确认记录[ADR0006](../../../decisions/0006-file-pipeline-and-aggregate-contracts.md)；后续退出证明契约经独立确认记录[ADR0007](../../../decisions/0007-execution-exit-confirmation.md)。每卡独立实施者/Worktree、独立Spec/Quality审查；产品写任务串行，主Agent集成与独立验证，均走普通merge PR，不直推master。全部裁决及代价按顺序存[裁决档案](t39-t50-rulings.md)，历史偏差与失败记录保留。

## 逐卡结果

| 卡 | PR | 首次完整主线/dev | 证据 |
|---|---|---|---|
| T39 | [#123](https://github.com/xingjianxu/knoted/pull/123) | bbdef36 | [事项页面](t39-task-page.md) |
| T40 | [#136](https://github.com/xingjianxu/knoted/pull/136) | aee7c2e | [看板时间线](t40-board-timeline.md) |
| T41 | [#139](https://github.com/xingjianxu/knoted/pull/139) | f2423b4 | [概览](t41-dashboard.md) |
| T42 | [#141](https://github.com/xingjianxu/knoted/pull/141) | d7c74ec | [回收站](t42-trash.md) |
| T43 | [#144](https://github.com/xingjianxu/knoted/pull/144) | 987ef07 | [扫描](t43-scanner.md) |
| T44 | [#147](https://github.com/xingjianxu/knoted/pull/147) | 9160def | [采集](t44-capture.md) |
| T45 | [#150](https://github.com/xingjianxu/knoted/pull/150) | c786f3e | [移动关联](t45-identity.md) |
| T46 | [#154](https://github.com/xingjianxu/knoted/pull/154) | e10ee8e | [文本提取](t46-text.md) |
| T47 | [#157](https://github.com/xingjianxu/knoted/pull/157) | 0f65529 | [PDF/图片](t47-parse.md) |
| T48 | [#159](https://github.com/xingjianxu/knoted/pull/159) | c936e0a | [Office](t48-office.md) |
| T49 | [#161](https://github.com/xingjianxu/knoted/pull/161) | 8ffb240 | [文件浏览下载](t49-files.md) |
| T50 | [#162](https://github.com/xingjianxu/knoted/pull/162) | 2c89043 | [预览](t50-preview.md) |

逐卡状态收尾文档已普通合并；最新T50收尾PR[#174](https://github.com/xingjianxu/knoted/pull/174)为主线`4c6be25`，7MD独立审查、217本地链接通过，产品字节未动，dev保持`2c89043`。远程真实读回十二卡关联Issues全部CLOSED、Projects全部Done；数据在`/data/knoted-dev/tasks/t39-t50/final-remote-pr-issue-readback.json`及`final-project-readback.json`。

## 最终组合审查

最强模型独立审查真实起点`b496d89..2c89043`，94 commits/195变更文件的组合包；生产边界分段审读，未冒充逐行审计全部测试或重跑同字节全套。完整报告`/data/knoted-dev/tasks/t39-t50/reviews/final-t39-t50-review.md`。原结论Spec FAIL / Quality CHANGES_REQUESTED，3 Important /0 Critical：周期补扫永久保留重复暂存、relink未核对当前启用根、DOCX特殊表格包装成功漏字。一个独立subagent/Worktree负责全部修复，受影响完整门禁、唯一scoped复审、主线/dev及产品工作区清理均已完成，结果见下节。[修复证据](t39-t50-final-fixes.md)。

## 最终修复验证

- [x] 三项有效RED→GREEN、冻结、父Rust/Python完整门禁及静态检查。
- [x] 原问题I1/I2/I3唯一scoped复审与新引入回归审查。
- [x] 普通PR合并、主线针对性验证、同SHA固定产物与dev更新。
- [x] 实际重复扫描对象稳定/正常DOCX及两个包装拒绝/清理与退出证明。
- [x] 所有产品任务Worktree/已合并分支清理，其他任务不动，远程状态读回；协调区按末节清理。

## 非阻断项与证据边界

最终review全部deferred已分诊：#138 TestDb析构、#143 jsdomscrollTo、#149锁顺序覆盖、#153临时目录、#156依赖弃用、#163共享错误体无界、#169下载busy回焦、#170旧ETag提示、#171MIME继承键、#172解析积压延迟扫描、#175 FileDetail JSX可读性、#176 T48注释/命名。均保留独立问题，不作为本轮Important修复顺带重构；T50A原始TDD偏差无法倒补历史，证据区分业务RED、fixture失败与事后敏感性探针。

最终review的七项Declined-to-judge逐项处理见裁决20：未来能力/平台/性能未承诺，最后文档/主线/dev/清理仍是必需项；不排除I1预防新增重复物，也不把T50先等待提取终态的功能验收当作积压扫描性能达标。裁决21保留旧DOCX持久尝试退出证明，不能误路由后跳过监督。

T50实际浏览器十源和原生两页中文PDF已通过，原两轮90秒扫描等待失败保留；独立核对是113秒FIFO排队/7ms扫描，功能复验增加116647ms提取准备等待后仍保留原删除90秒断言。43次执行GET echo与退出证明均有效、完整38320文件运行后精确，详见T50证据。最终审计还检测共享Strata PID变化（12:15启动，原因未知/本任务未管理其服务），PG/Caddy/Knoted进程保持，不能声称所有共享PID不变。e9da413的四源与退出核对已单独实测，见下文；浏览器仅按61个Web产物逐字节相同复用，不声称重新运行。

修复提交b5b9fc9、纯空行2e5968b、仅已审7MD正常merge后78ba568；PR177普通merge主线/dev e9da413。完整Rust415顶层+8嵌套/1ignored与fmt/clippy0，完整Python171/9既有warning370.68s，最终Ruff/7focused0；唯一复审3ADDRESSED/SpecPASS/QualityAPPROVED/newCI0。父13文件SHA逐项核验，纯空行前后AST相同，原格式失败和API夹具失败分别保留。主线Rust9/Python3与fixedbuild0，38320文件清单SHA95e309224c9dcc035c4697fc8b20445ab13d90723d0e66af0ebc8a305305e095，实际部署三fixedexec/CPU/loopback、health ok/active_executions 0，备份20261118T045610Z-t39-t50-final-。

实际四源129→130→131→清理132全通过，两轮完整补扫staging/input均114件且SourceId/双版本/观察/inputObjectId/提取历史不变；正常DOCX完整、两个包装明确失败，源仅自有UUID清理且历史输入保留。最后49次执行全部GETecho/exitproof、health及jobs active0、.part0，schema15/0present75missing57成功提取17预期失败jobs/Passkey1，运行后38320清单精确。本次共享服务PID对部署before保持；先前Strata变更历史未抹去。新旧61个Web产物SHA相同，复用T50浏览器证据，不冒充新SHA再次运行。私有helper的set/dict与Btrfs长期dirFD枚举故障已离线定位、保持严格断言修正；原失败及成功cleanup留存，不作为产品RED。完整路径见[修复记录](t39-t50-final-fixes.md)。

## 收尾文档与协调工作区

本记录、22条顺序裁决、README/架构与计划在协调分支独立审查后按普通PR集成；属于纯Markdown，不重部署或重复模型门禁。合入后主Agent核对主线文档/链接、产品13SHA与dev仍e9da413，再归档本计划私有SDD工作区至Data并删除仅本计划临时目录，正常移除协调Worktree/已合并分支；保留其他任务的HTTPS Worktree。实际最终结果由`/data/knoted-dev/tasks/t39-t50/final-workspace-cleanup.json`及最终交付报告读回，不把计划中的动作当已执行。
