# syntax=docker/dockerfile:1.7
#
# Nimino relay image — published as ghcr.io/asopitech-labs/nimino:<tag>.
#
# Builds the `nimino-relay` binary (Rust 1.95) and the `nimino-web` static bundle
# (pnpm + vite), then assembles them into a small debian-slim runtime with
# `git` available (the relay shells out to git for repo hydrate / receive-pack
# / upload-pack — see crates/nimino-relay/src/api/git).
#
# Multi-arch is handled by running this same Dockerfile on native amd64 and
# native arm64 runners (see .github/workflows/docker.yml). The Dockerfile
# itself is platform-agnostic; do not add --platform pins.

ARG RUST_VERSION=1.95
ARG NODE_VERSION=24
ARG DEBIAN_VERSION=bookworm
ARG NIM_VERSION=2.2.10
ARG NIM_RELEASE=2026-04-24-version-2-2-bfeb3146d1638b39f69007a4ae5a23e23ae4e5ef

# Optional extra CA bundle for builds behind a TLS-intercepting corporate proxy
# (e.g. a Cloudflare/Zscaler gateway that re-signs TLS). Empty by default, so
# public CI builds are unaffected. Point it at a PEM file in the build context:
#   docker build --build-arg EXTRA_CA_CERTS=path/to/proxy-ca.pem ...
# Consumed by the network-touching stages below (cargo + pnpm).
ARG EXTRA_CA_CERTS=

# Optional npm registry for builds where the public registry is unreachable or
# policy-blocked (e.g. a corporate mirror / Artifactory). Empty default = public
# npmjs, so public CI builds are unaffected. Consumed by the web-builder stage.
ARG NPM_REGISTRY=

# ─── Stage 1: cargo-chef base ───────────────────────────────────────────────
FROM docker.io/library/rust:${RUST_VERSION}-${DEBIAN_VERSION} AS chef
# Trust an optional corporate-proxy CA before any network fetch (no-op if unset).
ARG EXTRA_CA_CERTS
COPY --chmod=0644 ${EXTRA_CA_CERTS:-Dockerfile} /tmp/extra-ca/src
RUN if [ -n "${EXTRA_CA_CERTS}" ]; then \
        cp /tmp/extra-ca/src /usr/local/share/ca-certificates/extra-proxy-ca.crt \
        && update-ca-certificates \
        && echo "CARGO_HTTP_CAINFO=/etc/ssl/certs/ca-certificates.crt" >> /etc/environment; \
    fi
ENV CARGO_HTTP_CAINFO=/etc/ssl/certs/ca-certificates.crt
RUN cargo install cargo-chef --locked --version 0.1.71
WORKDIR /build

