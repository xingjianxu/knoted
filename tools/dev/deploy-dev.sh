#!/usr/bin/env bash
# dev 一键部署：最新主线提交 → 固定产物 → 原子切换 current → 按迁移情况重启 → 验证。
#
# 这条脚本只编排顺序，不复制既有逻辑：
#   构建    tools/dev/build-artifact.sh（固定提交产物 + REVISION + SHA256SUMS）
#   探针    tools/dev/pc/service.sh probe-api
#   进程    @bixlab/pcwrap（pnpm pc down / up -D / process restart）
#
# 用法：bash tools/dev/deploy-dev.sh [<full-40-hex-sha> | --prune-current]
# 无参数时先 git fetch origin，再部署 origin/master 的提交。
#
# 迁移集合改变时完整重启以运行 prepare 和 migrate，其余发布逐个重启 API/Worker。
# 从含旧执行服务的产物升级时，先用旧编排停机并确认相关进程退出，再完整启动新编排。
# 本脚本只运行迁移。清空数据库和处理既有运行数据须在部署前单独完成。
# 验证通过后清理旧产物。同一 SHA 或 --prune-current 只核验当前运行版本。
#
# 运行变量按调用者环境、deploy.env、旧渲染 TOML、活动 API/Worker 的顺序解析。
# 搜索与 Office 的开关和凭据路径写入 <KNOTED_INSTANCE_ROOT>/deploy.env（0600）。
#
# 稳定实例锚定在主检出：pcwrap 的 daemon cwd 决定后续 prepare/restart 用哪份
# tools/dev/pc/service.sh，所以即使从任务 worktree 调用，进程编排也在主检出执行
# （AGENTS.md 规则 4/11）。
#
# 环境覆盖（默认值即本机 dev 实例，测试据此指向临时根）：
#   KNOTED_INSTANCE_ROOT          /data/knoted-dev/service
#   KNOTED_DEPLOY_RUNTIME_PARENT  /data/knoted-dev/runtime
#   KNOTED_DEPLOY_PROC_ROOT       /proc
#   KNOTED_DEPLOY_BUILD_CMD       <repo>/tools/dev/build-artifact.sh
#   KNOTED_DEPLOY_READY_TIMEOUT   180（秒）
#   KNOTED_BUILD_ROOT             /data/knoted-dev/tasks/deploy
#   CARGO_BUILD_JOBS              16
#   KNOTED_API_PORT 15602 / PC_PORT_NUM 15600
#   KNOTED_HTTPS_PORT 15601 / KNOTED_HTTPS_DOMAIN wlab.132140.xyz
#
# 不做：数据库/assets 备份、独立 --rollback、生产实例。
set -euo pipefail
# 与 build-artifact.sh 一致：清单与文件名排序必须固定字节序。
export LC_ALL=C
shopt -u patsub_replacement 2>/dev/null || true

script_dir=$(cd "${BASH_SOURCE[0]%/*}" && pwd)
repository=$(git -C "$script_dir" rev-parse --show-toplevel)
# 主检出（在 worktree 里调用时与 $repository 不同）；pcwrap daemon 的 cwd 取它。
instance_repository=$(dirname -- "$(git -C "$repository" rev-parse --path-format=absolute --git-common-dir)")
build_script=$script_dir/build-artifact.sh
service_script=$script_dir/pc/service.sh

instance_root=${KNOTED_INSTANCE_ROOT:-/data/knoted-dev/service}
runtime_parent=$(realpath -m -- "${KNOTED_DEPLOY_RUNTIME_PARENT:-/data/knoted-dev/runtime}")
proc_root=$(realpath -m -- "${KNOTED_DEPLOY_PROC_ROOT:-/proc}")
build_cmd=${KNOTED_DEPLOY_BUILD_CMD:-$build_script}
build_root=${KNOTED_BUILD_ROOT:-/data/knoted-dev/tasks/deploy}
ready_timeout=${KNOTED_DEPLOY_READY_TIMEOUT:-180}
cargo_jobs=${CARGO_BUILD_JOBS:-16}
api_port=${KNOTED_API_PORT:-15602}
control_port=${PC_PORT_NUM:-15600}
https_port=${KNOTED_HTTPS_PORT:-15601}
domain=${KNOTED_HTTPS_DOMAIN:-wlab.132140.xyz}

deploy_env_file=$instance_root/deploy.env
lock_file=$instance_root/.deploy.lock

source "$script_dir/pc/runtime-env.sh"

log() {
  printf 'knoted-deploy: %s\n' "$1" >&2
}

fail() {
  printf 'knoted-deploy: %s\n' "$1" >&2
  exit "${2:-1}"
}

