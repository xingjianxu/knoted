#!/usr/bin/env bash
# Knoted 测试实例的编排入口（由 process-compose 调用，人用 `pnpm pc …`）。
#
# 只负责测试应用：渲染运行配置、校验固定产物、准备目录、
# 执行迁移、运行 API 和 Worker；HTTPS 由宿主共享 Caddy 管理（ADR 0005）。
# 数据库由宿主用户直接创建（ADR 0004，本机用 trust 认证连接 `xingjian`），
# 编排只负责跑迁移，不建库、不建角色；Strata、共享模型根、ACME 证书与运行期工具
# 仍由 /etc/nixos 提供。
#
# 所有路径与端口可用环境变量覆盖，默认值即本机测试实例（隔离验证用同样的入口
# 指向临时根与临时端口）：
#   KNOTED_INSTANCE_ROOT  /data/knoted-dev/service
#   KNOTED_RUNTIME_ROOT   /data/knoted-dev/runtime/current（固定提交构建产物）
#   KNOTED_API_PORT       15602
#   KNOTED_HTTPS_PORT     15601
#   KNOTED_HTTPS_DOMAIN   wlab.132140.xyz
#   DATABASE_URL          postgres://xingjian@127.0.0.1:5432/knoted_dev
#   KNOTED_LOG_LEVEL      info
#   KNOTED_OFFICE_ORIGIN / KNOTED_OFFICE_JWT_SECRET_FILE  可选，必须同时提供；只记录凭据路径
#   KNOTED_VECTORS_ENABLED=1  启用固定模型向量发布
#   KNOTED_SEARCH_ENABLED=1 / KNOTED_SEARCH_CREDENTIAL_FILE  启用本机索引；可选 [secrets].meilisearch 凭据文件
#   KNOTED_CHAT_LOCAL_BASE_URL / KNOTED_CHAT_LOCAL_MODEL / KNOTED_CHAT_LOCAL_CREDENTIAL_FILE  本机模型，可选，必须同时提供
set -euo pipefail
# 与 tools/dev/build-artifact.sh 一致：清单排序必须固定字节序，否则 locale 会改变 sort 结果。
export LC_ALL=C
# bash >= 5.2 默认开启 patsub_replacement，会把替换串里的 & 当成"匹配到的文本"。
# 模板替换用的是 ${var//search/replace}，含 & 的 DATABASE_URL 会被破坏。
shopt -u patsub_replacement 2>/dev/null || true

# 运行期工具（sqlx/curl/iproute2）由 /etc/nixos 提供。它们不一定在调用者的 PATH 里
# （例如刚安装 boot 候选、尚未 switch 时），所以把这两个稳定 bin 目录**追加**到 PATH：
# 调用者自己的 PATH 仍优先（测试可以放 stub），缺工具时再从这里兜底。
# 两个来源都可用环境变量覆盖，测试据此把兜底关掉，保证同一套断言在切换前后都成立。
for tool_dir in "${KNOTED_SYSTEM_PROFILE_BIN:-/nix/var/nix/profiles/system/sw/bin}" "${KNOTED_STATIC_PATH:-/run/current-system/sw/bin}"; do
  [ -d "$tool_dir" ] || continue
  case ":$PATH:" in
  *":$tool_dir:"*) ;;
  *) PATH="$PATH:$tool_dir" ;;
  esac
done
export PATH

script_dir=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
source "$script_dir/runtime-env.sh"

