# A content-addressed, verified bundle built from one committed revision by
# tools/dev/build-artifact.sh. Nix retains the ELF's referenced store libraries.
{ pkgs, artifact, revision }:
assert builtins.match "[0-9a-f]{40}" revision != null;
pkgs.runCommand "knoted-${builtins.substring 0 12 revision}" {
  src = artifact;
  nativeBuildInputs = [ pkgs.coreutils pkgs.findutils pkgs.diffutils ];
  passthru = { inherit revision; };
} ''
  export LC_ALL=C
  cd "$src"
  test "$(cat share/knoted/REVISION)" = '${revision}'
  # Recompute the complete canonical manifest: checking only listed entries
  # would silently accept omitted files. Symlinks could escape the bundle.
  test -f SHA256SUMS && test ! -L SHA256SUMS
  test -z "$(find . -mindepth 1 -maxdepth 1 ! -name bin ! -name share ! -name SHA256SUMS -print -quit)"
  test -z "$(find bin share ! -type d ! -type f -print -quit)"
  find bin share -type f -print0 | sort -z | xargs -0 sha256sum > "$TMPDIR/actual-sums"
  cmp SHA256SUMS "$TMPDIR/actual-sums"
  test -x bin/knoted-api
  test -x bin/knoted-worker
  test ! -e bin/knoted-intelligence
  test ! -e share/knoted/intelligence
  test -s share/knoted/web/index.html
  test -n "$(find share/knoted/migrations -maxdepth 1 -name '*.sql' -type f -print -quit)"
  mkdir -p "$out"
  cp -r bin share SHA256SUMS "$out/"
''
