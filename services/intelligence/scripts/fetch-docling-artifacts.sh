#!/usr/bin/env bash
# Docling artifact provisioner for the machine-level shared root.
#
#   ./fetch-docling-artifacts.sh                      # provision into /data/llm/docling/<revision>
#   ./fetch-docling-artifacts.sh /some/root           # provision into /some/root/<revision>
#   ./fetch-docling-artifacts.sh --verify             # verify only, never write (default root)
#   ./fetch-docling-artifacts.sh /some/root --verify  # verify only, never write
#   ./fetch-docling-artifacts.sh --print-revision-id  # print the locked directory name
#
# Layout: <target-root>/<revision-id>/ IS Docling's artifacts_path. Revision
# directories are immutable: an existing file is never overwritten, a sha256
# mismatch is an error. `MANIFEST.json` is generated here, not written by hand.
#
# Needs only bash, curl and sha256sum. No Python, no venv, no project package.
# Downloads are pinned to a git commit (or the RapidOCR v3.10.0 tag) and every
# file is verified by size and sha256.
#
# Environment: HF_ENDPOINT, MODELSCOPE_ENDPOINT (mirrors).
set -euo pipefail

LAYOUT_REV=8f39ad3c0b4c58e9c2d2c84a38465abf757272d8
TABLEFORMER_REV=fc0f2d45e2218ea24bce5045f58a389aed16dc23
RAPIDOCR_VERSION=3.10.0
RAPIDOCR_TAG=v3.10.0
# Must equal parsing.models.revision_dirname(); a test asserts that.
REVISION_ID="heron-${LAYOUT_REV:0:8}_tf-${TABLEFORMER_REV:0:8}-fast_rapidocr-${RAPIDOCR_VERSION}"

DEFAULT_ROOT=/data/llm/docling
HF="${HF_ENDPOINT:-https://huggingface.co}"
MS="${MODELSCOPE_ENDPOINT:-https://www.modelscope.cn}"

VERIFY_ONLY=0
PRINT_REVISION=0
TARGET_ROOT=""
for arg in "$@"; do
  case "$arg" in
    --verify|-c) VERIFY_ONLY=1 ;;
    --print-revision-id) PRINT_REVISION=1 ;;
    -h|--help) sed -n '2,17p' "$0"; exit 0 ;;
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

