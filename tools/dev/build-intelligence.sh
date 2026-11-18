#!/usr/bin/env bash
# Fixed committed Python runtime. No venv/editable runtime or model provisioning.
set -euo pipefail
export LC_ALL=C
if [[ $# != 2 || ! $1 =~ ^[0-9a-f]{40}$ ]]; then
  echo 'usage: build-intelligence.sh <full-commit-sha> <new-output-directory>' >&2
  exit 2
fi
revision=$1
output=$(realpath -m -- "$2")
repository=$(git rev-parse --show-toplevel)
[[ $(git rev-parse --verify "$revision^{commit}") == "$revision" ]]
[[ ! -e $output ]] || { echo 'output already exists' >&2; exit 2; }
task_root=${KNOTED_BUILD_ROOT:-/data/knoted-dev/tasks/t47}
mkdir -p "$task_root"/{tmp,cache,build}
export TMPDIR="$task_root/tmp" UV_CACHE_DIR="$task_root/cache/uv"
export XDG_CACHE_HOME="$task_root/cache" PYTHONDONTWRITEBYTECODE=1
source_dir=$(mktemp -d "$task_root/build/intelligence-source.XXXXXXXX")
git -C "$repository" archive "$revision" | tar -x -C "$source_dir"
project="$source_dir/services/intelligence"
# Resolve the installed 3.12 executable, never copy uv's mutable alias symlink.
interpreter=$(realpath -- "${KNOTED_BUILD_PYTHON:-$(uv python find --no-python-downloads --project "$project")}")
base=$($interpreter -I -B -c 'import sys; assert sys.version_info[:2] == (3, 12); assert sys.prefix == sys.base_prefix; print(sys.base_prefix)')
base=$(realpath -- "$base")
root="$output/share/knoted/intelligence"
mkdir -p "$root" "$output/bin"
cp -rL -- "$base" "$root/python"
# The supplied interpreter must be a clean base install, not a development env.
site="$root/python/lib/python3.12/site-packages"
rm -rf -- "$site"
mkdir -p "$site"
uv export --project "$project" --locked --no-dev --no-editable --no-emit-project --output-file "$root/requirements.txt"
uv pip install --python "$root/python/bin/python3.12" --target "$site" \
  --require-hashes --no-deps --only-binary :all: --link-mode copy -r "$root/requirements.txt"
# Dependency console scripts contain absolute build paths; the service only uses imports.
rm -rf -- "$site/bin"
cp -r -- "$project/src/knoted_intelligence" "$site/knoted_intelligence"
cp -- "$project/uv.lock" "$root/uv.lock"
find "$root/python" -type d -name __pycache__ -prune -exec rm -rf -- {} +
# Wheels may contain internal file links. Materialize each, rejecting links out of tree.
"$root/python/bin/python3.12" -I -B - "$root" "$revision" "$interpreter" <<'PY'
import hashlib
import json
from pathlib import Path
import sys
root = Path(sys.argv[1])
for path in root.rglob('*'):
    if path.is_symlink():
        target = path.resolve(strict=True)
        if not target.is_relative_to(root) or not target.is_file():
            raise SystemExit(f'non-internal file link: {path}')
        contents = target.read_bytes()
        mode = target.stat().st_mode
        path.unlink()
        path.write_bytes(contents)
        path.chmod(mode)
    if not path.is_dir() and not path.is_file():
        raise SystemExit(f'non-regular file: {path}')
def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()
(root / 'BUILD.json').write_text(json.dumps({
    'revision': sys.argv[2], 'python_version': sys.version,
    'interpreter_sha256': sha(sys.executable),
    'source_interpreter_sha256': sha(sys.argv[3]),
    'uv_lock_sha256': sha(root / 'uv.lock'),
}, indent=2) + '\n')
PY
cat > "$output/bin/knoted-intelligence" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
: "${KNOTED_INTELLIGENCE_STATE_DIR:?set external state directory}"
: "${KNOTED_INTELLIGENCE_STAGING_DIR:?set external staging directory}"
: "${XDG_CACHE_HOME:?set external cache directory}"
: "${TMPDIR:?set external temporary directory}"
for path in "$KNOTED_INTELLIGENCE_STATE_DIR" "$KNOTED_INTELLIGENCE_STAGING_DIR" "$XDG_CACHE_HOME" "$TMPDIR"; do
  resolved=$(realpath -m -- "$path")
  case "$resolved/" in "$root/"*) echo 'writable paths must be outside artifact' >&2; exit 2;; esac
done
export PYTHONDONTWRITEBYTECODE=1
export HF_HOME="$XDG_CACHE_HOME/huggingface" TORCH_HOME="$XDG_CACHE_HOME/torch"
export MPLCONFIGDIR="$XDG_CACHE_HOME/matplotlib"
exec "$root/share/knoted/intelligence/python/bin/python3.12" -I -B -c 'from knoted_intelligence.app import main; main()' "$@"
SH
chmod 0755 "$output/bin/knoted-intelligence"
printf '%s\n' "$revision" > "$output/share/knoted/REVISION"
cd "$output"
[[ -z $(find bin share ! -type f ! -type d -print -quit) ]]
find bin share -type f -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
sha256sum --check --quiet SHA256SUMS
printf 'artifact=%s\nrevision=%s\n' "$output" "$revision"
