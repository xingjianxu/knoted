# Knoted v1 文档与实施任务入口

> 产品实施采用实际可用的 executing-plans 或 subagent-driven-development。当前 T01—T05 已完成、审查并集成到本地主线；T06—T72 仍待领取。

## 阅读顺序

1. [需求与确认记录](requirements.md)
2. [架构与目录](architecture.md)
3. [数据与API](data-and-api.md)
4. [索引与RAG](indexing-and-rag.md)
5. [开发与部署](development-and-deploy.md)
6. [验收](acceptance.md)
7. 按依赖领取下表单张任务卡；[文档交付记录](delivery.md)与产品状态分开。

## 约束与范围

本文档交付的原授权是将会话计划落成Markdown，不执行T01—T72。此后用户单独授权实施 T01—T05（已完成并集成，见下方“实施进展”）；T06—T72 仍按任务卡逐张领取，重大架构实施前按仓库ADR/RFC规则确认，不把本计划误作Accepted ADR。Issue 与远程操作按根 AGENTS.md 规则 6 与「远程操作授权」执行：查重后可直接创建并领取任务 Issue，push 非默认分支、Issue、Labels、Projects、PR 创建与普通 merge 默认授权、不再逐次许可；仓库级设置与规则 12 的边界仍需人工确认，任务卡“远程权限”行与此一致。T01—T05 全部成果此后经分支 `feat/t01-t05-skeleton` 推送，以 PR [#4](https://github.com/xingjianxu/knoted/pull/4) 普通 merge 进入远程 master `795d624`。基线master `6c00202c1a389a4e0e0ae797744c2159654c761d`。

默认串行按编号推进；直接依赖列用于定位输入，不等于自动授权并行。共享锁文件、API、迁移、主机配置由主Agent串行集成。T10/T11/T12/T21/T70有独立app/host子任务，必须分别领取。除这些明确跨仓库交接外，一张卡一个Agent、一个Worktree、一组可独立验收的结果。

每任务接口和校验值以契约为准。与未出现的工具版本相关的可运行性结果必须由前置任务实际验证并锁定；不靠大模型猜版本。配置值允许按明确参数覆盖，改变业务语义须更新计划记录。

## 任务索引

| 任务 | 成果 | 直接依赖 | 状态 |
|---|---|---|---|
| [T01](tasks/T01.md) | Rust workspace与配置 | — | done（已集成 cd2cb6a） |
| [T02](tasks/T02.md) | React与工具链骨架 | — | done（已集成 7b62fdc） |
| [T03](tasks/T03.md) | 公共OpenAPI契约 | T01, T02 | done（已集成 7e3c54b） |
| [T04](tasks/T04.md) | 生成客户端与查询入口 | T03 | done（已集成 892178e） |
| [T05](tasks/T05.md) | 数据库迁移与测试隔离 | T01, T03 | done（已集成 79a7d06） |
| [T06](tasks/T06.md) | API健康和日志 | T05 | pending |
| [T07](tasks/T07.md) | 任务领取与租约 | T05 | pending |
| [T08](tasks/T08.md) | 任务重试和取消 | T07 | pending |
| [T09](tasks/T09.md) | Python任务协议与暂存对象 | T03, T08 | pending |
| [T10](tasks/T10.md) | Docling与中文OCR验证 | T09 | pending |
| [T11](tasks/T11.md) | BGE-M3可运行性验证 | T09 | pending |
| [T12](tasks/T12.md) | Strata IQ3_S可运行性验证 | T03 | pending |
| [T13](tasks/T13.md) | 唯一账号与首次注册 | T05, T06 | pending |
| [T14](tasks/T14.md) | Passkey注册与登录 | T13 | pending |
| [T15](tasks/T15.md) | 会话与请求保护 | T14 | pending |
| [T16](tasks/T16.md) | 凭据管理与恢复 | T15 | pending |
| [T17](tasks/T17.md) | UI基础控件与样式 | T02 | pending |
| [T18](tasks/T18.md) | 弹层和菜单控件 | T17 | pending |
| [T19](tasks/T19.md) | 树列表与空状态 | T18 | pending |
| [T20](tasks/T20.md) | 路由登录与应用外壳 | T04, T16, T19 | pending |
| [T21](tasks/T21.md) | 最小HTTPS测试部署 | T06, T20 | pending |
| [T22](tasks/T22.md) | 笔记JSON与提取规则 | T03, T05 | pending |
| [T23](tasks/T23.md) | 笔记CRUD与版本 | T15, T22 | pending |
| [T24](tasks/T24.md) | 页面树与生命周期 | T23 | pending |
| [T25](tasks/T25.md) | 历史快照与恢复 | T08, T23 | pending |
| [T26](tasks/T26.md) | 图片资产与引用 | T15, T25 | pending |
| [T27](tasks/T27.md) | BlockNote编辑核心与工具栏 | T17, T18, T22, T23 | pending |
| [T28](tasks/T28.md) | 斜杠菜单与块菜单 | T27, T19 | pending |
| [T29](tasks/T29.md) | 图片块和代码块 | T26, T28 | pending |
| [T30](tasks/T30.md) | 自动保存与冲突草稿 | T25, T29 | pending |
| [T31](tasks/T31.md) | 笔记树和历史页面 | T24, T30 | pending |
| [T32](tasks/T32.md) | 标签和资料关联 | T05, T24 | pending |
| [T33](tasks/T33.md) | 项目API与归档 | T32 | pending |
| [T34](tasks/T34.md) | 事项与单层子任务 | T33 | pending |
| [T35](tasks/T35.md) | 状态优先级与日期 | T34 | pending |
| [T36](tasks/T36.md) | 跟进时间线与历史 | T35 | pending |
| [T37](tasks/T37.md) | 事项归档和回收站 | T36, T08 | pending |
| [T38](tasks/T38.md) | 项目页面 | T33, T20 | pending |
| [T39](tasks/T39.md) | 事项页面和编辑 | T35, T37, T38 | pending |
| [T40](tasks/T40.md) | 看板和时间线 | T36, T39 | pending |
| [T41](tasks/T41.md) | 工作概览与到期 | T31, T39 | pending |
| [T42](tasks/T42.md) | 统一回收站页面 | T24, T37, T41 | pending |
| [T43](tasks/T43.md) | 目录扫描与排除 | T08, T32 | pending |
| [T44](tasks/T44.md) | 稳定副本与版本 | T43, T09 | pending |
| [T45](tasks/T45.md) | 移动识别和重关联 | T44 | pending |
| [T46](tasks/T46.md) | 文本解析 | T44 | pending |
| [T47](tasks/T47.md) | PDF与图片解析 | T10, T44, T46 | pending |
| [T48](tasks/T48.md) | Office解析 | T47 | pending |
| [T49](tasks/T49.md) | 文件浏览备注下载 | T45, T48, T32 | pending |
| [T50](tasks/T50.md) | PDF图片文本预览 | T49 | pending |
| [T51](tasks/T51.md) | OnlyOffice授权后端 | T49, T15 | pending |
| [T52](tasks/T52.md) | OnlyOffice网页接入 | T51, T50 | pending |
| [T53](tasks/T53.md) | 全文索引发布 | T07, T48, T36 | pending |
| [T54](tasks/T54.md) | 统一搜索接口 | T53, T37 | pending |
| [T55](tasks/T55.md) | 搜索界面 | T54, T19 | pending |
| [T56](tasks/T56.md) | 索引任务状态与重试 | T08, T54 | pending |
| [T57](tasks/T57.md) | 分块和定位 | T48, T22, T36, T53 | pending |
| [T58](tasks/T58.md) | 嵌入批处理适配 | T11, T57 | pending |
| [T59](tasks/T59.md) | pgvector存储与检索 | T58 | pending |
| [T60](tasks/T60.md) | 混合检索和范围 | T59, T54 | pending |
| [T61](tasks/T61.md) | 模型服务适配器 | T12, T60 | pending |
| [T62](tasks/T62.md) | 会话消息与生成队列 | T61, T15, T08 | pending |
| [T63](tasks/T63.md) | 上下文预算与依赖 | T62 | pending |
| [T64](tasks/T64.md) | 引用快照与验证 | T63 | pending |
| [T65](tasks/T65.md) | 聊天界面 | T64, T18, T55 | pending |
| [T66](tasks/T66.md) | 回答转存笔记 | T65, T23, T32 | pending |
| [T67](tasks/T67.md) | 图片和临时对象回收 | T26, T37, T64 | pending |
| [T68](tasks/T68.md) | 一致性备份 | T67 | pending |
| [T69](tasks/T69.md) | 隔离恢复演练 | T68, T53, T59 | pending |
| [T70](tasks/T70.md) | 完整Nix模块与主机接入 | T21, T69 | pending |
| [T71](tasks/T71.md) | 功能与故障恢复集成验收 | T70 | pending |
| [T72](tasks/T72.md) | 最终测试部署和交接 | T71 | pending |

## 阶段交付

A基础T01—T12；B认证/UI T13—T21；C笔记T22—T32；D事务T33—T42；E文件/全文T43—T56；F RAG T57—T66；G运维T67—T72。部署T21后按可部署任务逐次更新，不把只有PR或测试通过称为生命周期完成。

## 实施进展（T01—T05）

基线 master `715e1c2`。五个任务各自在 `.worktrees/knoted-t0N` 的 `feat/t0N` 分支实施，经独立审查、修复循环后合并到本地 master；实施阶段未推送、未创建 PR。39 个提交随后整体经 `feat/t01-t05-skeleton` 分支推送，由 PR [#4](https://github.com/xingjianxu/knoted/pull/4) 普通 merge 进入远程 master `795d624`；本地 master 已快进同步，该分支已删。

| 任务 | 任务分支提交范围 | 主线集成 | 主线复跑证据 |
|---|---|---|---|
| T01 | `8a32d2a`..`cd2cb6a` | 快进至 `cd2cb6a` | `cargo test --workspace t01_` 7 passed；fmt/clippy 0；二进制缺库配置 exit 1、凭据零泄漏 |
| T02 | `d4d0ced`..`f01a8fa` | merge `7b62fdc` | `pnpm install --frozen-lockfile`、typecheck、lint、build 0；vitest t02 5 passed |
| T03 | `1dfd5b8`..`5d344bb` | merge `7e3c54b` | `pnpm api:check` 0（79 operations / 16 examples / 8 invalid rejected） |
| T04 | `a5d7231`..`b3691e3` | merge `892178e` | `pnpm api:generate` 无差异；web vitest 13 passed |
| T05 | `7805c6d`..`e9a08a4` | merge `79a7d06` | `cargo test --workspace t05_` 8 passed（真实 PG 18）；测试后无残留 schema |

主线整体验证：`cargo fmt --check`、`cargo test --workspace --locked`（15 passed）、`cargo clippy --workspace --all-targets -- -D warnings`、`pnpm install --frozen-lockfile`、`pnpm api:check`、`pnpm run typecheck`、`pnpm run lint`、`pnpm run test`（13 passed）、`pnpm run build` 全部 exit 0。逐卡证据与仍存在的限制写在各任务卡“执行证据”及 `/data/knoted-dev/tasks/t0N/report.md`；过程记录见 `/data/knoted-dev/tasks/knoted-v1-impl/progress.md`。

尚未执行：远程推送、PR、CI、测试实例部署（T21 前无测试实例，T01—T05 均为库/契约/前端骨架，无需部署）。
