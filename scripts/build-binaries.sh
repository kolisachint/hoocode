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
    # Not `mapfile`: that is bash 4+, and macOS still ships bash 3.2, so building
    # every target on a Mac died here before the first compile.
    while IFS= read -r name; do SELECTED+=("$name"); done < <(target_names)
fi

# Fail on an unknown target before spending a build on the valid ones.
for want in "${SELECTED[@]}"; do
    if ! target_names | grep -qx "$want"; then
        echo "Unknown target: $want" >&2
        echo "Known targets: $(target_names | paste -sd, -)" >&2
        exit 1
    fi
done

# `bun build --compile` appends its payload to a copy of the bun executable,
# which leaves the Mach-O carrying the linker's ad-hoc signature over content
# that no longer matches it. macOS on Apple Silicon does not warn about an
# invalid signature, it SIGKILLs the process -- `Killed: 9` on the first run,
# with nothing else to go on. So every darwin binary is re-signed ad hoc here,
# before it is packed, or the archive is dead on arrival on the machines most
# likely to download it.
#
# rcodesign first because it is the only one of the two that exists on the Linux
# runner the release cross-compiles from; codesign is for a local build on a Mac.
sign_darwin() {
    local exe="$1"

    if command -v rcodesign >/dev/null 2>&1; then
        rcodesign sign "$exe" >/dev/null
    elif command -v codesign >/dev/null 2>&1 && codesign --sign - --force "$exe" >/dev/null 2>&1; then
        :
    elif [[ "${HOOCODE_ALLOW_UNSIGNED_DARWIN:-0}" == "1" ]]; then
        echo "    WARNING: $exe is unsigned and macOS will kill it on launch" >&2
        return 0
    else
        echo "No ad-hoc signer available for $exe." >&2
        echo "Install rcodesign (https://github.com/indygreg/apple-platform-rs), or build on a Mac." >&2
        echo "To build anyway, knowing the binary cannot run: HOOCODE_ALLOW_UNSIGNED_DARWIN=1" >&2
        exit 1
    fi

    # codesign is authoritative, so use it when it is here. On Linux there is
    # nothing to check with -- `rcodesign verify` says of itself that it is buggy
    # and reports a failure for a valid ad-hoc signature -- so the signer's own
    # exit status is the check.
    if command -v codesign >/dev/null 2>&1; then
        codesign -v "$exe" || { echo "ad-hoc signature did not take for $exe" >&2; exit 1; }
    fi
}

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
    #
    # Copy-then-prune rather than `rsync --exclude`: rsync is not in coreutils and
    # is absent from plenty of containers, so depending on it meant the build ran
    # for several minutes and then died on its last step with "command not found".
    mkdir -p "$out/examples"
    cp -R examples/. "$out/examples/"
    find "$out/examples" -name node_modules -type d -prune -exec rm -rf {} +
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

    if [[ "$want" == darwin-* ]]; then
        echo "==> Signing $want (ad-hoc)..."
        sign_darwin "$exe"
    fi

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
# sha256sum is coreutils; macOS ships shasum instead, with the same output shape.
if command -v sha256sum >/dev/null 2>&1; then
    SHA256_CMD=(sha256sum)
else
    SHA256_CMD=(shasum -a 256)
fi
(
    cd binaries
    # The list is built explicitly because a partial build (--targets) leaves
    # one of the two globs unmatched, and under `set -o pipefail` that failed
    # the whole script after every binary had already been compiled.
    archives=()
    for f in *.tar.gz *.zip; do [[ -f "$f" ]] && archives+=("$f"); done
    [[ ${#archives[@]} -gt 0 ]] || { echo "no archives were produced" >&2; exit 1; }
    "${SHA256_CMD[@]}" "${archives[@]}" > checksums.txt
)

echo ""
echo "==> Build complete!"
echo "Archives available in packages/coding-agent/binaries/"
ls -lh binaries/
echo ""
cat binaries/checksums.txt
