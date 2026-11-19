# 09 全站搜索

实施 Issue [#259](https://github.com/xingjianxu/knoted/issues/259) · [验收与限制](evidence/03-12.md)

目标是让简单搜索在首屏留下结果空间，复杂搜索仍可按全部已有条件精确复现。路由为 `/search`，由 AdminLayout.Content 提供滚动区域，前置为 01-foundation 验收的最新正式版 mtone（当前仓库 0.6.0）。

## 页面审计与排布

SearchPage 把关键词和九个筛选／排序字段常驻在结果之前。窄屏逐列堆叠后，提交成功也难以看到首条命中。SearchResult 已能安全高亮摘要并链接五种实体，值得保留标题、摘要、来源路径的阅读顺序。

```text
搜索
[ 输入搜索词                         ]  搜索
[筛选条件 ▾]    已提交的条件摘要             排序 [相关性 ▾]
笔记   标题链接
       含命中词的摘要
       路径 · 更新时间 · 定位说明（适用时）
…                                          上一页  下一页
```

- 搜索词由 SearchInput 与同一 Form 的提交按钮承载；Enter 提交。保持 SearchInput 的清空功能只修改草稿。排序用 Select，沿用显式提交语义。已提交条件和仍待提交的修改在样式与可访问描述上清楚区分。
- 在表单内用带 aria-expanded 和 aria-controls 的 mtone Button 展开筛选区，折叠也保留草稿。按「对象」（类型、项目、标签、归档）、「更新时间」（起、止）、「文件位置」（路径、格式）排列 FormField。用 FilterBar 排布筛选入口、摘要与排序；多个标签过长时折行。类型、项目、归档的 Select 分别用公开 placeholder 显示「全部类型」「全部项目」「全部归档状态」，验证键盘可退回默认且请求省略相应条件。过滤值保留原有全实体语义。
- 结果用 List/ListItem。保持单一可打开的标题链接，类型文字靠近标题，摘要中的 mark 采用有对比度的主题色。笔记／文件仅能到详情时保留定位说明。聊天消息在 session 与 message 标识均有效时可直达，否则展示文本和原因。项目、事项直达各自详情。
- 窄屏保留「关键词→搜索→筛选→结果」顺序；折叠时首条结果可在 390×844 的标准夹具中直接看到。展开后字段一列，宽屏按组并列；日期和长路径可完整编辑。提交条件错误时展开所属区域并定位字段。

## 查询、状态与步骤

主要文件为 `apps/web/src/features/search/{SearchPage,SearchResult}.tsx`、`apps/web/tests/tasks/t55.test.tsx` 和 `tests/e2e/admin-layout.spec.ts`。对照 `features/search/queries.ts` 与 `apps/web/src/routes/_authenticated.search.tsx` 的校验和序列化。

- [ ] 以同时包含多标签、项目、false 归档值、时间起止、路径、格式、类型和排序的 URL 建立恢复用例。重新打开、刷新或浏览器前进后退都能复现提交结果；选项源读取失败时保留已选 ID，显示局部重试。
- [ ] 拆分关键词主行与可收起的字段组。保持草稿与已提交 URL 两套值，空词或反向日期范围在表单中提示；保留本地时间输入→UTC 查询时间的转换。空查询不请求结果，未搜索、搜索中、查询失败可重试、成功零匹配有各自状态。
- [ ] 组织五类命中与页尾游标，验证正则特殊字符、HTML 片段及长路径按文本显示。每次换条件或再次提交同一搜索均从第一页开始；每次最多 50 项，读取中禁翻页，末页或重复游标禁下一页。
- [ ] 在真实路由中由结果打开一个已缓存会话内的指定消息，验证其焦点和滚动位置。保留已有 E2E，增加筛选收放与窄屏首条命中的断言。390、768、1440 像素记录相同已提交条件的折叠、展开和一条带长摘要命中，记录空态及错误重试的测试输出。

```sh
pnpm --filter @knoted/web exec vitest run tests/tasks/t55.test.tsx
pnpm exec playwright test tests/e2e/admin-layout.spec.ts --project=chromium
pnpm --filter @knoted/web run typecheck
pnpm --filter @knoted/web run lint
pnpm --filter @knoted/web run build
```