usage() {
  cat <<'TEXT'
usage: bash tools/dev/deploy-dev.sh [<full-40-hex-sha> | --prune-current]

无参数时 fetch origin 并部署 origin/master 的最新提交。
脚本把 dev 固定产物构建到 <KNOTED_DEPLOY_RUNTIME_PARENT>/<sha>，原子切换 current，
按迁移文件集合决定 down + up 还是逐个重启，验证通过后清理旧产物。
从旧执行服务升级前，先使用旧编排停机并确认所有进程退出。
--prune-current 只核验当前运行版本并清理旧产物，无需构建、重启或迁移。
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
  for path in "$instance_root" "$runtime_parent" "$build_root" "$instance_repository"; do
    if forbidden_path "$path"; then
      fail "实例根/运行根/构建根/主检出位于 .worktrees 任务工作区：$path（稳定实例必须用主检出 + /data 运行根）" 2
    fi
  done
  if [ "$repository" != "$instance_repository" ]; then
    log "从任务 worktree 调用；进程编排仍在主检出 $instance_repository 执行"
  fi
}

require_tools() {
  local name missing=""
  for name in git pnpm curl sha256sum flock readlink realpath find findmnt sort ss cmp; do
    command -v "$name" >/dev/null 2>&1 || missing="$missing $name"
  done
  if [ -n "$missing" ]; then
    fail "缺少依赖：${missing# }（git/pnpm 由仓库提供，curl/iproute2 由 /etc/nixos 提供）" 2
  fi
  test -d "$proc_root" && test -r "$proc_root" || fail "无法读取进程目录：$proc_root" 2
  test -x "$build_cmd" || fail "构建命令不可执行：$build_cmd" 2
  test -r "$service_script" || fail "缺少编排入口：$service_script" 2
  test -f "$instance_repository/process-compose.yaml" ||
    fail "主检出缺少 process-compose.yaml：$instance_repository" 2
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
  local generation selected_generation
  generation=$(cat "$script_dir/artifact-generation")
  if ! selected_generation=$(git -C "$repository" cat-file blob "$revision:tools/dev/artifact-generation" 2>/dev/null) ||
     [[ -z $generation || $selected_generation != "$generation" ]]; then
    fail "目标提交 $revision 的产物代不受当前部署规则支持" 2
  fi
}

