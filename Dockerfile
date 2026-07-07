# syntax=docker/dockerfile:1

# --- Frontend build -------------------------------------------------------
FROM node:22-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY web/ ./
RUN npm run build

# --- Backend build --------------------------------------------------------
FROM rust:1-slim AS api
WORKDIR /app
# Deliberately do NOT copy rust-toolchain.toml: it pins channel = "stable",
# which makes rustup re-resolve and re-download the stable toolchain on every
# build. The base image already ships a stable toolchain, which is fine here.
COPY Cargo.toml Cargo.lock ./
COPY api/ api/
# The frontend is built in its own stage and copied into the runtime image,
# so skip build.rs's npm invocation here.
ENV SKIP_FRONTEND_BUILD=1
# Cache the cargo registry and target dir across builds so only changed crates
# recompile. Cache mounts do not persist into the image, so copy the binary out.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked \
    && cp target/release/cityhall /usr/local/bin/cityhall

# --- Runtime --------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=api /usr/local/bin/cityhall /usr/local/bin/cityhall
COPY --from=web /web/dist ./web/dist
ENV STATIC_DIR=/app/web/dist \
    BIND_ADDR=0.0.0.0:3000
EXPOSE 3000
CMD ["cityhall"]
