#!/bin/sh
# Builds the WebAssembly artefact a separate repository can depend on.
#
# One command, and everything a consumer cannot reconstruct travels inside:
# the ABI version, the commit the module was built from, and the reference
# host. The output directory is self-contained; nothing in it needs this
# repository.
set -eu

here="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$here/target/xtex-wasm-release}"

commit="$(git -C "$here" rev-parse HEAD)"
if [ -n "$(git -C "$here" status --porcelain)" ]; then
    echo "refusing to release a dirty working tree: the manifest would name a commit the bytes are not from" >&2
    exit 1
fi
abi="$(grep -m1 '^## ABI version' "$here/docs/wasm.md" | sed 's/.*version //')"
if [ -z "$abi" ]; then
    echo "docs/wasm.md carries no ABI version heading" >&2
    exit 1
fi

cargo build --quiet -p xtex-wasm --target wasm32-unknown-unknown --release

rm -rf "$out"
mkdir -p "$out"
cp "$here/target/wasm32-unknown-unknown/release/xtex_wasm.wasm" "$out/xtex.wasm"
# The reference host: the same file the parity suite runs, copied verbatim so
# the artefact documents itself with code that is tested rather than prose
# that is not.
cp "$here/crates/xtex-wasm/tests/parity.mjs" "$out/reference-host.mjs"
cp "$here/docs/wasm.md" "$out/wasm.md"

hash="$(shasum -a 256 "$out/xtex.wasm" 2>/dev/null | cut -d' ' -f1 || sha256sum "$out/xtex.wasm" | cut -d' ' -f1)"
cat > "$out/manifest.json" <<MANIFEST
{
  "abi": "$abi",
  "commit": "$commit",
  "sha256": "$hash",
  "module": "xtex.wasm"
}
MANIFEST

echo "released ABI $abi from $commit"
echo "  $out/xtex.wasm ($(wc -c < "$out/xtex.wasm" | tr -d ' ') bytes)"
