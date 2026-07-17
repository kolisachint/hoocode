#!/usr/bin/env bash
#
# Build a hoocode Unix standalone binary (bun-only).
# Companion to scripts/build-binaries.sh (Windows). Covers Linux and macOS on
# x64 and arm64, and is consumed by the container image (see Dockerfile) and the
# `binaries` job in .github/workflows/release.yml.
#
# The compiled binary resolves its sidecar assets (themes, wasm, docs, ...)
# relative to the directory containing the executable — see getPackageDir() in
# packages/coding-agent/src/config.ts. So the whole binaries/<target>/ directory
# must ship together, not just the binary.
#
# Usage:
#   ./scripts/build-unix-binary.sh <target>
#     target: linux-x64 | linux-arm64 | darwin-x64 | darwin-arm64
#             (defaults to linux-x64)
#
# Builds run either natively (target matches the runner) or via bun's
# cross-compilation. Each target's native clipboard addon is a prebuilt npm
# package, so cross-building from any host works.
#
# Output:
#   packages/coding-agent/binaries/
#     <target>/               (assembled binary + sidecar assets)
#     hoocode-<target>.tar.gz

set -euo pipefail

cd "$(dirname "$0")/.."

TARGET="${1:-linux-x64}"

# Pin the clipboard native variant to the version the js loader resolves.
CLIPBOARD_VERSION="0.3.2"

case "$TARGET" in
	linux-x64)    BUN_TARGET="bun-linux-x64";    CLIPBOARD_PKG="@mariozechner/clipboard-linux-x64-gnu" ;;
	linux-arm64)  BUN_TARGET="bun-linux-arm64";  CLIPBOARD_PKG="@mariozechner/clipboard-linux-arm64-gnu" ;;
	darwin-x64)   BUN_TARGET="bun-darwin-x64";   CLIPBOARD_PKG="@mariozechner/clipboard-darwin-x64" ;;
	darwin-arm64) BUN_TARGET="bun-darwin-arm64"; CLIPBOARD_PKG="@mariozechner/clipboard-darwin-arm64" ;;
	*)
		echo "Unknown target: $TARGET" >&2
		echo "Expected one of: linux-x64 | linux-arm64 | darwin-x64 | darwin-arm64" >&2
		exit 1
		;;
esac

echo "==> Building hoocode $TARGET ($BUN_TARGET)"

echo "==> Installing dependencies (bun)..."
bun install --frozen-lockfile

# Ensure the target's native clipboard binding is resolvable so bun can embed
# it. On a matching native runner bun already installed it; --no-save keeps the
# lockfile clean when cross-building. The addon load is wrapped in try/catch
# (see clipboard-native.ts) so a fetch failure is non-fatal — it just falls back
# to the platform CLI tools (wl-copy/xclip/pbcopy) at runtime.
echo "==> Ensuring native clipboard binding ($CLIPBOARD_PKG)..."
bun add --no-save "${CLIPBOARD_PKG}@${CLIPBOARD_VERSION}" \
	|| echo "warn: could not add ${CLIPBOARD_PKG}; relying on host-installed binding"

echo "==> Building all packages..."
bun run build

echo "==> Building binary..."
cd packages/coding-agent

OUTDIR="binaries/${TARGET}"
ARCHIVE="hoocode-${TARGET}.tar.gz"

# Clean previous builds
rm -rf "$OUTDIR" "binaries/${ARCHIVE}"
mkdir -p "$OUTDIR"

# Externalize koffi: it is only used on Windows for VT input and the call site
# has a try/catch fallback, so Unix never needs the native module. Keeping it
# external also avoids embedding all 18 platform .node files (~74MB).
bun build --compile --external koffi --target="$BUN_TARGET" ./dist/bun/cli.js --outfile "$OUTDIR/hoocode"
chmod +x "$OUTDIR/hoocode"

echo "==> Assembling release directory..."

cp package.json "$OUTDIR/"
cp README.md "$OUTDIR/"
cp CHANGELOG.md "$OUTDIR/"
cp ../../node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm "$OUTDIR/"
mkdir -p "$OUTDIR/theme"
cp dist/modes/interactive/theme/*.json "$OUTDIR/theme/"
mkdir -p "$OUTDIR/assets"
if [ -d dist/modes/interactive/assets ]; then
	cp dist/modes/interactive/assets/* "$OUTDIR/assets/"
fi
cp -r dist/core/export-html "$OUTDIR/"
cp -r ../../docs "$OUTDIR/"
# Copy example sources but drop node_modules: they contain bun workspace
# symlinks that do not resolve once copied, and the runtime only needs sources.
cp -r examples "$OUTDIR/examples"
find "$OUTDIR/examples" -type d -name node_modules -prune -exec rm -rf {} +
# templates/ intentionally not copied — seed content is embedded into the
# compiled binary by scripts/embed-templates.mjs (see src/init-templates.generated.ts).

echo "==> Creating release archive..."
tar -czf "binaries/${ARCHIVE}" -C "$OUTDIR" .

echo ""
echo "==> Build complete!"
echo "Archive available in packages/coding-agent/binaries/"
ls -lh "binaries/${ARCHIVE}" 2>/dev/null || true
