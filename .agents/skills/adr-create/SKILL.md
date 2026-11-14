---
name: adr-create
description: Use when an architecture decision needs a numbered ADR, an existing proposal has human confirmation, or an accepted decision is being explicitly superseded with preserved history.
---

# 创建和维护 ADR

## 输入与触发

输入决策标题、背景、方案及理由、备选、正负影响、关联 Issue/PR、人工确认依据和替代关系。读取 [ADR 索引](../../../docs/decisions/README.md)、[指南](../../../docs/development/adr-guide.md) 和 [模板](../../../docs/decisions/template.md)。写文件须使用合规 Worktree。

## 步骤

1. 检查现有记录和待集成草稿，避免同一决策重复建档。只修普通问题时报告无需 ADR。
2. 核对人工确认者、日期、确定方案及范围、可追溯证据。没有明确证据则 Status: Proposed，并说明待确认；Issue 可开发不等于所有架构已批准。
3. 主 Agent 在集成前读取最新主线及待集成 ADR 的**最大编号**，分配 max+1，至少四位零填充。不要使用文件数；并行 Agent 先给未编号草稿，由主 Agent 串行分配，重新核对是否被占用，必要时重编号并修复链接。
4. 写 `docs/decisions/NNNN-short-title.md`，填齐模板字段，状态仅 Proposed / Accepted / Rejected / Superseded；模板不占号。Accepted 必须包含确认依据。
5. 更新索引。替代 Accepted 时，先确认新 ADR 已获人工批准，再给新 ADR 写 Supersedes，给旧 ADR 显式更新 Status 和 Superseded by；保留旧背景、决策、理由和历史，不删除或静默改写。
6. 检查编号唯一、状态合法、双向链接、关联 Issue/PR 真实及确认记录。调用项目测试/文档检查；提交前读取有效 git-commit，缺失则保留成果并报告，只有用户明确指定替代流程时才按新授权执行。

## 输出与边界

输出 ADR 路径/编号/状态、确认依据、索引和替代关系变更、验证结果及待决定项。未得到人工确认不标 Accepted；缺失权限仅交付草稿，不替用户作决策。

示例：最大编号 0007，有两个并行草稿 → 主 Agent 集成时分配 0008/0009；没有人工确认的那份仍为 Proposed，编号不表示批准。