# 旧 prepare 只生成下列搜索和 Office 字段。配置中其他段落无需读取字段值。
# 无法识别的设置交由操作者明确迁移，防止清空或错误地转换现有功能配置。
read_legacy_runtime_config() {
  local line section="" field value search_enabled="" key_configured=0
  local search_file="" office_origin="" office_file=""
  local table='^\[([A-Za-z0-9_.]+)\]$'
  local quoted='^([a-z_]+) = "([^"]*)"$'
  local https_origin='^https://([a-zA-Z0-9.-]+|\[[a-fA-F0-9:.]+\])(:[0-9]{1,5})?$'
  local -A seen=()

  while IFS= read -r line || [ -n "$line" ]; do
    if [[ $line =~ $table ]]; then
      section=${BASH_REMATCH[1]}
      case "$section" in
      server|database|files|search|credentials|office)
        [[ ! -v seen[$section] ]] || return 1
        seen[$section]=1
        ;;
      esac
      continue
    fi
    [[ $line =~ ^[[:blank:]]*\[ ]] && return 1
    [[ -z $line || $line == \#* ]] && continue
    case "$section" in
    search)
      case "$line" in
      "enabled = true"|"enabled = false")
        [[ -z $search_enabled ]] || return 1
        if [[ $line == "enabled = true" ]]; then search_enabled=1; else search_enabled=0; fi
        ;;
      'api_key_credential = "meilisearch"')
        [[ $key_configured == 0 ]] || return 1
        key_configured=1
        ;;
      *) return 1 ;;
      esac
      ;;
    credentials|office)
      [[ $line =~ $quoted ]] || return 1
      field=${BASH_REMATCH[1]}
      value=${BASH_REMATCH[2]}
      if [[ $section == office && $field == document_server_origin ]]; then
        [[ -z $office_origin && $value =~ $https_origin ]] || return 1
        office_origin=$value
      else
        [[ $value == /* && $value != *['"\']* && ! $value =~ [[:cntrl:]] ]] || return 1
        case "$section.$field" in
        credentials.credential_file) [[ -z $search_file ]] || return 1; search_file=$value ;;
        office.jwt_secret_file) [[ -z $office_file ]] || return 1; office_file=$value ;;
        *) return 1 ;;
        esac
      fi
      ;;
    esac
  done <"$1" || return 1

  [[ -v seen[server] && -v seen[database] && -v seen[files] ]] || return 1
  [[ ! -v seen[search] || -n $search_enabled ]] || return 1
  [[ $key_configured == 0 || ( $search_enabled == 1 && -n $search_file ) ]] || return 1
  [[ ! -v seen[office] || ( -n $office_origin && -n $office_file ) ]] || return 1
  if [[ -n $search_enabled && -z ${resolved[KNOTED_SEARCH_ENABLED]:-} ]]; then
    resolved[KNOTED_SEARCH_ENABLED]=$search_enabled
  fi
  if [[ $key_configured == 1 && ${resolved[KNOTED_SEARCH_ENABLED]:-} == 1 &&
        -z ${resolved[KNOTED_SEARCH_CREDENTIAL_FILE]:-} ]]; then
    resolved[KNOTED_SEARCH_CREDENTIAL_FILE]=$search_file
  fi
  if [[ -n $office_origin && -z ${resolved[KNOTED_OFFICE_ORIGIN]:-} ]]; then
    resolved[KNOTED_OFFICE_ORIGIN]=$office_origin
  fi
  if [[ -n $office_file && -z ${resolved[KNOTED_OFFICE_JWT_SECRET_FILE]:-} ]]; then
    resolved[KNOTED_OFFICE_JWT_SECRET_FILE]=$office_file
  fi
}

# 只从 exe 已确认的 API 或 Worker 读取环境，按发现顺序补齐缺省值。
capture_problem=""
capture_running_variables() {
  local pid status env_file env_fd entry key value found=0 unreadable_pid=""
  local -a candidates=()
  capture_problem=""
  for pid in "$proc_root"/[0-9]*; do
    [ -d "$pid" ] || continue
    status=0
    identify_process "$pid" || status=$?
    [ "$status" -eq 1 ] && continue
    [ "$image_role" = knoted-intelligence ] && continue
    # argv 只用于诊断权限，不能让未经 exe 确认的进程提供运行变量。
    if [ "$image_source" != exe ]; then
      unreadable_pid=${unreadable_pid:-$pid}
      continue
    fi
    candidates+=("$pid/environ")
  done
  for env_file in "${candidates[@]}"; do
    # -r 只检查文件权限；内核会在实际打开 environ 时再检查进程访问权限。
    if ! { exec {env_fd}<"$env_file"; } 2>/dev/null; then
      unreadable_pid=${unreadable_pid:-${env_file%/environ}}
      continue
    fi
    found=1
    while IFS= read -r -d '' entry <&"$env_fd"; do
      [[ $entry == *=* ]] || continue
      key=${entry%%=*}
      value=${entry#*=}
      [[ $value != *$'\n'* && $value != *$'\r'* ]] || continue
      case " ${runtime_variable_keys[*]} " in
      *" $key "*) [ -n "$value" ] && [ -z "${resolved[$key]:-}" ] && resolved[$key]=$value ;;
      esac
    done
    exec {env_fd}<&-
  done
  if [ -n "$unreadable_pid" ]; then
    capture_problem="活动进程的 /proc/<pid>/environ 不可读或无法确认来源（$(describe_process_gid "$unreadable_pid")）；$(sg_hint)，或先 export 再重跑"
  fi
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

# Without the rendered TOML, disabled optional settings must be explicit.
legacy_environment_known() {
  [[ ${resolved[KNOTED_SEARCH_ENABLED]:-} =~ ^[01]$ ]] || return 1
  if [[ ${resolved[KNOTED_SEARCH_ENABLED]} == 1 && -z ${resolved[KNOTED_SEARCH_CREDENTIAL_FILE]:-} &&
        ! -v KNOTED_SEARCH_CREDENTIAL_FILE ]]; then
    return 1
  fi
  if [[ -n ${resolved[KNOTED_OFFICE_ORIGIN]:-} && -n ${resolved[KNOTED_OFFICE_JWT_SECRET_FILE]:-} ]]; then
    return 0
  fi
  [[ -v KNOTED_OFFICE_ORIGIN && -v KNOTED_OFFICE_JWT_SECRET_FILE &&
     -z ${resolved[KNOTED_OFFICE_ORIGIN]:-} && -z ${resolved[KNOTED_OFFICE_JWT_SECRET_FILE]:-} ]]
}

# 先解析调用者环境和 deploy.env，冷切换可从旧渲染配置补齐。
# 同一编排更新还可以从活动进程读取环境，最终结果回写文件。
resolve_runtime_variables() {
  local key captured=0 missing="" file_before=""
  load_deploy_env "$deploy_env_file"
  declare -gA resolved=()
  for key in "${runtime_variable_keys[@]}"; do
    if [ -n "${!key:-}" ]; then resolved[$key]=${!key}; fi
  done
  if [ -f "$deploy_env_file" ]; then
    file_before=$(grep -E '^[A-Z_]+=' "$deploy_env_file" 2>/dev/null || true)
  fi
  for key in "${runtime_variable_keys[@]}"; do
    [ -n "${resolved[$key]:-}" ] || missing="$missing $key"
  done
  if [ "$legacy_transition" = 1 ] && [ -n "$missing" ]; then
    local old_config=$instance_root/config/knoted.toml
    if [ -f "$old_config" ]; then
      read_legacy_runtime_config "$old_config" ||
        fail "无法从旧渲染配置恢复搜索和 Office，请显式提供这些运行设置" 2
    elif ! legacy_environment_known; then
      fail "旧实例没有可用的部署环境或渲染配置，请显式提供搜索和 Office 设置" 2
    fi
    missing=""
    for key in "${runtime_variable_keys[@]}"; do
      [ -n "${resolved[$key]:-}" ] || missing="$missing $key"
    done
  fi
  if [ -n "$missing" ]; then
    if capture_running_variables; then
      captured=1
      missing=""
      for key in "${runtime_variable_keys[@]}"; do
        [ -n "${resolved[$key]:-}" ] || missing="$missing $key"
      done
    fi
    if [ -n "$missing" ] && [ -n "$capture_problem" ]; then
      log "$capture_problem"
    fi
  fi
  if [ -n "$missing" ]; then
    log "未设置的可选运行变量：${missing# }（不影响启动，相关能力保持关闭）"
  fi
  for key in "${runtime_variable_keys[@]}"; do
    if [ -n "${resolved[$key]:-}" ]; then export "$key=${resolved[$key]}"; fi
  done
  if [ "$captured" = 1 ] || deploy_env_outdated "$file_before"; then
    write_deploy_env
    log "运行变量写入 $deploy_env_file（0600）"
  fi
}

# 文件内容与本次解析结果不一致（缺失键、调用者覆盖、或首次）就需要回写。
deploy_env_outdated() {
  local key value expected=""
  for key in "${runtime_variable_keys[@]}"; do
    [ -n "${resolved[$key]:-}" ] || continue
    expected+="$key=${resolved[$key]}"$'\n'
  done
  test "$expected" != "${1:+$1$'\n'}"
}

# 产物可用性检查：不直接退出，让调用方决定“重建还是报错”。
artifact_problem=""
artifact_ok() {
  local actual
  artifact_problem=""
  if ! actual=$(KNOTED_RUNTIME_ROOT="$target" bash "$service_script" verify-artifact); then
    artifact_problem="固定产物校验失败"
    return 1
  fi
  if [ "$actual" != "$revision" ]; then
    artifact_problem="产物 REVISION 与目标提交 $revision 不一致"
    return 1
  fi
}

verify_artifact() {
  artifact_ok || fail "产物不可用（$artifact_problem）：$target（确认后 rm -rf 该目录重试）"
}

list_migrations() {
  find "$1/share/knoted/migrations" -maxdepth 1 -name '*.sql' -type f -printf '%f\n' 2>/dev/null | sort
}

# 迁移集合变化（新增、删除或改名）就必须重跑 prepare+migrate；restart 不会跑迁移。
# find 失败也算变化：宁可多跑一次幂等的 migrate，也不能在 schema 落后时只重启。
migrations_changed=1
compare_migrations() {
  local before after added
  if [ -z "$previous" ] || [ ! -d "$previous/share/knoted/migrations" ]; then
    migrations_changed=1
    return 0
  fi
  if ! before=$(list_migrations "$previous") || ! after=$(list_migrations "$target"); then
    log "无法枚举迁移目录，按“已变化”处理"
    migrations_changed=1
    return 0
  fi
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

# 所有 pcwrap 调用都在主检出执行：daemon 的 cwd 决定后续 prepare/restart 用哪份
# tools/dev/pc/service.sh，不能让它指向可丢弃的任务 worktree。
pc() {
  (cd "$instance_repository" && pnpm pc "$@")
}

instance_running() {
  pc process list >/dev/null 2>&1
}

# 停止失败既可能是实例本来就停着，也可能是停不下来；后者不能继续 up。
stop_instance() {
  if ! instance_running; then
    log "实例当前未运行，跳过 down"
    return 0
  fi
  pc down
}

start_instance() {
  KNOTED_RUNTIME_ROOT="$runtime_parent/current" pc up -D
}

restart_running() {
  local name
  for name in knoted-api knoted-worker; do
    pc process restart "$name" && continue
    # 实例可能在过程中退出：退回到完整启动（本分支没有新迁移，migrate 是幂等的）。
    log "process restart $name 失败，改用 down + up -D 完整启动"
    stop_instance || return 1
    start_instance || return 1
    return 0
  done
}

probe_api() {
  KNOTED_RUNTIME_ROOT=$target KNOTED_API_PORT=$api_port \
    bash "$service_script" probe-api >/dev/null 2>&1
}

# 同时核验 API readiness 和两个 Rust 进程的产物路径。
wait_for_convergence() {
  local timeout=$1 start=$SECONDS deadline=$((SECONDS + $1)) ready=0
  while [ "$SECONDS" -lt "$deadline" ]; do
    if probe_api; then
      ready=1
      if check_processes; then
        log "readiness 与进程映像就位（$((SECONDS - start))s）"
        return 0
      fi
      # 判不出新旧版本时再等也不会变：立即停，别把预算耗在不可能收敛的判据上。
      [ "$process_unverifiable" = 1 ] && break
    fi
    sleep 1
  done
  if [ "$process_unverifiable" = 1 ]; then
    # 不是超时：判据本身不成立，再等也不会变。
    log "停止等待（$((SECONDS - start))s）：$process_problem"
  elif [ "$ready" = 1 ]; then
    log "等待超时（${timeout}s）：${process_problem}"
  else
    log "等待超时（${timeout}s）：API 未就绪（${process_problem:-进程未就绪}）"
  fi
  return 1
}

# 进程映像必须已经来自新产物：/proc/<pid>/exe 会解析到具体 revision 目录。
# knoted-intelligence 的入口是 bash 包装器，它 exec 固定解释器，所以真正看到的
# exe 是 <产物>/share/knoted/intelligence/python/bin/python3.12。
#
# 但内核解析 /proc/<pid>/exe 走 PTRACE_MODE_READ_FSCREDS，要求调用者与目标的
# uid **和 gid** 都匹配。dev 进程由 pcwrap 托管的 process-compose 拉起，主组
# 可能是 strata-clients 而操作者是 users，此时 readlink 直接 EPERM——这不是
# "进程不存在"。退路是世界可读的 /proc/<pid>/cmdline：service.sh 在 exec 前把
# 运行根解析成具体产物目录，argv[0] 因此带着 <sha>。如果 argv[0] 只有别名，
# 无法判定版本，属于"不可验证"，既不猜也不回滚。
image_role=""
image_dir=""
image_source=""
criterion_notice=0
process_problem=""
process_unverifiable=0

# 从路径识别角色与产物目录：0 = 命中。
image_of_path() {
  case "$1" in
  "$runtime_parent"/*/bin/knoted-api)
    image_role=knoted-api
    image_dir=${1%/bin/knoted-api}
    ;;
  "$runtime_parent"/*/bin/knoted-worker)
    image_role=knoted-worker
    image_dir=${1%/bin/knoted-worker}
    ;;
  "$runtime_parent"/*/share/knoted/intelligence/python/bin/python3.12)
    image_role=knoted-intelligence
    image_dir=${1%/share/knoted/intelligence/python/bin/python3.12}
    ;;
  # 包装器到 exec 解释器之间会有一瞬，旧判据也认它。
  "$runtime_parent"/*/bin/knoted-intelligence)
    image_role=knoted-intelligence
    image_dir=${1%/bin/knoted-intelligence}
    ;;
  *) return 1 ;;
  esac
  return 0
}

# 0 = 命中本实例的 knoted 进程（image_* 已填）；1 = 无关进程；
# 2 = 是本实例进程但定位不到具体产物目录（不可验证）。
identify_process() {
  local pid=$1 exe cmdline=""
  image_role=""
  image_dir=""
  image_source=""
  exe=$(readlink -- "$pid/exe" 2>/dev/null) || exe=""
  exe=${exe% (deleted)}
  if [ -n "$exe" ]; then
    image_of_path "$exe" || return 1
    image_source=exe
    return 0
  fi
  if IFS= read -r -d '' cmdline 2>/dev/null <"$pid/cmdline" && image_of_path "$cmdline"; then
    if [[ ${image_dir#"$runtime_parent/"} =~ ^[0-9a-f]{40}$ ]]; then
      image_source=cmdline
      return 0
    fi
    # 不能事后解析别名：链接可能在这次启动之后已被切换过。
    image_source=alias
    return 2
  fi
  # exe 读不到、cmdline 也不指向已知角色：无法证明它属于本实例，不当成 knoted 进程。
  return 1
}

legacy_artifact() {
  [ -n "$1" ] && { [ -e "$1/bin/knoted-intelligence" ] || [ -e "$1/share/knoted/intelligence" ]; }
}

guard_retired_runtime() {
  local require_stopped=${1:-0} pid status listeners
  for pid in "$proc_root"/[0-9]*; do
    [ -d "$pid" ] || continue
    status=0
    identify_process "$pid" || status=$?
    [ "$status" -eq 1 ] && continue
    if [ "$image_role" = knoted-intelligence ] || [ "$require_stopped" = 1 ]; then
      fail "旧运行进程 $image_role（PID ${pid##*/}）仍存在，请先完成旧编排停机和执行退出检查" 2
    fi
  done

  if pc process get knoted-intelligence >/dev/null 2>&1; then
    fail "旧进程编排仍在运行，请先使用旧编排停止本实例" 2
  fi
  [ "$require_stopped" = 1 ] || return 0

  if instance_running; then
    fail "旧进程编排仍在运行，请先使用旧编排停止本实例" 2
  fi
  listeners=$(ss -H -ltn "sport = :$control_port") || fail "无法核对旧编排控制端口 $control_port" 2
  [ -z "$listeners" ] || fail "旧编排控制端口 $control_port 仍有监听，请先停止本实例" 2
  cmp -s "$repository/process-compose.yaml" "$instance_repository/process-compose.yaml" ||
    fail "请先将主检出的 process-compose.yaml 同步到新版编排再切换" 2
}

