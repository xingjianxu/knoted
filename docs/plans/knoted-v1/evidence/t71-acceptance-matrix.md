# T71 高风险场景验收矩阵

本次核对 T69 主线固定产物 `5308378799a768ded33d26d1e08c37473f30eca5`。下表按验收清单的顺序列出 26 项。每行计数只统计该行点名、在同一份 Rust 主线日志中返回 `ok` 的测试函数；浏览器、组件和恢复演练分别计数。完整 Rust workspace 为 534 通过、0 失败、1 个既有文档示例忽略。 [验收清单](../acceptance.md) · [T71 任务卡](../tasks/T71.md)

| 场景 | 主线 Rust 用例 | 通过数 | 断言与其他层级 |
|---|---|---:|---|
| Passkey重放/错误origin | `t14_1_replayed_challenge_rejected`、`t14_2_wrong_origin_or_rp_id_rejected` | 2 | 认证 API 拒绝挑战重放和错误来源，两条失败路径均检查会话状态。 |
| 恢复/会话过期 | `t15_1_idle_and_absolute_expiry`、`t16_1_recovery_affects_only_owner`、`t16_2_old_sessions_invalid_after_recovery` | 3 | 会话空闲/绝对过期；恢复后旧会话失效，唯一账号身份保留。 |
| 自动保存乱序/冲突 | `t23_1_concurrent_updates_have_exactly_one_winner` | 1 | 并发版本更新只有一次成功。组件层另看 T30 的 `t30_1 serializes pending edits and never cleans latest draft on old acknowledgement` 与 `t30_query_out_of_order_dirty_and_clean never advances editing base from Query`。 |
| 异常关闭 | `t25_2_crash_recovery_backfills_latest_once_before_five_minutes`、`t25_2_real_worker_stays_running_backfills_and_restarts_idempotently` | 2 | 异常后的最新正文可读；真实 Worker 补存并重启，重复运行不增加快照。 |
| 树移动成环/父删除 | `t24_1_move_into_descendant_or_self_is_atomic_conflict`、`t24_2_delete_never_implicitly_cascades`、`t24_rejections_leave_versions_positions_and_events_unchanged` | 3 | 拒绝环和隐式级联删除；核对原位置、版本和事件。 |
| 子任务完成条件 | `t35_3_parent_done_requires_all_live_children_terminal`、`t35_completion_vs_reopen` | 2 | 有未终结子事项时，父事项无法完成；覆盖并发重开。 |
| 到期日跨午夜 | `t35_1_date_only_midnight_boundary`、`t35_2_no_due_and_terminal` | 2 | 上海日期的次日零点切换；无到期日与终态单独断言。 |
| 归档后编辑/恢复 | `t37_1_archive_is_readonly_and_unarchive_restores_writes`、`t33_3_batch_isolation`、`t37_5_project_unarchive_never_cancels_manual_task_archives` | 3 | 归档拒写，撤销批次只恢复对应记录，并保留此前手动归档。 |
| 历史图片仍被引用 | `t26_3_history_current_trash_restore_prune_and_invalid_write_rollback`、`t67_1_history_retains_its_image_and_only_old_unreferenced_images_are_deleted` | 2 | 即使只剩历史引用，回收仍保留图片。 |
| 根失联/扫描中断 | `t43_1_root_loss_and_replacement_preserve_registered_sources`、`t43_2_cancelled_scan_cannot_complete` | 2 | 根失联保留来源；中止的扫描不能登记为成功完成。 |
| symlink/FIFO/越界 | `t43_3_symlinks_hidden_tools_and_special_files_are_excluded`、`t44_2_fifo_replacement_is_nonblocking`、`t44_subdirectory_symlink_replacement_rejects_even_same_leaf` | 3 | 跳过符号链接和特殊文件，复制前后替换入口也不能突破只读根。 |
| 复制时更新/旧结果迟到 | `t44_1_writes_between_stats_reject_stale_capture`、`t46_publication_rejects_every_stale_fence` | 2 | 复制前后状态变化抛弃该副本；发布时再查来源和租约。 |
| 相同hash不同文件 | `t45_1_same_hash_different_inode_not_merged`、`t45_same_bytes_real_copy_is_separate_source` | 2 | 相同字节的独立文件保持不同来源身份。 |
| Python超时/重启/取消 | `t47_real_running_cancel_proves_exit_before_replay`、`t47_python_deadline_crossed_waiting_for_publication_locks_rolls_back`、`t08_3_unconfirmed_cancel_keeps_slot` | 3 | 取消、过期与未证明退出的槽位。T70 隔离 VM 还验证实际 Python 主进程 SIGKILL 后 guardian 确认退出、重启可读取终态，缺失证明报告 degraded。 |
| Worker租约转移 | `t07_3_stale_token_cannot_finish`、`t07_2_expired_lease_is_taken_over`、`t07_runner_stops_when_the_lease_is_lost` | 3 | 租约接管后旧 token 无法发布，旧执行者停止。 |
| Meili接收但未应用 | `t53_2_accepted_engine_task_is_not_published`、`t53_9_permanent_engine_rejection_ends_in_failed` | 2 | 引擎接收和最终应用分别记账。T71 浏览器第二步另让索引写入失败后恢复。 |
| 全文好向量坏 | `t54_3_only_latest_fulltext_is_visible_without_vectors`、`t59_1_source_change_immediately_revokes_vector_visibility` | 2 | 全文索引与向量发布分别判断新鲜度。浏览器另核对新版文件的真实向量任务失败、任务页显示“失败”和“重试任务”，同一文件仍能通过全文检索命中。 |
| 删除后迟到索引 | `t54_1_soft_delete_hides_the_existing_engine_hit`、`t59_2_filtering_returns_only_real_matching_current_vectors`、`t60_2_current_visibility_filters_both_paths` | 3 | 旧引擎命中须过 PostgreSQL 可见性检查，两条召回路径都受限。 |
| 中文词语位于长句中 | `t53_1_chinese_word_inside_a_long_sentence_is_found` | 1 | 长句内中文可命中；T55 组件 `t55_3 highlights matching text without interpreting HTML` 检查片段呈现。 |
| 聊天含关键词 | `t62_4_completed_reply_is_searchable_and_soft_delete_is_immediate`、`t60_1_document_index_and_chat_cannot_supply_evidence` | 2 | 聊天进入全文索引；RAG 排除聊天证据。 |
| 历史依赖来源已删 | `t63_1_inherited_dependency_discards_the_entire_round`、`t63_source_removal_fences_stream_and_restore_leaves_history_excluded`、`t64_2_citations_keep_fixed_snapshot_after_version_change_and_deletion` | 3 | 源删除切断直接/继承依赖，已存引用快照仍可查看。 |
| 删除来源后独立转存笔记 | `t66_2_note_and_citation_survive_deletion_of_sources` | 1 | 删除来源后仍能读取独立笔记及保存时的来源快照。 |
| 取消/重复提交/断流 | `t62_1_duplicate_idempotency_key_creates_one_turn_and_run`、`t62_2_cancel_holds_next_run_until_upstream_confirms`、`t62_3_taking_over_does_not_restart_an_unfinished_generation` | 3 | 取消与接管检查持久状态；T65 组件 `t65_1 retains partial text after disconnect and queries the run without resending` 检查断流正文。 |
| 模型伪造引用 | `t64_1_only_server_issued_citation_ids_are_evidence`、`t64_4_protects_history_and_binds_cursor_to_the_answer` | 2 | 伪造 URL 与引用 ID 得到空证据 ID 和“依据不足。”；T65 组件 `shows the persisted answer when terminal validation changes the draft` 检查页面显示服务端定稿。 |
| Office授权过期/源变化 | `t51_1_source_expires_and_revoke_is_idempotent`、`t51_3_preview_keeps_original_bytes_and_rejects_stale_version`、`t51_2_outbox_scope_is_process_preview_file_and_session` | 3 | 到期和换版后不能借旧授权取得新内容；授权限当前文件、预览及会话。历史 T52 实际查看器见下。 |
| 恢复缺图/摘要错 | `t69_1_digest_mismatch_rejects_archive`、`t69_1_changed_archive_never_installs_unverified_bytes` | 2 | 隔离恢复另实跑两种负例，登记图片从归档缺失、摘要错误。均未创建恢复目标，原会话和原笔记继续可读。 |

