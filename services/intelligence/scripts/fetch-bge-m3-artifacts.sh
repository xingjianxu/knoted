#!/usr/bin/env bash
# BGE-M3 snapshot provisioner for the machine-level shared root.
#
#   ./fetch-bge-m3-artifacts.sh                      # provision into /data/llm/bge-m3/<revision>
#   ./fetch-bge-m3-artifacts.sh /some/root           # provision into /some/root/<revision>
#   ./fetch-bge-m3-artifacts.sh --verify             # verify only, never write (default root)
#   ./fetch-bge-m3-artifacts.sh /some/root --verify  # verify only, never write
#   ./fetch-bge-m3-artifacts.sh --print-revision-id  # print the locked directory name
#   KEEP_BIN=1 ./fetch-bge-m3-artifacts.sh           # keep the upstream pytorch_model.bin
#
# Layout: <target-root>/<revision-id>/ IS the directory passed to
# SentenceTransformer. Revision directories are immutable; MANIFEST.json is
# generated here and must keep the shape bge.manifest() validates.
#
# Upstream publishes only `pytorch_model.bin` (fp32 LFS). The pinned revision is
# downloaded, checked byte for byte, then converted to `model.safetensors` and
# verified with a tensor-level digest (name-sorted, container-order independent).
# Conversion needs torch+safetensors; point PYTHON at an interpreter that has
# them. Verification needs only the python3 standard library.
set -euo pipefail

REPO="BAAI/bge-m3"
REV="5617a9f61b028005a4858fdac845db406aefb181"
# Must equal embedding.bge.snapshot_dirname(); a test asserts that.
REVISION_ID="bge-m3-${REV:0:8}-dense"
DEFAULT_ROOT=/data/llm/bge-m3

ENDPOINT="${HF_ENDPOINT:-https://huggingface.co}"
PYTHON="${PYTHON:-python3}"
KEEP_BIN="${KEEP_BIN:-0}"

VERIFY_ONLY=0
PRINT_REVISION=0
TARGET_ROOT=""
for arg in "$@"; do
  case "$arg" in
    --verify|-c) VERIFY_ONLY=1 ;;
    --print-revision-id) PRINT_REVISION=1 ;;
    -h|--help) sed -n '2,21p' "$0"; exit 0 ;;
    -*) echo "unknown option: $arg" >&2; exit 2 ;;
    *) TARGET_ROOT="$arg" ;;
  esac
done

if [ "$PRINT_REVISION" = 1 ]; then
  printf '%s\n' "$REVISION_ID"
  exit 0
fi

TARGET_ROOT="${TARGET_ROOT:-$DEFAULT_ROOT}"
DEST="$TARGET_ROOT/$REVISION_ID"

# --verify must not create anything, so the root and the lock exist only when
# provisioning. One writer at a time: two consumers may provision the same
# revision concurrently.
if [ "$VERIFY_ONLY" = 0 ]; then
  install -d -m 0755 "$TARGET_ROOT" "$DEST"
  exec 9>"$TARGET_ROOT/.provision.lock"
  flock 9
fi

# 上游文件（相对路径 字节数 sha256）。LFS 文件的 sha256 == HF API 的 lfs.oid。
FILES=$(cat <<'EOF'
config.json 687 26159e7ad065073448460117eb24b7a4572f6f4e78eadff65dc0a11c052449fa
config_sentence_transformers.json 123 1eef72430e7194a1e59680e635aed81ffa083f05668dbc5bb1c56c04c0999c38
modules.json 349 84e40c8e006c9b1d6c122e02cba9b02458120b5fb0c87b746c41e0207cf642cf
sentence_bert_config.json 54 eb9b44b13c0f52a3b3685c3b1cbdea1ba8b04bea123b98f61610048940776eb1
1_Pooling/config.json 191 e54c164a07274f2eb45bb724f54a79d1efcc90c41573887cd9a29aeee0597352
special_tokens_map.json 964 8c785abebea9ae3257b61681b4e6fd8365ceafde980c21970d001e834cf10835
tokenizer_config.json 444 a62b2b6784f990259fddef5f16388693a8043be4f69179e6a5257eeb3f9abac4
tokenizer.json 17098108 21106b6d7dab2952c1d496fb21d5dc9db75c28ed361a05f5020bbba27810dd08
sentencepiece.bpe.model 5069051 cfc8146abe2a0488e9e2a0c56de7952f7c11ab059eca145a0a727afce0db2865
sparse_linear.pt 3516 45c93804d2142b8f6d7ec6914ae23a1eee9c6a1d27d83d908a20d2afb3595ad9
colbert_linear.pt 2100674 19bfbae397c2b7524158c919d0e9b19393c5639d098f0a66932c91ed8f5f9abb
README.md 15822 0b81ccf9134e5874d620a86e6905062ea999e779c34eb1a7e65eaeb7fe00e450
EOF
)

