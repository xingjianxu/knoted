#!/usr/bin/env bash
# Usage: bash tests/fixtures/t71/instance.sh pnpm exec playwright test tests/e2e/t71.spec.ts
# Requires a fixed T71_ARTIFACT, T71_PGVECTOR, KNOTED_INTELLIGENCE_BGE_M3_DIR, and
# the release's KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR. Optional T71_MEILI_BIN,
# T71_PG_BIN and T71_CHROMIUM select installed binaries; pgvector requires PostgreSQL 18.
# The normal Playwright webServer also reserves 4177 for its component test fixture.
set -euo pipefail
umask 077
artifact=${T71_ARTIFACT:?set T71_ARTIFACT to the absolute path of a fixed release}
pg=${T71_PG_BIN:-$(pg_config --bindir)}
vector=${T71_PGVECTOR:?set T71_PGVECTOR to the installed pgvector prefix}
meili=${T71_MEILI_BIN:-meilisearch}
fixtures=$(cd "$(dirname "$0")" && pwd)
task=/data/knoted-dev/tasks/t71
mkdir -p "$task/tmp"
run=$(mktemp -d "$task/tmp/instance.XXXXXXXX")
export T71_RUN="$run"
test -z "$(ss -Hltn '( sport >= :17820 and sport <= :17829 )')"
pids=()
cleanup() {
  local status=$?
  trap - EXIT
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  for pid in "${pids[@]}"; do wait "$pid" 2>/dev/null || true; done
  if test -s "$run/pg/postmaster.pid"; then "$pg/pg_ctl" -D "$run/pg" -m fast -w stop >/dev/null; fi
  rm -f "$run"/{model.key,meili.key,credentials.toml,tls.key}
  exit "$status"
}
trap cleanup EXIT
mkdir "$run"/{socket,files,staging,assets,python,cache,tmp}
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
extension_control_path = '$vector/share/postgresql:\$system'
dynamic_library_path = '$vector/lib:\$libdir'
CONF
"$pg/pg_ctl" -D "$run/pg" -l "$run/postgres.log" -w start >/dev/null
export PGHOST="$run/socket" PGPORT=17828 PGDATABASE=t71
"$pg/createdb" t71
db="postgres://$(id -un)@localhost/t71?host=$run/socket&port=17828"
DATABASE_URL="$db" sqlx migrate run --source "$artifact/share/knoted/migrations" > "$run/migrate.log"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -noenc -days 1 -subj '/CN=t71.knoted.test' \
  -addext 'subjectAltName=DNS:t71.knoted.test' -out "$run/tls.crt" -keyout "$run/tls.key" >/dev/null 2>&1
openssl rand -hex 32 > "$run/meili.key"
openssl rand -hex 32 > "$run/model.key"
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
rag_chunk_index_uid = "t71_chunks"
api_key_credential = "meilisearch"
[intelligence]
base_url = "http://127.0.0.1:17826"
[vectors]
enabled = true
[chat.local]
base_url = "http://127.0.0.1:17825"
model = "t71-fixture"
credential_file = "$run/model.key"
CONFIG
MEILI_MASTER_KEY=$(cat "$run/meili.key") "$meili" --http-addr 127.0.0.1:17824 --db-path "$run/meili" \
  --env production --no-analytics > "$run/meili.log" 2>&1 &
pids+=("$!")
node "$fixtures/search-gate.mjs" > "$run/gate.log" 2>&1 &
pids+=("$!")
python3 -B "$fixtures/provider.py" "$run/model.key" 17825 > "$run/provider.log" 2>&1 &
pids+=("$!")
export KNOTED_INTELLIGENCE_BGE_M3_DIR=${KNOTED_INTELLIGENCE_BGE_M3_DIR:?set the fixed BGE-M3 model directory}
env KNOTED_INTELLIGENCE_HOST=127.0.0.1 KNOTED_INTELLIGENCE_PORT=17826 \
  KNOTED_INTELLIGENCE_STATE_DIR="$run/python" KNOTED_INTELLIGENCE_STAGING_DIR="$run/staging" \
  XDG_CACHE_HOME="$run/cache" TMPDIR="$run/tmp" KNOTED_INTELLIGENCE_DOCLING_DEVICE=cpu \
  KNOTED_INTELLIGENCE_GPU_SLOTS=1 KNOTED_EMBED_GPU_UUID= CUDA_VISIBLE_DEVICES= \
  "$artifact/bin/knoted-intelligence" > "$run/python.log" 2>&1 &
pids+=("$!")
for url in http://127.0.0.1:17826/internal/v1/health http://127.0.0.1:17823/health; do
  curl -fsS --retry 30 --retry-connrefused --retry-delay 1 --max-time 3 "$url" >/dev/null
done
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
"$@"
