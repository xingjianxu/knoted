#!/usr/bin/env bash
# dev 一键部署：最新主线提交 → 固定产物 → 原子切换 current → 按迁移情况重启 → 验证。
#
# 这条脚本只编排顺序，不复制既有逻辑：
#   构建    tools/dev/build-artifact.sh（固定提交产物 + REVISION + SHA256SUMS）
#   探针    tools/dev/pc/service.sh probe-api / probe-intelligence
#   进程    @bixlab/pcwrap（pnpm pc down / up -D / process restart）
#
# 用法：bash tools/dev/deploy-dev.sh [<full-40-hex-sha>]
#   无参数时先 `git fetch origin`，再部署 origin/master 的提交。
#
# 语义（见 docs/plans/knoted-v1/development-and-deploy.md「一键部署」）：
#   1. 迁移文件集合与当前运行的产物不同 → pnpm pc down + pnpm pc up -D（重跑 prepare/migrate）
#   2. 集合相同 → 只 process restart api / worker / intelligence
#   3. 目标提交已是 current → 不构建不重启，只做验证
#   4. 切换后任一验证失败 → 指针复位到上一个产物并按同样方式重启，退出非 0
#
# 运行变量：dev 实例需要 6 个只存在于"启动它的 shell"里的变量（两个模型根、
# search/vectors 开关、Office origin 与凭据路径）。脚本把它们持久化到
# <KNOTED_INSTANCE_ROOT>/deploy.env（0600）：首次从活动 knoted-intelligence 进程的
# /proc/<pid>/environ 抓取，之后读文件；调用者环境变量优先。
#
# 环境覆盖（默认值即本机 dev 实例，测试据此指向临时根）：
#   KNOTED_INSTANCE_ROOT          /data/knoted-dev/service
#   KNOTED_DEPLOY_RUNTIME_PARENT  /data/knoted-dev/runtime
#   KNOTED_DEPLOY_PROC_ROOT       /proc
#   KNOTED_DEPLOY_BUILD_CMD       <repo>/tools/dev/build-artifact.sh
#   KNOTED_DEPLOY_READY_TIMEOUT   180（秒）
#   KNOTED_BUILD_ROOT             /data/knoted-dev/tasks/deploy
#   CARGO_BUILD_JOBS              16
#   KNOTED_API_PORT 15602 / KNOTED_INTELLIGENCE_PORT 15603
#   KNOTED_HTTPS_PORT 15601 / KNOTED_HTTPS_DOMAIN wlab.132140.xyz
#
# 不做：数据库/assets 备份、历史产物回收、独立 --rollback、生产实例。
set -euo pipefail
# 与 build-artifact.sh 一致：清单与文件名排序必须固定字节序。
export LC_ALL=C
shopt -u patsub_replacement 2>/dev/null || true

script_dir=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
repository=$(git -C "$script_dir" rev-parse --show-toplevel)
build_script=$script_dir/build-artifact.sh
service_script=$script_dir/pc/service.sh

instance_root=${KNOTED_INSTANCE_ROOT:-/data/knoted-dev/service}
runtime_parent=${KNOTED_DEPLOY_RUNTIME_PARENT:-/data/knoted-dev/runtime}
proc_root=${KNOTED_DEPLOY_PROC_ROOT:-/proc}
build_cmd=${KNOTED_DEPLOY_BUILD_CMD:-$build_script}
build_root=${KNOTED_BUILD_ROOT:-/data/knoted-dev/tasks/deploy}
ready_timeout=${KNOTED_DEPLOY_READY_TIMEOUT:-180}
cargo_jobs=${CARGO_BUILD_JOBS:-16}
api_port=${KNOTED_API_PORT:-15602}
intelligence_port=${KNOTED_INTELLIGENCE_PORT:-15603}
https_port=${KNOTED_HTTPS_PORT:-15601}
domain=${KNOTED_HTTPS_DOMAIN:-wlab.132140.xyz}

deploy_env_file=$instance_root/deploy.env
lock_file=$instance_root/.deploy.lock

