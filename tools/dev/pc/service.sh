#!/usr/bin/env bash
# Knoted 测试实例的编排入口（由 process-compose 调用，人用 `pnpm pc …`）。
#
# 只负责测试应用：渲染运行配置与 Caddyfile、校验固定产物、准备目录、
# 执行迁移、运行 API 与独立 HTTPS 入口。PostgreSQL 角色/库、Strata、共享模型根、
# ACME 证书都由 /etc/nixos 管理，这里只消费。
#
# 所有路径与端口可用环境变量覆盖，默认值即本机测试实例（隔离验证用同样的入口
# 指向临时根与临时端口）：
#   KNOTED_INSTANCE_ROOT  /data/knoted-dev/service
#   KNOTED_RUNTIME_ROOT   /data/knoted-dev/runtime/current（固定提交构建产物）
#   KNOTED_API_PORT       15602
#   KNOTED_HTTPS_PORT     15601
#   KNOTED_HTTPS_DOMAIN   wlab.132140.xyz
#   KNOTED_CERT_DIR       /var/lib/acme/<domain>
#   DATABASE_URL          postgres://xingjian@127.0.0.1:5432/knoted_dev
#   KNOTED_LOG_LEVEL      info
set -euo pipefail
# 与 tools/dev/build-artifact.sh 一致：清单排序必须固定字节序，否则 locale 会改变 sort 结果。
export LC_ALL=C
# bash >= 5.2 默认开启 patsub_replacement，会把替换串里的 & 当成"匹配到的文本"。
# 模板替换用的是 ${var//search/replace}，含 & 的 DATABASE_URL 会被破坏。
shopt -u patsub_replacement 2>/dev/null || true

# 运行期工具（caddy/sqlx/openssl）由 /etc/nixos 提供。它们不一定在调用者的 PATH 里
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
cert_dir=${KNOTED_CERT_DIR:-/var/lib/acme/$domain}
cert_file=$cert_dir/fullchain.pem
key_file=$cert_dir/key.pem
# 开发/测试库由宿主用户直接创建与管理（ADR 0004）：沿用本机 trust 认证，
# 不再有专用 knoted_dev 角色，也没有 NixOS 侧的 provisioning 单元。
database_url=${DATABASE_URL:-postgres://xingjian@127.0.0.1:5432/knoted_dev}
log_level=${KNOTED_LOG_LEVEL:-info}

config_dir=$instance_root/config
state_dir=$instance_root/state
bin_dir=$instance_root/bin
logs_dir=$instance_root/logs
assets_dir=$instance_root/assets
staging_dir=$instance_root/staging
backups_dir=$instance_root/backups
admin_socket=$state_dir/https/admin.sock

config_file=$config_dir/knoted.toml
caddy_file=$config_dir/Caddyfile

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
  command -v openssl >/dev/null || missing="$missing openssl"
  command -v caddy >/dev/null || missing="$missing caddy"
  command -v curl >/dev/null || missing="$missing curl"
  command -v ss >/dev/null || missing="$missing iproute2(ss)"
  test -z "$missing" || die "缺少运行期依赖：${missing# }（由 /etc/nixos 提供：openssl / caddy / curl / iproute2）"
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
  for port in "$api_port" "$https_port"; do
    if ss -H -ltn "sport = :$port" | grep -q .; then
      die "端口 $port 已被占用；先停止占用者（本实例用 pnpm pc down；历史 NixOS 单元用 systemctl stop knoted-api knoted-https）"
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
  local web_root
  web_root=$(readlink -f "$runtime_root/share/knoted/web")

  install -d -m 0700 "$config_dir" "$state_dir" "$state_dir/https" \
    "$state_dir/https/data" "$state_dir/https/config" "$bin_dir" \
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

  render_template "$script_dir/Caddyfile.tmpl" "$caddy_file" \
    @ADMIN_SOCKET@ "$admin_socket" \
    @DOMAIN@ "$domain" \
    @HTTPS_PORT@ "$https_port" \
    @CERT_FILE@ "$cert_file" \
    @KEY_FILE@ "$key_file" \
    @REVISION@ "$revision" \
    @API_PORT@ "$api_port" \
    @WEB_ROOT@ "$web_root"

  "$script_dir/check-certificate.sh" "$cert_file" "$key_file" "$domain"
  check_ports
  command_install_launchers

  log "prepare 完成：配置 $config_file，Caddyfile $caddy_file"
}