## 可复核的运行记录

- Rust 主线执行 `cargo test --locked --workspace`；`/data/knoted-dev/tasks/t69/evidence/t69-mainline-rust.log` 记录提交、命令、逐项结果和 ignored 示例。涉及受控上游的 Rust 用例检查状态机；实际引擎验证见下述独立演练。
- T71 浏览器执行 `bash tests/fixtures/t71/instance.sh pnpm exec playwright test tests/e2e/t71.spec.ts`，Chromium 1 项通过、0 失败。同一用例先从文件问答、查看引用、转存笔记并关联事项，再阻断索引写入、确认旧引擎命中被应用过滤、解除故障并等待新版可检索。向量故障代理仅更改该新版一次真实 BGE 执行的结果大小，验证同一执行在数据库和真实任务页面终止失败、无可见向量，而全文结果与文件身份一致。真实 HTTPS、PostgreSQL/pgvector、Meilisearch 和固定 BGE-M3 参与；聊天生成使用读取合成上下文后返回 SSE 的确定性夹具，认证使用 Chromium 虚拟 CTAP2。日志为 `/data/knoted-dev/tasks/t71/evidence/t71-vector-ui-final.log`。
- T69 恢复演练执行一轮隔离实例恢复，包含第 26 行的两个拒绝场景，并核验源实例 HTTP 200、恢复实例的旧会话 401、同一虚拟 Passkey 重新登录，以及历史、引用、图片和重建的 1024 维向量。日志为 `/data/knoted-dev/tasks/t71/evidence/t71-restore-missing-image.log`。
- T70 在同一应用产物的独立 KVM 内执行 1 组生命周期测试；正常停止为 cancelled，主进程异常退出为 interrupted，两者都有退出证明。删除证明后为 degraded。日志为 `/data/knoted-dev/tasks/t70/evidence/t69-module-final.log`。这些运行未验证生产 GPU 或实体认证器。
- Web 组件回归在 T71 测试分支执行 23 文件、442 项通过（`/data/knoted-dev/tasks/t71/evidence/t71-gate-web-tests.log`）。编辑器与 mtone 定向执行 6 文件、82 项通过（`t71-editor-mtone-vitest.log`）；当前锁版 `@bixlab/mtone@0.4.0` 的 T17–T19 Chromium 用例 15 项通过（`t71-locked-mtone-browser.log`），两份日志均位于同一 T71 evidence 目录。
- Linux 原生输入法实跑使用 IBus/libpinyin、GTK 候选窗、隔离 Xvfb 和 Chromium。在 T27 编辑器中，X11 逐键输入拼音，候选窗显示“你好”在首位，Space 提交后保存 JSON 包含“你好”。再次输入“再见”的拼音后按 Escape，保存 JSON 逐字未变，caret 偏移保持 6，编辑器保留焦点。命令为 `dbus-run-session -- bash /data/knoted-dev/tasks/t71/tmp/ime-secondary/session.sh`，退出 0。日志和候选窗截图为 `t71-native-ime.log`、`t71-native-ime-candidates.png`。本次覆盖 Linux IBus/X11。
- T52 的实际 OnlyOffice 证据对应主线 `4e722319dc850ee7627c94bb7d7919cdf0020e97`；T61–T65 的历史集成对应 `867b004910efd5711e26a9578421054c76413ba0`，其中 T65 组件定向 10 项通过。 [T52 查看器验收](t52-office-preview.md) · [T61–T65 集成记录](t61-t65-integration.md)

T70 生产主机和 T71 最终集成由各自任务记录提交、运行产物与验收结果。本矩阵所列结果固定在上述各轮版本。
