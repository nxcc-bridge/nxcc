# This Dockerfile bundles nXCC as an all-in-one image for convenience and platforms that only support one enclave per server.

# Stage 0: Download workerd
FROM alpine:3.20 AS workerd-downloader

ARG WORKERD_VERSION=v1.20250520.0
ARG WORKERD_ARCH=linux-64
RUN apk add --no-cache curl gzip

RUN curl -sSL "https://github.com/cloudflare/workerd/releases/download/${WORKERD_VERSION}/workerd-${WORKERD_ARCH}.gz" | gunzip > /opt/workerd

# Stage 1: Builder
FROM rust:1.85.0-slim-bookworm AS builder

ENV CARGO_TERM_COLOR=always

# Install build dependencies: protoc for tonic_build, and dependencies for building capnp
RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler libprotobuf-dev \
    curl tar gzip build-essential autoconf automake libtool pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Install latest Cap'n Proto from source
ARG CAPNP_VERSION=1.1.0
WORKDIR /tmp
RUN curl -sSL "https://capnproto.org/capnproto-c++-${CAPNP_VERSION}.tar.gz" -o capnproto.tar.gz && \
    tar zxf capnproto.tar.gz && \
    cd capnproto-c++-${CAPNP_VERSION} && \
    ./configure && \
    make -j$(nproc || echo 2) install && \
    ldconfig && \
    cd .. && \
    rm -rf capnproto-c++-${CAPNP_VERSION} capnproto.tar.gz

WORKDIR /app

# Copy toolchain file first to ensure it's used.
COPY rust-toolchain.toml ./rust-toolchain.toml

# Copy Cargo.toml and Cargo.lock for workspace and members to cache dependency resolution/downloads.
COPY Cargo.toml Cargo.lock ./
COPY interface/Cargo.toml ./interface/Cargo.toml
COPY daemon/Cargo.toml ./daemon/Cargo.toml
COPY enclave/Cargo.toml ./enclave/Cargo.toml
COPY vm/base/Cargo.toml ./vm/base/Cargo.toml
COPY vm/workerd/Cargo.toml ./vm/workerd/Cargo.toml
RUN mkdir -p interface/src daemon/src enclave/src vm/base/src vm/workerd/src && \
    touch interface/src/lib.rs daemon/src/main.rs enclave/src/main.rs vm/base/src/lib.rs vm/workerd/src/main.rs

# Cache the crates
RUN cargo metadata --format-version 1 > /dev/null

# Copy all source code.
# This is done after Cargo files for better layer caching if only source code changes.
COPY interface ./interface
COPY daemon ./daemon
COPY enclave ./enclave
COPY vm ./vm

# Build mode configuration. Set `--build-arg BUILD_MODE=` to do a debug build.
ARG BUILD_MODE=release
ENV BUILD_FLAG=${BUILD_MODE:+--release}

# Build crates in separate layers to improve cacheability
RUN cargo build --locked ${BUILD_FLAG} -p nxcc-workerd-vm --no-default-features --features uds
RUN cargo build --locked ${BUILD_FLAG} -p nxcc-platform-enclave --no-default-features --features uds
RUN cargo build --locked ${BUILD_FLAG} -p nxcc-daemon

# Stage 2: Runtime
FROM debian:12-slim AS runtime

# Install tini as an init system, and ca-certificates.
RUN apt-get update && apt-get install -y --no-install-recommends \
    tini \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

ARG USERNAME=nxcc
ARG USER_UID=1001
ARG USER_GID=${USER_UID}

# Create a non-root user and group.
RUN groupadd --gid ${USER_GID} ${USERNAME} && \
    useradd --uid ${USER_UID} --gid ${USER_GID} --shell /bin/false --create-home ${USERNAME}

# Create directory for Unix Domain Sockets.
RUN mkdir -p /run/nxcc && chown ${USERNAME}:${USERNAME} /run/nxcc

WORKDIR /home/${USERNAME}/app
USER ${USERNAME}

# Copy the workerd binary from the downloader stage.
COPY --from=workerd-downloader --chown=${USERNAME}:${USERNAME} /opt/workerd /usr/local/bin/workerd

# Copy compiled Rust binaries from the builder stage, using BUILD_MODE to determine the path
ARG BUILD_MODE=release
# Use debug directory when BUILD_MODE is empty, otherwise use the specified BUILD_MODE
COPY --from=builder --chown=${USERNAME}:${USERNAME} /app/target/${BUILD_MODE:-debug}/nxcc-daemon /usr/local/bin/nxcc-daemon
COPY --from=builder --chown=${USERNAME}:${USERNAME} /app/target/${BUILD_MODE:-debug}/nxcc-platform-enclave /usr/local/bin/nxcc-platform-enclave
COPY --from=builder --chown=${USERNAME}:${USERNAME} /app/target/${BUILD_MODE:-debug}/nxcc-workerd-vm /usr/local/bin/nxcc-workerd-vm

# Copy the entrypoint script.
COPY --chown=${USERNAME}:${USERNAME} entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Environment variables for default configuration.
# These can be overridden at `docker run -e ...`.
ENV NXCC_ALL_VERBOSE="false"
ENV DAEMON_GRPC_TARGET_ADDR="0.0.0.0:50051"
ENV DAEMON_P2P_LISTEN_ADDR="/ip4/0.0.0.0/tcp/9000"
ENV ENCLAVE_UDS_SOCKET="/run/nxcc/enclave.sock"
ENV WORKERD_UDS_SOCKET="/run/nxcc/workerd.sock"
ENV WORKERD_BIN_PATH_ABS="/usr/local/bin/workerd"
# Optional: Define paths for identity and policy cache if persistent storage is desired.
# These would require creating and chowning the directories (e.g., /data) and mounting volumes.
# ENV NXCC_IDENTITY_PATH="/data/identity.key"
# ENV NXCC_POLICY_CACHE_DIR="/data/policy_cache"

# Expose ports for daemon's libp2p and gRPC services.
EXPOSE 9000/tcp
EXPOSE 50051/tcp

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/entrypoint.sh"]
