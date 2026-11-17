# T34 事项与单层子任务证据

Issue [#91](https://github.com/xingjianxu/knoted/issues/91)，任务卡[T34](../tasks/T34.md)。状态：implemented；首轮独立审查发现的P2已修复并通过实施者验证，待主Agent定向复审、主线与dev收尾。

## 实现边界

基线0a043d3，实施提交83de6f9。迁移0010扩展既有tasks，不重建、不改0009、不使用假标题默认；未知最小旧行安全拒绝且保留。完整Task DTO/CRUD/move_project沿用已有OpenAPI；单层父子、项目继承、nullable字段出现性、CAS/ETag、软删、游标过滤与outbox均在服务事务内。父迁项目与子同事务，活归档子或项目归属与原父不一致的存活子拒绝迁移，不借迁移修复历史关系；软删子仅同步项目关系/version/time，保留内容和生命周期。

复用T33的33001关系锁，发现集合后跨projects/tasks全局UUID顺序锁行并重读。T32/T33 fixture适配真实Task服务，projects/tests.rs移除模拟写。状态/优先级闭集与DateValue基础校验唯一复用；T35继续完整终态守卫和注入时钟逾期计算，不另造DTO。

## 初版实施者实际验证

完整报告与输出 `/data/knoted-dev/tasks/t34/t34-report.md`、`logs/`。首批7/9真实HTTP场景因缺路由404产生有效RED；实现后通过。旧T32最小fixture因title NOT NULL失败后改为真实Task创建，事件计数保留真实创建事件而非删除它们。初版T34 14个真实HTTP/PG用例、T33 14个、t32_选择器11个（含2个T33匹配名称）通过；包含受控真实并发、未知旧行升级拒绝、回滚、auth/CSRF和字段出现性。

| 实际命令 | 结果 |
|---|---|
| cargo fmt --check | exit0，无输出 |
| cargo test --locked | exit0，209passed/0failed/1既有ignored doctest |
| cargo clippy --locked --all-targets -- -D warnings | exit0 |

上述为初版实施者记录；首轮独立审查已完成，修复后的定向复审与父Agent主线独立复验待执行。无前端/OpenAPI/锁文件变更，无重复不受影响的检查。本地测试只写隔离schema，未部署dev。

## 集成与数据前提

T33更新后父Agent实际dev只读确认tasks表0行（含软删），无假标题回填/删数据；迁移0010前仍应再次核验并一致备份。当前保留feat/t34与任务Worktree。PR、主线SHA、固定产物、真实HTTPS与清理证据由父Agent后续补记；本记录不提前认定完成。


## 独立审查修复轮1

首轮独立Spec/Quality审查（anchor `6d7063d`）发现P2：父迁移项目会静默修复历史错配的存活子事项，可能把子事项移出已归档/删除的原项目；子已位于请求目标时还会跳过其版本/事件。作者核对实际路径，原锁集合已包含子与其项目，缺少保存父之前的继承一致性守卫。现对迁项目操作在既有完整锁集合后、任何写入/事件前，拒绝未删除且project_id与原父不同的子，409返回父current_version；保留活归档子守卫与已删除子关系迁移例外，不增加锁或自动修复。

新增五个真实HTTP/Postgres用例分别覆盖active/archived/deleted/NULL来源和子已在请求目标。修复前全部返回200而非409产生RED；修复后逐例409、current_version=1且完整tasks/registry/outbox快照不变。另新增已删除错配子源项目归档时仍可迁移的回归，保留其内容/生命周期/registry并精确增加父子两条事件。定向选择器从2passed/5failed转为7passed；完整T34现20项，受影响T32选择器11项、T33 14项全部通过。

| 本轮实际命令 | 结果 |
|---|---|
| cargo test --locked -p knoted-api --test t34 t34_parent_move_ | RED exit101，2passed/5failed；GREEN exit0，7passed |
| cargo test --locked --workspace t34_ | exit0，20passed/0failed |
| cargo test --locked --workspace t32_ | exit0，11passed/0failed |
| cargo test --locked --workspace t33_ | exit0，14passed/0failed |
| cargo fmt --check | exit0，无输出 |
| cargo test --locked | exit0，215passed/0failed/1既有ignored doctest |
| cargo clippy --locked --all-targets -- -D warnings | exit0 |

完整输出为`/data/knoted-dev/tasks/t34/logs/fix1-{red,green-regression,focused-t34,focused-t32,focused-t33,gate-fmt,gate-test,gate-clippy}.log`，逐命令退出码记录在`fix1-focused-status.json`和`fix1-gate-status.json`。本轮只改事项守卫、相应测试、本证据及任务卡；无迁移/契约/依赖/前端变更。作者已自查，主Agent定向复审、主线验证、dev升级/烟测仍待执行；本轮未做远程写入或部署。

## 主 Agent 集成验收

PR [#98](https://github.com/xingjianxu/knoted/pull/98) → 主线 `aa58c1a`。独立 Spec/Quality 审查 PASS，主线 Rust 215 passed / 0 failed / 1 既有 ignored，fmt/clippy exit0。实际输出 `/data/knoted-dev/tasks/t27-t35/evidence/t34-main-*.log`。固定产物aa58c1a、迁移10及真实HTTPS事项CRUD/层级/移动/日期验证通过。本卡 Issue 已关闭且 Projects Done；任务 worktree/已合并分支已安全清理，其他任务成果保留。前述“待执行”是当时实施阶段的历史记录，以本节实际验收为准。
