#!/usr/bin/env bash
#
# Build the hoocode Windows standalone binary locally.
# Mirrors .github/workflows/release.yml (binaries job).
#
# Usage:
#   ./scripts/build-binaries.sh [--skip-deps]
#
# Options:
#   --skip-deps   Skip installing cross-platform dependencies
#
# Output:
#   packages/coding-agent/binaries/
#     hoocode-windows-x64.zip

set -euo pipefail

cd "$(dirname "$0")/.."

SKIP_DEPS=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-deps)
            SKIP_DEPS=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo "==> Installing dependencies (bun)..."
# Use bun instead of npm to avoid an arborist regression in npm 11.x that
# crashes with "Invalid Version" when resolving this monorepo's workspace
# cycle. Bun's default isolated layout is then hoisted to a flat node_modules
# tree (see scripts/hoist-bun-deps.mjs) so tsc / tsgo still resolve packages
# the way npm install would.
bun install --frozen-lockfile

if [[ "$SKIP_DEPS" == "false" ]]; then
    echo "==> Installing Windows native bindings..."
    # Bun pulls all optional-deps regardless of os/cpu, so no --force needed.
    bun add --no-save \
        @mariozechner/clipboard-win32-x64-msvc@0.3.0 \
        @img/sharp-win32-x64@0.34.5
else
    echo "==> Skipping Windows native bindings (--skip-deps)"
fi

echo "==> Hoisting bun deps to flat node_modules layout..."
node scripts/hoist-bun-deps.mjs

echo "==> Building all packages..."
npm run build

echo "==> Building Windows binary..."
cd packages/coding-agent

# Clean previous builds
rm -rf binaries
mkdir -p binaries/windows-x64

# Externalize koffi to avoid embedding all 18 platform .node files (~74MB)
# into the binary. Koffi is only used on Windows for VT input and the call
# site has a try/catch fallback. We copy the appropriate .node file alongside
# the binary below.
bun build --compile --external koffi --target=bun-windows-x64 ./dist/bun/cli.js --outfile binaries/windows-x64/hoocode.exe

echo "==> Assembling release directory..."

cp package.json binaries/windows-x64/
cp README.md binaries/windows-x64/
cp CHANGELOG.md binaries/windows-x64/
cp ../../node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm binaries/windows-x64/
mkdir -p binaries/windows-x64/theme
cp dist/modes/interactive/theme/*.json binaries/windows-x64/theme/
mkdir -p binaries/windows-x64/assets
if [ -d dist/modes/interactive/assets ]; then
  cp dist/modes/interactive/assets/* binaries/windows-x64/assets/
fi
cp -r dist/core/export-html binaries/windows-x64/
cp -r docs binaries/windows-x64/
cp -r examples binaries/windows-x64/
# templates/ intentionally not copied — seed content is embedded into the
# compiled binary by scripts/embed-templates.mjs (see src/init-templates.generated.ts).

# Copy koffi native module for Windows (needed for VT input support)
mkdir -p binaries/windows-x64/node_modules/koffi/build/koffi/win32_x64
cp ../../node_modules/koffi/index.js binaries/windows-x64/node_modules/koffi/
cp ../../node_modules/koffi/package.json binaries/windows-x64/node_modules/koffi/
cp ../../node_modules/koffi/build/koffi/win32_x64/koffi.node binaries/windows-x64/node_modules/koffi/build/koffi/win32_x64/

echo "==> Creating release archive..."
cd binaries
echo "Creating hoocode-windows-x64.zip..."
(cd windows-x64 && zip -r ../hoocode-windows-x64.zip .)

# Extract archive for easy local testing
echo "==> Extracting archive for testing..."
rm -rf windows-x64
mkdir -p windows-x64 && (cd windows-x64 && unzip -q ../hoocode-windows-x64.zip)

echo ""
echo "==> Build complete!"
echo "Archive available in packages/coding-agent/binaries/"
ls -lh *.zip 2>/dev/null || true
