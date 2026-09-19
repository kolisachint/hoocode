#!/usr/bin/env sh
#
# HooCode one-click installer (macOS / Linux).
#
#   curl -fsSL https://kolisachint.github.io/hoocode/install.sh | sh
#
# What it does
#   1. Works out your platform (including musl vs glibc, which decides the build).
#   2. Downloads the matching standalone binary from the latest GitHub release
#      and verifies its sha256 against the release's checksums.txt.
#   3. Unpacks it into ~/.hoocode/lib/hoocode and links ~/.hoocode/bin/{hoocode,hoo}.
#   4. Optionally pre-seeds the external Rust tools (fd, rg, embsearch, webtools,
#      voicetools) into ~/.hoocode/bin, which is exactly where HooCode looks for
#      them -- so a first run is fast and works offline.
#
# It is deliberately POSIX sh: `sh -c "$(curl ...)"` must work on a machine that
# has no bash, which is the default on Alpine and on some minimal images.
#
# Options (flags or environment variables):
#   --version <v>        HOOCODE_VERSION      release tag to install (default: latest)
#   --dir <path>         HOOCODE_INSTALL_DIR  install root (default: ~/.hoocode)
#   --no-tools           HOOCODE_SKIP_TOOLS=1 skip the external Rust tools
#   --tools a,b          HOOCODE_TOOLS        subset of fd,rg,embsearch,webtools,voicetools
#   --no-modify-path     HOOCODE_NO_MODIFY_PATH=1  do not touch shell rc files
#   --help

set -eu

REPO="kolisachint/hoocode"
WEBSITE="https://kolisachint.github.io/hoocode/"
# Every tool HooCode can use. fd and rg are the two that silently make everything
# faster; the other three add capability HooCode otherwise does not have.
DEFAULT_TOOLS="fd rg embsearch webtools voicetools"

VERSION="${HOOCODE_VERSION:-latest}"
INSTALL_DIR="${HOOCODE_INSTALL_DIR:-$HOME/.hoocode}"
SKIP_TOOLS="${HOOCODE_SKIP_TOOLS:-0}"
TOOLS="${HOOCODE_TOOLS:-}"
NO_MODIFY_PATH="${HOOCODE_NO_MODIFY_PATH:-0}"

# ---------------------------------------------------------------- output ----
# Colour only when stdout is a terminal: piped into a log, escape codes are noise.
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    C_DIM='\033[2m'; C_B='\033[1m'; C_OK='\033[32m'; C_WARN='\033[33m'; C_ERR='\033[31m'; C_ACC='\033[36m'; C_0='\033[0m'
else
    C_DIM=''; C_B=''; C_OK=''; C_WARN=''; C_ERR=''; C_ACC=''; C_0=''
fi

say()  { printf '%b\n' "$*"; }
step() { printf '%b\n' "${C_ACC}==>${C_0} $*"; }
warn() { printf '%b\n' "${C_WARN}warning:${C_0} $*" >&2; }
die()  { printf '%b\n' "${C_ERR}error:${C_0} $*" >&2; exit 1; }

usage() {
    # The header comment block IS the help text; stop at the first line that is
    # not a comment so the two can never drift apart.
    awk 'NR==1 {next} /^[^#]/ {exit} {sub(/^# ?/, ""); print}' "$0"
    exit 0
}

# ------------------------------------------------------------- arguments ----
while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || die "--version needs a value"; VERSION="$2"; shift 2 ;;
        --version=*) VERSION="${1#*=}"; shift ;;
        --dir) [ $# -ge 2 ] || die "--dir needs a value"; INSTALL_DIR="$2"; shift 2 ;;
        --dir=*) INSTALL_DIR="${1#*=}"; shift ;;
        --tools) [ $# -ge 2 ] || die "--tools needs a value"; TOOLS="$2"; shift 2 ;;
        --tools=*) TOOLS="${1#*=}"; shift ;;
        --no-tools) SKIP_TOOLS=1; shift ;;
        --no-modify-path) NO_MODIFY_PATH=1; shift ;;
        -h|--help) usage ;;
        *) die "unknown option: $1 (try --help)" ;;
    esac
done

# ------------------------------------------------------------ toolchecks ----
have() { command -v "$1" >/dev/null 2>&1; }

if have curl; then
    DL="curl"
elif have wget; then
    DL="wget"
else
    die "need curl or wget on PATH to download anything."
fi

# One place that knows how to fetch a URL to a file, so the curl/wget split does
# not spread through the script.
fetch() {
    # fetch <url> <dest>
    if [ "$DL" = "curl" ]; then
        curl -fsSL --retry 3 --retry-delay 1 -o "$2" "$1"
    else
        wget -q -t 3 -O "$2" "$1"
    fi
}

