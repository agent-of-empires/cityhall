# syntax=docker/dockerfile:1

# --- Frontend build -------------------------------------------------------
FROM node:22-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# --- Backend build --------------------------------------------------------
FROM rust:1-slim AS api
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY api/ api/
# The frontend is built in its own stage and copied into the runtime image,
# so skip build.rs's npm invocation here.
ENV SKIP_FRONTEND_BUILD=1
RUN cargo build --release --locked

# --- Runtime --------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=api /app/target/release/cityhall /usr/local/bin/cityhall
COPY --from=web /web/dist ./web/dist
ENV STATIC_DIR=/app/web/dist \
    BIND_ADDR=0.0.0.0:3000
EXPOSE 3000
CMD ["cityhall"]