# 运行变量白名单：全部是路径或开关，不含密钥本身。
runtime_variable_keys=(
  KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR
  KNOTED_INTELLIGENCE_BGE_M3_DIR
  KNOTED_SEARCH_ENABLED
  KNOTED_SEARCH_CREDENTIAL_FILE
  KNOTED_VECTORS_ENABLED
  KNOTED_OFFICE_ORIGIN
  KNOTED_OFFICE_JWT_SECRET_FILE
  KNOTED_CHAT_LOCAL_BASE_URL
  KNOTED_CHAT_LOCAL_MODEL
  KNOTED_CHAT_LOCAL_CREDENTIAL_FILE
)
# 缺了这两个，knoted-intelligence 直接拒绝启动（PC 编排里 worker 又依赖它健康）。
required_variable_keys=(
  KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR
  KNOTED_INTELLIGENCE_BGE_M3_DIR
)

log() {
  printf 'knoted-deploy: %s\n' "$1" >&2
}

fail() {
  printf 'knoted-deploy: %s\n' "$1" >&2
  exit "${2:-1}"
}

usage() {
  cat <<'TEXT'
usage: bash tools/dev/deploy-dev.sh [<full-40-hex-sha>]

无参数时 fetch origin 并部署 origin/master 的最新提交。
脚本把 dev 固定产物构建到 <KNOTED_DEPLOY_RUNTIME_PARENT>/<sha>，原子切换 current，
按迁移文件集合决定 down + up 还是只 restart，最后验证并报告。
TEXT
}

# 稳定实例不得位于任务工作区（AGENTS.md 规则 4/11，与 service.sh 同语义）。
forbidden_path() {
  case "$1" in
  */.worktrees/*) return 0 ;;
  *) return 1 ;;
  esac
}

guard_worktree() {
  local path
  for path in "$instance_root" "$runtime_parent" "$build_root"; do
    if forbidden_path "$path"; then
      fail "实例根/运行根/构建根位于 .worktrees 任务工作区：$path（稳定实例必须用主检出 + /data 运行根）" 2
    fi
  done
}

require_tools() {
  local name missing=""
  for name in git pnpm curl sha256sum flock readlink find sort; do
    command -v "$name" >/dev/null 2>&1 || missing="$missing $name"
  done
  if [ -n "$missing" ]; then
    fail "缺少依赖：${missing# }（git/pnpm 由仓库提供，curl/iproute2 由 /etc/nixos 提供）" 2
  fi
  test -x "$build_cmd" || fail "构建命令不可执行：$build_cmd" 2
  test -r "$service_script" || fail "缺少编排入口：$service_script" 2
}

resolve_revision() {
  if [ "$#" -gt 1 ]; then
    fail "只接受一个提交参数" 2
  fi
  if [ "$#" -eq 1 ]; then
    if [[ ! $1 =~ ^[0-9a-f]{40}$ ]]; then
      fail "需要完整 40 位小写提交 SHA（收到：$1）" 2
    fi
    revision=$1
  else
    log "fetch origin，取最新主线"
    git -C "$repository" fetch --quiet origin || fail "git fetch origin 失败" 2
    revision=$(git -C "$repository" rev-parse --verify --quiet 'origin/master^{commit}') ||
      fail "无法解析 origin/master（先 git fetch origin）" 2
  fi
  if ! git -C "$repository" rev-parse --verify --quiet "$revision^{commit}" >/dev/null; then
    fail "本地没有该提交：$revision（先 git fetch origin）" 2
  fi
}

# deploy.env 只接受 KEY=VALUE 行，不 source（避免执行文件内容）。
read_deploy_env() {
  local key value
  while IFS='=' read -r key value || [ -n "$key" ]; do
    [ -n "$key" ] || continue
    case " ${runtime_variable_keys[*]} " in
    *" $key "*) [ -n "$value" ] && resolved[$key]=$value ;;
    esac
  done <"$deploy_env_file"
}

# 从活动进程读回运行变量（机器上真实运行的实例即事实来源）。三个进程的 exe 各不相同：
# api/worker 直接跑 Rust 二进制，intelligence 的包装器 exec 固定解释器（exe 是产物里的
# python3.12），且 service.sh 用 `env -i` 只给 intelligence 留模型根与端口。所以不能只看
# 一个进程：按 api → worker → intelligence 并集取值，先到先得。
capture_running_variables() {
  local exe_link exe env_file key value found=0
  local -a candidates=()
  for exe_link in "$proc_root"/[0-9]*/exe; do
    exe=$(readlink -- "$exe_link" 2>/dev/null) || continue
    exe=${exe% (deleted)}
    case "$exe" in
    "$runtime_parent"/*/bin/knoted-api | "$runtime_parent"/*/bin/knoted-worker)
      candidates+=("${exe_link%/exe}/environ")
      ;;
    "$runtime_parent"/*/share/knoted/intelligence/python/bin/python3.12 | "$runtime_parent"/*/bin/knoted-intelligence)
      candidates+=("${exe_link%/exe}/environ")
      ;;
    *) continue ;;
    esac
  done
  for env_file in "${candidates[@]}"; do
    [ -r "$env_file" ] || continue
    found=1
    while IFS='=' read -r key value; do
      case " ${runtime_variable_keys[*]} " in
      *" $key "*) [ -n "$value" ] && [ -z "${resolved[$key]:-}" ] && resolved[$key]=$value ;;
      esac
    done < <(tr '\0' '\n' <"$env_file")
  done
  test "$found" = 1
}

