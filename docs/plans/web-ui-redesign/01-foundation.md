# 01 · 工作区外壳、页面节奏与系统反馈

**领取条件** 首卡，由一名 Web Agent 负责共享样式与路由壳。输入为总方案、既有 [AdminLayout 设计](../2026-11-19-mtone-admin-layout-design.md)、[UI 指南](../../development/ui-mtone.md) 与 [ADR 0002](../../decisions/0002-mtone-ui-ownership.md)。产物是后续各卡可直接消费的应用布局及对照样例。

**实施记录** [#249](https://github.com/xingjianxu/knoted/issues/249) · [浏览器对照和验证](evidence/01-foundation.md)。

## 现状与拟定布局

`apps/web/src/routes/_authenticated.tsx` 已使用 fullHeight、可折叠 Sider、Header 和独立滚动的 Content。顶栏只有退出按钮；页内标题从裸 h1 到 text-2xl 不等。庞大的详情表单与简短的入口页目前共享相同的内容边距。根错误与 404 均有独立 main。

| 部位 | 桌面 | 窄屏与键盘 |
| --- | --- | --- |
| Sider | 保留品牌、工作空间和账号分组；选中态落在父模块，含详情及索引任务页 | 沿用 768px 原生折叠；逐个图标验证名称、当前项、焦点和长导航可达性 |
| Header | 左侧在项目/事项详情提供一次返回所属清单的入口。笔记、文件、索引任务和问答各自负责模块内导航，右侧保留退出；首页与一级页避免再印一次页标题 | 名称可收缩，退出始终可触达；不将导航替换成第二套控件 |
| Content | 标题 24–28px、区标题 18–20px、正文 14–16px，数字或路径用等宽仅限有比对价值处。列表页可用宽度约 1120px；正文阅读约 68 个西文字符宽；看板与编辑区可用满宽 | 以扣除图标栏的实际内容宽度决定换行。外缘从约 12px 起，宽屏 24px；段间 24px、相关字段 8–12px；标题+动作可换行 |
| 系统反馈 | 根路由错误用 Alert 与重试；404 用 Result 或标题配真实首页链接；内容级错误留在出错区域 | 页首可被读屏识别。重试失效期间给反馈；长错误可换行，不把焦点丢到 body |

## 实施步骤

1. 在独立 Worktree 执行 `npm view @bixlab/mtone dist-tags.latest`，比对 manifest、pnpm-lock 与发行包 dist/index.d.ts。本次基准 0.6.0。若 latest 有变，由本卡独占版本迁移，记录 peer、CSS 和关键交互差异并精确锁版，再释放下游卡。
2. 调整 `src/lib/ui-provider.tsx`、`src/styles.css`（路径均相对 apps/web）与外壳。统一沿用暖白画布、白色操作面、深色正文、青绿链接及主操作，错误与警告用 mtone 语义色和文字。复用公开 tokens 使 portal 同步。用共同的布局约定控制宽度与间距，可直接使用语义标签及布局类。
3. 在 `src/routes/__root.tsx` 对齐全页反馈，并在 `_authenticated.tsx` 检查嵌套滚动、当前路由、高度和导航重置。保留 AuthGate、会话清理、笔记离开拦截。业务页 Agent 独占自己的标题与行为。
4. 用首页、长回收站、带消息锚点的问答各验一遍内容宽度、导航后滚动及前进后退。记录窄屏折叠、桌面展开、404、加载失败的前后对照。

## 验收

- 1440、768、390 与 320 CSS px 下，退出按钮、导航、页面主要操作均可达。缩放至 200% 后读完长标题，外壳无水平滚动，局部宽内容单独处理。
- 键盘进入内容、打开浮层再关闭，焦点可见并返回触发点。模拟笔记保存失败后退出，仍可继续处理草稿。认证页保持公开入口。
- 运行 `pnpm exec playwright test tests/e2e/admin-layout.spec.ts`、`pnpm --filter @knoted/web exec vitest run tests/tasks/t17.test.tsx tests/tasks/t18.test.tsx tests/tasks/t20.test.tsx`，并执行总方案的合并检查。PR 附实测视口、公开组件与版本、前后截图和重试/返回步骤。
