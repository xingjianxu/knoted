# T24 页面树与生命周期实施证据

- Issue：[#75](https://github.com/xingjianxu/knoted/issues/75)；基线 `b180e0c`；Worktree `.worktrees/knoted-t24`，分支 `feat/t24`。
- 状态：实施及本地验证完成；待主 Agent 独立审查、合入、主线验证、测试实例更新及清理。
- 范围：新增 `notes/tree.rs` 与 HTTP move/restore 动作，沿用 T23 的删除保护、AppContext、content_version CAS、全树事务锁和 outbox。未修改 OpenAPI、生成客户端或迁移。

## 行为与并发

- move 的 `parent_id` 是必填可空字段，负数位置及非法版本拒绝；restore_parent 可省略，不能为 null，内部 parent_id 必填可空。未知字段拒绝。
- restore_parent 省略时保留原父；父失效返回 409，显式 `{parent_id:null}` 才恢复到根；允许指定其他存活父。
- 结构写先取得既有全树 advisory lock，再锁操作对象和祖先；检查存活父、缺失父、自环、后代及重复祖先，并检查包含已删除行的完整存储链。
- move、恢复、同级位置调整、版本和事件在同一事务完成；失败等待 rollback 后响应。版本陈旧、非法父、成环及 outbox 故障没有笔记或事件副作用。
- 顺序规则细化：超出末尾的位置夹到末尾，恢复按已保存位置插入并夹到末尾；相同位置移动不递增版本、不写事件。调整兄弟位置不改变兄弟正文版本。
- 并发验证覆盖：互移产生环、两个节点插入同一位置、move 与正文更新争用同一 CAS、重复恢复、移动到父节点与删除该父竞争。

## TDD 与门禁

实际命令均在任务 Worktree 执行，设置 `TMPDIR=/data/knoted-dev/tasks/t24`、`CARGO_TARGET_DIR=/data/knoted-dev/tasks/t24/target`。完整输出保存在该任务目录，未使用其他任务缓存或目录。

| 命令 | 退出码 | 结果/完整输出 |
| --- | --- | --- |
| `cargo test --workspace --locked t24_`（实现前） | 101 | 12 执行；11 因路由尚不存在返回 404 失败，既有非级联删除保护通过；`red.log` |
| `cargo test --workspace --locked t24_`（实现后） | 0 | 最初 12 通过，再补充越界位置和历史祖先链防御验证；最终 14 通过；`green.log` |
| `cargo fmt --check` | 0 | 无输出；`fmt.log` |
| `cargo test --workspace --locked` | 0 | 139 通过、0 失败；1 个既有 ignored doctest；`workspace-test.log` |
| `cargo clippy --locked --all-targets -- -D warnings` | 0 | 无告警；`clippy.log` |
| `git diff --check`、`git diff --cached --check` | 0 | 无空白/冲突标记问题 |

摘录本轮真实测试输出：

```text
running 14 tests
test t24_1_move_into_descendant_or_self_is_atomic_conflict ... ok
test t24_2_delete_never_implicitly_cascades ... ok
test t24_3_sibling_order_is_stable_across_moves_and_restore ... ok
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

T24 属于运行行为变更，须在集成后由主 Agent 更新测试实例并记录真实运行 SHA。本实施者未 push、合并、部署、关闭 Issue 或清理 Worktree。