write_deploy_env() {
  local key tmp
  mkdir -p "$instance_root"
  tmp=$(mktemp "$instance_root/.deploy.env.XXXXXX")
  (
    umask 077
    for key in "${runtime_variable_keys[@]}"; do
      [ -n "${resolved[$key]:-}" ] || continue
      printf '%s=%s\n' "$key" "${resolved[$key]}"
    done >"$tmp"
  )
  chmod 0600 "$tmp"
  mv -f "$tmp" "$deploy_env_file"
}

# 解析顺序：调用者环境 > deploy.env > 活动进程抓取。抓取到新值就回写文件。
resolve_runtime_variables() {
  local key captured=0 missing=""
  declare -gA resolved=()
  for key in "${runtime_variable_keys[@]}"; do
    if [ -n "${!key:-}" ]; then resolved[$key]=${!key}; fi
  done
  if [ -f "$deploy_env_file" ]; then read_deploy_env; fi
  for key in "${runtime_variable_keys[@]}"; do
    [ -n "${resolved[$key]:-}" ] || missing="$missing $key"
  done
  if [ -n "$missing" ]; then
    if capture_running_variables; then
      captured=1
      missing=""
      for key in "${runtime_variable_keys[@]}"; do
        [ -n "${resolved[$key]:-}" ] || missing="$missing $key"
      done
    fi
  fi
  for key in "${required_variable_keys[@]}"; do
    if [ -z "${resolved[$key]:-}" ]; then
      fail "缺少运行变量 $key：既没有 $deploy_env_file，也无法从活动 knoted-intelligence 进程读取。先 export 后重跑（见 docs/plans/knoted-v1/development-and-deploy.md）" 2
    fi
  done
  if [ -n "$missing" ]; then
    log "未设置的可选运行变量：${missing# }（不影响启动，相关能力保持关闭）"
  fi
  for key in "${runtime_variable_keys[@]}"; do
    if [ -n "${resolved[$key]:-}" ]; then export "$key=${resolved[$key]}"; fi
  done
  if [ "$captured" = 1 ]; then
    write_deploy_env
    log "运行变量写入 $deploy_env_file（0600）"
  fi
}

