#!/usr/bin/env bash
#
# Build hoocode binaries for all platforms locally.
# Mirrors .github/workflows/build-binaries.yml
#
# Usage:
#   ./scripts/build-binaries.sh [--skip-deps] [--platform <platform>]
#
# Options:
#   --skip-deps         Skip installing cross-platform dependencies
#   --platform <name>   Build only for specified platform (darwin-arm64, darwin-x64, linux-x64, linux-arm64, windows-x64)
#
# Output:
#   packages/coding-agent/binaries/
#     hoocode-darwin-arm64.tar.gz
#     hoocode-darwin-x64.tar.gz
#     hoocode-linux-x64.tar.gz
#     hoocode-linux-arm64.tar.gz
#     hoocode-windows-x64.zip
#     hoocode-windows-x64.exe (standalone for winget)
#     hoocode-windows-x64.exe.sha256

set -euo pipefail

cd "$(dirname "$0")/.."

SKIP_DEPS=false
PLATFORM=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-deps)
            SKIP_DEPS=true
            shift
            ;;
        --platform)
            PLATFORM="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Validate platform if specified
if [[ -n "$PLATFORM" ]]; then
    case "$PLATFORM" in
        darwin-arm64|darwin-x64|linux-x64|linux-arm64|windows-x64)
            ;;
        *)
            echo "Invalid platform: $PLATFORM"
            echo "Valid platforms: darwin-arm64, darwin-x64, linux-x64, linux-arm64, windows-x64"
            exit 1
            ;;
    esac
fi

echo "==> Installing dependencies..."
npm ci

if [[ "$SKIP_DEPS" == "false" ]]; then
    echo "==> Installing cross-platform native bindings..."
    # bun --linker=hoisted only installs optional deps for the current platform.
    # We need all platform bindings for bun cross-compilation.
    # Use --force to bypass platform checks (os/cpu restrictions in package.json)
    # Install all in one command to avoid npm removing packages from previous installs
    npm install --no-save --force \
        @mariozechner/clipboard-darwin-arm64@0.3.0 \
        @mariozechner/clipboard-darwin-x64@0.3.0 \
        @mariozechner/clipboard-linux-x64-gnu@0.3.0 \
        @mariozechner/clipboard-linux-arm64-gnu@0.3.0 \
        @mariozechner/clipboard-win32-x64-msvc@0.3.0 \
        @img/sharp-darwin-arm64@0.34.5 \
        @img/sharp-darwin-x64@0.34.5 \
        @img/sharp-linux-x64@0.34.5 \
        @img/sharp-linux-arm64@0.34.5 \
        @img/sharp-win32-x64@0.34.5 \
        @img/sharp-libvips-darwin-arm64@1.2.4 \
        @img/sharp-libvips-darwin-x64@1.2.4 \
        @img/sharp-libvips-linux-x64@1.2.4 \
        @img/sharp-libvips-linux-arm64@1.2.4
else
    echo "==> Skipping cross-platform native bindings (--skip-deps)"
fi

echo "==> Building all packages..."
npm run build

echo "==> Building binaries..."
cd packages/coding-agent

# Clean previous builds
rm -rf binaries
mkdir -p binaries/{darwin-arm64,darwin-x64,linux-x64,linux-arm64,windows-x64}

# Determine which platforms to build
if [[ -n "$PLATFORM" ]]; then
    PLATFORMS=("$PLATFORM")
else
    PLATFORMS=(darwin-arm64 darwin-x64 linux-x64 linux-arm64 windows-x64)
fi

for platform in "${PLATFORMS[@]}"; do
    echo "Building for $platform..."
    # Externalize koffi to avoid embedding all 18 platform .node files (~74MB)
    # into every binary. Koffi is only used on Windows for VT input and the
    # call site has a try/catch fallback. For Windows builds, we copy the
    # appropriate .node file alongside the binary below.
    if [[ "$platform" == "windows-x64" ]]; then
        bun build --compile --external koffi --target=bun-$platform ./dist/bun/cli.js --outfile binaries/$platform/hoocode.exe
    else
        bun build --compile --external koffi --target=bun-$platform ./dist/bun/cli.js --outfile binaries/$platform/hoocode
    fi
done

echo "==> Creating release archives..."

# Copy shared files to each platform directory
for platform in "${PLATFORMS[@]}"; do
    cp package.json binaries/$platform/
    cp README.md binaries/$platform/
    cp CHANGELOG.md binaries/$platform/
    cp ../../node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm binaries/$platform/
    mkdir -p binaries/$platform/theme
    cp dist/modes/interactive/theme/*.json binaries/$platform/theme/
    mkdir -p binaries/$platform/assets
    if [ -d dist/modes/interactive/assets ]; then
      cp dist/modes/interactive/assets/* binaries/$platform/assets/
    fi
    cp -r dist/core/export-html binaries/$platform/
    cp -r docs binaries/$platform/
    cp -r examples binaries/$platform/

    # Copy koffi native module for Windows (needed for VT input support)
    if [[ "$platform" == "windows-x64" ]]; then
        mkdir -p binaries/$platform/node_modules/koffi/build/koffi/win32_x64
        cp ../../node_modules/koffi/index.js binaries/$platform/node_modules/koffi/
        cp ../../node_modules/koffi/package.json binaries/$platform/node_modules/koffi/
        cp ../../node_modules/koffi/build/koffi/win32_x64/koffi.node binaries/$platform/node_modules/koffi/build/koffi/win32_x64/
    fi
done

# Create archives
cd binaries

for platform in "${PLATFORMS[@]}"; do
    if [[ "$platform" == "windows-x64" ]]; then
        # Windows (zip)
        echo "Creating hoocode-windows-x64.zip..."
        (cd $platform && zip -r ../hoocode-windows-x64.zip .)
    else
        # Unix platforms (tar.gz) - use wrapper directory for mise compatibility
        echo "Creating hoocode-$platform.tar.gz..."
        mv $platform hoocode && tar -czf hoocode-$platform.tar.gz hoocode && mv hoocode $platform
    fi
done

# Standalone .exe for winget (raw binary, no bundled assets)
if [[ " ${PLATFORMS[*]} " =~ " windows-x64 " ]]; then
    if [ -f "windows-x64/hoocode.exe" ]; then
        cp "windows-x64/hoocode.exe" "hoocode-windows-x64.exe"
        echo "  -> hoocode-windows-x64.exe (standalone for winget)"

        # Generate SHA256 checksum (works on both macOS and Linux)
        if command -v sha256sum &>/dev/null; then
            sha256sum "hoocode-windows-x64.exe" > "hoocode-windows-x64.exe.sha256"
        elif command -v shasum &>/dev/null; then
            shasum -a 256 "hoocode-windows-x64.exe" > "hoocode-windows-x64.exe.sha256"
        fi
        echo "  -> hoocode-windows-x64.exe.sha256"
    fi
fi

# Extract archives for easy local testing
echo "==> Extracting archives for testing..."
for platform in "${PLATFORMS[@]}"; do
    rm -rf $platform
    if [[ "$platform" == "windows-x64" ]]; then
        mkdir -p $platform && (cd $platform && unzip -q ../hoocode-windows-x64.zip)
    else
        tar -xzf hoocode-$platform.tar.gz && mv hoocode $platform
    fi
done

echo ""
echo "==> Build complete!"
echo "Archives available in packages/coding-agent/binaries/"
ls -lh *.tar.gz *.zip 2>/dev/null || true
echo ""
echo "Extracted directories for testing:"
for platform in "${PLATFORMS[@]}"; do
    echo "  binaries/$platform/hoocode"
done