instance_root=${KNOTED_INSTANCE_ROOT:-/data/knoted-dev/service}
runtime_root=${KNOTED_RUNTIME_ROOT:-/data/knoted-dev/runtime/current}
api_port=${KNOTED_API_PORT:-15602}
intelligence_port=${KNOTED_INTELLIGENCE_PORT:-15603}
https_port=${KNOTED_HTTPS_PORT:-15601}
domain=${KNOTED_HTTPS_DOMAIN:-wlab.132140.xyz}
# 开发/测试库由宿主用户直接创建与管理（ADR 0004）：沿用本机 trust 认证，
# 不再有专用 knoted_dev 角色，也没有 NixOS 侧的 provisioning 单元。
database_url=${DATABASE_URL:-postgres://xingjian@127.0.0.1:5432/knoted_dev}
log_level=${KNOTED_LOG_LEVEL:-info}

config_dir=$instance_root/config
state_dir=$instance_root/state
logs_dir=$instance_root/logs
assets_dir=$instance_root/assets
staging_dir=$instance_root/staging
backups_dir=$instance_root/backups

config_file=$config_dir/knoted.toml

die() {
  printf 'knoted-pc: %s\n' "$1" >&2
  exit 1
}

log() {
  printf 'knoted-pc: %s\n' "$1" >&2
}

# 运行期依赖一次性前置检查：缺工具时在写任何文件之前就失败，不留半个实例。
require_tools() {
  local missing=""
  command -v curl >/dev/null || missing="$missing curl"
  command -v ss >/dev/null || missing="$missing iproute2(ss)"
  test -z "$missing" || die "缺少运行期依赖：${missing# }（由 /etc/nixos 提供：curl / iproute2）"
}

# 稳定实例不得从任务工作树运行（AGENTS.md 规则 4/11）。
forbidden_path() {
  case "$1" in
  */.worktrees/*) return 0 ;;
  *) return 1 ;;
  esac
}

guard_worktree() {
  if forbidden_path "$instance_root" || forbidden_path "$(readlink -f "$runtime_root" 2>/dev/null || printf '%s' "$runtime_root")"; then
    die "实例根或运行根位于 .worktrees 任务工作区：$instance_root / $runtime_root（稳定实例必须用主检出 + /data 运行根）"
  fi
}

prepare_instance_root() {
  if [ -e "$instance_root" ]; then
    test -d "$instance_root" || die "实例路径不是目录：$instance_root"
    test -w "$instance_root" || die "实例根不可写：$instance_root（属主应为运行编排的用户）"
    return 0
  fi
  local parent
  parent=$(dirname "$instance_root")
  test -d "$parent" -a -w "$parent" || die "无法创建实例根：$parent 不存在或不可写"
  install -d -m 0700 "$instance_root"
}

# 与 nix/package.nix 相同的完整性规则：逐文件重算清单并逐字比对，
# 拒绝遗漏项、符号链接与特殊文件。
verify_runtime() {
  test -d "$runtime_root" || die "运行根不存在：$runtime_root"
  local revision
  revision=$(cat "$runtime_root/share/knoted/REVISION" 2>/dev/null || true)
  [[ $revision =~ ^[0-9a-f]{40}$ ]] || die "运行根缺少合法的 share/knoted/REVISION"
  test -x "$runtime_root/bin/knoted-api" || die "运行根缺少可执行 bin/knoted-api"
  test -x "$runtime_root/bin/knoted-worker" || die "运行根缺少可执行 bin/knoted-worker"
  test -s "$runtime_root/share/knoted/web/index.html" || die "运行根缺少 share/knoted/web/index.html"
  test -n "$(find "$runtime_root/share/knoted/migrations" -maxdepth 1 -name '*.sql' -type f -print -quit 2>/dev/null)" ||
    die "运行根缺少 share/knoted/migrations/*.sql"
  local python_root="$runtime_root/share/knoted/intelligence" required
  test -x "$runtime_root/bin/knoted-intelligence" || die "Python 产物缺少可执行入口"
  test -x "$python_root/python/bin/python3.12" || die "Python 产物缺少固定解释器"
  for required in BUILD.json requirements.txt uv.lock \
    python/lib/python3.12/os.py \
    python/lib/python3.12/site-packages/knoted_intelligence/app.py \
    python/lib/python3.12/site-packages/knoted_intelligence/guardian.py \
    python/lib/python3.12/site-packages/knoted_intelligence/supervision.py \
    python/lib/python3.12/site-packages/knoted_intelligence/parsing/pdf.py \
    python/lib/python3.12/site-packages/docling/document_converter.py \
    python/lib/python3.12/site-packages/torch/__init__.py \
    python/lib/python3.12/site-packages/onnxruntime/__init__.py; do
    test -s "$python_root/$required" || die "Python 产物缺少 $required"
  done
  local unexpected
  unexpected=$(cd "$runtime_root" && find bin share ! -type d ! -type f -print -quit)
  test -z "$unexpected" || die "运行根含非普通文件（拒绝符号链接与特殊文件）：$unexpected"
  local actual
  actual=$(cd "$runtime_root" && find bin share -type f -print0 | sort -z | xargs -0 sha256sum)
  printf '%s\n' "$actual" | cmp -s - "$runtime_root/SHA256SUMS" ||
    die "运行根文件摘要与 SHA256SUMS 不一致（产物被改动或清单不完整）"
  printf '%s' "$revision"
}

render_template() {
  # render_template <template> <placeholder> <value> [<placeholder> <value> …]
  local template=$1
  shift
  local content
  content=$(cat "$template") || return 1
  while [ $# -ge 2 ]; do
    content=${content//"$1"/$2}
    shift 2
  done
  if [[ $content =~ @[A-Z_]+@ ]]; then
    die "模板渲染后仍有未替换占位符：$template"
  fi
  printf '%s\n' "$content"
}

publish_config() {
  local output=$1 content=$2
  (
    umask 077
    local tmp
    tmp=$(mktemp "$(dirname "$output")/.render.XXXXXX")
    printf '%s\n' "$content" >"$tmp"
    chmod 0600 "$tmp"
    mv -f "$tmp" "$output"
  )
}

check_ports() {
  command -v ss >/dev/null || die "需要 iproute2 的 ss 检查端口占用"
  if [ "${KNOTED_ALLOW_BUSY_PORTS:-0}" = "1" ]; then
    log "跳过端口占用检查（KNOTED_ALLOW_BUSY_PORTS=1）"
    return 0
  fi
  local port
  for port in "$api_port" "$intelligence_port"; do
    if ss -H -ltn "sport = :$port" | grep -q .; then
      die "端口 $port 已被占用；先停止占用者（本实例用 pnpm pc down）"
    fi
  done
}

office_config_block() {
  local origin=${KNOTED_OFFICE_ORIGIN:-} path=${KNOTED_OFFICE_JWT_SECRET_FILE:-}
  if [ -z "$origin" ] && [ -z "$path" ]; then
    return 0
  fi
  local https_origin='^https://([a-zA-Z0-9.-]+|\[[a-fA-F0-9:.]+\])(:[0-9]{1,5})?$'
  # Keep interpolation TOML-safe; the API additionally validates the exact
  # canonical origin. Prepare never opens or copies the referenced key file.
  if [[ ! $origin =~ $https_origin || $path != /* || $path == *['"\']* || $path =~ [[:cntrl:]] ]]; then
    die "Office 需要一起配置 HTTPS origin 与绝对凭据文件路径"
  fi
  printf '\n[office]\ndocument_server_origin = "%s"\njwt_secret_file = "%s"\n' "$origin" "$path"
}

search_config_block() {
  local enabled=${KNOTED_SEARCH_ENABLED:-0} path=${KNOTED_SEARCH_CREDENTIAL_FILE:-}
  if [[ $enabled == 0 && -z $path ]]; then
    return 0
  fi
  [[ $enabled == 1 ]] || die "KNOTED_SEARCH_ENABLED 必须是 0 或 1，且凭据路径需要启用检索"
  if [[ -n $path && ( $path != /* || $path == *['"\']* || $path =~ [[:cntrl:]] ) ]]; then
    die "全文索引凭据需要安全的绝对文件路径"
  fi
  if [[ -n $path ]]; then
    printf '\n[credentials]\ncredential_file = "%s"\n' "$path"
  fi
  printf '\n[search]\nenabled = true\n'
  if [[ -n $path ]]; then
    printf 'api_key_credential = "meilisearch"\n'
  fi
}

chat_local_config_block() {
  local base_url=${KNOTED_CHAT_LOCAL_BASE_URL:-} model=${KNOTED_CHAT_LOCAL_MODEL:-}
  local path=${KNOTED_CHAT_LOCAL_CREDENTIAL_FILE:-}
  if [[ -z $base_url && -z $model && -z $path ]]; then
    return 0
  fi
  if [[ -z $base_url || -z $model || $path != /* ]]; then
    die "KNOTED_CHAT_LOCAL_BASE_URL、KNOTED_CHAT_LOCAL_MODEL 与绝对 KNOTED_CHAT_LOCAL_CREDENTIAL_FILE 必须一起提供"
  fi
  local value
  for value in "$base_url" "$model" "$path"; do
    if [[ $value == *['"\']* || $value =~ [[:cntrl:]] ]]; then
      die "KNOTED_CHAT_LOCAL 配置包含不安全的 TOML 字符"
    fi
  done

  # Use the WHATWG URL parsing already available in pcwrap's Node runtime.
  # Suppress parser diagnostics because a malformed URL could include userinfo.
  command -v node >/dev/null || die "KNOTED_CHAT_LOCAL 配置需要 node 校验回环 URL"
  if ! node - >/dev/null 2>&1 <<'JS'
const url = new URL(process.env.KNOTED_CHAT_LOCAL_BASE_URL);
const model = process.env.KNOTED_CHAT_LOCAL_MODEL;
const loopback = url.hostname === "[::1]" || /^127(?:\.[0-9]+){3}$/.test(url.hostname);
if (!loopback || !["http:", "https:"].includes(url.protocol) || url.username || url.password ||
    /[?#]/.test(url.href) || /\p{Cc}|^\p{White_Space}|\p{White_Space}$/u.test(model)) {
  process.exitCode = 1;
}
JS
  then
    die "KNOTED_CHAT_LOCAL_BASE_URL 必须是无凭据、查询和片段的 HTTP(S) 回环地址，KNOTED_CHAT_LOCAL_MODEL 不得含控制字符或首尾空白"
  fi

  if [[ ! -f $path || ! -r $path ]]; then
    die "KNOTED_CHAT_LOCAL_CREDENTIAL_FILE 指定的普通文件无法读取（Strata 请在已继承 strata-clients 组的会话中启动）"
  fi
  printf '\n[chat.local]\nbase_url = "%s"\nmodel = "%s"\ncredential_file = "%s"\n' "$base_url" "$model" "$path"
}

active_chat_config() {
  if [[ -e $config_file ]]; then
    awk '/^\[/ { chat = ($0 == "[chat.local]") } chat && NF { print }' "$config_file"
  fi
}

chat_config_digest() {
  local hash
  if [[ -z $1 ]]; then
    printf 'none\n'
  else
    hash=$(printf '%s\n' "$1" | sha256sum) || return 1
    printf '%s\n' "${hash%% *}"
  fi
}

# 0: 与已渲染配置相符；3: 需重新 prepare；其他非零：配置无效或无法检查。
# stdout 仅含预期配置摘要（不含凭据）。部署还会核对 API 和 Worker 启动时加载的摘要。
command_check_chat_config() {
  local desired active office search chat_local vectors=''
  resolve_config_blocks || return 1
  render_config >/dev/null || return 1
  desired=${chat_local#$'\n'}
  active=$(active_chat_config) || return 1
  chat_config_digest "$desired" || return 1
  [[ $active == "$desired" ]] || return 3
}

# 只逆向读取本脚本生成的配置项，避免把待部署的 deploy.env 当作已应用值。
command_active_runtime_env() {
  [[ -e $config_file ]] || return 0
  awk '
    function setting(key, value) {
      sub(/^[^"]*"/, "", value); sub(/"$/, "", value)
      print key "=" value
    }
    /^\[/ { section = $0 }
    section == "[office]" && /^document_server_origin = ".*"$/ { setting("KNOTED_OFFICE_ORIGIN", $0) }
    section == "[office]" && /^jwt_secret_file = ".*"$/ { setting("KNOTED_OFFICE_JWT_SECRET_FILE", $0) }
    section == "[credentials]" && /^credential_file = ".*"$/ { search_path = $0 }
    section == "[search]" && /^enabled = true$/ { search = 1 }
    section == "[vectors]" && /^enabled = true$/ { vectors = 1 }
    section == "[chat.local]" && /^base_url = ".*"$/ { setting("KNOTED_CHAT_LOCAL_BASE_URL", $0) }
    section == "[chat.local]" && /^model = ".*"$/ { setting("KNOTED_CHAT_LOCAL_MODEL", $0) }
    section == "[chat.local]" && /^credential_file = ".*"$/ { setting("KNOTED_CHAT_LOCAL_CREDENTIAL_FILE", $0) }
    END {
      print "KNOTED_SEARCH_ENABLED=" (search + 0)
      if (search && search_path) setting("KNOTED_SEARCH_CREDENTIAL_FILE", search_path)
      print "KNOTED_VECTORS_ENABLED=" (vectors + 0)
    }
  ' "$config_file"
}

# 准备和发布前检查用同一套验证；通过调用方的局部变量返回 TOML 片段。
resolve_config_blocks() {
  office=$(office_config_block) || return 1
  search=$(search_config_block) || return 1
  chat_local=$(chat_local_config_block) || return 1
  case "${KNOTED_VECTORS_ENABLED:-0}" in
    0) ;;
    1) vectors=$'\n[vectors]\nenabled = true' ;;
    *) die "KNOTED_VECTORS_ENABLED 必须是 0 或 1" ;;
  esac
}

render_config() {
  render_template "$script_dir/knoted.toml.tmpl" \
    @API_PORT@ "$api_port" \
    @INTELLIGENCE_PORT@ "$intelligence_port" \
    @DATABASE_URL@ "$database_url" \
    @DOMAIN@ "$domain" \
    @HTTPS_PORT@ "$https_port" \
    @LOG_LEVEL@ "$log_level" \
    @ASSETS_ROOT@ "$instance_root/assets" \
    @STAGING_ROOT@ "$instance_root/staging" \
    @OFFICE_CONFIG@ "$office" \
    @SEARCH_CONFIG@ "$search" \
    @VECTORS_CONFIG@ "$vectors" \
    @CHAT_LOCAL_CONFIG@ "$chat_local"
}

command_prepare() {
  guard_worktree
  local revision rendered office search chat_local vectors=''
  revision=$(verify_runtime)
  log "固定产物 revision=$revision"
  require_tools
  resolve_config_blocks
  rendered=$(render_config)
  check_ports
  prepare_instance_root
  install -d -m 0700 "$config_dir" "$state_dir" \
    "$logs_dir" "$assets_dir" "$staging_dir" "$backups_dir" \
    "$state_dir/intelligence" "$instance_root/cache/intelligence" "$instance_root/tmp/intelligence"
  if [ -L "$config_file" ]; then
    log "替换旧的 Nix store 配置链接：$config_file"
  fi
  publish_config "$config_file" "$rendered"
  log "prepare 完成：配置 $config_file；HTTPS 由宿主 Caddy 管理"
}

command_migrate() {
  command -v sqlx >/dev/null ||
    die "需要 sqlx-cli（由 /etc/nixos 提供：pkgs.sqlx-cli；不要把迁移塞进 API 启动）"
  log "迁移数据库（source=$runtime_root/share/knoted/migrations）"
  local status=0
  DATABASE_URL="$database_url" sqlx migrate run --source "$runtime_root/share/knoted/migrations" || status=$?
  if [ "$status" -ne 0 ]; then
    # 库不存在是最常见的一次性前置；库由宿主用户直接创建（ADR 0004），这里只给命令。
    log "迁移失败（exit $status）。开发/测试库由宿主用户创建，例如："
    log "  createdb -h 127.0.0.1 knoted_dev   # 以 $(id -un) 身份"
    exit "$status"
  fi
}

start_rust_service() {
  local block
  test -r "$config_file" || die "缺少运行配置 $config_file；先完成 prepare"
  block=$(active_chat_config) || die "无法读取已渲染的问答配置"
  KNOTED_DEV_CHAT_CONFIG_DIGEST=$(chat_config_digest "$block") || die "无法计算已渲染的问答配置摘要"
  export KNOTED_DEV_CHAT_CONFIG_DIGEST
  exec "$runtime_root/bin/$1" --config "$config_file"
}

command_intelligence() {
  guard_worktree
  test -r "$config_file" || die "缺少运行配置；先完成 prepare"
  test -d "${KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR:-}" || die "缺少 Docling 只读模型根配置"
  test -d "${KNOTED_INTELLIGENCE_BGE_M3_DIR:-}" || die "缺少 BGE-M3 只读模型根配置"
  # Whitelist the child environment: no PostgreSQL credentials or test hooks.
  exec env -i PATH="$PATH" HOME="$state_dir/intelligence" \
    NIX_LD="${NIX_LD:-}" NIX_LD_LIBRARY_PATH="${NIX_LD_LIBRARY_PATH:-}" \
    LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-}" \
    KNOTED_INTELLIGENCE_HOST=127.0.0.1 KNOTED_INTELLIGENCE_PORT="$intelligence_port" \
    KNOTED_INTELLIGENCE_STATE_DIR="$state_dir/intelligence" \
    KNOTED_INTELLIGENCE_STAGING_DIR="$staging_dir" \
    XDG_CACHE_HOME="$instance_root/cache/intelligence" TMPDIR="$instance_root/tmp/intelligence" \
    KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR="$KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR" \
    KNOTED_INTELLIGENCE_BGE_M3_DIR="$KNOTED_INTELLIGENCE_BGE_M3_DIR" \
    KNOTED_INTELLIGENCE_DOCLING_DEVICE=cpu KNOTED_INTELLIGENCE_GPU_SLOTS=1 \
    CUDA_VISIBLE_DEVICES= KNOTED_EMBED_GPU_UUID= \
    "$runtime_root/bin/knoted-intelligence"
}

command_probe_intelligence() {
  local response
  response=$(curl -fsS --max-time 3 "http://127.0.0.1:$intelligence_port/internal/v1/health") || return 1
  "$runtime_root/share/knoted/intelligence/python/bin/python3.12" -I -B -c \
    'import json,sys; data=json.loads(sys.argv[1]); sys.exit(0 if data.get("status")=="ok" else 1)' "$response" || \
    die "intelligence 未就绪（degraded 或无效响应）"
}

command_probe_api() {
  curl -fsS --max-time 3 "http://127.0.0.1:$api_port/health/ready" >/dev/null
}

command_paths() {
  printf 'instance_root=%s\n' "$instance_root"
  printf 'runtime_root=%s\n' "$runtime_root"
  printf 'config_file=%s\n' "$config_file"
  printf 'api_port=%s\n' "$api_port"
  printf 'https_port=%s\n' "$https_port"
  printf 'domain=%s\n' "$domain"
  printf 'database_url=%s\n' "$(printf '%s' "$database_url" | sed -E 's#(://)[^/@]*@#\1***@#')"
}

command=${1:-}
case "$command" in
prepare | intelligence | check-chat-config)
  load_deploy_env "$instance_root/deploy.env" || exit 1
  ;;
esac
case "$command" in
prepare) command_prepare ;;
check-chat-config) command_check_chat_config ;;
active-runtime-env) command_active_runtime_env ;;
migrate) command_migrate ;;
api) start_rust_service knoted-api ;;
worker) start_rust_service knoted-worker ;;
intelligence) command_intelligence ;;
probe-intelligence) command_probe_intelligence ;;
probe-api) command_probe_api ;;
paths) command_paths ;;
*)
  echo "usage: service.sh {prepare|check-chat-config|active-runtime-env|migrate|api|worker|intelligence|probe-api|probe-intelligence|paths}" >&2
  exit 2
  ;;
esac