# url relative-path size sha256   (relative to the revision directory)
read -r -d '' ARTIFACTS <<EOF || true
$HF/docling-project/docling-layout-heron/resolve/$LAYOUT_REV/config.json docling-project--docling-layout-heron/config.json 3268 fdea30805ce2f5666b147fca941dcdd27ad468e27d6ed21902207d3da056a97d
$HF/docling-project/docling-layout-heron/resolve/$LAYOUT_REV/model.safetensors docling-project--docling-layout-heron/model.safetensors 171658996 00333a43451945aaf89db8ca9c0a17e75d1537c17db60fdb91aa95f4c7929e0c
$HF/docling-project/docling-layout-heron/resolve/$LAYOUT_REV/preprocessor_config.json docling-project--docling-layout-heron/preprocessor_config.json 444 cd38cd59999e7a95d68e487fbe5132df3d4e5c32a0836add57e6126ba0c4eaf1
$HF/docling-project/docling-layout-heron-onnx/resolve/40bde044036bb181c130ddf6c51792187268748f/config.json docling-project--docling-layout-heron-onnx/config.json 878 c6e67b6cdf64fa245779d0409bb994aa96e8c1790e15ec07a65843628e232180
$HF/docling-project/docling-layout-heron-onnx/resolve/40bde044036bb181c130ddf6c51792187268748f/model.onnx docling-project--docling-layout-heron-onnx/model.onnx 171220471 59c81a3a2923042d85034ffc487f8f47e4854117e879aef89b2b9f728fb4922a
$HF/docling-project/docling-layout-heron-onnx/resolve/40bde044036bb181c130ddf6c51792187268748f/preprocessor_config.json docling-project--docling-layout-heron-onnx/preprocessor_config.json 445 54d086cf0d7d371f7fac36e5d3a0dae31e211298affc816106cc376506799d58
$HF/docling-project/docling-models/resolve/$TABLEFORMER_REV/model_artifacts/tableformer/fast/tableformer_fast.safetensors docling-project--docling-models/model_artifacts/tableformer/fast/tableformer_fast.safetensors 145453276 3119563aab5a7c96fda4d621119b63fd8806272b86c30936d15507616422f718
$HF/docling-project/docling-models/resolve/$TABLEFORMER_REV/model_artifacts/tableformer/fast/tm_config.json docling-project--docling-models/model_artifacts/tableformer/fast/tm_config.json 7060 dca6762508dddfae6d57d6cb4ef822c6000119dff0f3b6489db7413118c2622a
$HF/docling-project/docling-models/resolve/$TABLEFORMER_REV/model_artifacts/tableformer/accurate/tableformer_accurate.safetensors docling-project--docling-models/model_artifacts/tableformer/accurate/tableformer_accurate.safetensors 212758388 2a7d6c924b3cd12fb99a09280ca9c33a89c5d60b93253617d2e088c1a40374d9
$HF/docling-project/docling-models/resolve/$TABLEFORMER_REV/model_artifacts/tableformer/accurate/tm_config.json docling-project--docling-models/model_artifacts/tableformer/accurate/tm_config.json 7060 984e122ceb8ccf84d84c9d2882f6f2302a44b4f1e577babd6289892c36f3cffd
$MS/models/RapidAI/RapidOCR/resolve/$RAPIDOCR_TAG/onnx/PP-OCRv6/det/PP-OCRv6_det_small.onnx RapidOcr/PP-OCRv6_det_small.onnx 9929594 090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f
$MS/models/RapidAI/RapidOCR/resolve/$RAPIDOCR_TAG/onnx/PP-OCRv4/cls/ch_ppocr_mobile_v2.0_cls_mobile.onnx RapidOcr/ch_ppocr_mobile_v2.0_cls_mobile.onnx 585532 e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c
$MS/models/RapidAI/RapidOCR/resolve/$RAPIDOCR_TAG/onnx/PP-OCRv6/rec/PP-OCRv6_rec_small.onnx RapidOcr/PP-OCRv6_rec_small.onnx 21234383 6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884
$MS/models/RapidAI/RapidOCR/resolve/$RAPIDOCR_TAG/paddle/PP-OCRv6/rec/PP-OCRv6_rec_small/ppocrv6_dict.txt RapidOcr/ppocrv6_dict.txt 74947 b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d
EOF

sha256_of() { sha256sum "$1" | cut -d' ' -f1; }
size_of() { stat -c%s "$1"; }

# verify_file <path> <size> <sha256>: 0 ok / 1 mismatch / 2 missing
verify_file() {
  [ -f "$1" ] || return 2
  [ "$(size_of "$1")" = "$2" ] && [ "$(sha256_of "$1")" = "$3" ] && return 0
  return 1
}

problems=0
verify_all() {
  local url rel size sha rc
  while read -r url rel size sha; do
    [ -n "${rel:-}" ] || continue
    rc=0
    verify_file "$DEST/$rel" "$size" "$sha" || rc=$?
    case "$rc" in
      0) printf 'ok      %s\n' "$rel" ;;
      2) printf 'MISSING %s\n' "$rel"; problems=$((problems + 1)) ;;
      *) printf 'MISMATCH %s (expected %s B sha256 %s)\n' "$rel" "$size" "$sha"; problems=$((problems + 1)) ;;
    esac
  done <<< "$ARTIFACTS"
}