# 两边 gid 写进输出：换组重跑是恢复强判据的入口。
describe_process_gid() {
  local caller_gid gid
  caller_gid=$(id -g 2>/dev/null || true)
  gid=$(awk '/^Gid:/{print $2}' "$1/status" 2>/dev/null || true)
  printf '调用者 gid=%s，进程 Gid=%s' "${caller_gid:-?}" "${gid:-?}"
}

sg_hint() {
  printf "用该进程主组重跑可恢复 exe 判据：sg <进程主组> -c 'pnpm deploy:dev'（组名见 getent group <gid>）"
}

note_image_source() {
  [ "$1" = cmdline ] || return 0
  [ "$criterion_notice" = 1 ] && return 0
  criterion_notice=1
  log "/proc/<pid>/exe 不可读（跨 uid/gid 的 ptrace 限制），改用 /proc/<pid>/cmdline 判定进程映像"
}

# 现有进程若只能给到别名，重启后必须靠 launcher 固定 argv[0]。pcwrap daemon 用的是
# 主检出，不是本脚本所在的任务 worktree；不能让旧 launcher 把问题带到切换之后。
preflight_process_evidence() {
  local pid status expected_root launcher_paths current_root=$runtime_parent/current
  for pid in "$proc_root"/[0-9]*; do
    [ -d "$pid" ] || continue
    status=0
    identify_process "$pid" || status=$?
    [ "$status" -eq 1 ] && continue
    [ "$status" -eq 2 ] || continue
    # 这里只检查 launcher 能否解析链接；历史产物目录可能仍是短 SHA。
    expected_root=$(readlink -f -- "$current_root" 2>/dev/null) || expected_root=""
    if [ -L "$current_root" ] && [ -n "$expected_root" ] && [ "$expected_root" != "$current_root" ] &&
      launcher_paths=$(KNOTED_RUNTIME_ROOT="$current_root" bash "$instance_repository/tools/dev/pc/service.sh" paths 2>/dev/null) &&
      grep -Fxq -- "runtime_root=$expected_root" <<<"$launcher_paths"; then
      log "现有进程只提供运行根别名（$(describe_process_gid "$pid")）；主检出的 launcher 能为新进程固定产物路径"
      return 0
    fi
    fail "无法从现有进程验证具体产物路径（$(describe_process_gid "$pid")）。请先更新主检出 $instance_repository 的 launcher；也可$(sg_hint)。未切换指针" 2
  done
}