verify_artifact() {
  local revision_file=$target/share/knoted/REVISION actual
  test -x "$target/bin/knoted-api" || fail "产物缺少 bin/knoted-api：$target"
  test -x "$target/bin/knoted-worker" || fail "产物缺少 bin/knoted-worker：$target"
  test -s "$revision_file" || fail "产物缺少 share/knoted/REVISION：$target"
  actual=$(cat "$revision_file")
  test "$actual" = "$revision" || fail "产物 REVISION=$actual 与目标提交 $revision 不一致"
  test -s "$target/SHA256SUMS" || fail "产物缺少 SHA256SUMS：$target"
  (cd "$target" && sha256sum --check --quiet SHA256SUMS) || fail "产物摘要校验失败：$target"
}

list_migrations() {
  find "$1/share/knoted/migrations" -maxdepth 1 -name '*.sql' -type f -printf '%f\n' 2>/dev/null | sort || true
}

# 迁移集合变化（新增、删除或改名）就必须重跑 prepare+migrate；restart 不会跑迁移。
migrations_changed=1
compare_migrations() {
  if [ -z "$previous" ] || [ ! -d "$previous/share/knoted/migrations" ]; then
    migrations_changed=1
    return 0
  fi
  local before after added
  before=$(list_migrations "$previous")
  after=$(list_migrations "$target")
  if [ "$before" = "$after" ]; then
    migrations_changed=0
    return 0
  fi
  migrations_changed=1
  added=$(comm -13 <(printf '%s\n' "$before") <(printf '%s\n' "$after") | tr '\n' ' ')
  log "迁移集合变化${added:+，新增：$added}"
}

switch_current() {
  local link=$runtime_parent/.current.$$
  ln -s "$1" "$link"
  mv -Tf "$link" "$runtime_parent/current"
}

restart_processes() {
  local name
  (
    cd "$repository"
    if [ "$migrations_changed" = 1 ]; then
      pnpm pc down || log "pnpm pc down 返回非零（此前可能未运行），继续启动"
      pnpm pc up -D || return 1
      exit 0
    fi
    for name in knoted-api knoted-worker knoted-intelligence; do
      pnpm pc process restart "$name" && continue
      # 实例当时可能整个停着，restart 无处可用：退回到完整启动（此分支无新迁移）。
      log "process restart $name 失败，改用 pnpm pc up -D 完整启动"
      pnpm pc down || true
      pnpm pc up -D || return 1
      exit 0
    done
  )
}

probe_api() {
  KNOTED_RUNTIME_ROOT=$target KNOTED_API_PORT=$api_port \
    bash "$service_script" probe-api >/dev/null 2>&1
}

probe_intelligence() {
  KNOTED_RUNTIME_ROOT=$target KNOTED_INTELLIGENCE_PORT=$intelligence_port \
    bash "$service_script" probe-intelligence >/dev/null 2>&1
}

wait_ready() {
  local timeout=$1 start=$SECONDS deadline=$((SECONDS + $1))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if probe_api && probe_intelligence; then
      log "readiness 通过（$((SECONDS - start))s）"
      return 0
    fi
    sleep 1
  done
  log "readiness 超时（${timeout}s）：API 或 intelligence 未就绪"
  return 1
}

