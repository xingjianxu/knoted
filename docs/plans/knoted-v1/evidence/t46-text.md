# T46 文本解析实施证据

任务关联 #131，基线 `a57881ecf42c687d9f44e926647ee6f7deaa299c`，任务分支 `feat/t46`。

- [x] 核对 T44/T45 实际采集、双版本及来源身份，读取 ADR0006 与任务卡。
- [x] 空文本、编码、字符保真首轮 RED→GREEN。
- [x] 实现受控 staging 解码、固定大小正文块、成功发布事务与现有 Dispatcher。
- [x] 完成受影响完整门禁及自审（最终 Rust 363 顶层用例及 1 个既有 IO child、Web 248 用例通过；fmt/clippy/typecheck/lint/build 均 exit 0）。
- [ ] 主 Agent 独立审查、集成、主线与 dev 验收（不由实施 Agent 预填）。

## 实现范围

新增迁移 0015 创建 ADR0006 的 `file_extractions`，只保存成功完整 blocks。同一来源 content_version/parser_revision/options_hash 唯一；Rust 记录真实 job_id，execution_id 为 NULL。成功结果不受元数据 version 变化影响，重放不覆盖结果或重复发送成功 outbox。

采集成功事务及闭合采集回放均可补排文本任务，沿用唯一 Dispatcher 和 jobs 租约、取消与重试。发布事务锁定 jobs/root/source，复核 live lease、根未暂停/完整扫描、present/fresh、采集与当前观察一致、content_version、摘要及输入对象；提交前重查租约期限。失败与取消不发布成功产物。

严格 UTF-8（可含 BOM）及带 BOM UTF-16 LE/BE；非法编码与 decoded NUL 永久失败。空文件成功且 blocks 为空。HTML/XML/代码仅作为原文，不执行。正文不去空白、不替换字符，按最多 64KiB UTF-8 字节块存储（字符边界切分），元数据随块数增长。位置记录 decoded UTF-8 字节半开区间、1-based Unicode scalar 列及行、空白行分段计数；行列 end 是排他位置。段落位置可结合完整保留的块正文定位，不为每字符/每行分配对象。100MiB 输入不被截断。

扩展名大小写归一：txt/md/markdown、rs/py/js/jsx/ts/tsx/c/h/cpp/hpp/cc/java/go/rb/php/sh/bash/zsh/fish/sql/html/htm/css/scss/xml/json/toml/yaml/yml/ini/cfg/conf/csv/tsv/log/nix/swift/kt/kts/lua/r/pl/ex/exs/erl/hrl/vue/svelte；无扩展名 basename README/LICENSE/Makefile/Dockerfile/Justfile。仍受 scanner 原有隐藏/排除规则约束。其他格式保留采集，不进入 Rust 文本解析；PDF/图片留 T47，Office 留 T48，未知格式无文本成功声明。

## 验证与交付记录

完整原始日志与退出码：`/data/knoted-dev/tasks/t46/logs/`。
实施报告：`/data/knoted-dev/tasks/t39-t50/reports/t46-implementation.md`。
本证据不声明 PR 合入、主线验收或 dev 运行完成。

最终门禁输出：`/data/knoted-dev/tasks/t46/logs/gates2.log`；Rust 完整输出 `final2-cargo-test.log`，Web 完整输出 `final2-web-test.log`。Rust 另有 1 个既有文档测试 ignored，不计为通过；Web 既有 scrollTo 提示与 build chunk 提示不影响退出码。

## R1：普通补扫不能重开失败预算

独立审查指出采集扫描 ID 进入幂等键，使相同来源/内容的已失败解析在普通补扫后重新排队。真实 UTF-8/NUL RED 均证实：强身份、SHA、version/content_version/observation_version 不变，captured_scan_id 变化后任务数 1→2。

修复在采集入队事务按 source/content_version/parser/options 查询既有 `failed` 任务并保留终态，包括临时故障耗尽预算；不读取错误展示字符串。新任务 opaque payload 显式记录 parser_revision/options_hash，旧 T46 无字段任务映射到固定 Rust v1 身份。新内容或不同 parser/options 不受旧失败阻挡；Cancelled 旧观察仍可由新采集恢复。成功产物唯一性、发布 pointer 校验、任务重试 API 均未变更。

新增真实 Runner 测试覆盖上述 UTF-8/NUL、staging 输入缺失导致的 TemporaryFailure/max_attempts=1 耗尽、新内容、不同 parser/options、旧 payload 兼容；保留已有 Cancelled→新观察成功场景。定点日志在 `/data/knoted-dev/tasks/t46/logs/round1-*.log`，最终门禁与提交见实施报告。

R1 最终 Rust 门禁 `round1-gates.log`：fmt/test/clippy 均 exit 0；全 workspace 369 个顶层测试通过，另 1 个既有 IO child 通过、1 个既有 doc ignored。Web/API 字节未变，本轮不重复验证。仅 Failed 语义终态不重开；active 跨新观察的总预算与接管留 T47 设计，不声称本轮证明全局四次尝试上限。