# 进程映像必须已经来自新产物。
check_processes() {
  local pid status seen_api=0 seen_worker=0
  process_problem=""
  process_unverifiable=0
  for pid in "$proc_root"/[0-9]*; do
    [ -d "$pid" ] || continue
    status=0
    identify_process "$pid" || status=$?
    [ "$status" -eq 1 ] && continue
    if [ "$status" -eq 2 ]; then
      process_unverifiable=1
      process_problem="无法判定进程 ${pid##*/} 的映像来自哪个产物目录（$image_source；$(describe_process_gid "$pid")）；$(sg_hint)"
      return 1
    fi
    note_image_source "$image_source"
    case "$image_role" in
    knoted-api) seen_api=1 ;;
    knoted-worker) seen_worker=1 ;;
    knoted-intelligence)
      process_problem="旧运行进程 knoted-intelligence（PID ${pid##*/}）仍存在"
      return 1
      ;;
    esac
    if [ "$image_dir" != "$target" ]; then
      process_problem="$image_role 仍运行在 $image_dir（期望 $target）"
      return 1
    fi
  done
  test "$seen_api" = 1 || {
    process_problem="没有找到运行中的 knoted-api"
    return 1
  }
  test "$seen_worker" = 1 || {
    process_problem="没有找到运行中的 knoted-worker"
    return 1
  }
  return 0
}

