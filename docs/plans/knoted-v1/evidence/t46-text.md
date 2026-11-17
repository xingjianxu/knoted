# T46 文本解析实施证据

状态：done（PR [#154](https://github.com/xingjianxu/knoted/pull/154)，主线/dev `e10ee8ea1c5b3c20187a478f6b17e070f0a3edad`）。以下保留实施阶段记录，最终集成见文末。

任务关联 #131，基线 `a57881ecf42c687d9f44e926647ee6f7deaa299c`，任务分支 `feat/t46`。

- [x] 核对 T44/T45 实际采集、双版本及来源身份，读取 ADR0006 与任务卡。
- [x] 空文本、编码、字符保真首轮 RED→GREEN。
- [x] 实现受控 staging 解码、固定大小正文块、成功发布事务与现有 Dispatcher。
- [x] 完成受影响完整门禁及自审（初次实现 Rust 363 顶层用例及 1 个既有 IO child、Web 248 用例通过；fmt/clippy/typecheck/lint/build 均 exit 0）。
- [x] 主 Agent 独立审查、集成、主线与 dev 验收。

## 实现范围

新增迁移 0015 创建 ADR0006 的 `file_extractions`，只保存成功完整 blocks。同一来源 content_version/parser_revision/options_hash 唯一；Rust 记录真实 job_id，execution_id 为 NULL。成功结果不受元数据 version 变化影响，重放不覆盖结果或重复发送成功 outbox。

采集成功事务及闭合采集回放均可补排文本任务，沿用唯一 Dispatcher 和 jobs 租约、取消与重试。发布事务锁定 jobs/root/source，复核 live lease、根未暂停/完整扫描、present/fresh、采集与当前观察一致、content_version、摘要及输入对象；提交前重查租约期限。失败与取消不发布成功产物。

严格 UTF-8（可含 BOM）及带 BOM UTF-16 LE/BE；非法编码与 decoded NUL 永久失败。空文件成功且 blocks 为空。HTML/XML/代码仅作为原文，不执行。正文不去空白、不替换字符，按最多 64KiB UTF-8 字节块存储（字符边界切分），元数据随块数增长。位置记录 decoded UTF-8 字节半开区间、1-based Unicode scalar 列及行、空白行分段计数；行列 end 是排他位置。段落位置可结合完整保留的块正文定位，不为每字符/每行分配对象。100MiB 输入不被截断。

扩展名大小写归一：txt/md/markdown、rs/py/js/jsx/ts/tsx/c/h/cpp/hpp/cc/java/go/rb/php/sh/bash/zsh/fish/sql/html/htm/css/scss/xml/json/toml/yaml/yml/ini/cfg/conf/csv/tsv/log/nix/swift/kt/kts/lua/r/pl/ex/exs/erl/hrl/vue/svelte；无扩展名 basename README/LICENSE/Makefile/Dockerfile/Justfile。仍受 scanner 原有隐藏/排除规则约束。其他格式保留采集，不进入 Rust 文本解析；PDF/图片留 T47，Office 留 T48，未知格式无文本成功声明。

## 验证与交付记录

完整原始日志与退出码：`/data/knoted-dev/tasks/t46/logs/`。
实施报告：`/data/knoted-dev/tasks/t39-t50/reports/t46-implementation.md`。
以上为实施交接时记录；最终 PR/主线/dev 事实见下文。

最终门禁输出：`/data/knoted-dev/tasks/t46/logs/gates2.log`；Rust 完整输出 `final2-cargo-test.log`，Web 完整输出 `final2-web-test.log`。Rust 另有 1 个既有文档测试 ignored，不计为通过；Web 既有 scrollTo 提示与 build chunk 提示不影响退出码。

## R1：普通补扫不能重开失败预算

父 Agent 最后核对发现、独立审查确认采集扫描 ID 进入幂等键，使相同来源/内容的已失败解析在普通补扫后重新排队。真实 UTF-8/NUL RED 均证实：强身份、SHA、version/content_version/observation_version 不变，captured_scan_id 变化后任务数 1→2。

修复在采集入队事务按 source/content_version/parser/options 查询既有 `failed` 任务并保留终态，包括临时故障耗尽预算；不读取错误展示字符串。新任务 opaque payload 显式记录 parser_revision/options_hash，旧 T46 无字段任务映射到固定 Rust v1 身份。新内容或不同 parser/options 不受旧失败阻挡；Cancelled 旧观察仍可由新采集恢复。成功产物唯一性、发布 pointer 校验、任务重试 API 均未变更。

新增真实 Runner 测试覆盖上述 UTF-8/NUL、staging 输入缺失导致的 TemporaryFailure/max_attempts=1 耗尽、新内容、不同 parser/options、旧 payload 兼容；保留已有 Cancelled→新观察成功场景。定点日志在 `/data/knoted-dev/tasks/t46/logs/round1-*.log`，最终门禁与提交见实施报告。

R1 最终 Rust 门禁 `round1-gates.log`：fmt/test/clippy 均 exit 0；全 workspace 369 个顶层测试通过，另 1 个既有 IO child 通过、1 个既有 doc ignored。Web/API 字节未变，本轮不重复验证。仅 Failed 语义终态不重开；active 跨新观察的总预算与接管留 T47 设计，不声称本轮证明全局四次尝试上限。


## 最终集成与运行验收

PR [#154](https://github.com/xingjianxu/knoted/pull/154) 普通 merge 到 `e10ee8ea1c5b3c20187a478f6b17e070f0a3edad`，dev 同 SHA。Issue [#131](https://github.com/xingjianxu/knoted/issues/131) CLOSED/Projects Done；独立 review_t46 对 R1 给出 ADDRESSED，最终 Spec compliant / Quality Approved，无 Critical/Important。初次批准在父 Agent 发现 R1 后曾改为 Needs fixes，修复和限定复审完成后才合并，没有隐藏这一轮。

父 Agent 独立及主线实际 fmt/test/clippy 均exit0；最终369顶层通过、0失败、1既有doc忽略，另1次IOchild通过；Web248、typecheck/lint/build均exit0。T46精确命令23例：core7/publication4/runner12。原始 `logs/parent-fix1-*.log`、`logs/main-*.log`；API/pc/Python/依赖/锁本卡字节未改，已有一致性证明，不将未重跑套件写成新通过。测试可移植性Minor独立登记 [#153](https://github.com/xingjianxu/knoted/issues/153)，既有scrollTo见 [#143](https://github.com/xingjianxu/knoted/issues/143)，大chunk提示保留。

官方builder从上述完整SHA的git archive构建，完整SHA256SUMS exit0。停API/Worker写入并验证备份 `/data/knoted-dev/service/backups/20261117T204708Z-t46-` 后迁移schema15，固定产物运行两个进程exe实际位于该SHA目录；HTTPS ready200，共享PostgreSQL/Caddy/Strata PID不变。`logs/artifact-build.log`、`logs/deploy.log` 与 `logs/runtime-{before,after}.json` 保存实际输出。此卡没有启动Python持久服务或实施新的退出证明协议。

真实运行 `/data/knoted-dev/tasks/t46/live-acceptance.py`，仅批准根下自有UUID样本，所有步骤exit0；四阶段 `runtime-{initial,unchanged,changed,cleaned}.log/.exit` 与 `evidence/runtime-*.json`：

| 阶段 | 真实结果 |
| --- | --- |
| 初始 generation41 | 5个来源；空文本/UTF8/UTF16成功，3份提取；非法UTF8/NUL各failed一次，2失败任务，无部分产物；decoded文本逐字一致，空blocks为空，execution_id均NULL |
| 未改内容补扫 generation42 | 成功仍3份且ID不变，Failed仍2份，未重开任务预算；metadata添加/幂等添加/移除标签只变version，正文ID/content_version/SHA不变 |
| 同inode修改正文 generation43 | 同来源content_version1→2，新提取使总数3→4，旧正文提取完整保留；另外两个非法来源失败总数仍2 |
| 样本删除 generation44 | 自有5个来源变missing/fresh=false，4份成功与2份故意失败历史保留，不物理删除引用数据 |

真实HTTPS/Chromium验证概览显示已支持文本解析，文件数与DB一致、索引未启用；`logs/live-browser.log` 的 metadata/no-op正文身份保持、阶段文案、Passkey未变和会话撤销全部true。Unicode样本 `4bfdbb81-fe63-404d-a594-ee4f9cb0f0f1` 的四阶段 metadata/content版本依次1/1、3/1、5/2、6/2，验证元数据与正文版本分离。

最终 `evidence/runtime-cleanup.json`：0present、9missing历史、4成功提取、2故意非法输入failed历史；active Note/Tag/验收session均0，Passkey仍1，staging `.part`为0。自有UUID目录已删除，已发布闭合对象仍由应用引用，未盲删。

`feat/t46` 已确认master祖先，tracked/untracked干净，ignored仅可重建node_modules后安全清理本任务worktree/分支，保留他人任务。reviewer的主线/dev/清理待核项由以上实证解决。T47退出确认RFC [讨论](../../../rfcs/2026-11-18-t47-execution-exit-confirmation.md) 仍等待人工决定，既有ADR0006与本卡结果有效。
