# 数据模型与API契约

## 通用类型

ID为UUID字符串；绝对时间UTC RFC3339。`DateValue = {date: YYYY-MM-DD, time: HH:mm|null, timezone: "Asia/Shanghai"}`，日期型不要转成UTC午夜；开始不得晚于截止。比较时将截止日期无time解释为本地下一日00:00（此时开始逾期）。无截止不算逾期。

`Page<T> = {items:T[], next_cursor:string|null}`，limit默认50、最大100。游标是base64url编码的版本化JSON，服务端完整验证类型、排序键及过滤摘要；不是授权凭据。业务列表采用 `(sort_value,id)` 键集分页；前端保留游标栈实现上一页。搜索使用引擎偏移封装的游标，固定上限10000条，超过时提示缩小范围；查询变化重置游标，不承诺稳定快照或精确总数。

`ApiError = {code:string,message:string,request_id:UUID,field_errors?:{field:string,message:string}[],current_version?:int}`。400语法错误、401未登录、403授权失败、404不存在、409版本/状态冲突、413超限、422业务字段错误、429准入受限、503依赖不可用。错误日志不含密钥、正文或授权临时URL。

浏览器同源 `/api/v1`。详情返回ETag与整数version；更新/删除/恢复请求body带 `expected_version`。SQL更新条件同时匹配ID/version/可写状态，零行区分404/409。创建消息、图片完成登记、任务提交使用 `Idempotency-Key`，按账号+操作+键存请求摘要和结果，7天过期；同键不同请求409。

## 版本划分

| 版本 | 定义 |
|---|---|
| content_version | 每次语义内容变化递增；用于CAS和检索新鲜度 |
| snapshot_id | 笔记历史快照ID；不与自动保存次数等同 |
| extraction_id | 来源content_version+parser_revision+options_hash |
| chunk_set_id | extraction_id+chunker_revision |
| embedding_set_id | chunk_set_id+model_revision+dimension |
| publication | 每个目标引擎独立发布，记录source/version/config/status |

生命周期变化也更新授权/可见性版本；正文未变时不必重新嵌入，但读取必须查当前deleted/archived状态。全文和向量分别可用，不让向量失败阻塞最新全文。

## 实体与约束

- users：唯一固定owner；passkeys(credential_id唯一, public_key, counter, transports)、sessions(token_hash,idle_expires,absolute_expires)、ceremonies/challenges、bootstrap_tokens。
- notes：id,parent_id,title,document JSONB,format_version=1,content_version,deleted_at。parent FK且服务端事务检测循环，删除父要求无未删除子。全局有序树使用同级position整数，移动由事务重排。
- note_snapshots：note_id、snapshot_id、content_version、document/title、reason、created_at。到期删除不删除当前内容。
- assets：id,storage_key,mime,size,sha256,state；asset_refs记录当前笔记/快照/回收站/转存笔记引用。零引用且过24小时才回收。
- projects：id,title,description,version,archived_at,deleted_at。项目归档连带归档本项目尚未归档的终态事项，记archive_batch_id；恢复仅恢复该批次。
- tasks：id,project_id?,parent_id?,title,description,status,priority,start?,due?,version,archived_at,deleted_at。parent只能指向顶层任务、不能自身；project必须继承parent。顶层迁移项目时子任务同事务迁移。
- activities/activity_revisions：人工备注和修订；系统状态事件append-only；删除人工备注保留标记，旧内容仅历史查看，不再检索。
- tags与notes/tags、files/tags、tasks/tags、projects/tags独立FK关联表。entity_links限定note/file/project/task类型，服务层校验两端，删除目标留失效信息。
- file_sources：id,relative_path,device,inode,size,mtime_ns,sha256,content_version,status。路径只是当前定位，不是永久身份。
- source_documents：统一可检索投影，映射来源ID/version/type/title/text/filter字段；提取和分块独立表。chat投影只全文。
- chat_sessions/messages/generation_runs/citations/message_source_dependencies：消息正文、执行状态、服务名、快照、传递来源依赖。
- jobs/outbox/publications：领取令牌、租约、输入版本、attempts、错误、每引擎发布状态。

JSONB只接受白名单块：heading(level1—3)、paragraph、bulletListItem、numberedListItem、checkListItem、image、codeBlock。inline允许text/link及bold/italic/underline/strike/code；拒绝可执行HTML及任意协议。块ID在一文档内唯一，最大嵌套8层，JSON上限2MiB、5000块；后端用保存格式fixture验证而非信任编辑器。图片只引用asset_id，传输时映射受鉴权URL。未知format_version返回422，不自动丢字段。

## HTTP操作清单

以下动作均需写入OpenAPI；ID路径与DTO由T03定义，任务不得另造同义接口。普通CRUD：POST集合、GET集合/详情、PATCH详情、DELETE详情（软删）；动作POST子路径。