https_problem=""
check_https() {
  local status
  if ! status=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 8 "https://$domain:$https_port/") ||
    [[ "$status" != 200 ]]; then
    https_problem="HTTPS 入口不可用：https://$domain:$https_port/（HTTP ${status:-未知}）"
    return 1
  fi
}

# 切换后失败的处理分两种：迁移集合没变，回退指针并按同样方式重启是安全的；
# 迁移已跑过就不能只回退指针（旧产物的迁移集合对不上已应用的 schema，sqlx 会拒绝
# 启动），此时保留新产物并打印人工入口，避免留下“指针旧、数据库新”的死锁。
fail_after_switch() {
  local reason=$1
  if [ "$process_unverifiable" = 1 ]; then
    log "$reason"
    log "无法用进程映像证明新旧版本，指针保持指向 $target（不回滚：判断依据不会因回滚改变，只会扩大空窗）"
    log "排查入口：pnpm pc process list / pnpm pc process logs knoted-api -n 200 / pnpm pc process logs knoted-worker -n 200"
    exit 1
  fi
  if [ "$legacy_transition" = 1 ]; then
    log "$reason；跨编排切换保留新指针与两份产物。请排查新编排后重试启动"
    exit 1
  fi
  if [ "$migrations_changed" = 1 ]; then
    log "$reason；本次部署包含迁移，指针保持指向 $target（只回退指针会与已应用的 schema 冲突）"
    log "排查入口：pnpm pc process logs knoted-api -n 200 / pnpm pc process logs knoted-worker -n 200"
    exit 1
  fi
  if [ -z "$previous" ] || [ ! -d "$previous" ]; then
    log "$reason；没有可回退的上一个产物，指针保持指向 $target"
    log "排查入口：pnpm pc process logs knoted-api -n 200"
    exit 1
  fi
  log "$reason；复位 current → $previous 并重启回旧版本"
  switch_current "$previous"
  restart_running || log "复位后的重启返回非零，请手工检查（pnpm pc process list）"
  exit 1
}

