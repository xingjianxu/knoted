# T34 事项与单层子任务证据

Issue [#91](https://github.com/xingjianxu/knoted/issues/91)，任务卡[T34](../tasks/T34.md)。状态：implemented，待独立审查、主线与dev收尾。

## 实现边界

基线0a043d3，实施提交83de6f9。迁移0010扩展既有tasks，不重建、不改0009、不使用假标题默认；未知最小旧行安全拒绝且保留。完整Task DTO/CRUD/move_project沿用已有OpenAPI；单层父子、项目继承、nullable字段出现性、CAS/ETag、软删、游标过滤与outbox均在服务事务内。父迁项目与子同事务，活归档子拒绝迁移；软删子仅同步项目关系/version/time，保留内容和生命周期。

复用T33的33001关系锁，发现集合后跨projects/tasks全局UUID顺序锁行并重读。T32/T33 fixture适配真实Task服务，projects/tests.rs移除模拟写。状态/优先级闭集与DateValue基础校验唯一复用；T35继续完整终态守卫和注入时钟逾期计算，不另造DTO。

## 实施者实际验证

完整报告与输出 `/data/knoted-dev/tasks/t34/t34-report.md`、`logs/`。首批7/9真实HTTP场景因缺路由404产生有效RED；实现后通过。旧T32最小fixture因title NOT NULL失败后改为真实Task创建，事件计数保留真实创建事件而非删除它们。最终T34 14个真实HTTP/PG用例、T33 14个、t32_选择器11个（含2个T33匹配名称）通过；包含受控真实并发、未知旧行升级拒绝、回滚、auth/CSRF和字段出现性。

| 实际命令 | 结果 |
|---|---|
| cargo fmt --check | exit0，无输出 |
| cargo test --locked | exit0，209passed/0failed/1既有ignored doctest |
| cargo clippy --locked --all-targets -- -D warnings | exit0 |

这些为实施者记录，父Agent独立复验和审查尚待完成。无前端/OpenAPI/锁文件变更，无重复不受影响的检查。本地测试只写隔离schema，未部署dev。

## 集成与数据前提

T33更新后父Agent实际dev只读确认tasks表0行（含软删），无假标题回填/删数据；迁移0010前仍应再次核验并一致备份。当前保留feat/t34与任务Worktree。PR、主线SHA、固定产物、真实HTTPS与清理证据由父Agent后续补记；本记录不提前认定完成。