# 派生文件：上游 pytorch_model.bin -> model.safetensors
BIN="pytorch_model.bin"
BIN_SIZE=2271145830
BIN_SHA256=b5e0ce3470abf5ef3831aa1bd5553b486803e83251590ab7ff35a117cf6aad38
SAFE="model.safetensors"
SAFE_TENSORS=391
SAFE_TENSOR_BYTES=2271019008
SAFE_DIGEST=a10a594cfabcdb18d33a54bb535a0d273c24d3a76fb67e2b9321c27213885999
SAFE_SHA256=5632ea99fd6c2dc9c1d0837660b581ccbff68b0eca17f95d86373e16385d2f69

sha256_of() { sha256sum "$1" | cut -d' ' -f1; }
size_of() { stat -c%s "$1"; }
log_ok() { printf 'ok      %s\n' "$*"; }
log_get() { printf 'get     %s\n' "$*"; }

fetch() { # fetch <relpath> <tmppath>
  local rel="$1" tmp="$2"
  mkdir -p "$(dirname "$tmp")"
  curl -fL --progress-bar --retry 3 --retry-delay 2 -o "$tmp" "$ENDPOINT/$REPO/resolve/$REV/$rel"
}

verify_file() { # verify_file <path> <size> <sha>  -> 0 ok / 1 mismatch / 2 missing
  [ -f "$1" ] || return 2
  [ "$(size_of "$1")" = "$2" ] && [ "$(sha256_of "$1")" = "$3" ] && return 0
  return 1
}

verify_plain() { # verify_plain <rel> <size> <sha>
  verify_file "$DEST/$1" "$2" "$3"
}

# 纯标准库校验 safetensors 的 tensor 级摘要（不需要 torch）
verify_safetensors() { # verify_safetensors <file> <expected-digest> <expected-tensors> <expected-bytes>
  python3 - "$1" "$2" "$3" "$4" <<'PY'
import hashlib, json, struct, sys
path, expected, want_n, want_bytes = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
with open(path, "rb") as f:
    hlen = struct.unpack("<Q", f.read(8))[0]
    header = json.loads(f.read(hlen))
    base = 8 + hlen
    digest, total = hashlib.sha256(), 0
    names = sorted(k for k in header if k != "__metadata__")
    for name in names:
        off = header[name]["data_offsets"]
        f.seek(base + off[0])
        raw = f.read(off[1] - off[0])
        assert len(raw) == off[1] - off[0], f"truncated tensor {name}"
        total += len(raw)
        digest.update(f"{name}\t{header[name]['dtype']}\t{header[name]['shape']}\t".encode())
        digest.update(hashlib.sha256(raw).digest())
got = digest.hexdigest()
if len(names) != want_n or total != want_bytes or got != expected:
    sys.exit(f"model.safetensors mismatch: tensors={len(names)}/{want_n} bytes={total}/{want_bytes} digest={got} != {expected}")
print(f"        model.safetensors: {len(names)} tensors, {total} B, tensor digest ok")
PY
}

convert_bin() {
  if ! "$PYTHON" -c 'import torch, safetensors' >/dev/null 2>&1; then
    echo "ERROR: $PYTHON cannot import torch+safetensors; point PYTHON at a venv that can (see header)" >&2
    exit 1
  fi
  PYTHONPATH= "$PYTHON" - "$DEST/$BIN" "$DEST/$SAFE.tmp" "$REPO@$REV" <<'PY'
import sys, torch
from safetensors.torch import save_file
src, dst, src_id = sys.argv[1:4]
sd = torch.load(src, map_location="cpu", weights_only=True, mmap=True)
sd = {k: v.contiguous() for k, v in sd.items()}
save_file(sd, dst, metadata={"format": "pt", "source": src_id})
print(f"converted {src} -> {dst} ({len(sd)} tensors)")
PY
  chmod 0644 "$DEST/$SAFE.tmp"
  mv -f "$DEST/$SAFE.tmp" "$DEST/$SAFE"
}

echo "repo    = $REPO"
echo "rev     = $REV"
echo "dest    = $DEST"
echo "check   = $VERIFY_ONLY"