| 根路径（均在/api/v1） | 操作与专属输入/输出 | 所有者 |
|---|---|---|
| /auth/bootstrap/options、/finish | 一次凭证→注册challenge；credential→session | T13/T14 |
| /auth/recovery/options、/finish | recovery_token→注册challenge；新credential+ceremony_id→撤销旧凭据/session并完成恢复 | T16 |
| /auth/login/options、/finish | challenge→认证结果 | T14 |
| /auth/session、/auth/logout、/auth/sessions | 当前/退出/列出；DELETE /sessions/{id}撤销 | T15 |
| /auth/passkeys | GET；POST /options、/finish；DELETE /{id} | T16 |
| /notes | title,parent_id?,document；返回Note | T23 |
| /notes/{id}/move | parent_id?,position,expected_version | T24 |
| /notes/{id}/restore | expected_version, restore_parent?: {parent_id: UUID或null}；省略保留原父，显式null移根；父缺失时409 | T24 |
| /notes/{id}/snapshots | GET；POST显式flush；POST /{snapshot}/restore | T25 |
| /assets | POST multipart，返回asset_id；GET /{id}/content | T26 |
| /tags、/links | 标签CRUD、关联POST/DELETE；link两端严格枚举 | T32 |
| /projects | CRUD；POST /{id}/archive、/restore、/unarchive | T33 |
| /tasks | CRUD；PATCH含project,parent,status,priority,date | T34/T35 |
| /tasks/{id}/activities | 人工POST/PATCH/DELETE；GET修订 | T36 |
| /tasks/{id}/archive、/unarchive、/restore | expected_version | T37 |
| /dashboard、/trash | 聚合GET；trash只当前账号对象 | T41/T42 |
| /files、/files/{id} | 浏览与详情；GET /content；PATCH仅备注/标签 | T49 |
| /files/{id}/relink | new_source_id,expected_version，确认歧义 | T45 |
| /files/{id}/office-preview | POST产生短时只读配置；DELETE撤销 | T51 |
| /search | q,type,tags,project,updated_from/to,archived,path,format,sort,cursor | T54 |
| /jobs | GET；POST /{id}/retry、/cancel；POST /scan触发扫描 | T56 |
| /chat/sessions | CRUD；provider=local或external配置名 | T62 |
| /chat/sessions/{id}/messages | POST问题和范围；返回message/run ID | T62 |
| /chat/runs/{id}/events | POST流式SSE；GET run详情；POST /cancel、/retry | T62 |
| /chat/messages/{id}/citations | GET固定快照与当前来源状态 | T64 |
| /chat/messages/{id}/save-note | POST title,parent_id?；返回note_id | T66 |

SSE事件为queued、started、delta、citation、completed、failed、cancelled；每条含run_id及递增seq。最终消息持久化后才completed。断连客户端GET查询状态，不自动重发生成；部分正文标记不完整。生成取消由显式API处理，断连不隐式增加任务。

## 业务操作和测试定位

core的公开服务函数统一接受 `&AppContext` 与命名输入结构，返回 `Result<Output,DomainError>`。例如 `notes::service::update(ctx, UpdateNote{id,expected_version,title,document}) -> Note`；HTTP处理器只转换。任务卡的“产出”定义该任务公开名称；SQL在repository，事务由service持有。schema迁移使用 `migrations/NNNN_tXX_<purpose>.sql`，编号由主Agent串行分配，禁止改已应用迁移。

## 登录细则

使用webauthn-rs完成密码学验证，不自行实现。注册/登录challenge有效5分钟、单次消费；CLI bootstrap/recovery有效10分钟、单次消费、数据库存摘要。唯一账号无公开注册。CLI签发token带purpose=bootstrap或recovery；两种token不能互换。recovery/options和finish不要求已有session，但必须验证未过期recovery token及绑定的challenge。options只建立5分钟ceremony，finish事务单次消费token，保存新凭据后再撤销旧凭据/session；失败不得让用户失去原凭据。恢复成功建立新session。RP ID=wlab.132140.xyz；origin精确为HTTPS15601，端口属于origin而不是RP ID。服务端session令牌随机256bit且只存摘要；Secure/HttpOnly/SameSite=Lax；写请求检查Origin及CSRF token。闲置7天、绝对30天，敏感凭据操作要求5分钟内Passkey验证。普通删除最后一个Passkey拒绝；恢复允许替换并撤销旧凭据及会话。

## 历史和备份

当前正文每次保存更新；历史每5分钟有修改生成，离开flush。后台补齐未生成快照的当前版本；不承诺恢复每次瞬间编辑。恢复旧版本先保存当前版本再写新版本。30天回收站期间保留原有关联；永久清理仍保留聊天引用快照，但删除凭据和应用数据遵守相应生命周期。备份包含快照、引用依赖、资产引用，不包含临时授权token的可恢复有效性，恢复后全部撤销。