if [ "$VERIFY_ONLY" = 1 ]; then
  verify_all
  if [ "$problems" -ne 0 ]; then
    echo "$problems artifact(s) failed verification under $DEST" >&2
    exit 1
  fi
  echo "all $(printf '%s\n' "$ARTIFACTS" | grep -c .) artifacts verified under $DEST (read-only)"
  exit 0
fi

install -d -m 0755 "$TARGET_ROOT" "$DEST"
# One writer at a time: two consumers may provision the same new revision.
exec 9>"$TARGET_ROOT/.provision.lock"
flock -w 300 9

fetch() { # fetch <url> <dest> <sha256> <size>
  local url="$1" dest="$2" sha="$3" size="$4" got
  mkdir -p "$(dirname "$dest")"
  if ! curl -fsSL --retry 3 --retry-delay 2 -o "$dest.part" "$url"; then
    rm -f "$dest.part"
    printf 'DOWNLOAD FAIL %s\n' "$dest" >&2
    return 1
  fi
  got="$(sha256_of "$dest.part")"
  if [ "$got" != "$sha" ] || [ "$(size_of "$dest.part")" != "$size" ]; then
    printf 'SHA256/SIZE FAIL %s: expected %s B %s, got %s B %s\n' \
      "$dest" "$size" "$sha" "$(size_of "$dest.part")" "$got" >&2
    rm -f "$dest.part"
    return 1
  fi
  chmod 0644 "$dest.part"
  mv "$dest.part" "$dest"
}

fail=0
while read -r url rel size sha; do
  [ -n "${rel:-}" ] || continue
  rc=0
  verify_file "$DEST/$rel" "$size" "$sha" || rc=$?
  if [ "$rc" = 0 ]; then
    printf 'ok      %s\n' "$rel"
    continue
  fi
  if [ "$rc" = 1 ]; then
    printf 'MISMATCH %s (on disk differs from the pinned sha256; not overwriting)\n' "$rel" >&2
    fail=1
    continue
  fi
  printf 'get     %s\n' "$rel"
  fetch "$url" "$DEST/$rel" "$sha" "$size" || fail=1
done <<< "$ARTIFACTS"

if [ "$fail" -ne 0 ]; then
  echo "one or more artifacts could not be provisioned under $DEST" >&2
  exit 1
fi

# Generated manifest: the same table, relative to the revision root.
manifest_tmp="$DEST/MANIFEST.json.tmp"
{
  printf '{\n  "schema": "knoted-docling-artifacts/2",\n'
  printf '  "revision": "%s",\n' "$REVISION_ID"
  printf '  "sources": {\n'
  printf '    "layout": "docling-project/docling-layout-heron@%s",\n' "$LAYOUT_REV"
  printf '    "layout_onnx": "docling-project/docling-layout-heron-onnx@40bde044036bb181c130ddf6c51792187268748f",\n'
  printf '    "tableformer": "docling-project/docling-models@%s",\n' "$TABLEFORMER_REV"
  printf '    "rapidocr": "RapidAI/RapidOCR@%s"\n' "$RAPIDOCR_TAG"
  printf '  },\n  "artifacts": [\n'
  first=1
  count=0
  bytes=0
  while read -r url rel size sha; do
    [ -n "${rel:-}" ] || continue
    if [ "$first" = 1 ]; then first=0; else printf ',\n'; fi
    printf '    {"path": "%s", "size": %s, "sha256": "%s"}' "$rel" "$size" "$sha"
    count=$((count + 1))
    bytes=$((bytes + size))
  done <<< "$ARTIFACTS"
  printf '\n  ],\n'
  printf '  "artifact_count": %s,\n  "artifact_bytes": %s\n}\n' "$count" "$bytes"
} >"$manifest_tmp"
chmod 0644 "$manifest_tmp"
mv "$manifest_tmp" "$DEST/MANIFEST.json"

problems=0
verify_all
if [ "$problems" -ne 0 ]; then
  echo "$problems artifact(s) failed verification under $DEST" >&2
  exit 1
fi
echo "all $count artifacts present and sha256-verified under $DEST"
