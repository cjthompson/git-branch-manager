#!/usr/bin/env bash
# Regenerate the layered architecture diagram for git-branch-manager.
#
# Pipeline:  cargo modules dependencies --lib  ->  scripts/layer_dot.py  ->  dot
# Outputs (under docs/architecture/):
#   modules.dot       raw cargo-modules dependency graph (source of truth)
#   architecture.dot  layered, clustered graph
#   architecture.svg  rendered diagram (scalable)
#   architecture.png  rendered diagram (raster, for previews)
#
# Re-run any time the module structure changes. Requires cargo-modules and
# Graphviz; both are checked below.
set -euo pipefail

# cargo lives in the rustup toolchain dir, which isn't on PATH under fish.
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$HOME/.cargo/bin:$PATH"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out_dir="$repo_root/docs/architecture"
mkdir -p "$out_dir"

if ! cargo modules --version >/dev/null 2>&1; then
    echo "error: cargo-modules not found. Install with: cargo install cargo-modules" >&2
    exit 1
fi
if ! command -v dot >/dev/null 2>&1; then
    echo "error: Graphviz 'dot' not found. Install with: brew install graphviz" >&2
    exit 1
fi

echo "==> generating raw module graph (cargo-modules)"
cargo modules dependencies \
    --manifest-path "$repo_root/Cargo.toml" \
    --lib --no-externs --no-sysroot --no-fns --no-traits --no-types \
    --layout dot \
    > "$out_dir/modules.dot"

echo "==> applying layer grouping"
python3 "$repo_root/scripts/layer_dot.py" \
    "$out_dir/modules.dot" "$out_dir/architecture.dot"

echo "==> rendering SVG + PNG (Graphviz)"
dot -Tsvg "$out_dir/architecture.dot" -o "$out_dir/architecture.svg"
dot -Tpng -Gdpi=140 "$out_dir/architecture.dot" -o "$out_dir/architecture.png"

echo "==> done:"
echo "    $out_dir/architecture.svg"
echo "    $out_dir/architecture.png"
