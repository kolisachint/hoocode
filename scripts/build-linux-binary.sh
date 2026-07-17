#!/usr/bin/env bash
#
# Build the hoocode Linux x64 standalone binary (bun-only).
# Mirrors scripts/build-binaries.sh (Windows) for the Linux target and is
# consumed by the container image (see Dockerfile) and the `binaries` job in
# .github/workflows/release.yml.
#
# The compiled binary resolves its sidecar assets (themes, wasm, docs, ...)
# relative to the directory containing the executable — see getPackageDir() in
# packages/coding-agent/src/config.ts. So the whole binaries/linux-x64/
# directory must be shipped together, not just the binary.
#
# Usage:
#   ./scripts/build-linux-binary.sh
#
# Output:
#   packages/coding-agent/binaries/
#     linux-x64/            (assembled binary + sidecar assets)
#     hoocode-linux-x64.tar.gz

set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Installing dependencies (bun)..."
bun install --frozen-lockfile

echo "==> Building all packages..."
bun run build

echo "==> Building Linux binary..."
cd packages/coding-agent

# Clean previous builds
rm -rf binaries/linux-x64 binaries/hoocode-linux-x64.tar.gz
mkdir -p binaries/linux-x64

# Externalize koffi: it is only used on Windows for VT input and the call site
# has a try/catch fallback, so Linux never needs the native module. Keeping it
# external also avoids embedding all 18 platform .node files (~74MB).
bun build --compile --external koffi --target=bun-linux-x64 ./dist/bun/cli.js --outfile binaries/linux-x64/hoocode
chmod +x binaries/linux-x64/hoocode

echo "==> Assembling release directory..."

cp package.json binaries/linux-x64/
cp README.md binaries/linux-x64/
cp CHANGELOG.md binaries/linux-x64/
cp ../../node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm binaries/linux-x64/
mkdir -p binaries/linux-x64/theme
cp dist/modes/interactive/theme/*.json binaries/linux-x64/theme/
mkdir -p binaries/linux-x64/assets
if [ -d dist/modes/interactive/assets ]; then
  cp dist/modes/interactive/assets/* binaries/linux-x64/assets/
fi
cp -r dist/core/export-html binaries/linux-x64/
cp -r ../../docs binaries/linux-x64/
# Copy example sources but drop node_modules: they contain bun workspace
# symlinks that do not resolve once copied, and the runtime only needs sources.
cp -r examples binaries/linux-x64/examples
find binaries/linux-x64/examples -type d -name node_modules -prune -exec rm -rf {} +
# templates/ intentionally not copied — seed content is embedded into the
# compiled binary by scripts/embed-templates.mjs (see src/init-templates.generated.ts).

echo "==> Creating release archive..."
cd binaries
tar -czf hoocode-linux-x64.tar.gz -C linux-x64 .

echo ""
echo "==> Build complete!"
echo "Archive available in packages/coding-agent/binaries/"
ls -lh hoocode-linux-x64.tar.gz 2>/dev/null || true
