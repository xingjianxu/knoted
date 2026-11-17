# T26 图片资产与引用实施证据

Issue [#80](https://github.com/xingjianxu/knoted/issues/80)，[计划](../../2026-11-17-t26-assets.md)。状态：实现和本地检查完成，待独立审查、主线集成及实例验收。

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

独立审查、备份/迁移、固定产物、真实HTTPS、运行SHA和清理记录将在实际执行后追加。
