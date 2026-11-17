# T27–T35 跨卡集成验收

依据用户直接委派与最多三个子 Agent 的授权执行[实施计划](../../2026-11-17-t27-t35.md)。原始基线 `249db1953dc2dbff043a97340c40de5b6edb4b5b`；本页记录实际结果，未完成步骤不提前认定通过。

当前：九卡均已完成全生命周期，最终代码主线与运行 SHA 为 `40e9c3db70ee481337f6588472f6e60cf7bba872`；仅本汇总文档与最终跨卡独立审查待收尾。

| 卡 | Issue | PR / 主线 | 本卡主线实际门禁 | 适用实例与清理 |
|---|---|---|---|---|
| T27 | #84 | #95 / `8081764` |181 Rust、77 Web；其余全0|纯未挂载库；任务工作区/分支已清理|
| T28 | #86 | #97 / `7d28fb2` |195 Rust、92 Web；其余全0|纯未挂载库；任务工作区/分支已清理|
| T29 | #87 | #99 / `7bf185d` |215 Rust、109 Web；其余全0|真实HTTPS上传/读取/重开通过；纯库；工作区/分支已清理|
| T30 | #88 | #101 / `752a093` |229 Rust、158 Web；其余全0|纯未挂载库；任务工作区/分支已清理|
| T31 | #89 | #102 / #103 / #104，`40e9c3d` |229 Rust、192 Web；其余全0|完整HTTPS及失败重认证通过；工作区/三分支已清理|
| T32 | #85 | #94 / `cef4e7d` |181 Rust、56 Web；其余全0|迁移8/HTTPS关联与标签通过；工作区/分支已清理|
| T33 | #90 | #96 / `0a043d3` |195 Rust；fmt/clippy全0|迁移9/HTTPS项目生命周期通过；工作区/分支已清理|
| T34 | #91 | #98 / `aa58c1a` |215 Rust；fmt/clippy全0|迁移10/HTTPS事项CRUD与层级通过；工作区/分支已清理|
| T35 | #92 | #100 / `e42c1d8` |229 Rust、109 Web；其余全0|仍迁移10/HTTPS终态及历史日期通过；工作区/分支已清理|

所有 Rust 通过数均为真实 `cargo test --locked` 各目标输出之和，失败0，另有1个既有 ignored doctest。Rust其余门禁为 `cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`；Web其余门禁为包scripts的typecheck/lint/build。完整输出及逐条退出码保留在 `/data/knoted-dev/tasks/t27-t35/evidence/t*-{root,main}-*.log` / `*-results.json`。详细实现/修复见每卡证据，不将已有功能声明为新增RED。

## 审查与真实验收的修复

- T27：代码块 Enter、语义字段旧sidecar和URI拼写问题经有效RED/GREEN及独立定向复审关闭。
- T32：笔记链接锁与树移动的死锁、CAS响应current_version遗漏经真实PostgreSQL并发与HTTP回归关闭。
- T34：父迁项目静默修复历史错配存活子项，已改事务写入前409且保留已删除子项例外。
- T30：保存过程中撤销回旧稿、响应丢失后重试假报成功，已独立记录unresolvedWrite并保持旧CAS真实重试。
- T31：401后在途标签创建的关联续发已加同步发包检查；真实浏览器另发现往返笔记的新挂载使用旧seed，以及大写UUID失败重认证后身份key不一致；均已有效RED/GREEN、独立定向复审及新固定SHA HTTPS复验关闭。

无已停放的Minor或未处理的重要发现。逐卡发现全部关闭；本汇总自身仍待最终跨卡审查与PR收尾。

## 契约、实例及凭据边界

`pnpm api:check` 实际输出：

```text
api:check ok — 83 operations, 16 examples (8 invalid rejected)
```

`pnpm api:verify` exit0，生成客户端无差异。新增迁移仅0008/0009/0010，部署前停本实例写进程并对数据库及资产做一致备份；迁移0010前两次实查tasks为空，不填假标题、不删除未知数据。T35无新迁移。实例固定SHA产物校验和、current与API/Worker实际/proc/exe逐次读回；HTTPS由既有宿主Caddy提供，未改宿主或生产配置。

真实浏览器只使用自己创建的短期会话和验收笔记/标签，凭据仅在内存；无HAR/trace/storageState/请求头输出。后端smoke及浏览器均不拦截API。自己的业务fixture通过正常API软删除，短期会话撤销，Passkey计数不变；资产沿既有引用/回收流程保留。T31真实脚本 `/data/knoted-dev/tasks/t31/live-browser-check.mjs` 与 `uppercase-expiry-check.mjs`，最终输出见下节。

