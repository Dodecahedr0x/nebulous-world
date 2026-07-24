# Builds the Carbon indexer as a standalone binary for Render's Docker
# runtime (Render has no native Rust buildpack). Build context is the repo
# root (see render.yaml's dockerContext: .) — paths below are root-relative,
# not relative to indexer/ — because indexer/Cargo.toml is a member of the
# root Cargo workspace (see /Cargo.toml) and depends on two of that
# workspace's other members via path (`../programs/nebulous_world`,
# `decoder`), so the build needs the root manifest/lockfile plus every
# member's source, not just indexer/'s own.
#
# sqlx::migrate!() embeds migrations/*.sql into the binary at compile time,
# so the runtime image needs nothing beyond the compiled executable. The
# runtime image DOES also need Node — the indexer spawns dlmm-bridge/ (see
# src/dlmm_bridge.rs) as a child process at startup, and that piece
# deliberately stays TypeScript (reusing the already-tested
# @meteora-ag/dlmm SDK) rather than being reimplemented in Rust — see
# dlmm-bridge/README.md.
FROM rust:1.89-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY programs/nebulous_world ./programs/nebulous_world
COPY indexer/Cargo.toml ./indexer/Cargo.toml
COPY indexer/decoder ./indexer/decoder
COPY indexer/src ./indexer/src
COPY indexer/migrations ./indexer/migrations

RUN cargo build --release --bin indexer

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl gnupg \
    && mkdir -p /etc/apt/keyrings \
    && curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg \
    && echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_20.x nodistro main" > /etc/apt/sources.list.d/nodesource.list \
    && apt-get update && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home indexer

COPY --from=builder /build/target/release/indexer /usr/local/bin/indexer

WORKDIR /app/dlmm-bridge
COPY indexer/dlmm-bridge/package.json indexer/dlmm-bridge/package-lock.json* ./
# Not --omit=dev: there's no separate build step, `tsx` (a devDependency,
# same as in app/package.json) runs src/index.ts directly at container
# startup too — see src/dlmm_bridge.rs's spawn().
RUN npm install
COPY indexer/dlmm-bridge/src ./src
COPY indexer/dlmm-bridge/tsconfig.json ./
RUN chown -R indexer:indexer /app

USER indexer
ENV RUST_LOG=info
# src/dlmm_bridge.rs resolves a `dlmm-bridge` subdirectory relative to
# wherever the indexer binary runs from — /app here, the parent of the
# dlmm-bridge/ directory copied above.
WORKDIR /app
EXPOSE 8090
ENTRYPOINT ["/usr/local/bin/indexer"]