verify_deployed() {
  check_https || {
    log "$https_problem"
    return 1
  }
  log "验证通过：$revision"
}

# 只认部署工具的目录命名、匹配的版本标记及清单。清理时不需要读取旧文件的摘要。
is_release_directory() {
  local artifact=$1 name=${1##*/} marker recorded digest path
  [[ -d "$artifact" && ! -L "$artifact" ]] || return 1
  if [[ "$name" =~ ^([0-9a-f]{40})(\.incomplete\.[0-9]{8}T[0-9]{6}Z\.[0-9]+)?$ ]]; then
    name=${BASH_REMATCH[1]}
  elif [[ "$name" =~ ^(\.staging-)?([0-9a-f]{12})$ ]]; then
    name=${BASH_REMATCH[2]}
  else
    return 1
  fi

  marker=$artifact/share/knoted/REVISION
  [[ $(realpath -e -- "$marker" 2>/dev/null) == "$marker" && -f "$marker" ]] || return 1
  IFS= read -r recorded <"$marker" || return 1
  [[ "$recorded" =~ ^[0-9a-f]{40}$ && "$recorded" == "$name"* ]] || return 1
  [[ -f "$artifact/SHA256SUMS" && ! -L "$artifact/SHA256SUMS" ]] || return 1
  for path in "$artifact/bin/knoted-api" "$artifact/share/knoted/web/index.html"; do
    [[ $(realpath -e -- "$path" 2>/dev/null) == "$path" && -f "$path" ]] || return 1
  done
  digest=$(sha256sum <"$marker") || return 1
  grep -Fxq -- "${digest%% *}  share/knoted/REVISION" "$artifact/SHA256SUMS"
}

prune_old_artifacts() {
  local artifact pruned=0 mounts mount
  [[ $(readlink -f -- "$runtime_parent/current") == "$target" ]] ||
    fail "current 已变动，跳过历史产物清理"
  guard_retired_runtime
  mounts=$(findmnt --kernel --noheadings --raw --output TARGET) ||
    fail "无法核对运行根挂载点"
  while IFS= read -r mount; do
    printf -v mount '%b' "$mount"
    [[ "$mount" != "$runtime_parent/"* ]] || fail "运行根下存在挂载点：$mount"
  done <<<"$mounts"
  for artifact in "$runtime_parent"/* "$runtime_parent"/.staging-*; do
    [[ "$artifact" != "$target" ]] || continue
    is_release_directory "$artifact" || continue
    # 旧产物有将目录设为只读的情况；只向待删除的目录恢复属主写权限。
    if ! find -P "$artifact" -xdev -type d ! -perm -u=w -exec chmod u+w -- {} + ||
      ! rm -rf --one-file-system -- "$artifact"; then
      fail "当前版本已验证，清理旧产物失败：$artifact"
    fi
    ((pruned += 1))
  done
  log "已清理 $pruned 份旧部署产物"
}

main() {
  local prune_only=0 legacy_transition=0
  case "${1:-}" in
  -h | --help | help)
    usage
    exit 0
    ;;
  --prune-current)
    [[ $# == 1 ]] || fail "--prune-current 不接受其他参数" 2
    prune_only=1
    ;;
  esac
  guard_worktree
  require_tools
  if [[ "$prune_only" == 0 ]]; then
    resolve_revision "$@"
  fi
  mkdir -p "$instance_root" "$runtime_parent"
  exec {lock_fd}>"$lock_file"
  flock -n "$lock_fd" || fail "另一个部署正在进行（$lock_file）" 2
  # 不同实例配置可能指向同一运行根，按该目录加锁保护 current 和候选产物。
  exec {runtime_lock_fd}<"$runtime_parent"
  flock -n "$runtime_lock_fd" || fail "另一个部署正在使用运行根：$runtime_parent" 2

  previous=$(readlink -f "$runtime_parent/current" 2>/dev/null || true)
  if [[ "$prune_only" == 1 ]]; then
    revision=${previous##*/}
    [[ "$revision" =~ ^[0-9a-f]{40}$ && "$previous" == "$runtime_parent/$revision" ]] ||
      fail "current 必须指向运行根内的完整提交目录"
  fi
  target=$runtime_parent/$revision
  [[ ! -L "$target" ]] || fail "产物路径是软链接：$target"
  if [ "$previous" != "$target" ] && legacy_artifact "$previous"; then
    legacy_transition=1
  fi
  guard_retired_runtime "$legacy_transition"

  if [ "$previous" = "$target" ]; then
    log "已是最新：current 已指向 $revision，只做验证"
    verify_artifact
    wait_for_convergence "$ready_timeout" || fail "实例未收敛：${process_problem:-readiness 未通过}（pnpm pc process list / pnpm pc process logs knoted-api）"
    verify_deployed || return 1
    prune_old_artifacts
    return 0
  fi

  preflight_process_evidence
  # 构建前就要有运行变量：Vite 用 KNOTED_OFFICE_ORIGIN 决定是否编入 Office。
  resolve_runtime_variables

  build_or_reuse_artifact
  compare_migrations

  # 构建期间可能重新启动旧编排，切换前再次确认退出。
  guard_retired_runtime "$legacy_transition"
  if [ "$migrations_changed" = 1 ] && [ "$legacy_transition" = 0 ]; then
    stop_instance || fail "停止实例失败：指针未切换，实例仍在旧版本（pnpm pc process list）"
  fi
  switch_current "$target"
  log "current → $target"

  if [ "$migrations_changed" = 1 ] || [ "$legacy_transition" = 1 ]; then
    start_instance || fail_after_switch "pnpm pc up -D 失败"
  else
    restart_running || fail_after_switch "重启失败"
  fi

  wait_for_convergence "$ready_timeout" || fail_after_switch "${process_problem:-readiness 超时}"
  verify_deployed || fail_after_switch "验证未通过"
  prune_old_artifacts
  log "完成：$revision 已上线（https://$domain:$https_port/）"
}

# 已存在的产物不可用（构建中断、磁盘损坏）时要能重试：移到旁路后重建。
build_or_reuse_artifact() {
  if [ -d "$target" ] && artifact_ok; then
    log "复用已构建产物：$target"
    return 0
  fi
  if [ -d "$target" ]; then
    local aside=$target.incomplete.$(date -u +%Y%m%dT%H%M%SZ).$$
    log "已有产物不可用（$artifact_problem），移到 $aside 后重建"
    mv -- "$target" "$aside"
  fi
  log "构建固定产物 $revision（CARGO_BUILD_JOBS=$cargo_jobs，KNOTED_BUILD_ROOT=$build_root）"
  mkdir -p "$build_root"
  CARGO_BUILD_JOBS=$cargo_jobs KNOTED_BUILD_ROOT=$build_root \
    "$build_cmd" "$revision" "$target" || fail "构建失败：$revision（日志见 $build_root）"
  verify_artifact
}

main "$@"