# 1) 上游文件
while read -r rel size sha; do
  [ -n "${rel:-}" ] || continue
  if verify_plain "$rel" "$size" "$sha"; then log_ok "$rel"; continue; fi
  if [ "$VERIFY_ONLY" = 1 ]; then
    echo "MISSING/CORRUPT: $DEST/$rel (expected $size B, sha256 $sha)" >&2
    exit 1
  fi
  log_get "$rel"
  rm -f "$DEST/$rel" "$DEST/$rel.part"
  fetch "$rel" "$DEST/$rel.part"
  if ! verify_file "$DEST/$rel.part" "$size" "$sha"; then
    echo "ERROR: $rel failed size/sha256 verification (kept as $rel.part)" >&2
    exit 1
  fi
  mv -f "$DEST/$rel.part" "$DEST/$rel"
  log_ok "$rel"
done <<< "$FILES"

# 2) 权重：model.safetensors（派生），必要时从 bin 转换
if [ -f "$DEST/$SAFE" ]; then
  verify_safetensors "$DEST/$SAFE" "$SAFE_DIGEST" "$SAFE_TENSORS" "$SAFE_TENSOR_BYTES"
  got=$(sha256_of "$DEST/$SAFE")
  if [ "$got" = "$SAFE_SHA256" ]; then log_ok "$SAFE"
  else echo "note    $SAFE file sha256=$got (recorded $SAFE_SHA256); tensor digest matches, converter version differs"; fi
else
  if [ "$VERIFY_ONLY" = 1 ]; then
    echo "MISSING: $DEST/$SAFE (run without --verify to convert from $BIN)" >&2
    exit 1
  fi
  if ! verify_plain "$BIN" "$BIN_SIZE" "$BIN_SHA256"; then
    log_get "$BIN"
    rm -f "$DEST/$BIN" "$DEST/$BIN.part"
    fetch "$BIN" "$DEST/$BIN.part"
    if ! verify_file "$DEST/$BIN.part" "$BIN_SIZE" "$BIN_SHA256"; then
      echo "ERROR: $BIN failed size/sha256 verification (kept as $BIN.part)" >&2
      exit 1
    fi
    mv -f "$DEST/$BIN.part" "$DEST/$BIN"
  fi
  log_ok "$BIN"
  convert_bin
  verify_safetensors "$DEST/$SAFE" "$SAFE_DIGEST" "$SAFE_TENSORS" "$SAFE_TENSOR_BYTES"
  log_ok "$SAFE"
  if [ "$KEEP_BIN" = 1 ]; then log_ok "$BIN (kept, KEEP_BIN=1)"; else rm -f "$DEST/$BIN"; echo "rm      $BIN (derived; KEEP_BIN=1 to keep)"; fi
fi

# Generated manifest: bge.manifest() validates schema/repo/revision and the
# runtime contract, so the shape here is a runtime contract, not documentation.
manifest_tmp="$DEST/MANIFEST.json.tmp"
{
  printf '{\n  "schema": "knoted-bge-m3-models/1",\n'
  printf '  "repo": "%s",\n  "revision": "%s",\n' "$REPO" "$REV"
  printf '  "endpoint": "%s",\n' "$ENDPOINT"
  printf '  "revision_dirname": "%s",\n' "$REVISION_ID"
  printf '  "runtime_contract": {"embedding_dim": 1024, "normalize_embeddings": true, "max_seq_length": 8192, "dtype": "float32"},\n'
  printf '  "files": [\n'
  first=1
  count=0
  bytes=0
  while read -r rel size sha; do
    [ -n "${rel:-}" ] || continue
    if [ "$first" = 1 ]; then first=0; else printf ',\n'; fi
    printf '    {"path": "%s", "size": %s, "sha256": "%s"}' "$rel" "$size" "$sha"
    count=$((count + 1))
    bytes=$((bytes + size))
  done <<< "$FILES"
  # The converted weight is part of the snapshot but not an upstream file.
  if [ "$first" = 1 ]; then first=0; else printf ',\n'; fi
  safe_size=$(size_of "$DEST/$SAFE")
  printf '    {"path": "%s", "size": %s, "sha256": "%s"}' "$SAFE" "$safe_size" "$(sha256_of "$DEST/$SAFE")"
  count=$((count + 1))
  bytes=$((bytes + safe_size))
  printf '\n  ],\n'
  printf '  "file_count": %s,\n  "total_bytes": %s\n}\n' "$count" "$bytes"
} >"$manifest_tmp"
chmod 0644 "$manifest_tmp"
mv "$manifest_tmp" "$DEST/MANIFEST.json"

echo "all artifacts present and sha256-verified under $DEST"
