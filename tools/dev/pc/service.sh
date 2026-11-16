#!/usr/bin/env bash
# Knoted 测试实例的编排入口（由 process-compose 调用，人用 `pnpm pc …`）。
#
# 只负责测试应用：渲染运行配置、校验固定产物、准备目录、
# 执行迁移、运行 API；HTTPS 由宿主共享 Caddy 管理（ADR 0005）。
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

instance_root=${KNOTED_INSTANCE_ROOT:-/data/knoted-dev/service}
runtime_root=${KNOTED_RUNTIME_ROOT:-/data/knoted-dev/runtime/current}
api_port=${KNOTED_API_PORT:-15602}
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
  test -s "$runtime_root/share/knoted/web/index.html" || die "运行根缺少 share/knoted/web/index.html"
  test -n "$(find "$runtime_root/share/knoted/migrations" -maxdepth 1 -name '*.sql' -type f -print -quit 2>/dev/null)" ||
    die "运行根缺少 share/knoted/migrations/*.sql"
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
  # render_template <template> <output> <placeholder> <value> [<placeholder> <value> …]
  local template=$1 output=$2
  shift 2
  local content
  content=$(cat "$template")
  while [ $# -ge 2 ]; do
    content=${content//"$1"/$2}
    shift 2
  done
  if [[ $content =~ @[A-Z_]+@ ]]; then
    die "模板渲染后仍有未替换占位符：$template"
  fi
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
  for port in "$api_port"; do
    if ss -H -ltn "sport = :$port" | grep -q .; then
      die "端口 $port 已被占用；先停止占用者（本实例用 pnpm pc down）"
    fi
  done
}

command_prepare() {
  # 顺序：守卫 → 验产物 → 验依赖 → 建目录 → 渲染，失败时不留半个实例。
  guard_worktree
  local revision
  revision=$(verify_runtime)
  log "固定产物 revision=$revision"
  require_tools
  prepare_instance_root
  install -d -m 0700 "$config_dir" "$state_dir" \
    "$logs_dir" "$assets_dir" "$staging_dir" "$backups_dir"

  if [ -L "$config_file" ]; then
    log "替换旧的 Nix store 配置链接：$config_file"
  fi

  render_template "$script_dir/knoted.toml.tmpl" "$config_file" \
    @API_PORT@ "$api_port" \
    @DATABASE_URL@ "$database_url" \
    @DOMAIN@ "$domain" \
    @HTTPS_PORT@ "$https_port" \
    @LOG_LEVEL@ "$log_level"

  check_ports
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

command_api() {
  test -r "$config_file" || die "缺少运行配置 $config_file；先完成 prepare"
  exec "$runtime_root/bin/knoted-api" --config "$config_file"
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
prepare) command_prepare ;;
migrate) command_migrate ;;
api) command_api ;;
probe-api) command_probe_api ;;
paths) command_paths ;;
*)
  echo "usage: service.sh {prepare|migrate|api|probe-api|paths}" >&2
  exit 2
  ;;
esac
