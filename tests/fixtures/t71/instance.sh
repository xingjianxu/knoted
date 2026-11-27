#!/usr/bin/env bash
# Usage: bash tests/fixtures/t71/instance.sh pnpm exec playwright test tests/e2e/t71.spec.ts
# Requires a fixed T71_ARTIFACT. Optional T71_MEILI_BIN, T71_PG_BIN and
# T71_CHROMIUM select installed binaries.
set -euo pipefail
umask 077
export T71_ARTIFACT
T71_ARTIFACT=$(realpath -e -- "${T71_ARTIFACT:?set T71_ARTIFACT to a fixed release}")
artifact=$T71_ARTIFACT
pg=${T71_PG_BIN:-$(pg_config --bindir)}
meili=$(command -v "${T71_MEILI_BIN:-meilisearch}")
meili=$(realpath -e -- "$meili")
repo=$(cd "$(dirname "$0")/../../.." && pwd)
KNOTED_RUNTIME_ROOT="$artifact" bash "$repo/tools/dev/pc/service.sh" verify-artifact >/dev/null
task=/data/knoted_dev/t71
mkdir -p "$task"
exec {lock_fd}>"$task/.instance.lock"
flock -n "$lock_fd" || {
  printf 'Another T71 test instance is running.\n' >&2
  exit 2
}
listeners=$(ss -Hltn '( sport >= :17820 and sport <= :17829 )')
if [[ -n "$listeners" ]]; then
  printf 'One of the T71 test ports 17820–17829 is already in use.\n' >&2
  exit 2
fi
run=$(mktemp -d "$task/instance.XXXXXXXX")
export T71_RUN="$run"
pids=()
test_pid=
cleanup() {
  local status=$?
  trap - EXIT
  if [[ -n "$test_pid" ]]; then
    kill -TERM -- "-$test_pid" 2>/dev/null || true
    wait "$test_pid" 2>/dev/null || true
  fi
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  for pid in "${pids[@]}"; do wait "$pid" 2>/dev/null || true; done
  rm -f "$run"/{meili.key,credentials.toml,tls.key}
  if test -s "$run/pg/postmaster.pid"; then
    if ! "$pg/pg_ctl" -D "$run/pg" -m fast -w stop >/dev/null; then
      printf 'Cannot stop isolated PostgreSQL in %s\n' "$run" >&2
      exit 1
    fi
  fi
  if test -d "$run/files/agent-fixtures/t71"; then
    chmod u+w "$run/files" "$run/files/agent-fixtures" "$run/files/agent-fixtures/t71"
  fi
  rm -rf -- "$run"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir "$run"/{socket,files,staging,assets,tmp}
export TMPDIR="$run/tmp"
sample_dir="$run/files/agent-fixtures/t71"
mkdir -p "$sample_dir"
printf '观测记录：紫檀标本的识别词为 violetamber。\n' > "$sample_dir/observations.txt"
chmod 0444 "$sample_dir/observations.txt"
chmod 0555 "$run/files" "$run/files/agent-fixtures" "$sample_dir"
"$pg/initdb" -D "$run/pg" --auth-local=peer --auth-host=reject --no-instructions > "$run/initdb.log"
cat >> "$run/pg/postgresql.conf" <<CONF
listen_addresses = ''
port = 17828
unix_socket_directories = '$run/socket'
CONF
"$pg/pg_ctl" -D "$run/pg" -l "$run/postgres.log" -w start >/dev/null
export PGHOST="$run/socket" PGPORT=17828 PGDATABASE=t71
"$pg/createdb" t71
db="postgres://$(id -un)@localhost/t71?host=$run/socket&port=17828"
DATABASE_URL="$db" sqlx migrate run --source "$artifact/share/knoted/migrations" > "$run/migrate.log"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -noenc -days 1 -subj '/CN=t71.knoted.test' \
  -addext 'subjectAltName=DNS:t71.knoted.test' -out "$run/tls.crt" -keyout "$run/tls.key" >/dev/null 2>&1
openssl rand -hex 32 > "$run/meili.key"
printf '[secrets]\nmeilisearch = "%s"\n' "$(cat "$run/meili.key")" > "$run/credentials.toml"
cat > "$run/app.toml" <<CONFIG
[server]
bind = "127.0.0.1:17822"
[database]
url = "$db"
[assets]
root = "$run/assets"
[auth]
rp_id = "knoted.test"
origin = "https://t71.knoted.test:17820"
[files]
enabled = true
root = "$run/files"
staging_root = "$run/staging"
rescan_seconds = 300
[credentials]
credential_file = "$run/credentials.toml"
[search]
enabled = true
base_url = "http://127.0.0.1:17823"
index_uid = "t71_documents"
api_key_credential = "meilisearch"
CONFIG
(
  cd "$run"
  exec env -i PATH="$PATH" HOME="$run" TMPDIR="$TMPDIR" MEILI_MASTER_KEY="$(cat "$run/meili.key")" \
    "$meili" --http-addr 127.0.0.1:17823 --db-path "$run/meili" --env production --no-analytics
) > "$run/meili.log" 2>&1 &
pids+=("$!")
curl --noproxy '*' -fsS --retry 30 --retry-connrefused --retry-delay 1 --max-time 3 \
  http://127.0.0.1:17823/health >/dev/null
"$artifact/bin/knoted-api" --config "$run/app.toml" > "$run/api.log" 2>&1 &
pids+=("$!")
"$artifact/bin/knoted-worker" --config "$run/app.toml" > "$run/worker.log" 2>&1 &
pids+=("$!")
cat > "$run/Caddyfile" <<CONF
{
  admin off
  auto_https disable_redirects
  default_bind 127.0.0.1
}
https://t71.knoted.test:17820 {
  tls $run/tls.crt $run/tls.key
  handle /api/* {
    reverse_proxy 127.0.0.1:17822
  }
  handle /health/* {
    reverse_proxy 127.0.0.1:17822
  }
  handle {
    root * $artifact/share/knoted/web
    try_files {path} /index.html
    file_server
  }
}
CONF
XDG_DATA_HOME="$run/caddy-data" XDG_CONFIG_HOME="$run/caddy-config" \
  caddy run --config "$run/Caddyfile" --adapter caddyfile > "$run/caddy.log" 2>&1 &
pids+=("$!")
curl --noproxy '*' --resolve t71.knoted.test:17820:127.0.0.1 --cacert "$run/tls.crt" \
  -fsS --retry 20 --retry-all-errors --retry-delay 1 --max-time 3 https://t71.knoted.test:17820/health/ready >/dev/null
printf 'Isolated T71 instance ready in %s\n' "$run"
setsid "$@" {lock_fd}>&- &
test_pid=$!
wait "$test_pid"
test_pid=