# ─── Stage 2: plan dependency graph ─────────────────────────────────────────
# Only the manifests are needed to compute the recipe; this layer rebuilds
# only when Cargo.{toml,lock} or crate manifests change, not on every source
# edit.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ─── Stage 3: cook dependencies, then build the binary ──────────────────────
FROM chef AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        libssl-dev \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/*
# Keep enough DWARF for native profilers to resolve optimized code to source
# locations. The normal runtime strips it below; runtime-debug retains it.
ENV CARGO_PROFILE_RELEASE_DEBUG=line-tables-only
COPY --from=planner /build/recipe.json recipe.json
# Cook the full workspace recipe — relay deps include workspace siblings, so
# scoping to -p nimino-relay misses transitive deps and re-builds them later.
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked -p nimino-relay --bin nimino-relay \
                                   -p nimino-admin --bin nimino-admin \
                                   -p nimino-pair-relay --bin nimino-pair-relay

# ─── Stage 4: exact Nim core worker ─────────────────────────────────────────
FROM docker.io/library/debian:${DEBIAN_VERSION}-slim AS nim-builder
ARG TARGETARCH
ARG NIM_VERSION
ARG NIM_RELEASE
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gcc libc6-dev xz-utils \
    && rm -rf /var/lib/apt/lists/*
RUN case "${TARGETARCH}" in \
      amd64) archive="nim-${NIM_VERSION}-linux_x64.tar.xz"; checksum="0a3a38752e97e9d44aa479b3a7b37336dfe0176daf22ee5b5218ad0991ecd211" ;; \
      arm64) archive="nim-${NIM_VERSION}-linux_arm64.tar.xz"; checksum="cd86a6e2bcbf029c4870aa51df5c0169345dbf9959889112fd15d403c13ae33a" ;; \
      *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
    && curl -fsSL "https://github.com/nim-lang/nightlies/releases/download/${NIM_RELEASE}/${archive}" -o /tmp/nim.tar.xz \
    && echo "${checksum}  /tmp/nim.tar.xz" | sha256sum -c - \
    && mkdir -p /opt/nim \
    && tar -xJf /tmp/nim.tar.xz --strip-components=1 -C /opt/nim \
    && rm /tmp/nim.tar.xz
WORKDIR /build/nimino_core
COPY nim/nimino_core/ .
RUN /opt/nim/bin/nim c -d:release --hints:off --nimcache:/tmp/nimcache \
    --out:/build/nimino-core-worker src/nimino_core_worker.nim

FROM nim-builder AS nim-stripped
RUN strip /build/nimino-core-worker

# Derive the normal release binaries from the same optimized ELF files as the
# debug image so the two variants cannot drift at code-generation time.
FROM builder AS stripped-binaries
RUN strip target/release/nimino-relay \
    && strip target/release/nimino-admin \
    && strip target/release/nimino-pair-relay

# ─── Stage 5: web bundle (pnpm + vite) ──────────────────────────────────────
# Independent of the Rust layers so a CSS change doesn't bust Rust cache and
# vice versa.
FROM docker.io/library/node:${NODE_VERSION}-${DEBIAN_VERSION}-slim AS web-builder
WORKDIR /build
# Trust an optional corporate-proxy CA so corepack + pnpm can fetch over an
# intercepting TLS gateway (no-op if EXTRA_CA_CERTS is unset).
ARG EXTRA_CA_CERTS
COPY --chmod=0644 ${EXTRA_CA_CERTS:-Dockerfile} /tmp/extra-ca/src
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && if [ -n "${EXTRA_CA_CERTS}" ]; then \
        cp /tmp/extra-ca/src /usr/local/share/ca-certificates/extra-proxy-ca.crt \
        && update-ca-certificates; \
    fi \
    && rm -rf /var/lib/apt/lists/*
ENV NODE_EXTRA_CA_CERTS=/etc/ssl/certs/ca-certificates.crt
# Point npm + corepack at an optional mirror (no-op when NPM_REGISTRY is unset).
# corepack reads COREPACK_NPM_REGISTRY to fetch the pinned pnpm; pnpm/npm read
# the .npmrc registry for dependency installs.
ARG NPM_REGISTRY
ENV COREPACK_NPM_REGISTRY=${NPM_REGISTRY}
# When using a mirror, disable corepack's npmjs signature check: the mirror
# republishes tarballs without the public registry's provenance signatures, so
# strict verification fails ("No compatible signature found"). Only relaxed on
# the mirror path — public builds (NPM_REGISTRY unset) keep strict verification.
RUN if [ -n "${NPM_REGISTRY}" ]; then \
        echo "registry=${NPM_REGISTRY}" > /build/.npmrc \
        && echo "COREPACK_INTEGRITY_KEYS=0" >> /etc/environment; \
    fi
ENV COREPACK_INTEGRITY_KEYS=${NPM_REGISTRY:+0}
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY patches/ patches/
COPY web/package.json web/
COPY admin-web/package.json admin-web/
RUN pnpm install --frozen-lockfile --filter nimino-web --filter nimino-admin-web
COPY web/ web/
COPY admin-web/ admin-web/
RUN pnpm -C web build && pnpm -C admin-web build

# ─── Stage 6: shared runtime ────────────────────────────────────────────────
FROM docker.io/library/debian:${DEBIAN_VERSION}-slim AS runtime-base

# OCI annotations: required for GHCR to auto-link the image to this repo and
# inherit its visibility. org.opencontainers.image.source is the load-bearing
# one — without it GHCR keeps the image private even when the repo is public.
LABEL org.opencontainers.image.title="Nimino" \
      org.opencontainers.image.description="Nimino distributed collaboration relay" \
      org.opencontainers.image.source="https://github.com/asopitech-labs/nimino" \
      org.opencontainers.image.url="https://github.com/asopitech-labs/nimino" \
      org.opencontainers.image.documentation="https://github.com/asopitech-labs/nimino#readme" \
      org.opencontainers.image.licenses="Apache-2.0"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        openssl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1000 nimino \
    && useradd  --uid 1000 --gid 1000 --home-dir /var/lib/nimino \
                --create-home --shell /usr/sbin/nologin nimino

COPY --from=web-builder /build/web/dist                 /srv/nimino/web
COPY --from=web-builder /build/admin-web/dist           /srv/nimino/admin-web

# The invite landing page is always served from the bundled web UI. Repository
# browser routes require the separate NIMINO_SERVE_GIT_WEB_GUI=true opt-in. The
# admin bundle is inert until NIMINO_ADMIN_HOST is configured.
ENV NIMINO_WEB_DIR=/srv/nimino/web \
    NIMINO_ADMIN_WEB_DIR=/srv/nimino/admin-web \
    NIMINO_BOUNDARY_WORKER=/usr/local/bin/nimino-core-worker

# 3000: app · 7443/udp: Chirps QUIC · 8080: health · 9102: metrics
EXPOSE 3000 8080 9102
EXPOSE 7443/udp

# deploy/compose mounts a volume here; pre-created for the unprivileged user.
RUN mkdir -p /data/git /var/lib/nimino/cluster /etc/nimino/chirps \
    && chown -R nimino:nimino /data/git /var/lib/nimino/cluster

USER nimino:nimino
WORKDIR /var/lib/nimino

ENTRYPOINT ["/usr/local/bin/nimino-relay"]

# Optimized binaries with line-table debug information for native profiling.
# Published under debug-* tags; runtime behavior otherwise matches the normal
# image exactly.
FROM runtime-base AS runtime-debug
COPY --from=nim-builder /build/nimino-core-worker /usr/local/bin/nimino-core-worker
COPY --from=builder /build/target/release/nimino-relay /usr/local/bin/nimino-relay
COPY --from=builder /build/target/release/nimino-admin /usr/local/bin/nimino-admin
COPY --from=builder /build/target/release/nimino-pair-relay /usr/local/bin/nimino-pair-relay

# Keep the stripped runtime as the final/default Dockerfile target so existing
# `docker build .` callers and release tags retain their current behavior.
FROM runtime-base AS runtime
COPY --from=nim-stripped /build/nimino-core-worker /usr/local/bin/nimino-core-worker
COPY --from=stripped-binaries /build/target/release/nimino-relay /usr/local/bin/nimino-relay
COPY --from=stripped-binaries /build/target/release/nimino-admin /usr/local/bin/nimino-admin
COPY --from=stripped-binaries /build/target/release/nimino-pair-relay /usr/local/bin/nimino-pair-relay