fetch_stdout() {
    if [ "$DL" = "curl" ]; then
        curl -fsSL --retry 3 --retry-delay 1 "$1"
    else
        wget -q -t 3 -O - "$1"
    fi
}

have tar || die "need tar on PATH to unpack the release."

# sha256 is spelled differently on macOS and Linux, and some images have neither.
# A missing checksum tool is a warning, not a failure: the download still
# happened over TLS.
if have sha256sum; then
    SHA_CMD="sha256sum"
elif have shasum; then
    SHA_CMD="shasum -a 256"
else
    SHA_CMD=""
fi

# --------------------------------------------------------------- platform ---
UNAME_S="$(uname -s)"
UNAME_M="$(uname -m)"

case "$UNAME_S" in
    Linux)  OS="linux" ;;
    Darwin) OS="darwin" ;;
    *) die "unsupported OS: $UNAME_S. HooCode ships Linux and macOS builds here; on Windows use install.ps1 (see $WEBSITE)." ;;
esac

case "$UNAME_M" in
    x86_64|amd64) ARCH="x64" ;;
    arm64|aarch64) ARCH="arm64" ;;
    *) die "unsupported CPU: $UNAME_M. HooCode ships x86_64 and arm64 builds; install from npm instead: npm install -g @kolisachint/hoocode-agent" ;;
esac

TARGET="$OS-$ARCH"

# musl vs glibc. The standalone binary is linked against one of them and will not
# start on the other, so getting this wrong is the difference between a working
# install and "No such file or directory" on a file that plainly exists.
if [ "$OS" = "linux" ]; then
    if have ldd && ldd --version 2>&1 | grep -qi musl; then
        TARGET="$TARGET-musl"
    elif [ -f /etc/alpine-release ]; then
        TARGET="$TARGET-musl"
    fi
fi

step "Platform: ${C_B}$TARGET${C_0}"

# ---------------------------------------------------------------- version ---
if [ "$VERSION" = "latest" ]; then
    step "Resolving the latest release..."
    # Ask the API, but do not require it: an unauthenticated rate limit should
    # not be the reason an install fails, so fall back to the redirect that
    # /releases/latest serves.
    TAG="$(fetch_stdout "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1 || true)"
    if [ -z "$TAG" ] && [ "$DL" = "curl" ]; then
        TAG="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" 2>/dev/null \
            | sed -n 's#.*/tag/##p' || true)"
    fi
    [ -n "$TAG" ] || die "could not resolve the latest release. Pass one explicitly: --version v1.2.3"
else
    # Accept both "1.2.3" and "v1.2.3" -- people copy the number, not the tag.
    case "$VERSION" in v*) TAG="$VERSION" ;; *) TAG="v$VERSION" ;; esac
fi

say "    version ${C_B}$TAG${C_0}"

ASSET="hoocode-$TARGET.tar.gz"
BASE="https://github.com/$REPO/releases/download/$TAG"

# ------------------------------------------------------------------ paths ---
LIB_DIR="$INSTALL_DIR/lib/hoocode"
BIN_DIR="$INSTALL_DIR/bin"

# A scratch dir that is cleaned up however the script exits, including a ^C
# halfway through a 60MB download.
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/hoocode-install.XXXXXX")"
cleanup() { rm -rf "$TMP_DIR"; }
trap cleanup EXIT INT TERM

# ------------------------------------------------------------- download -----
step "Downloading $ASSET..."
if ! fetch "$BASE/$ASSET" "$TMP_DIR/$ASSET"; then
    die "no build for $TARGET in $TAG.
    Checked: $BASE/$ASSET
    Install from npm instead (works anywhere Node >= 20 runs):
      npm install -g @kolisachint/hoocode-agent"
fi

if [ -n "$SHA_CMD" ] && fetch "$BASE/checksums.txt" "$TMP_DIR/checksums.txt" 2>/dev/null; then
    EXPECTED="$(grep " $ASSET\$" "$TMP_DIR/checksums.txt" 2>/dev/null | awk '{print $1}' | head -n 1 || true)"
    if [ -n "$EXPECTED" ]; then
        ACTUAL="$($SHA_CMD "$TMP_DIR/$ASSET" | awk '{print $1}')"
        [ "$EXPECTED" = "$ACTUAL" ] || die "checksum mismatch for $ASSET.
    expected $EXPECTED
    actual   $ACTUAL
    Refusing to install. Re-run, and if it persists open an issue:
      https://github.com/$REPO/issues"
        say "    ${C_OK}checksum verified${C_0}"
    else
        warn "no checksum listed for $ASSET; skipping verification."
    fi
else
    [ -n "$SHA_CMD" ] || warn "no sha256 tool found; skipping checksum verification."
fi

