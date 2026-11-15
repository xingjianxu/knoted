# Knoted 的本地模型 provider：**客户端契约**，不是部署。
#
# Strata 现在是一台主机上的系统级共享服务，由 host 侧声明
# （/etc/nixos/features/server/strata.nix）：
#   * strata-api-key.service（oneshot）→ /run/strata/api-key、/run/strata/auth-header；
#   * strata.service（User=strata）   → 127.0.0.1:15604，客户端组 strata-clients。
#
# 本文件只导出 Knoted 必须与 host 完全一致的常量（端点、模型别名、上下文窗口）、客户端
# 需要读取的凭据路径、以及 readiness/日志检查要用的单元名。它**不声明 unit、不建账号、
# 不写 tmpfiles、不做 preflight**：host 的 strata.service 是唯一承载，所以即使 host
# import 本模块，也不会再出现两个 `strata serve` 抢 127.0.0.1:15604 —— 这是 T12 证据
# §9 偏差 1/2/8 的裁决结果（见 evidence/t12-deployment.md §11）。
#
# 这些是双边契约：改端口、别名、上下文或凭据路径必须同时改 host 模块，两边 preflight /
# 客户端会各按一侧的值校验。
#
# 本机现状（2026-11-15）：/etc/nixos 的 strata 模块已改成共享服务，尚未 switch 激活；
# 激活前的运行期路径仍是 /run/knoted/strata-api-key（host 侧保留过渡软链）。
{ config, lib, ... }:

let
  cfg = config.knoted.strata;
in
{
  options.knoted.strata = {
    listen = {
      address = lib.mkOption {
        type = lib.types.str;
        default = "127.0.0.1";
        readOnly = true;
        description = ''
          服务只监听回环：密钥按组分发，暴露到 LAN 等于把模型交给任何能连上该接口的人。
          host 模块有同样的断言（loopback-only），两边都不接受其它值。
        '';
      };
      port = lib.mkOption {
        type = lib.types.port;
        default = 15604;
        readOnly = true;
        description = "共享服务端口（= /etc/nixos features.server.strata.port 的默认值）。";
      };
    };

    model = {
      alias = lib.mkOption {
        type = lib.types.str;
        default = "qwen3.8-flash-next-iq3_s";
        readOnly = true;
        description = ''
          /v1/models 上报、响应 `model` 字段回显的模型 id；host 侧 run config 的
          `model_name` 必须等于它（host preflight 校验）。**唯一权威**：T61 不得在应用
          配置里另定义一个默认别名。
        '';
      };
    };

    contextSize = lib.mkOption {
      type = lib.types.ints.positive;
      default = 32768;
      readOnly = true;
      description = ''
        上下文窗口（token），= host 侧 `--max-context`。Knoted 的提示词预算（系统+问题+
        引用+历史）必须留出输出余量后落在这个窗口内，见 indexing-and-rag.md。
      '';
    };

    clientGroup = lib.mkOption {
      type = lib.types.str;
      default = "strata-clients";
      readOnly = true;
      description = ''
        host 上可读运行期密钥的组（= features.server.strata.clientGroup）。Knoted 的
        API/Worker 服务账号必须在该组里，否则读 credentialFile 会失败；组成员变更对已运行
        进程不生效，要重启服务。
      '';
    };

    credentialFile = lib.mkOption {
      type = lib.types.str;
      default = "/run/strata/api-key";
      readOnly = true;
      description = ''
        运行期明文密钥（root:${cfg.clientGroup} 0440，每次开机由 strata-api-key.service
        从持久副本重建）。客户端读取它，不要复制到别处，也不要写进配置、日志或 argv。
      '';
    };

    authHeaderFile = lib.mkOption {
      type = lib.types.str;
      default = "/run/strata/auth-header";
      readOnly = true;
      description = ''
        验证便利文件（含 `Authorization: Bearer <key>`，0440 同组），供 `curl -H @<file>`
        使用；与 credentialFile 同一暴露面，只是为了不让密钥进 argv 或 shell 历史。
      '';
    };

    authScheme = lib.mkOption {
      type = lib.types.str;
      default = "Bearer";
      readOnly = true;
      description = "认证方案：`Authorization: Bearer <key>`（上游也接受 `x-api-key`，Knoted 只固定这一种）。";
    };

    serviceUnit = lib.mkOption {
      type = lib.types.str;
      default = "strata.service";
      readOnly = true;
      description = ''
        host 上的承载单元名。readiness/日志/停机语义都按这个单元判断（T12 证据 §9 偏差 2
        的裁决结果：以 host 单元名为准，不再写 `knoted-strata.service`）。
      '';
    };

    credentialUnit = lib.mkOption {
      type = lib.types.str;
      default = "strata-api-key.service";
      readOnly = true;
      description = "重建运行期密钥的 oneshot 单元名（开机与轮换后重启它即可刷新 credentialFile）。";
    };

    endpoints = {
      health = lib.mkOption {
        type = lib.types.str;
        default = "/health";
        readOnly = true;
        description = "无需密钥的健康检查路径，host 的 ExecStartPost 也轮询它。";
      };
      models = lib.mkOption {
        type = lib.types.str;
        default = "/v1/models";
        readOnly = true;
        description = "模型清单（`id` 必须等于 model.alias，`meta.n_ctx` 等于 contextSize）。";
      };
      chatCompletions = lib.mkOption {
        type = lib.types.str;
        default = "/v1/chat/completions";
        readOnly = true;
        description = "OpenAI 兼容补全端点（stream:true 时是 SSE）。";
      };
      status = lib.mkOption {
        type = lib.types.str;
        default = "/v1/status";
        readOnly = true;
        description = "运行状态（并发、上下文、在途请求），排障与并发假设以此为准。";
      };
    };

    provider = {
      baseUrl = lib.mkOption {
        type = lib.types.str;
        default = "http://${cfg.listen.address}:${toString cfg.listen.port}";
        readOnly = true;
        description = "OpenAI 兼容 base URL（配置里不带 /v1 后缀，客户端自己拼 endpoints.*）。";
      };
      model = lib.mkOption {
        type = lib.types.str;
        default = cfg.model.alias;
        readOnly = true;
        description = "请求体里的 `model` 字段值，与 model.alias 相同。";
      };
      credentialFile = lib.mkOption {
        type = lib.types.str;
        default = cfg.credentialFile;
        readOnly = true;
        description = "客户端运行时读取的凭据文件（与 credentialFile 同一个值）。";
      };
    };
  };

  # 纯契约：故意没有任何 config 段 —— 部署、账号、密钥、preflight 全在 host 侧模块里。
}
