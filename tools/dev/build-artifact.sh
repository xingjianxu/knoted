#!/usr/bin/env bash
# Build only committed sources. Persistent services must never run a worktree.
set -euo pipefail
export LC_ALL=C
if [[ $# != 2 || ! $1 =~ ^[0-9a-f]{40}$ ]]; then
  echo 'usage: build-artifact.sh <full-commit-sha> <new-output-directory>' >&2
  exit 2
fi
revision=$1
output=$(realpath -m -- "$2")
repository=$(git rev-parse --show-toplevel)
[[ $(git rev-parse --verify "$revision^{commit}") == "$revision" ]]
if [[ -e $output ]]; then
  echo 'output directory already exists; refusing to overwrite' >&2
  exit 2
fi
task_root=${KNOTED_BUILD_ROOT:-/data/knoted-dev/tasks/t21}
mkdir -p "$task_root"/{tmp,cache,build}
export TMPDIR="$task_root/tmp"
export CARGO_TARGET_DIR="$task_root/build/rust-target"
export npm_config_cache="$task_root/cache/npm"
export XDG_CACHE_HOME="$task_root/cache"
source_dir=$(mktemp -d "$task_root/build/source.XXXXXXXX")
# Retain isolated source and build logs for diagnosis; no secrets are copied.
git -C "$repository" archive "$revision" | tar -x -C "$source_dir"
cd "$source_dir"
cargo build --locked --release -p knoted-api
pnpm install --frozen-lockfile --store-dir "$task_root/cache/pnpm-store"
pnpm --filter @knoted/web build
mkdir -p "$output/bin" "$output/share/knoted"
install -m 0755 "$CARGO_TARGET_DIR/release/knoted-api" "$output/bin/knoted-api"
cp -r apps/web/dist "$output/share/knoted/web"
cp -r migrations "$output/share/knoted/migrations"
printf '%s\n' "$revision" > "$output/share/knoted/REVISION"
cd "$output"
find bin share -type f -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
sha256sum --check SHA256SUMS
printf 'artifact=%s\nrevision=%s\n' "$output" "$revision"