# -------------------------------------------------------------- install -----
step "Installing into $LIB_DIR..."
# Unpack to a staging dir and swap, so a failure midway leaves the previous
# install intact rather than a half-replaced one.
STAGE="$TMP_DIR/stage"
mkdir -p "$STAGE"
tar -xzf "$TMP_DIR/$ASSET" -C "$STAGE"

[ -f "$STAGE/hoocode" ] || die "the archive did not contain a hoocode binary. Please report this: https://github.com/$REPO/issues"
chmod +x "$STAGE/hoocode"

mkdir -p "$(dirname "$LIB_DIR")" "$BIN_DIR"
rm -rf "$LIB_DIR.old"
[ -d "$LIB_DIR" ] && mv "$LIB_DIR" "$LIB_DIR.old"
mv "$STAGE" "$LIB_DIR"
rm -rf "$LIB_DIR.old"

# Both names, because both are in the package's bin map.
ln -sf "$LIB_DIR/hoocode" "$BIN_DIR/hoocode"
ln -sf "$LIB_DIR/hoocode" "$BIN_DIR/hoo"

# macOS quarantines anything downloaded, and the quarantine bit on a CLI shows up
# as a Gatekeeper dialog nobody expects from a terminal. Clearing it here is the
# same thing the user would be told to do, done once, silently.
if [ "$OS" = "darwin" ] && have xattr; then
    xattr -d com.apple.quarantine "$LIB_DIR/hoocode" 2>/dev/null || true
fi

say "    ${C_OK}installed${C_0} $("$LIB_DIR/hoocode" --version 2>/dev/null || echo "$TAG")"

# -------------------------------------------------- external Rust tools -----
# HooCode fetches these itself on first use. Doing it here means the first run is
# immediate, and a machine that is about to go offline still gets them.
#
# The asset names mirror src/utils/tools-manager.ts. If that table changes, this
# one has to change with it -- a mismatch here just 404s, and the install carries
# on without the tool, which is the same state as not pre-seeding it.
tool_repo() {
    case "$1" in
        fd) echo "sharkdp/fd" ;;
        rg) echo "BurntSushi/ripgrep" ;;
        embsearch) echo "kolisachint/embeddingsearchtools" ;;
        webtools) echo "kolisachint/webtools" ;;
        voicetools) echo "kolisachint/voicetools" ;;
    esac
}

# The Rust target triples the upstream release matrices actually publish.
tool_asset() {
    # tool_asset <tool> <version-without-v>
    _t="$1"; _v="$2"
    case "$ARCH" in x64) _a="x86_64" ;; arm64) _a="aarch64" ;; esac
    case "$_t" in
        fd)
            [ "$OS" = "darwin" ] && echo "fd-v$_v-$_a-apple-darwin.tar.gz" || echo "fd-v$_v-$_a-unknown-linux-gnu.tar.gz" ;;
        rg)
            if [ "$OS" = "darwin" ]; then echo "ripgrep-$_v-$_a-apple-darwin.tar.gz"
            elif [ "$ARCH" = "arm64" ]; then echo "ripgrep-$_v-aarch64-unknown-linux-gnu.tar.gz"
            # ripgrep publishes x86_64 Linux as a musl build only.
            else echo "ripgrep-$_v-x86_64-unknown-linux-musl.tar.gz"; fi ;;
        embsearch)
            [ "$OS" = "darwin" ] && echo "embsearch-$_a-apple-darwin.tar.gz" || echo "embsearch-$_a-unknown-linux-gnu.tar.gz" ;;
        webtools)
            if [ "$OS" = "darwin" ]; then echo "webtools-$_a-apple-darwin.tar.gz"
            # aarch64 Linux is published as musl, x86_64 as gnu.
            elif [ "$ARCH" = "arm64" ]; then echo "webtools-aarch64-unknown-linux-musl.tar.gz"
            else echo "webtools-x86_64-unknown-linux-gnu.tar.gz"; fi ;;
        voicetools)
            [ "$OS" = "darwin" ] && echo "voicetools-$_a-apple-darwin.tar.gz" || echo "voicetools-$_a-unknown-linux-gnu.tar.gz" ;;
    esac
}

