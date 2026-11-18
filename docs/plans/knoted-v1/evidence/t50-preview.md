# T50 阶段 A 预览业务证据

状态：**NEEDS_CONTEXT**。Issue [#135](https://github.com/xingjianxu/knoted/issues/135)；BASE `11b1f5a49e4fab9358eace67c92cb0ec89d92408`。
独立 FilePreview、输入守卫与生命周期；T47→T48→T49 完整运行链及阶段 B 页面/HTTPS 验收仍待，未部署、未合入。

- [x] 核对 ADR 0002/0006、T50、controller rulings、T49 A `876a9eb`。
- [x] 使用已隔离 `.worktrees/knoted-t50` / `feat/t50`；frozen install；基线 Web 248 项通过。
- [x] TDD 输入、资源与业务组件；定向 108/108。
- [x] Web typecheck/lint/test/build；全量 356/356。
- [ ] 主 Agent 独立审查、集成及阶段 B 真浏览器验收。

## 实际验证

工作目录为本任务 Worktree；日志根 `/data/knoted-dev/tasks/t50/evidence/`，每项有同名 `.log`、`.exit`、`.count`。

| 命令 | 证据前缀 | 退出码 | 结果 |
| --- | --- | --- | --- |
| `pnpm install --frozen-lockfile` | install | 0 | 既有锁安装，无依赖变更 |
| `pnpm --filter @knoted/web test`（基线） | baseline | 0 | 17 files / 248 tests |
| `pnpm --filter @knoted/web exec vitest run tests/tasks/t50.test.tsx` | targeted | 0 | 1 file / 108 tests |
| `pnpm --filter @knoted/web typecheck` | typecheck | 0 | 两个 TypeScript 项目 |
| `pnpm --filter @knoted/web lint` | lint | 0 | eslint src/tests/vite |
| `pnpm --filter @knoted/web test` | test | 0 | 18 files / 356 tests |
| `pnpm --filter @knoted/web build` | build | 0 | tsc + Vite |

测试输出保留既有 jsdom `Window's scrollTo()` 三条提示（#143）；构建保留既有大 chunk 警告。没有全局消音、没有 Rust/Python 改动或无关门禁。

TDD 原始记录：`red` 初轮 44 failed / 42 passed（86）；其中 40 项为缺失业务行为的有效失败，4 项（三真实图片成功样本及 t50_2）是 Vite fixture 路径错误，不计入业务 RED；42 个初轮通过项也不计 RED 证据。修正路径后 `green-second` 86/86；补测后 `expanded` 101/101。T46 ASCII/隐藏文件选择回归 `red-selection` 1 failed / 107 passed，`green-selection` 108/108。实施后对已修正 fixture 的三真实图片成功样本及 t50_2 做“回归敏感性探针”（stub 回退重放，不能补造原始先失败后实现历史）：`red-fixture-replay` 4 failed / 104 skipped（108），立即恢复实现后 `targeted` 108/108。初轮类型错误与测试 Blob/MediaSource 收窄错误保存在 `typecheck-initial`、`typecheck-first-full`、`build-first-full`，没有删去失败证据。

## 接口与资源

公开 `FilePreview({ api, file, open, onOpenChange, onDownload })`；file 直接使用生成 `components["schemas"]["FileSource"] | null`，api 为 ApiClient，下载回调 `(file: FileSource) => void | Promise<void>`。不引用未集成 T49 模块；不注册页面/路由，不改 OpenAPI/客户端。

通过已有 `GET /files/{fileId}/content`、`parseAs: "stream"` 取原件，沿用客户端同源凭据/401。metadata ETag 与 `file.version` 不符时提示刷新详情，不把元数据变化误称正文变化。实际逐块 cap 52,428,800 字节；metadata/合法 Content-Length 只作提前拒绝。超限取消 reader 并 abort，不先 blob/arrayBuffer；普通下载回调没有预览 cap。

PDF 使用固定 MIME Blob 和有 title 的 iframe；文本严格 UTF-8/UTF-16 BOM、单层 BOM、拒绝 UTF-32/坏字节/NUL，以 React 文本节点完整显示。图片仅静态 PNG/JPEG/WebP，完整容器 bounds、唯一尺寸/单 bitstream 与 40,000,000 像素前置检查。JPEG 遍历 SOS 后 entropy/marker，拒绝 late SOF/DNL；PNG/APNG、WebP 动画拒绝。三份真实图片 fixture 总计约 160 KiB；其余恶意容器为可复现的小型结构 fixture，大小边界为受控流，无 50 MiB 文件进 Git。

每个打开身份（id/content_version/version/sha/name/mime/size/status/path）以 React key 隔离，渲染时不会借用旧 state；每个 effect 有独立 owner/controller/URL。A→B→A、StrictMode、ignored abort、流中关闭、URL 创建后关闭、图片错误、卸载、401 均有消费测试；各轮仅撤销自己的 URL，不写缓存/storage。外部会话失效由既有认证路由卸载/父组件关闭；阶段 B 需接好该生命周期。

mtone 0.4.0 公开 Dialog/Image（preview=false）/Spinner/Alert/Button 可以组合；本轮真实组件消费测试通过，无通用缺口、无新增依赖。

## 未验收与限制

阶段 A 无页面 callsite，故 build 通过不能证明预览已进入生产包。PDF 签名不是净化；iframe load 不表示页面解析成功，损坏/密码保护/无 viewer 等仍需普通下载。图片头守卫不验证压缩像素/PNG CRC，也不承诺 codec 沙箱或峰值内存上限。文本完整 DOM/输入合并仍可消耗多于 50 MiB 内存。

T49 页面最小接入、真实鉴权 HTTPS 200/401、实际 CSP、PDF 多页/损坏/密码/无 viewer、图片真实解码、键盘 Esc/焦点与 PDF 聚焦后的关闭回焦，均留阶段 B；jsdom 不是这些验收。T47 退出补充 RFC 仍待独立人工确认，不以此分支解除阻塞。

详细交接 `/data/knoted-dev/tasks/t39-t50/reports/t50-implementation.md`；源文件摘要和算法见日志根 `source-manifest.json` / `sourcehash.txt`。主 Agent 负责独立审查和后续集成；本轮不 push/PR/merge/dev/清理。

## 阶段 B 恢复计划

以上 A 记录为历史，不改写其 TDD 偏差。T47/48/49 已完成实际链路；新 brief 授权恢复，旧 RFC/上游等待不再是技术阻塞。
B 起始 HEAD `6434f9a36f582ef9ec4aa0123e071dfc19ecdafd`，整卡 A+B review BASE `8ffb2404d0fe44446ebe4020ad65499f5946b2a8`。

- [x] 新 B FileDetail 消费测试取得有效 RED 9/9（缺少预览入口）；修复后新 B 9/9。
- [x] FileDetail→FilePreview 使用受控打开快照，保留 A guard，普通下载显式使用同一选中 DTO。
- [x] T50 117 + T49 24 共 141 通过，typecheck/lint 退出 0；已冻结交父完整 Web 门禁。
- [ ] 安全提交、追加报告；父独立审查/合并/dev/native PDF 与清理。

### 阶段 B 实施证据

FileDetail 新入口“预览文件”对 missing/当前忙状态禁用，打开时捕获当前 FileSource。开关仅修改预览状态，不替换备注/标签草稿；关闭按钮/Escape 回到仍存在、启用且 owner 未失效的入口。切换 FileDetail 或默认会话退出会卸载预览，原 A owner 清理保持不变。Dialog/Button 使用 mtone 0.4.0 公开组件与 Button ref，无通用组件扩展。

普通下载复用既有 bundle.api blob 路径，以回调给定的 snapshot id/version/name 读取、核验 ETag、命名；不复用转码文本/预览 Blob，不加 50 MiB 下载 cap。预览内下载错误传回 FilePreview 显示，既有详情错误处理继续保留。默认 Router→QueryClient→ApiClient 401 使用真实 loopback HTTP 测试，未修改全局认证或共享客户端。

日志根继续使用 `/data/knoted-dev/tasks/t50/evidence/`，每项有 `.log/.exit/.count`：

| 前缀 | 命令 | 退出与数量 |
| --- | --- | --- |
| b-red | `pnpm --filter @knoted/web exec vitest run tests/tasks/t50.test.tsx -t '^B '` | 1；9 failed/108 skipped，含标签 fixture querystring 错误导致的 5 个未捕获错误，不计业务 RED |
| b-product-red | 同上（修正测试 fixture 后、生产修改前） | 1；9 failed/108 skipped，全部为缺失“预览文件”入口，无 fixture 错误 |
| b-green-initial | 同上（接入后） | 0；9 passed/108 skipped |
| b-targeted | `pnpm --filter @knoted/web exec vitest run tests/tasks/t50.test.tsx tests/tasks/t49.test.tsx` | 0；2 files/141 tests |
| b-typecheck | `pnpm --filter @knoted/web typecheck` | 0 |
| b-lint | `pnpm --filter @knoted/web lint` | 0 |

初次 b-typecheck-initial 捕获两个测试 spy 的 this 类型缺失，补注释后 b-typecheck 通过；保留失败日志。历史 A 108 例/流程偏差未重做或改写。新增 snapshot 测试在打开后将详情元数据替换为新 version/name，仍要求下载打开时的四字节 UTF-16 原件及原名称；另一用例真实读取 52,428,801 字节普通下载并撤销 URL。字节由测试内存生成，不新增大 fixture。default preview401 和 loaded-detail401 用例验证安全 return-to、旧草稿/缓存/预览 URL 清除。T49 全 24 例保持通过；原有 jsdom scrollTo 输出保留。

冻结源摘要：`600bb0682073e1704a580429969944a4f1df25ad8359151f1110c1d59fd20b0f`，逐文件/算法在 `b-source-manifest.json`。仅 FileDetail.tsx 与 t50.test.tsx 产品/测试差异；A 四 source/三 fixture 原字节保留。父负责最终完整 Web 门禁、独立审查、PR #162、实际 HTTPS/native PDF 截图、主线/dev/清理；此处不提前填写通过。没有 Rust/Python/协议/依赖/全局认证改动、没有自行运行 dev 脚本。
