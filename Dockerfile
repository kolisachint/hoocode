# HooCode container image.
#
# Ships the compiled Bun standalone binary on a glibc base. The container is
# the trust boundary: run it hardened (drop capabilities, no-new-privileges,
# read-only rootfs, a restrictive egress policy) and you do not need the
# in-container bash sandbox. See docs/install.md "Containers & Kubernetes".
#
# Build:  docker build -t hoocode .
# Run:    docker run --rm -it \
#           -e ANTHROPIC_API_KEY=... \
#           -v "$PWD":/work -w /work \
#           hoocode
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
# Stage 2: minimal glibc runtime.
# The compiled binary is dynamically linked against glibc, so this must NOT be
# an Alpine/musl or `static` distroless base.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# ca-certificates: TLS to the model provider. git: core to coding workflows.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*

# Non-root by default. The container is the boundary; nothing here needs root.
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

USER hoo
WORKDIR /home/hoo

ENTRYPOINT ["hoocode"]