command_install_launchers() {
  install -d -m 0700 "$bin_dir"
  install -m 0755 "$script_dir/check-certificate.sh" "$bin_dir/check-certificate"
  # reload 由 host 的 ACME oneshot 调用，PATH 不可控：把 prepare 时解析到的
  # openssl/caddy 目录写进启动脚本（缺 caddy 时留空，`https` 子命令会自行报错）。
  local openssl_bin caddy_bin launcher_path
  openssl_bin=$(command -v openssl || true)
  test -n "$openssl_bin" || die "需要 openssl（由 /etc/nixos 提供：pkgs.openssl）"
  caddy_bin=$(command -v caddy || true)
  if [ -n "$caddy_bin" ]; then
    launcher_path="$(dirname "$openssl_bin"):$(dirname "$caddy_bin"):\$PATH"
  else
    log "警告：当前 PATH 没有 caddy，reload 入口仅写裸命令 caddy（启动 https 时会失败）"
    launcher_path="$(dirname "$openssl_bin"):\$PATH"
  fi
  # ACME 续期后由 /etc/nixos 的 oneshot 调用；不依赖仓库路径，只依赖稳定实例路径。
  # 该单元在 ProtectSystem=strict 下运行（/data 只读），所以这里只读证书、连 admin
  # socket，不写锁文件、不建目录；幂等重复 reload 由 caddy 自身处理。
  local reload tmp
  reload=$bin_dir/knoted-https-reload
  (
    umask 077
    tmp=$(mktemp "$bin_dir/.reload.XXXXXX")
    cat >"$tmp" <<EOF
#!/usr/bin/env bash
# 由 tools/dev/pc/service.sh 生成的稳定入口（ACME 续期后 reload 测试 HTTPS）。
set -euo pipefail
bin_dir=\${BASH_SOURCE[0]%/*}
export PATH="$launcher_path"
"\$bin_dir/check-certificate" "$cert_file" "$key_file" "$domain"
exec caddy reload --config "$caddy_file" --adapter caddyfile \\
  --address "unix/$admin_socket" --force
EOF
    chmod 0755 "$tmp"
    mv -f "$tmp" "$reload"
  )
}

command_reload_https() {
  local reload
  reload=$bin_dir/knoted-https-reload
  test -x "$reload" || die "缺少 $reload；先运行 pnpm pc up -D 生成（prepare 步骤）"
  exec "$reload"
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

command_https() {
  command -v caddy >/dev/null || die "需要 caddy（由 /etc/nixos 提供：pkgs.caddy）"
  test -r "$caddy_file" || die "缺少 Caddyfile $caddy_file；先完成 prepare"
  # 重启 https 也要重新校验证书，不能只依赖 prepare 当时的结果。
  "$script_dir/check-certificate.sh" "$cert_file" "$key_file" "$domain"
  export XDG_DATA_HOME=$state_dir/https/data
  export XDG_CONFIG_HOME=$state_dir/https/config
  exec caddy run --config "$caddy_file" --adapter caddyfile
}

command_probe_api() {
  curl -fsS --max-time 3 "http://127.0.0.1:$api_port/health/ready" >/dev/null
}

command_probe_https() {
  curl -fsS --max-time 5 --resolve "$domain:$https_port:127.0.0.1" \
    "https://$domain:$https_port/health/ready" >/dev/null
}

command_paths() {
  printf 'instance_root=%s\n' "$instance_root"
  printf 'runtime_root=%s\n' "$runtime_root"
  printf 'config_file=%s\n' "$config_file"
  printf 'caddy_file=%s\n' "$caddy_file"
  printf 'admin_socket=%s\n' "$admin_socket"
  printf 'api_port=%s\n' "$api_port"
  printf 'https_port=%s\n' "$https_port"
  printf 'domain=%s\n' "$domain"
  printf 'cert_file=%s\n' "$cert_file"
  printf 'key_file=%s\n' "$key_file"
  printf 'database_url=%s\n' "$(printf '%s' "$database_url" | sed -E 's#(://)[^/@]*@#\1***@#')"
}

command=${1:-}
case "$command" in
prepare) command_prepare ;;
install-launchers) command_install_launchers ;;
reload-https) command_reload_https ;;
migrate) command_migrate ;;
api) command_api ;;
https) command_https ;;
probe-api) command_probe_api ;;
probe-https) command_probe_https ;;
paths) command_paths ;;
*)
  echo "usage: service.sh {prepare|install-launchers|reload-https|migrate|api|https|probe-api|probe-https|paths}" >&2
  exit 2
  ;;
esac
