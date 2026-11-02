#!/bin/sh
# Tests the npm launcher (npm/hoocode/bin/hoocode) against temp node_modules
# layouts with a stub platform binary. No network, no npm needed.
#
#     scripts/npm/test_launcher.sh            # runs under /bin/sh
#     SHELL_UNDER_TEST=dash scripts/npm/test_launcher.sh

set -u
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd -P)
launcher=$root/npm/hoocode/bin/hoocode
sh_=${SHELL_UNDER_TEST:-/bin/sh}
case $sh_ in /*) ;; *) sh_=$(command -v "$sh_") || exit 1 ;; esac
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fails=0

case $(uname -s)-$(uname -m) in
Darwin-arm64 | Darwin-aarch64) key=darwin-arm64 other=darwin-x64 ;;
Darwin-x86_64) key=darwin-x64 other=darwin-arm64 ;;
Linux-aarch64 | Linux-arm64) key=linux-arm64 other= ;;
Linux-x86_64) key=linux-x64 other= ;;
*) echo "unsupported test host" >&2 && exit 1 ;;
esac

# install_pkg <pkg-dir> <name>: the launcher package.
install_pkg() {
	mkdir -p "$1/bin"
	cp "$launcher" "$1/bin/hoocode"
	chmod 755 "$1/bin/hoocode"
	printf '{\n  "name": "%s",\n  "version": "0.0.0"\n}\n' "$2" >"$1/package.json"
}

# install_stub <node_modules-dir> <key>: a platform package whose binary
# prints its args one per line in brackets and exits with $STUB_EXIT.
install_stub() {
	mkdir -p "$1/@kolisachint/hoocode-$2/bin"
	cat >"$1/@kolisachint/hoocode-$2/bin/hoocode" <<EOF
#!/bin/sh
echo "stub $2"
for a in "\$@"; do printf '[%s]\n' "\$a"; done
exit \${STUB_EXIT:-0}
EOF
	chmod 755 "$1/@kolisachint/hoocode-$2/bin/hoocode"
}

check() { # check <name> <expected-output> <expected-status> <cmd...>
	name=$1 want=$2 want_status=$3
	shift 3
	got=$("$@" 2>&1)
	status=$?
	if [ "$got" = "$want" ] && [ "$status" = "$want_status" ]; then
		echo "ok   $name"
	else
		echo "FAIL $name (status $status, want $want_status)"
		printf '  got:\n%s\n  want:\n%s\n' "$got" "$want"
		fails=$((fails + 1))
	fi
}

# Hoisted (npm -g): sibling under the same node_modules, bin is a relative
# symlink from <prefix>/bin.
p=$tmp/hoisted
install_pkg "$p/lib/node_modules/hoocode" hoocode
install_stub "$p/lib/node_modules" "$key"
mkdir -p "$p/bin"
ln -s ../lib/node_modules/hoocode/bin/hoocode "$p/bin/hoocode"
ln -s hoocode "$p/bin/hoo" # symlink to a symlink
check hoisted "stub $key
[a b]
[]
[c]" 0 "$sh_" "$p/bin/hoocode" "a b" "" c
check hoo-chain "stub $key" 0 "$sh_" "$p/bin/hoo"
check exit-code "stub $key" 7 env STUB_EXIT=7 "$sh_" "$p/bin/hoocode"

# Nested: the platform package inside the launcher package's node_modules.
p=$tmp/nested
install_pkg "$p/node_modules/@kolisachint/hoocode" @kolisachint/hoocode
install_stub "$p/node_modules/@kolisachint/hoocode/node_modules" "$key"
check nested "stub $key" 0 "$sh_" "$p/node_modules/@kolisachint/hoocode/bin/hoocode"

# pnpm virtual store: the package dir is a symlink into .pnpm, and the
# platform package sits next to it there, not in the top node_modules.
p=$tmp/pnpm
store=$p/node_modules/.pnpm/hoocode@0.0.0/node_modules
install_pkg "$store/hoocode" hoocode
install_stub "$store" "$key"
ln -s .pnpm/hoocode@0.0.0/node_modules/hoocode "$p/node_modules/hoocode"
mkdir -p "$p/node_modules/.bin"
ln -s ../hoocode/bin/hoocode "$p/node_modules/.bin/hoocode"
check pnpm "stub $key" 0 "$sh_" "$p/node_modules/.bin/hoocode"

# macOS: only the other arch's package is there (Rosetta) -> fall back.
if [ -n "$other" ]; then
	p=$tmp/rosetta
	install_pkg "$p/node_modules/hoocode" hoocode
	install_stub "$p/node_modules" "$other"
	check darwin-fallback "stub $other" 0 "$sh_" "$p/node_modules/hoocode/bin/hoocode"
fi

# Path with spaces, through a relative symlink.
p="$tmp/with space"
install_pkg "$p/lib/node_modules/hoocode" hoocode
install_stub "$p/lib/node_modules" "$key"
mkdir -p "$p/bin"
ln -s "../lib/node_modules/hoocode/bin/hoocode" "$p/bin/hoocode"
check spaces "stub $key
[a b]" 0 "$sh_" "$p/bin/hoocode" "a b"

# bun global: <root>/install/global/node_modules/{hoocode,@kolisachint/...},
# bin linked from <root>/bin.
p=$tmp/bun
install_pkg "$p/install/global/node_modules/hoocode" hoocode
install_stub "$p/install/global/node_modules" "$key"
mkdir -p "$p/bin"
ln -s ../install/global/node_modules/hoocode/bin/hoocode "$p/bin/hoocode"
check bun-global "stub $key" 0 "$sh_" "$p/bin/hoocode"

# CDPATH exported, launcher run by a relative path: `cd` must not search
# CDPATH (a decoy `bin/..` is there) or print anything.
mkdir -p "$tmp/decoy/bin"
# shellcheck disable=SC2016 # $1.. are for the inner sh -c
check cdpath "stub $key" 0 sh -c 'cd "$1" && CDPATH=$2 "$3" bin/hoocode' sh \
	"$tmp/bun" "$tmp/decoy" "$sh_"
# shellcheck disable=SC2016
check cdpath-hoisted "stub $key" 0 sh -c 'cd "$1" && CDPATH=$2:. "$3" bin/hoocode' sh \
	"$tmp/hoisted" "$tmp/decoy" "$sh_"

# No readlink on PATH: say so, not a reinstall hint.
mkdir -p "$tmp/empty"
check no-readlink "hoocode: readlink not found" 1 \
	/usr/bin/env PATH="$tmp/empty" "$sh_" "$tmp/bun/bin/hoocode"

# Missing platform package: the hint names the installing package.
for n in hoocode @kolisachint/hoocode; do
	p=$tmp/missing-$(echo "$n" | tr '@/' '__')
	install_pkg "$p/node_modules/$n" "$n"
	check "missing ($n)" "hoocode: the platform package @kolisachint/hoocode-$key is not installed.
It is an optional dependency; reinstall without --no-optional / --omit=optional:
  npm install -g $n" 1 "$sh_" "$p/node_modules/$n/bin/hoocode"
done

# Not executable: reported, not exec'd.
p=$tmp/noexec
install_pkg "$p/node_modules/hoocode" hoocode
install_stub "$p/node_modules" "$key"
chmod 644 "$p/node_modules/@kolisachint/hoocode-$key/bin/hoocode"
bin=$(CDPATH='' cd -- "$p/node_modules/@kolisachint/hoocode-$key/bin" && pwd -P)/hoocode
check noexec "hoocode: could not start $bin: not executable" 1 \
	"$sh_" "$p/node_modules/hoocode/bin/hoocode"

if [ "$fails" -ne 0 ]; then
	echo "$fails failed"
	exit 1
fi
echo "all passed"