一次父验证同时使用共享Cargo target与不同revision/CARGO_HOME，在215业务测试通过后rustdoc报E0463。已确认构建元数据被并发替换，保留失败日志并改为串行完整重跑；后续主线门禁全部实际通过。T31浏览器脚本的持久化类型名（codeBlock）及ConfirmDialog的alertdialog角色已校正，失败记录保留，不算产品修复或有效RED。

## 实施裁决及误判代价

1. 已批准v1规范及直接委派足以授权普通实施，不重复普通设计审批；重大新架构边界仍须确认。若误判，可能造成未批准普通行为的返工，故逐项对照原卡/规范。
2. 两条独立实施线各用任务工作区，最多三个子Agent，主Agent串行处理共享文件和合并。若独立性误判，会产生冲突/集成缺陷，须由逐卡和主线门禁发现；共享target验证编排失误已纠正。
3. T32分类型标签表沿用registry稳定类型身份并另查真实业务可见性；缺失模块不造生产表、不扩统一生命周期。若契约理解错误，后续实体可能无法关联，已由T33/T34真实模块回归验证。
4. T34父迁项目时，存活归档子拒绝，已删除子仅随迁关系/版本/时间/事件，保持内容和生命周期。代价是已删除子版本增长、旧CAS需重读；若规则判断错误，未来恢复行为可能需要调整，错配存活子已fail-closed。
5. T31预检initialDocument示例不足以处理往返笔记新挂载；新挂载应取保留会话最新draft，普通保存依旧靠固定key与NoteEditor初始值语义保持实例。若误判，会破坏选区或覆盖正文，须同时验证DOM稳定、显式重载及真实往返。

## 限制

真实浏览器/合成及CDP组合事务不等同操作系统原生中文候选/取消的人工验收。关闭浏览器仅best effort，工作区内存草稿不承诺跨进程恢复。构建通过，保留shared约629kB、notes约860kB的chunk提示；本轮不扩展性能或打包任务。未确认mtone通用能力缺口，没有本地通用UI替代、上游源码修改或未发布依赖。

## 最终代码主线与真实浏览器输出

固定SHA40e9c3d的父Agent实际门禁（其余Web命令与退出码见同组results.json）：

```json
[
  {
    "command": [
      "cargo",
      "fmt",
      "--check"
    ],
    "exit": 0,
    "log": "/data/knoted-dev/tasks/t27-t35/evidence/t31-fix3-main-fmt.log"
  },
  {
    "command": [
      "cargo",
      "test",
      "--locked"
    ],
    "exit": 0,
    "log": "/data/knoted-dev/tasks/t27-t35/evidence/t31-fix3-main-test.log",
    "passed": 229,
    "failed": 0,
    "ignored": 1
  },
  {
    "command": [
      "cargo",
      "clippy",
      "--locked",
      "--all-targets",
      "--",
      "-D",
      "warnings"
    ],
    "exit": 0,
    "log": "/data/knoted-dev/tasks/t27-t35/evidence/t31-fix3-main-clippy.log"
  }
]
```

```text
 Test Files  12 passed (12)
      Tests  192 passed (192)
```

真实HTTPS，无API拦截；使用已发布的实际生产页面，不是开发fixture：

```json
{"result":"passed","phase":"expired-draft","revision":"40e9c3db70ee481337f6588472f6e60cf7bba872","checks":{"autosave_keeps_editor":true,"readonly_history":true,"history_restore_new_version":true,"own_move_fresh_CAS":true,"real_asset_upload":true,"real_code_controls":true,"real_tag_assignment":true,"return_note_latest_document":true,"parent_delete_guard":true,"real_delete_undo_root_fallback":true,"asset_reopen":true,"conflict_copy_preserves_original":true,"expired_draft_visible_copyable":true,"passkeys_unchanged":true},"cleanup_ok":true}
{"result":"passed","phase":"failed-reauth-keeps-draft","revision":"40e9c3db70ee481337f6588472f6e60cf7bba872","cleanup_ok":true}
```

API/Worker实际可执行路径与current均为40e9c3d，最终部署一致备份 `20261117T071447Z-t27-t35`。全量主流程证明编辑器稳定、真实图片/代码/标签、往返最新正文、历史只读/恢复、移动新CAS、父删除门槛、原父失效明确root恢复、冲突副本保原稿、会话失效可复制；独立定点证明大写UUID失败重认证仍保稿。

## 汇总收尾

最终跨卡独立审查：待执行；汇总文档PR及协调工作区的实际合并/验证/清理回执由其PR核对记录。各卡实现工作区与已合并分支已全部安全清理，原有HTTPS工作区保留。
