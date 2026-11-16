# ADR 0002: 通用 UI 由 mtone 提供，Knoted 直接消费 npm 包

- Date: 2026-11-16
- Status: Accepted
- Decision owner: 项目维护者（本次会话用户）
- Confirmation: 2026-11-16 用户明确要求「现在不要在本项目实现通用UI库，请使用 @bixlab/mtone 的npm库」，通用组件需求整理为提示词由用户添加；审阅直接消费、T17 移除占位包、任务卡/文档/Skill 分阶段方案后回复「请实施」。会话授权记录见[设计](../plans/2026-11-16-mtone-ui-design.md)与 Issue。
- Related issues: [#39](https://github.com/xingjianxu/knoted/issues/39)
- Related PRs: [#40](https://github.com/xingjianxu/knoted/pull/40)（仅补充交付元数据，不改决策内容）
- Supersedes: 无（替换 v1 计划的 R25/自有 UI 提案，不替代已有 Accepted ADR）
- Superseded by: 无

## 背景与问题

T17—T19 原计划在 `packages/ui` 构建基础控件、弹层、树列表。主线该包仅含 `cn()` 工具，尚无通用组件。用户要求不在 Knoted 维护通用 UI 库，以免重复建设并分散组件维护。

2026-11-16 只读核查 npm `@bixlab/mtone@0.2.0` 的元数据、README 和公开类型：已有基础表单、弹层、列表、树、布局与预编译 CSS。此为接口调研，不是 Knoted 运行兼容性证明。

## 决策内容

1. `apps/web` 直接依赖精确锁定的 npm `@bixlab/mtone` 版本，消费公开 API 与 CSS。不维护 `@knoted/ui` 转发层；T17 实施时移除占位包并迁移引用/检查入口。
2. 通用控件、交互、样式能力归 mtone；业务页面、API/Query 绑定、权限、错误映射、游标状态及编辑器命令归 Knoted。可以组合现有 mtone 组件，不把业务 DTO 或请求逻辑送入 mtone。
3. Tailwind 可用于应用布局，不能形成第二套基础控件体系；不以直接包装 Base UI、复制源码、私有导入或 patch 绕过 mtone 的公开能力边界。
4. BlockNote 只保留编辑核心，关闭默认 UI，使用 mtone 组合工具栏/菜单；编辑器状态与选区适配仍在 Knoted。
5. 缺口先核查实际锁定版本及组合能力。真实通用能力缺失时，由 Agent 交付可复制提示词给用户；Agent 不修改 mtone、不代发上游 PR 或 npm 包。
6. 等待上游是技术依赖，不是默认权限审批。记录关联任务及恢复条件；npm 发布、消费端锁版验证、正常任务集成全部完成后才算交付。不以 link/Git/未发布 tarball 或“master 已修复”解除阻塞。

## 备选方案

| 方案 | 收益 | 成本与结论 |
|---|---|---|
| 继续自建 UI | 控制实现与发布 | 违背用户方向，重复建设；排除 |
| 本地转发/包装库 | 统一本地入口 | 当前仅 cn 无迁移包袱，额外 API 层易变为另一个通用库；不采用 |
| 直接消费 npm mtone | 单一通用组件来源 | 需要锁版与消费验收，发布节奏可能阻塞；采用 |

## 决策理由

明确的用户归属要求优先；已有实现很少，迁移成本可控。用有证据的上游需求交接而不是本地补洞，确保职责长期一致。npm 导出名称不必与旧卡一致，例如 Dialog 对应 Modal、Menu 对应 DropdownMenu、EmptyState 对应 Empty；名称映射不替代行为测试。

## 影响

- 正面：减少重复 UI 实现、版本接口分叉和样式漂移。
- 负面：上游不支持的交互需等待用户处理及 npm 发布；升级需要兼容性复验。
- 兼容风险：分页 total 与游标、日期时间精度、树移动、虚拟锚点/焦点、样式层次和 cn 合并语义均按真实场景核查，不预先断言缺组件。
- 迁移：先调整规则/任务卡/Skill，后按 T17—T19 安装/迁移/验收；T02/T04 历史证据不变。无存储/API 迁移。
- 回退：消费故障可回到上一个已验证的 npm 版本；没有可用版本则阻塞并交接需求，不自动回退到本地通用实现。改变本决策需新的人工确认与 ADR。

## 验证与后续任务

[UI 开发指南](../development/ui-mtone.md)、[实施计划](../plans/2026-11-16-mtone-ui-plan.md)、[T17](../plans/knoted-v1/tasks/T17.md)—[T19](../plans/knoted-v1/tasks/T19.md)。本次只验证文档和工作流，不声称 mtone 已接入。版本来源见指南。

## 状态历史

- 2026-11-16：会话提案经用户确认后记录为 Accepted；授权实施第一阶段文档/Skill，不提前执行产品卡。
