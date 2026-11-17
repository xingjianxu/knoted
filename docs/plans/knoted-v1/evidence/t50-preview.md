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
