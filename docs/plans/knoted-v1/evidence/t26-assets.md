# T26 图片资产与引用实施证据

Issue [#80](https://github.com/xingjianxu/knoted/issues/80)，[计划](../../2026-11-17-t26-assets.md)。状态：done。实现38967ac、主线文档集成5809cfc；PR [#82](https://github.com/xingjianxu/knoted/pull/82)普通merge至`ac8cd1ff1b040244a46e68e618db17fc4f879599`，主线检查与目标dev验收及实施工作区清理完成。

## 实现边界

现有OpenAPI的multipart上传与Asset DTO、鉴权content路由；PNG/JPEG/WebP、20MiB/40M像素、哈希检查、每账号/操作/键7天幂等。文件先私有暂存检查、像素解码校验，再UUID原子无覆盖发布；ready数据库记录才允许读取。客户端取消后登记任务继续；提交结果不确定时保留完整文件，不误删可能已成功登记的资产。

新迁移0007创建assets、asset_refs和幂等请求表。笔记当前/回收站与历史引用在原有写事务中替换，历史恢复和到期清理覆盖引用；零引用时钟供未来T67使用，本卡不实现回收。所有笔记/树/历史写先取schema范围的引用事务锁，再取树/节点锁；调用方连接完成全部SQL，避免多批维护的锁顺序冲突。

资产根为绝对`[assets].root`，默认`/data/knoted-dev/service/assets`；测试AppContext可注入独立路径，pcwrap按instance_root渲染assets，原件目录不变。依赖image精确0.25.10，仅png/jpeg/webp功能，锁文件新增206行、未更新原有依赖。OpenAPI DTO和客户端未变；无UI或host/生产改动。

## TDD与真实门禁

完整日志 `/data/knoted-dev/tasks/t26/`；初始4HTTP用例缺失路由404失败（red-http.log）。后续引用锁顺序、容器尾、幂等键/冲突优先级、取消、pc资产根隔离及WebP帧尺寸问题均有保留的RED/GREEN。主Agent接管后复现WebP帧校验失败，再加解码前头尺寸守卫，正常扩展与动画WebP同样通过。

| 实际命令 | 结果 |
| --- | --- |
| cargo fmt --check | exit0，无输出 |
| cargo test --workspace --locked | exit0，172passed0failed，1既有ignored doctest |
| T26实际用例（上述全量中执行） | 19passed（core12、HTTP7），含t26_1/2/3 |
| cargo clippy --locked --all-targets -- -D warnings | exit0，无告警 |
| pnpm test:pc | exit0，24passed0failed |
| git diff/cached --check、本地Markdown链接、shell语法 | 提交前实际检查 |

真实HTTP/PostgreSQL、独立文件目录、注入时钟与SQL故障覆盖鉴权/CSRF、三格式、限额/损坏/截尾、幂等并发/冲突/到期、ID路径、当前/历史/回收站/恢复/清理、note/outbox/refs回滚、文件/数据库错误及取消。测试不构成性能或容量基准。

## 已知限制与后续

第三方解码器的max_alloc是尽力限制，应用在分配像素前独立检查容器/编码帧像素和输出大小，捕获解码异常；不宣称进程RSS硬上限或穷尽所有损坏编码。没有在真实网络不确定COMMIT或硬件Passkey上新增验收声明。可能的未登记完整文件留给T67；读取总是由已提交ready元数据控制。

## 独立审查、主线与测试实例收尾

T26任务审查和最终T24–T26跨卡审查均Spec/Quality PASS，无Critical/Important或新增可操作缺陷；报告 `/data/knoted-dev/tasks/t26/{review,final-review}.md`。T24的Minor注释已修正；T25可选覆盖建议仍记录在其证据，不作为已执行测试。

主线ac8cd1f再次实际运行cargo fmt/test/clippy和pc场景，全部exit0；172passed0failed，1既有ignored；pc24passed。完整实际输出main-*.log，与提交前root-*.log分别保留。

停止API与Worker并确认dev数据库其他连接0后，创建0700一致备份 `/data/knoted-dev/service/backups/20261117T014301Z-t26`（custom dump+assets，摘要与pg_restore目录检查通过；未声称本轮完成完整恢复演练）。固定产物5809cfc含二进制/网页/迁移且完整SHA256SUMS独立验证通过，原子切换current并pc down/up。迁移1—7全部success，实际API/Worker可执行路径均匹配`5809cfc6c1ec03109dfb144527d4724139b2cba8`，见runtime-readback.json。

真实HTTPS验收（live-smoke.json）：

```json
{"result": "passed", "revision": "5809cfc6c1ec03109dfb144527d4724139b2cba8", "upload": 201, "idempotent_replay": true, "key_conflict": 409, "protected_read": 200, "anonymous_download": 401, "anonymous_upload": 401, "csrf": 403, "historical_image_ref": true, "history_restore": 200, "trash_ref": true, "pixel_bomb": 413, "credentials_changed": false}
```

现场SQL逐项确认移除当前图片后仅历史引用保留、恢复后可读取、软删后trash引用存在/current引用为0，仍引用资产zero_ref_at为null。并在同一固定产物复跑T24树/恢复/CAS与T25快照/反向恢复/停启Worker补存；live-t24-compatibility.json和live-t25-compatibility.json均passed。

所有临时会话最终撤销、本任务笔记正常软删除，保留历史及图片按正常生命周期；不改用户Passkey、源文件、host或生产。目标dev在迁移前确认为0个含图片的笔记/快照；没有向其他未知既有数据集泛化迁移验收结论。

实施 `.worktrees/knoted-t26` 与feat/t26已正常移除，远程分支由仓库自动删除；忽略依赖和技能scratch归档到任务worktree-generated目录。Issue80经PR关闭、Projects Done，保留原有HTTPS任务工作区。此文档收尾分支合并后也清理，运行产物与代码行为不变。
