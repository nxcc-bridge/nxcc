# NXCC Development Environment (multi-platform)
# Universal Docker environment for building and testing the complete NXCC project
FROM rust:1.89.0-slim-bookworm

ENV CARGO_TERM_COLOR=always
ENV NODE_ENV=development

# Install system dependencies for all project components
RUN apt-get update && apt-get install -y --no-install-recommends \
    # Rust build dependencies
    protobuf-compiler libprotobuf-dev \
    build-essential autoconf automake libtool pkg-config \
    # Node.js and development tools
    curl tar gzip wget unzip \
    # Git and editor tools
    git vim nano less \
    # Kubernetes/Docker tools for e2e testing
    ca-certificates sudo \
    # Additional tools for testing and development
    jq \
    && rm -rf /var/lib/apt/lists/*

# Install Cap'n Proto (matching production version)
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

# Install Node.js 22.x and pnpm
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - && \
    apt-get install -y nodejs && \
    npm install -g pnpm@latest && \
    node --version && \
    npm --version && \
    pnpm --version

# Install Foundry toolchain (forge, anvil, chisel)
RUN curl -L https://foundry.paradigm.xyz | bash && \
    export PATH="$PATH:/root/.foundry/bin" && \
    /root/.foundry/bin/foundryup && \
    ln -s /root/.foundry/bin/* /usr/local/bin/

# Install kubectl for e2e testing (runtime platform detection)
RUN ARCH=$(dpkg --print-architecture | sed 's/amd64/amd64/; s/arm64/arm64/') && \
    curl -LO "https://dl.k8s.io/release/$(curl -L -s https://dl.k8s.io/release/stable.txt)/bin/linux/${ARCH}/kubectl" && \
    chmod +x kubectl && \
    mv kubectl /usr/local/bin/

# Install kind for local kubernetes testing (runtime platform detection)
RUN ARCH=$(dpkg --print-architecture | sed 's/amd64/amd64/; s/arm64/arm64/') && \
    curl -Lo ./kind https://kind.sigs.k8s.io/dl/v0.24.0/kind-linux-${ARCH} && \
    chmod +x kind && \
    mv kind /usr/local/bin/

# Install grpcurl for gRPC testing (runtime platform detection)
RUN ARCH=$(dpkg --print-architecture | sed 's/amd64/x86_64/; s/arm64/arm64/') && \
    curl -sSL "https://github.com/fullstorydev/grpcurl/releases/download/v1.9.1/grpcurl_1.9.1_linux_${ARCH}.tar.gz" | \
    tar -xz -C /usr/local/bin grpcurl

# Create development user (matches host user for file permissions)
ARG USERNAME=dev
ARG USER_UID=1000
ARG USER_GID=$USER_UID

RUN groupadd --gid $USER_GID $USERNAME \
    && useradd --uid $USER_UID --gid $USER_GID -m $USERNAME -s /bin/bash \
    && echo $USERNAME ALL=\(root\) NOPASSWD:ALL > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

# Switch to development user
USER $USERNAME
WORKDIR /home/$USERNAME

# Install Rust components
RUN rustup component add clippy rustfmt

# Install Foundry for the development user
RUN curl -L https://foundry.paradigm.xyz | bash && \
    export PATH="$PATH:/home/$USERNAME/.foundry/bin" && \
    /home/$USERNAME/.foundry/bin/foundryup

# Update PATH for development user to include Foundry
ENV PATH="/home/$USERNAME/.foundry/bin:$PATH"

# Pre-warm Cargo registry for faster first builds
RUN cargo search cargo-watch --limit 1 > /dev/null 2>&1 || true

# Set working directory
WORKDIR /workspace

# Verify all tools are installed correctly
RUN echo "Development container architecture: $(uname -m)" && \
    echo "=== Tool Versions ===" && \
    rustc --version --verbose && \
    echo "Node.js: $(node --version)" && \
    echo "npm: $(npm --version)" && \
    echo "pnpm: $(pnpm --version)" && \
    echo "forge: $(forge --version)" && \
    echo "anvil: $(anvil --version)" && \
    echo "kubectl: $(kubectl version --client --short)" && \
    echo "kind: $(kind --version)" && \
    echo "grpcurl: $(grpcurl --version)" && \
    echo "jq: $(jq --version)" && \
    echo "=== All tools ready ==="
