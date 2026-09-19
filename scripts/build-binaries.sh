#!/usr/bin/env bash
#
# Build the hoocode standalone binaries locally (bun-only).
# Mirrors the `binaries` job in .github/workflows/release.yml and
# .github/workflows/merge-release.yml.
#
# Every target is cross-compiled from one host: `bun build --compile` takes a
# `--target` triple, so a Linux runner produces the macOS and Windows artifacts
# too. That is why the release workflow has a single `binaries` job rather than
# a matrix of runners — and why the per-target native modules below are fetched
# by explicit package name instead of by letting the package manager pick the
# host's.
#
# Usage:
#   ./scripts/build-binaries.sh [--targets a,b,c] [--skip-deps] [--list]
#
# Options:
#   --targets     Comma-separated subset of the targets below (default: all)
#   --skip-deps   Skip installing the per-target native modules
#   --list        Print the target names and exit
#
# Output:
#   packages/coding-agent/binaries/
#     hoocode-<target>.tar.gz   (unix targets)
#     hoocode-<target>.zip      (windows targets)
#     checksums.txt             (sha256 over every archive above)

set -euo pipefail

cd "$(dirname "$0")/.."

# target | bun --target triple | archive ext | clipboard native package
#
# The musl rows exist because the glibc binary cannot run on Alpine or on
# `static` distroless, which used to be a documented dead end (docs/install.md).
ALL_TARGETS=(
    "linux-x64|bun-linux-x64|tar.gz|@mariozechner/clipboard-linux-x64-gnu"
    "linux-arm64|bun-linux-arm64|tar.gz|@mariozechner/clipboard-linux-arm64-gnu"
    "linux-x64-musl|bun-linux-x64-musl|tar.gz|@mariozechner/clipboard-linux-x64-musl"
    "linux-arm64-musl|bun-linux-arm64-musl|tar.gz|@mariozechner/clipboard-linux-arm64-musl"
    "darwin-x64|bun-darwin-x64|tar.gz|@mariozechner/clipboard-darwin-x64"
    "darwin-arm64|bun-darwin-arm64|tar.gz|@mariozechner/clipboard-darwin-arm64"
    "windows-x64|bun-windows-x64|zip|@mariozechner/clipboard-win32-x64-msvc"
)

target_names() {
    local row
    for row in "${ALL_TARGETS[@]}"; do printf '%s\n' "${row%%|*}"; done
}

SKIP_DEPS=false
SELECTED=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-deps)
            SKIP_DEPS=true
            shift
            ;;
        --targets)
            [[ $# -ge 2 ]] || { echo "--targets needs a value" >&2; exit 1; }
            IFS=',' read -r -a SELECTED <<< "$2"
            shift 2
            ;;
        --targets=*)
            IFS=',' read -r -a SELECTED <<< "${1#*=}"
            shift
            ;;
        --list)
            target_names
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