install_tool() {
    _tool="$1"
    _repo="$(tool_repo "$_tool")"
    [ -n "$_repo" ] || { warn "unknown tool: $_tool"; return 0; }

    _tag="$(fetch_stdout "https://api.github.com/repos/$_repo/releases/latest" 2>/dev/null \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1 || true)"
    if [ -z "$_tag" ]; then
        warn "$_tool: could not resolve a release; HooCode will fetch it on first use."
        return 0
    fi
    _ver="${_tag#v}"
    _asset="$(tool_asset "$_tool" "$_ver")"

    if ! fetch "https://github.com/$_repo/releases/download/$_tag/$_asset" "$TMP_DIR/$_asset" 2>/dev/null; then
        warn "$_tool: no $ARCH build published for $OS; HooCode falls back to its built-in path."
        return 0
    fi

    _ex="$TMP_DIR/x-$_tool"
    rm -rf "$_ex"; mkdir -p "$_ex"
    tar -xzf "$TMP_DIR/$_asset" -C "$_ex" 2>/dev/null || { warn "$_tool: archive did not unpack."; return 0; }

    # Upstream archives vary between a flat layout and one directory deep, so
    # find the executable rather than assuming where it sits.
    _bin="$(find "$_ex" -type f -name "$_tool" -perm -u+x 2>/dev/null | head -n 1 || true)"
    [ -n "$_bin" ] || _bin="$(find "$_ex" -type f -name "$_tool" 2>/dev/null | head -n 1 || true)"
    if [ -z "$_bin" ]; then
        warn "$_tool: no '$_tool' executable inside the archive."
        return 0
    fi

    install -m 0755 "$_bin" "$BIN_DIR/$_tool" 2>/dev/null || { cp "$_bin" "$BIN_DIR/$_tool"; chmod +x "$BIN_DIR/$_tool"; }
    [ "$OS" = "darwin" ] && have xattr && xattr -d com.apple.quarantine "$BIN_DIR/$_tool" 2>/dev/null || true
    say "    ${C_OK}+${C_0} $_tool ${C_DIM}$_tag${C_0}"
}

if [ "$SKIP_TOOLS" = "1" ]; then
    step "Skipping external tools (--no-tools). HooCode fetches what it needs on demand."
else
    step "Pre-seeding external tools into $BIN_DIR..."
    WANT="${TOOLS:-$DEFAULT_TOOLS}"
    # Accept comma- or space-separated lists; people type both.
    WANT="$(printf '%s' "$WANT" | tr ',' ' ')"
    for t in $WANT; do install_tool "$t"; done
fi

# ------------------------------------------------------------------ PATH ----
# Append to whichever rc files exist rather than picking one: the shell that runs
# the installer is often not the shell the user will open next.
add_to_path() {
    _line="export PATH=\"\$HOME/.hoocode/bin:\$PATH\""
    # A custom --dir cannot use the $HOME shorthand.
    [ "$INSTALL_DIR" = "$HOME/.hoocode" ] || _line="export PATH=\"$BIN_DIR:\$PATH\""

    _touched=""
    for rc in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
        [ -f "$rc" ] || continue
        grep -qF "$BIN_DIR" "$rc" 2>/dev/null && continue
        grep -qF '.hoocode/bin' "$rc" 2>/dev/null && continue
        printf '\n# Added by the HooCode installer\n%s\n' "$_line" >> "$rc"
        _touched="$_touched $(basename "$rc")"
    done

    # fish keeps its path elsewhere and does not read the lines above.
    _fish="$HOME/.config/fish/config.fish"
    if [ -f "$_fish" ] && ! grep -qF "$BIN_DIR" "$_fish" 2>/dev/null; then
        printf '\n# Added by the HooCode installer\nfish_add_path %s\n' "$BIN_DIR" >> "$_fish"
        _touched="$_touched config.fish"
    fi

    printf '%s' "$_touched"
}

case ":$PATH:" in
    *":$BIN_DIR:"*) ON_PATH=1 ;;
    *) ON_PATH=0 ;;
esac

if [ "$ON_PATH" = "1" ]; then
    :
elif [ "$NO_MODIFY_PATH" = "1" ]; then
    warn "$BIN_DIR is not on your PATH. Add it yourself:
    export PATH=\"$BIN_DIR:\$PATH\""
else
    TOUCHED="$(add_to_path)"
    if [ -n "$TOUCHED" ]; then
        step "Added $BIN_DIR to PATH in:$TOUCHED"
        say "    ${C_DIM}open a new terminal, or: export PATH=\"$BIN_DIR:\$PATH\"${C_0}"
    else
        warn "could not find a shell rc file to update. Add this yourself:
    export PATH=\"$BIN_DIR:\$PATH\""
    fi
fi

# ------------------------------------------------------------------ done ----
say ""
say "${C_OK}${C_B}HooCode is installed.${C_0}"
say ""
say "  ${C_B}hoocode${C_0}          start in build mode ${C_DIM}(or ${C_B}hoo${C_0}${C_DIM}, same thing)${C_0}"
say "  ${C_B}hoocode --help${C_0}   every flag"
say "  ${C_B}/login${C_0}           pick a provider once you are in"
say ""
say "  Docs      ${C_ACC}$WEBSITE${C_0}"
say "  Source    ${C_ACC}https://github.com/$REPO${C_0}"
say ""
say "  ${C_WARN}★${C_0} If HooCode saves you time, a star is the cheapest way to say so:"
say "    ${C_ACC}https://github.com/$REPO${C_0}"
say ""
