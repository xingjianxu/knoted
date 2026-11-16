# 验收矩阵与执行命令

## 本次文档验收

只检查Markdown本地链接、任务编号及依赖DAG、字段完整性、需求映射、明确禁止项、修改范围及git diff --check。独立审阅核对契约和任务可执行性。没有运行产品测试、GPU推理或部署，不将计划中命令写成已通过。

## 后续测试入口契约

T01/T02/T09建立以下命令；建立前不得声称可执行。每个任务卡的TEST命令是其目标入口，测试函数名统一含小写任务ID，避免过滤到零测试而误报通过。

- Rust：`cargo test --workspace tXX_`，按任务实际ID小写替换；输出必须至少1测试通过。最终 `cargo fmt --check`、`cargo test --workspace --locked`、`cargo clippy --workspace --all-targets -- -D warnings`。
- Web/UI：`pnpm exec vitest run tests/tasks/tXX.test.tsx`（apps/web；T17—T19 也在此目录，不再在 packages/ui）；浏览器 `pnpm exec playwright test tests/e2e/tXX.spec.ts`（仓库根）。任务明确哪一入口，不把虚拟WebAuthn当真实设备验收。
- Python：在services/intelligence运行 `uv run pytest tests/test_tXX.py -q`；静态 `uv run ruff check .`。
- 契约：T03提供 `pnpm api:check`，验证OpenAPI及样例；T04提供 `pnpm api:generate` 与 `pnpm api:verify`（生成后要求 `git status --porcelain -- packages/api-client/src/generated` 为空，既检查过期内容也检查新增未跟踪文件，必须从干净工作区运行）。
- 部署：host子任务在独立主机工作区执行nix eval/build，测试服务健康和定向功能；实际命令与固定ref写入该卡证据。

根pnpm脚本聚合typecheck/lint/test/build；T02建立Vitest、Testing Library、Playwright基础，Python T09建立pytest入口。集成检查按受影响范围运行，不重复运行无关测试。

## mtone 消费验收

依据 [UI 指南](../../development/ui-mtone.md)。通用组件实现及其内部单测归 mtone；Knoted 验证真实发布包在本应用的消费行为，不复制上游组件测试来冒充业务集成。T17 核查实际锁版、React peers、CSS/Provider、应用主题与无远程字体；T18 覆盖 Portal、键盘和焦点返回；T19 覆盖树展开/选择、游标上下页、空/错/加载状态和危险操作取消。Vitest 检查接入逻辑，真实浏览器检查实际焦点/样式，不能以纯结构检查替代。

T27—T29 继续验证中文输入法、选区、格式往返及菜单单次事件，UI 改为组合 mtone 不降低业务验收。Pagination 不得伪造 total；没有发布或没有公开 API 的能力按技术依赖记录，发布后锁版并复跑消费测试。上游需求已交接（提示词 + 上游 Issue）不等于卡完成；正常 PR/主线/适用部署门槛保留。

文档阶段检查任务 DAG、R25 映射、历史证据、Skill 分类及输出契约；不填 T17—T19 功能通过。Skill 场景模拟与 npm 运行验收分开报告。

## 高风险验收

| 场景 | 必须断言 | 所有者 |
|---|---|---|
| Passkey重放/错误origin | 拒绝，不产生session | T14 |
| 恢复/会话过期 | 所有旧session失效，保留唯一用户 | T15/T16 |
| 自动保存乱序/冲突 | 服务器较新版本不覆盖，本地dirty草稿保留 | T23/T30 |
| 异常关闭 | 最新正文可读，Worker补历史，不声称捕获每次编辑 | T25 |
| 树移动成环/父删除 | 拒绝，原结构不变 | T24 |
| 子任务完成条件 | 非终态子任务阻止父完成 | T35 |
| 到期日跨午夜 | 上海时区下一日00:00开始逾期 | T35 |
| 归档后编辑/恢复 | 写入拒绝；恢复仅本批次 | T33/T37 |
| 历史图片仍被引用 | 回收不删除资产 | T26/T67 |
| 根失联/扫描中断 | 不发批量删除、不标全扫成功 | T43 |
| symlink/FIFO/越界 | 拒绝读取，不阻塞、不打开外部文件 | T43/T44 |
| 复制时更新/旧结果迟到 | 丢弃，不覆盖当前来源版本 | T44 |
| 相同hash不同文件 | 不合并来源或关联 | T45 |
| Python超时/重启/取消 | 确认旧执行退出后才释放GPU槽 | T09/T47 |
| Worker租约转移 | 旧token提交失败 | T07/T08 |
| Meili接收但未应用 | 不标发布成功 | T53 |
| 全文好向量坏 | 最新全文可用、向量显示未就绪 | T59/T60 |
| 删除后迟到索引 | 读时PG复核拒绝 | T54/T59 |
| 中文词语位于长句中 | 能命中并显示原文片段 | T53/T55 |
| 聊天含关键词 | 全文可搜但RAG不召回聊天 | T60 |
| 历史依赖来源已删 | 直接/间接相关消息退出prompt，历史快照可查看 | T63 |
| 删除来源后独立转存笔记 | 笔记保留且有provenance，不暗中删除 | T66 |
| 取消/重复提交/断流 | 状态与正文一致，不重复生成 | T62/T65 |
| 模型伪造引用 | 标无效，不显示为真实证据 | T64 |
| Office授权过期/源变化 | 不能获取新源或扩大目录访问 | T51/T52 |
| 恢复缺图/摘要错 | 恢复失败，不切换运行实例 | T69 |

## 模型与OCR小样本

仓库只保留人工构造、无隐私的固定样本：中英文混排PDF、文字型PDF、扫描图片、基本表格DOCX/XLSX、两页PPTX。每份提供预期关键词、结构和定位；不要求像素级还原或逐字OCR完美。检测错误须可定位，人工确认中英关键内容可检索。模型嵌入测试检查形状/有限数值/同一模型版本；RAG用mock验证编排并用少量实模型样例验证接口，不做排名基准或性能测量。

## 交付证据

任务报告必须写命令、退出码、测试数量、样例路径、commit、运行SHA、未验证项及worktree状态。无测试不能报告0失败等于通过。生产、远程PR/CI、真实性能均不在本次授权内；未执行必须明确写未执行。