if [[ ${#SELECTED[@]} -eq 0 ]]; then
    mapfile -t SELECTED < <(target_names)
fi

# Fail on an unknown target before spending a build on the valid ones.
for want in "${SELECTED[@]}"; do
    if ! target_names | grep -qx "$want"; then
        echo "Unknown target: $want" >&2
        echo "Known targets: $(target_names | paste -sd, -)" >&2
        exit 1
    fi
done

row_for() {
    local want="$1" row
    for row in "${ALL_TARGETS[@]}"; do
        [[ "${row%%|*}" == "$want" ]] && { printf '%s' "$row"; return 0; }
    done
    return 1
}

echo "==> Targets: ${SELECTED[*]}"

echo "==> Installing dependencies (bun)..."
# bun is the toolchain. The hoisted (npm-compatible) linker is pinned in
# bunfig.toml, so a plain `bun install` yields a flat node_modules that tsgo
# resolves correctly -- no post-install hoisting step is needed.
bun install --frozen-lockfile

if [[ "$SKIP_DEPS" == "false" ]]; then
    echo "==> Installing cross-platform native modules..."
    # Bun pulls optional deps regardless of os/cpu, so every target's clipboard
    # binding can be fetched on one host. The clipboard module is loaded through
    # a try/catch createRequire (src/utils/clipboard-native.ts), so a target
    # whose binding fails to resolve degrades to "no clipboard" rather than
    # failing to start.
    NATIVE_PKGS=()
    for want in "${SELECTED[@]}"; do
        row="$(row_for "$want")"
        NATIVE_PKGS+=("$(printf '%s' "$row" | cut -d'|' -f4)")
    done
    # Windows also needs koffi (VT input) and, historically, the sharp binding.
    # koffi is externalized from the binary and copied in below.
    if printf '%s\n' "${SELECTED[@]}" | grep -q '^windows-'; then
        NATIVE_PKGS+=("@img/sharp-win32-x64@0.34.5")
    fi
    bun add --no-save "${NATIVE_PKGS[@]}"
else
    echo "==> Skipping native modules (--skip-deps)"
fi

echo "==> Building all packages..."
bun run build

cd packages/coding-agent

rm -rf binaries
mkdir -p binaries

# Everything that ships beside the executable, identical for every target.
# Kept in one function so a new target cannot quietly miss a runtime asset.
assemble_payload() {
    local out="$1"

    cp package.json "$out/"
    cp README.md "$out/"
    cp CHANGELOG.md "$out/"
    cp ../../node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm "$out/"

    mkdir -p "$out/theme"
    cp dist/modes/interactive/theme/*.json "$out/theme/"

    mkdir -p "$out/assets"
    if [ -d dist/modes/interactive/assets ]; then
        cp dist/modes/interactive/assets/* "$out/assets/"
    fi

    cp -r dist/core/export-html "$out/"
    # Canvas: the forked child imports the SDK shim from a real path on disk, so
    # the whole canvas directory ships beside the exe with its relative layout
    # intact (sdk-shim/index.js imports ../protocol.js). See config.ts
    # getCanvasDir().
    cp -r dist/core/canvas "$out/"
    cp -r docs "$out/"
    # Exclude examples' node_modules: they contain bun workspace symlinks that do
    # not resolve once copied, and the runtime only needs the example sources.
    rsync -a --exclude 'node_modules' examples/ "$out/examples/"
    # templates/ intentionally not copied — seed content is embedded into the
    # compiled binary by scripts/embed-templates.mjs (see
    # src/init-templates.generated.ts).
}

for want in "${SELECTED[@]}"; do
    row="$(row_for "$want")"
    bun_target="$(printf '%s' "$row" | cut -d'|' -f2)"
    ext="$(printf '%s' "$row" | cut -d'|' -f3)"

    echo ""
    echo "==> Building $want ($bun_target)..."

    out="binaries/$want"
    mkdir -p "$out"

    if [[ "$want" == windows-* ]]; then
        exe="$out/hoocode.exe"
    else
        exe="$out/hoocode"
    fi

    # Externalize koffi to avoid embedding all 18 platform .node files (~74MB)
    # into every binary. Koffi is only used on Windows for VT input and the call
    # site has a try/catch fallback; the Windows binary gets the one .node file
    # it needs copied in below.
    bun build --compile --external koffi --target="$bun_target" ./dist/bun/cli.js --outfile "$exe"

    assemble_payload "$out"

    if [[ "$want" == windows-* ]]; then
        # Copy koffi native module for Windows (needed for VT input support)
        mkdir -p "$out/node_modules/koffi/build/koffi/win32_x64"
        cp ../../node_modules/koffi/index.js "$out/node_modules/koffi/"
        cp ../../node_modules/koffi/package.json "$out/node_modules/koffi/"
        cp ../../node_modules/koffi/build/koffi/win32_x64/koffi.node "$out/node_modules/koffi/build/koffi/win32_x64/"
    else
        chmod +x "$exe"
    fi

    echo "==> Packing hoocode-$want.$ext..."
    if [[ "$ext" == "zip" ]]; then
        (cd "$out" && zip -qr "../hoocode-$want.zip" .)
    else
        # -C so the archive has no leading directory: the installer untars
        # straight into the install prefix.
        tar -czf "binaries/hoocode-$want.tar.gz" -C "$out" .
    fi

    rm -rf "$out"
done

echo ""
echo "==> Writing checksums..."
(cd binaries && sha256sum ./*.tar.gz ./*.zip 2>/dev/null | sed 's#\./##' > checksums.txt)

echo ""
echo "==> Build complete!"
echo "Archives available in packages/coding-agent/binaries/"
ls -lh binaries/
echo ""
cat binaries/checksums.txt