# 进程映像必须已经来自新产物：/proc/<pid>/exe 会解析到具体 revision 目录。
# knoted-intelligence 的入口是 bash 包装器，它 exec 固定解释器，所以真正看到的
# exe 是 <产物>/share/knoted/intelligence/python/bin/python3.12。
check_processes() {
  local exe_link exe name dir seen_api=0 seen_worker=0 seen_intelligence=0
  for exe_link in "$proc_root"/[0-9]*/exe; do
    exe=$(readlink -- "$exe_link" 2>/dev/null) || continue
    exe=${exe% (deleted)}
    case "$exe" in
    "$runtime_parent"/*/bin/knoted-api) {
      name=knoted-api
      dir=${exe%/bin/$name}
      seen_api=1
    } ;;
    "$runtime_parent"/*/bin/knoted-worker) {
      name=knoted-worker
      dir=${exe%/bin/$name}
      seen_worker=1
    } ;;
    "$runtime_parent"/*/share/knoted/intelligence/python/bin/python3.12) {
      name=knoted-intelligence
      dir=${exe%/share/knoted/intelligence/python/bin/python3.12}
      seen_intelligence=1
    } ;;
    *) continue ;;
    esac
    if [ "$dir" != "$target" ]; then
      log "$name 仍运行在 $dir（期望 $target）"
      return 1
    fi
  done
  test "$seen_api" = 1 || {
    log "没有找到运行中的 knoted-api"
    return 1
  }
  test "$seen_worker" = 1 || {
    log "没有找到运行中的 knoted-worker"
    return 1
  }
  test "$seen_intelligence" = 1 || {
    log "没有找到运行中的 knoted-intelligence"
    return 1
  }
  return 0
}

check_https() {
  curl -fsS -o /dev/null --max-time 8 "https://$domain:$https_port/" || {
    log "HTTPS 入口不可用：https://$domain:$https_port/"
    return 1
  }
}

# 切换后失败的处理分两种：迁移集合没变，回退指针并按同样方式重启是安全的；
# 迁移已跑过就不能只回退指针（旧产物的迁移集合对不上已应用的 schema，sqlx 会拒绝
# 启动），此时保留新产物并打印人工入口，避免留下“指针旧、数据库新”的死锁。
fail_after_switch() {
  local reason=$1
  if [ "$migrations_changed" = 1 ]; then
    log "$reason；本次部署包含迁移，指针保持指向 $target（只回退指针会与已应用的 schema 冲突）"
    log "排查入口：pnpm pc process logs knoted-api -n 200 / pnpm pc process logs knoted-intelligence -n 200"
    exit 1
  fi
  if [ -z "$previous" ] || [ ! -d "$previous" ]; then
    log "$reason；没有可回退的上一个产物，指针保持指向 $target"
    log "排查入口：pnpm pc process logs knoted-api -n 200"
    exit 1
  fi
  log "$reason；复位 current → $previous 并重启回旧版本"
  switch_current "$previous"
  restart_processes || log "复位后的重启返回非零，请手工检查（pnpm pc process list）"
  exit 1
}

verify_deployed() {
  check_processes
  check_https
  log "验证通过：$revision"
}

main() {
  case "${1:-}" in
  -h | --help | help)
    usage
    exit 0
    ;;
  esac
  guard_worktree
  require_tools
  resolve_revision "$@"
  mkdir -p "$instance_root" "$runtime_parent"
  exec {lock_fd}>"$lock_file"
  flock -n "$lock_fd" || fail "另一个部署正在进行（$lock_file）" 2

  target=$(realpath -m -- "$runtime_parent/$revision")
  previous=$(readlink -f "$runtime_parent/current" 2>/dev/null || true)

  if [ "$previous" = "$target" ]; then
    log "已是最新：current 已指向 $revision，只做验证"
    verify_artifact
    verify_deployed
    return 0
  fi

  # 构建前就要有运行变量：Vite 用 KNOTED_OFFICE_ORIGIN 决定是否编入 Office。
  resolve_runtime_variables

  if [ -d "$target" ]; then
    log "复用已构建产物：$target"
    verify_artifact
  else
    log "构建固定产物 $revision（CARGO_BUILD_JOBS=$cargo_jobs，KNOTED_BUILD_ROOT=$build_root）"
    mkdir -p "$build_root"
    CARGO_BUILD_JOBS=$cargo_jobs KNOTED_BUILD_ROOT=$build_root \
      "$build_cmd" "$revision" "$target" || fail "构建失败：$revision（日志见 $build_root）"
    verify_artifact
  fi

  compare_migrations
  switch_current "$target"
  log "current → $target"
  restart_processes || fail_after_switch "重启失败（prepare/migrate/readiness）"

  wait_ready "$ready_timeout" || fail_after_switch "readiness 未通过"
  verify_deployed || fail_after_switch "验证未通过"
  log "完成：$revision 已上线（https://$domain:$https_port/）"
}

main "$@"
