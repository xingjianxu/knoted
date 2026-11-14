# 文件索引与RAG实现约定

## 发现与读取

一个只读根；通知合并2秒，完整补扫每10分钟（可配置）。只有完整成功扫描才根据缺失判定来源消失；根身份变化/权限错/挂载丢失暂停，不能清空来源。默认排除.git、node_modules、target、缓存、临时/隐藏文件；显式include仅允许普通隐藏文件，不允许突破根和跟随symlink。

两次间隔2秒的stat一致后采集。使用根目录FD相对打开和不跟随链接的安全解析，普通文件限定，拒绝设备/FIFO/socket。复制到staging随机对象，流式摘要并限制100MiB；读取前后fstat核对身份/长度/mtime/ctime，必要再次摘要验证；变化则丢弃重排。发布前重新核对source版本及当前观察值。不能证明任意外部并发写下的原子性，只保证变化时拒绝发布并最终收敛。永远不修改源。

移动仅在一次完整扫描内：旧路径缺失、新路径唯一、device/inode相同、内容摘要相同且无硬链接歧义时继承；有明确成对rename事件也需复核。inode重用或缺失后再发现无法证明时视新来源。不能仅按hash自动合并同内容文件。用户手动relink移动标签/备注/关联到新来源，旧引用快照保持原版本。

## 解析和GPU

100MiB/500 PDF页/900秒是准入与取消上限，不是性能承诺；输入损坏/格式不支持/超限→明确终态，不发布截断结果。XLSX逐工作表提取可见单元格与坐标，不执行公式/宏/外链；PPTX按幻灯片；DOCX按段落/表格，缺乏可靠页码时使用结构位置。图片上限40M像素，压缩炸弹拒绝。纯文本UTF-8（含BOM）、UTF-16 BOM可识别，其他编码失败待人工处理，不静默乱码。

Python提供POST /internal/v1/executions、GET /{id}、POST /{id}/cancel、GET /{id}/result。请求 `execution_id,kind=parse|embed,input_object_id,input_sha256,source_version,deadline,options_revision`。状态queued/running/succeeded/failed/cancel_requested/cancelled/interrupted；interrupted为可重试终态，由Rust创建新execution_id，不能把同ID视为仍在运行。重复ID+同摘要返回原记录，不同参数409。受控object_id不能包含路径分隔，拒绝网络URL；结果保存在staging，对象清单与摘要由Rust验证。

Python调度器每个执行启动可取消子进程，收到取消杀掉子进程组并wait确认退出后释放槽；Rust租约失效也必须取消/观察旧执行。调度状态持久到专用SQLite执行账本（只保存计算状态，不是业务数据库），重启把running标为interrupted；不盲目接续计算。HTTP超时不代表计算取消。BGE-M3只用dense，1024维，归一化；同模型revision统一输入规则。NaN/Inf/错维拒绝。

最多两卡，配置GPU UUID而非永久依赖0/1。一张Strata，一张Python；Python GPU槽1，OCR与嵌入串行，共用卡不同时驻留大模型。模型在管理员预安装后运行，业务请求禁止联网下载。模型revision、ONNX检测/识别/方向模型、tokenizer和解析选项全部锁定。

## 任务、发布与重建

PG jobs状态queued/running/retry_wait/succeeded/failed/cancel_requested/cancelled；claim返回lease_token，心跳20秒、租约90秒。结果提交比较token和source_version。临时故障初次后最多3次自动重试（总执行最多4次），延迟5/30/120秒；人工重试创建新job并指向原job。没有取消确认不能释放资源并发配额。

业务事务同时写outbox。Meilisearch task ID落库，轮询成功后发布；按version生成文档键，过期文档可后台删但绝不靠删除速度保证可见性。全文/向量每条结果由PG验证当前content_version和deleted状态。新内容保存后旧结果立即不可返回，当前全文先好先用。重建用新index generation，完成后切换；不清空当前唯一索引后再建。

## 分块与混合检索

以BGE tokenizer计数；段落/标题/表格行优先，目标512 token、最大768、重叠64；超长段落按token边界拆分，记录原结构定位。chunk_id由source/version/chunker_revision/ordinal确定，模型升级新embedding_set，不混用空间。pgvector cosine，HNSW；删除条件先由PG约束，受过滤影响不足不编造结果。

全文分两种索引：T53的document索引用于统一搜索；T57建立独立rag_chunks词法索引，仅含file/note/task片段，字段chunk_id/source_id/source_version/chunk_set_id/text/location及过滤字段。T57在ChunkSet持久化事务写词法outbox，并复用T53发布确认协议。全文候选从rag_chunks取，不将document命中临时猜成chunk。两套索引代次和发布状态独立。T60按当前ChunkSet复核；模型更新不需要重建同一chunk文本索引。

词法chunk与向量各取最多40条当前候选；按chunk_id去重，RRF常数60，最多选12块（同来源最多4块）。文件/笔记/事项范围与项目过滤同时作用两路。项目说明可全文搜，但不额外纳入RAG；事项及跟进记录以独立片段提供。聊天只全文。

## 问答与删除依赖

Strata和external兼容接口显式配置base_url/model/credential_file，默认local，禁止自动fallback。外部调用前显示服务选择；不同服务内容不得缓存串用。32K总预算含输出，系统+问题+引用+历史不超过28K；模型tokenizer可用则使用其计数，外部未知tokenizer采用保守UTF-8字节上界，超限拒绝/减历史而非静默超发。

先删最早完整历史轮次，再降低候选资料数；当前问题和最少一个完整来源仍放不下时返回context_too_large。不自动摘要历史。每run保存模型/输入来源/预算配置与可见性generation。生成前及完成前复核资料，来源中途删除则取消/标记答案过期，不继续流出新内容。

消息依赖=本轮检索来源∪实际带入历史消息的传递依赖。来源删除导致相关消息退出后续上下文，但历史页面仍展示快照并提示。来源恢复不自动恢复上下文资格。保存回答为笔记是显式创建独立内容，不随原来源删除，保留provenance；此独立笔记可参与RAG，界面明确其为用户保存的内容。这与历史消息自动排除不同，不能静默级联删用户笔记。

资料文本为不可信数据，提示词不执行其中指令；模型没有文件写入/执行工具。服务端生成引用ID白名单；未知引用标为无效，无法支持的回答显示依据不足，不把模型自造链接当证据。失败/取消保留partial标志，不能显示为完整成功回答。
