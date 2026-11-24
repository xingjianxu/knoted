#!/usr/bin/env bash
# Run the workspace suite with a dedicated, disposable Meilisearch.
set -euo pipefail
umask 077

test_root=/data/knoted_dev
address=127.0.0.1:17830
meili=$(command -v "${KNOTED_TEST_MEILI_BIN:-meilisearch}") || {
  printf 'Meilisearch is required. Set KNOTED_TEST_MEILI_BIN to its executable.\n' >&2
  exit 2
}
meili=$(realpath -e -- "$meili")
mkdir -p "$test_root"
exec {lock_fd}>"$test_root/.rust-test.lock"
flock -n "$lock_fd" || {
  printf 'Another Rust test run holds the Meilisearch port.\n' >&2
  exit 2
}
listeners=$(ss -Hltn 'sport = :17830')
if [[ -n "$listeners" ]]; then
  printf 'The test Meilisearch port %s is already in use.\n' "$address" >&2
  exit 2
fi

run=$(mktemp -d "$test_root/rust.XXXXXXXX")
meili_pid=
test_pid=
cleanup() {
  local status=$? pid_file pg_data
  if [[ -n "$test_pid" ]]; then
    kill -TERM -- "-$test_pid" 2>/dev/null || true
    wait "$test_pid" 2>/dev/null || true
  fi
  if [[ -n "$meili_pid" ]]; then
    kill "$meili_pid" 2>/dev/null || true
    wait "$meili_pid" 2>/dev/null || true
  fi
  # pg_ctl starts the T69 postmaster in its own session.
  for pid_file in "$run/tmp"/knoted-t69-postgres-*/pg/postmaster.pid; do
    [[ -f "$pid_file" ]] || continue
    pg_data=${pid_file%/postmaster.pid}
    if ! pg_ctl -D "$pg_data" -m fast -w stop >/dev/null; then
      printf 'Cannot stop isolated PostgreSQL in %s.\n' "$pg_data" >&2
      exit 1
    fi
  done
  rm -rf -- "$run"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir "$run/tmp"
export TMPDIR="$run/tmp"
export KNOTED_TEST_MEILI_URL="http://$address"

(
  cd "$run"
  exec env -i PATH="$PATH" HOME="$run" TMPDIR="$TMPDIR" "$meili" \
    --http-addr "$address" --db-path "$run/meili" --env development --no-analytics
) >"$run/meili.log" 2>&1 {lock_fd}>&- &
meili_pid=$!

if ! curl --noproxy '*' -fsS --retry 15 --retry-connrefused --retry-delay 1 \
  --max-time 2 "$KNOTED_TEST_MEILI_URL/health" >/dev/null 2>&1 ||
  ! kill -0 "$meili_pid" 2>/dev/null; then
  cat "$run/meili.log" >&2
  exit 1
fi

setsid cargo test --workspace --locked "$@" {lock_fd}>&- &
test_pid=$!
wait "$test_pid"
test_pid=
