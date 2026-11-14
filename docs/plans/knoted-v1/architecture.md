# 架构与目录职责

## 现状、备选和选择

基线为master提交 `6c00202c1a389a4e0e0ae797744c2159654c761d`，只有Rust 2024 Hello World。保留现状不能满足需求；每业务独立服务/独立crate增加跨模块集成成本。用户选择少量crate、内部按业务模块组织，API与Worker分进程，同仓库React与Python服务。不是微服务拆分，也不建立Android占位。

```text
Cargo.toml                       Rust workspace
crates/knoted-core/src/           auth notes projects tasks files assets search rag jobs db
crates/knoted-api/src/            routes middleware main.rs
crates/knoted-worker/src/         handlers main.rs
apps/web/src/routes/              TanStack文件路由
apps/web/src/features/            auth dashboard notes projects tasks files search chat trash
apps/web/src/lib/                 路由/请求/时间等应用适配
packages/ui/src/                 自有组件，禁止业务依赖
packages/api-client/src/          OpenAPI生成类型及统一请求
services/intelligence/src/knoted_intelligence/   parsing embedding jobs
api/openapi.yaml                 浏览器契约
api/intelligence.openapi.yaml    Rust/Python契约
migrations/                      按任务编号分配迁移
tests/fixtures/                  小型可公开固定样本
tests/e2e/                       浏览器场景
nix/                             项目打包和模块
tools/                           开发与部署入口
```

## 依赖方向

网页→生成客户端→API→core→Postgres/索引适配器；Worker→core→Python/Meilisearch；Python只做受控计算，禁止直接写业务数据库。API不得从Worker导入实现。core可用SQLx和外部适配器，但不得依赖Axum请求/响应。每业务模块使用 `model.rs/service.rs/repository.rs` 起步；只有职责实质变复杂才继续拆分，不建立空抽象。

API与Worker仅持有各自需要的服务权限。共用配置类型，按进程验证必需字段。Python服务必须经内部协议访问，不能公开任意路径解析接口。Strata独立安装，不复制上游源码进项目。OnlyOffice复用本机服务，但文件授权由Knoted独立实现。

## UI边界

Tailwind统一设计变量；Base UI只提供无样式行为。packages/ui维护Button、Input、FormField、Dialog、Menu、Popover、Toast、Tree、List及空态，业务不得绕过公共组件另造基础控件。BlockNote仅保留编辑核心，自建工具栏、斜杠菜单、块菜单、链接/图片交互；默认工具栏、菜单与文件面板关闭。不安装Mantine/shadcn样式体系。

路由负责URL及鉴权跳转，TanStack Query是服务端数据缓存唯一所有者；路由loader只预取同一query定义。编辑草稿归编辑器本地状态，查询重新验证不得覆盖dirty草稿。写后按实体ID与列表key失效；退出登录清除所有缓存。表单React Hook Form+Zod；后端再次验证。

## 技术来源与锁定

- [Base UI](https://base-ui.com/react/overview/quick-start)：无样式组件。
- [BlockNote自定义UI](https://www.blocknotejs.org/examples/ui-components/custom-ui)：关闭默认UI并替换。
- [OpenAPI TypeScript](https://openapi-ts.dev/readme)：生成类型及fetch调用。
- [Docling RapidOCR示例](https://docling-project.github.io/docling/_generated/examples/rapidocr_with_custom_models/)：显式模型路径。
- [BGE-M3](https://huggingface.co/BAAI/bge-m3)：1024维稠密输出。
- [Strata模型说明](https://github.com/Niko1221/Strata/blob/main/docs/MODELS.md)：IQ3_S，具体能力须按锁定revision验证。

T01/T02/T10—T12记录工具、包、模型commit及下载摘要，提交lock文件，不使用浮动latest。GPU可运行性任务验证接口及小样本，不跑速度或容量基准。无法在NixOS锁定组合上运行时报告证据，不静默换选型。

## 跨仓库

产品任务只写Knoted；/etc/nixos服务安装作为独立host任务，读取该仓库规则，不能将其嵌套进Knoted worktree。具体交接、提交顺序见[部署](development-and-deploy.md)。未来模块边界决策执行前检查ADR要求，不把本目录当作正式Accepted ADR索引。
