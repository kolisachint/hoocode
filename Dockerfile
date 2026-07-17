# HooCode container image.
#
# Ships the compiled Bun standalone binary on a glibc base. The container is
# the trust boundary: run it hardened (drop capabilities, no-new-privileges,
# read-only rootfs, a restrictive egress policy) and you do not need the
# in-container bash sandbox. See docs/install.md "Containers & Kubernetes".
#
# Two build targets:
#   docker build -t hoocode .                 # default: minimal, non-root, hardened
#   docker build --target ssh -t hoocode-ssh . # opt-in: adds an SSH server
#
# Run (default):
#   docker run --rm -it -e ANTHROPIC_API_KEY=... -v "$PWD":/work -w /work hoocode
#
# ---------------------------------------------------------------------------
# Stage 1: build the linux-x64 standalone binary (+ sidecar assets).
# oven/bun is debian/glibc-based, matching the runtime stage's libc.
# ---------------------------------------------------------------------------
FROM oven/bun:1.3.13 AS builder

WORKDIR /src

# Copy the whole monorepo; .dockerignore keeps node_modules/dist/.git out.
COPY . .

# Compile the binary and assemble its sidecar assets into /out. The compiled
# binary resolves assets (themes, wasm, docs, examples) relative to the
# directory containing the executable — see getPackageDir() in
# packages/coding-agent/src/config.ts — so the whole /out dir ships together.
# koffi is externalized: it is Windows-only (VT input) with a try/catch
# fallback, and bundling it would embed ~74MB of per-platform .node files.
RUN <<'EOF'
set -eu
bun install --frozen-lockfile
bun run build
cd packages/coding-agent
mkdir -p /out/theme /out/assets
bun build --compile --external koffi --target=bun-linux-x64 ./dist/bun/cli.js --outfile /out/hoocode
chmod +x /out/hoocode
cp package.json README.md CHANGELOG.md /out/
cp ../../node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm /out/
cp dist/modes/interactive/theme/*.json /out/theme/
if [ -d dist/modes/interactive/assets ]; then cp -r dist/modes/interactive/assets/. /out/assets/; fi
cp -r dist/core/export-html /out/
cp -r ../../docs /out/
cp -r examples /out/examples
find /out/examples -type d -name node_modules -prune -exec rm -rf {} +
EOF

# ---------------------------------------------------------------------------
# Stage 2: shared minimal glibc base (assets + user + env), no entrypoint yet.
# The compiled binary is dynamically linked against glibc, so this must NOT be
# an Alpine/musl or `static` distroless base.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS base

# ca-certificates: TLS to the model provider. git: core to coding workflows.
# ripgrep + fd-find: native search. HOOCODE_OFFLINE=1 skips the fd/rg download,
# so without these the agent uses the slower pure-JS search fallback. hoocode
# resolves them from PATH (it already knows Debian's `fdfind` name), so no
# symlink is required — the extra `fd` symlink is just for humans in a shell.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git ripgrep fd-find \
    && ln -s "$(command -v fdfind)" /usr/local/bin/fd \
    && rm -rf /var/lib/apt/lists/*

# Non-root user. Login shell is nologin here; the ssh stage switches it to bash.
RUN useradd --create-home --uid 10001 --shell /usr/sbin/nologin hoo

# The whole assembled directory ships together — the binary resolves its
# sidecar assets (themes, wasm, docs, examples) relative to this location.
COPY --from=builder /out /opt/hoocode

ENV PATH="/opt/hoocode:${PATH}" \
    # Resolve package assets explicitly — never depends on symlink/execPath.
    HOOCODE_PACKAGE_DIR=/opt/hoocode \
    # Writable config/session dir, friendly to readOnlyRootFilesystem: true.
    HOOCODE_CODING_AGENT_DIR=/home/hoo/.hoocode \
    # Skip all startup network operations (binary downloads, version checks).
    HOOCODE_OFFLINE=1

# Mount a writable volume here when running with a read-only root filesystem.
VOLUME ["/home/hoo/.hoocode"]

# ---------------------------------------------------------------------------
# Stage 3 (opt-in, `--target ssh`): add an SSH server.
# sshd runs as root (privilege separation); sessions land as the non-root `hoo`
# user. Public-key auth only. This variant is intentionally less locked-down
# than the default image (a root daemon + a network listener), so run it with a
# normal capability set rather than --cap-drop=ALL, and keep isolation at the
# network/egress layer. The default `runtime` target below stays hardened.
# ---------------------------------------------------------------------------
FROM base AS ssh

RUN apt-get update \
    && apt-get install -y --no-install-recommends openssh-server \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /run/sshd \
    && usermod -s /bin/bash hoo

# Interactive SSH sessions start a fresh login shell that does NOT inherit the
# Docker ENV above — export the runtime settings via profile.d so a user who
# SSHes in and runs `hoocode` gets the same PATH and config as the entrypoint.
RUN cat > /etc/profile.d/hoocode.sh <<'PROFILE'
export PATH="/opt/hoocode:$PATH"
export HOOCODE_PACKAGE_DIR=/opt/hoocode
export HOOCODE_CODING_AGENT_DIR=/home/hoo/.hoocode
export HOOCODE_OFFLINE=1
PROFILE

# Key-only, non-root, single-user. Forwarding disabled for a locked-down box;
# relax if you need SSH tunnels.
RUN cat > /etc/ssh/sshd_config.d/hoocode.conf <<'SSHD'
PermitRootLogin no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AllowUsers hoo
X11Forwarding no
AllowAgentForwarding no
AllowTcpForwarding no
PrintMotd no
SSHD

# Entrypoint: ensure host keys + authorized_keys exist, then run sshd.
RUN cat > /usr/local/bin/hoocode-sshd <<'ENTRY' && chmod +x /usr/local/bin/hoocode-sshd
#!/bin/sh
set -eu
# Generate host keys if absent. Mount a volume at /etc/ssh to keep them stable
# across container restarts (otherwise clients see a changed host key).
ssh-keygen -A
# Seed authorized_keys from $SSH_PUBKEY when provided; otherwise a file must be
# mounted at /home/hoo/.ssh/authorized_keys.
if [ -n "${SSH_PUBKEY:-}" ]; then
	install -d -m 700 -o hoo -g hoo /home/hoo/.ssh
	printf '%s\n' "$SSH_PUBKEY" > /home/hoo/.ssh/authorized_keys
	chmod 600 /home/hoo/.ssh/authorized_keys
	chown hoo:hoo /home/hoo/.ssh/authorized_keys
fi
if [ ! -s /home/hoo/.ssh/authorized_keys ]; then
	echo "hoocode-ssh: no authorized_keys — set SSH_PUBKEY or mount /home/hoo/.ssh/authorized_keys" >&2
	exit 1
fi
exec /usr/sbin/sshd -D -e
ENTRY

EXPOSE 22
ENTRYPOINT ["/usr/local/bin/hoocode-sshd"]

# ---------------------------------------------------------------------------
# Stage 4 (default target): minimal hardened runtime. Runs the agent directly
# as the non-root user. Kept last so a bare `docker build` selects it.
# ---------------------------------------------------------------------------
FROM base AS runtime

USER hoo
WORKDIR /home/hoo

ENTRYPOINT ["hoocode"]
