# syntax=docker/dockerfile:1

# --- Frontend build -------------------------------------------------------
FROM node:22-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY web/ ./
RUN npm run build

# --- Backend build --------------------------------------------------------
# Pinned to bookworm to match the runtime stage's glibc: the floating
# `rust:1-slim` tag moved to trixie (glibc 2.39), which the bookworm runtime
# (2.36) cannot load.
FROM rust:1-slim-bookworm AS api
WORKDIR /app
# Deliberately do NOT copy rust-toolchain.toml: it pins channel = "stable",
# which makes rustup re-resolve and re-download the stable toolchain on every
# build. The base image already ships a stable toolchain, which is fine here.
COPY Cargo.toml Cargo.lock ./
COPY api/ api/
# The docker workspace backend embeds this at compile time (include_str!) to
# build workspace images when the registry has none.
COPY deploy/aoe-image/Dockerfile deploy/aoe-image/Dockerfile
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

# --- Workspace backend CLIs ------------------------------------------------
# Static docker and kubectl binaries so the containerized CityHall can drive
# the docker (socket-mounted compose) and kubernetes workspace backends.
FROM debian:bookworm-slim AS clis
ARG TARGETARCH
ARG DOCKER_VERSION=27.5.1
ARG BUILDX_VERSION=v0.20.1
ARG KUBECTL_VERSION=v1.32.2
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
# Every binary below is verified against a digest recorded here, not merely
# fetched over HTTPS from a pinned version. HTTPS authenticates the origin; it
# says nothing about the bytes, so a compromised release asset would be
# installed and then executed with access to the docker socket and the cluster.
# The digests sit next to the arch they belong to and are hardcoded rather than
# fetched, because a checksum served from the same place as the artifact proves
# only that the two agree. Bump them with the versions above; a mismatch fails
# the build and names the file.
RUN case "${TARGETARCH:-$(dpkg --print-architecture)}" in \
         amd64) DOCKER_ARCH=x86_64 \
                DOCKER_SHA256=4f798b3ee1e0140eab5bf30b0edc4e84f4cdb53255a429dc3bbae9524845d640 \
                BUILDX_SHA256=8c38f60308a895fa570f1410e453c5de11aafd65a99fa99965d96d24b6225a78 \
                KUBECTL_SHA256=4f6a959dcc5b702135f8354cc7109b542a2933c46b808b248a214c1f69f817ea ;; \
         arm64) DOCKER_ARCH=aarch64 \
                DOCKER_SHA256=e6b53725a73763ab3f988c73f8772eaed429754c1a579db5ff11f21990fd1817 \
                BUILDX_SHA256=f7d867e9f1a3c00b32dd580f56594e229df05e3fb1b083b7099c91c2e7d2ce1e \
                KUBECTL_SHA256=7381bea99c83c264100f324c2ca6e7e13738a73b8928477ac805991440a065cd ;; \
         *) echo "unsupported arch: $TARGETARCH" && exit 1 ;; \
       esac \
    # Downloaded to a file rather than piped into tar: bytes that go straight
    # into an extractor cannot be checked first.
    && curl -fsSL -o /tmp/docker.tgz \
       "https://download.docker.com/linux/static/stable/${DOCKER_ARCH}/docker-${DOCKER_VERSION}.tgz" \
    && echo "${DOCKER_SHA256}  /tmp/docker.tgz" | sha256sum -c - \
    && tar -xz --strip-components=1 -C /usr/local/bin -f /tmp/docker.tgz docker/docker \
    && rm /tmp/docker.tgz \
    # The reference workspace image needs BuildKit: it writes the entrypoint
    # with a COPY heredoc, and it selects a build stage by argument so an
    # unreachable stage is skipped. The static docker tarball ships no CLI
    # plugins, and this CLI has no built-in BuildKit client, so without buildx
    # `docker build` silently falls back to the classic builder, which supports
    # neither. Nothing else in CityHall builds an image, so this is the only
    # reason it is here.
    && mkdir -p /usr/local/lib/docker/cli-plugins \
    && curl -fsSL -o /usr/local/lib/docker/cli-plugins/docker-buildx \
       "https://github.com/docker/buildx/releases/download/${BUILDX_VERSION}/buildx-${BUILDX_VERSION}.linux-${TARGETARCH:-$(dpkg --print-architecture)}" \
    && echo "${BUILDX_SHA256}  /usr/local/lib/docker/cli-plugins/docker-buildx" | sha256sum -c - \
    && chmod +x /usr/local/lib/docker/cli-plugins/docker-buildx \
    && curl -fsSL -o /usr/local/bin/kubectl \
       "https://dl.k8s.io/release/${KUBECTL_VERSION}/bin/linux/${TARGETARCH:-$(dpkg --print-architecture)}/kubectl" \
    && echo "${KUBECTL_SHA256}  /usr/local/bin/kubectl" | sha256sum -c - \
    && chmod +x /usr/local/bin/kubectl

# --- Runtime --------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=clis /usr/local/bin/docker /usr/local/bin/docker
COPY --from=clis /usr/local/lib/docker/cli-plugins/docker-buildx /usr/local/lib/docker/cli-plugins/docker-buildx
COPY --from=clis /usr/local/bin/kubectl /usr/local/bin/kubectl
COPY --from=api /usr/local/bin/cityhall /usr/local/bin/cityhall
COPY --from=web /web/dist ./web/dist
ENV STATIC_DIR=/app/web/dist \
    BIND_ADDR=0.0.0.0:3000
EXPOSE 3000
CMD ["cityhall"]
