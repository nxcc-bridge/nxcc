#!/bin/bash

set -e

# --- Configuration ---
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
NODE_DIR="$REPO_ROOT/node"
BENCH_DIR="$REPO_ROOT/benchmarks"

# Default resource limits
CPUS="2"
MEMORY="2g"
NODE_IMAGE="nxcc-node:bench"
DOCKER_NETWORK="nxcc-bench-net"

# --- Helper Functions ---
info() {
  echo "INFO: $1"
}

error() {
  echo "ERROR: $1" >&2
  exit 1
}

# --- Argument Parsing ---
while [ "$#" -gt 0 ]; do
  case "$1" in
    --cpus) CPUS="$2"; shift 2;;
    --memory) MEMORY="$2"; shift 2;;
    --help)
      echo "Usage: $0 [--cpus <num>] [--memory <mem>]"
      echo "  --cpus: Number of CPUs to allocate to the node container (default: 2)"
      echo "  --memory: Memory to allocate to the node container (default: 2g)"
      exit 0
      ;;
    *) error "Unknown option: $1";;
  esac
done

# --- Main Functions ---

restart_node() {
  info "Restarting node container..."
  docker stop nxcc-bench-node >/dev/null 2>&1 || true
  docker run -d --rm \
    --name nxcc-bench-node \
    --cpus="$CPUS" \
    --memory="$MEMORY" \
    --network "$DOCKER_NETWORK" \
    --network-alias nxcc-node \
    -p 50051:50051 \
    -p 9000:9000 \
    -p 6922:6922 \
    -e NXCC_ALL_VERBOSE=true \
    -e DAEMON_GRPC_TARGET_ADDR="0.0.0.0:50051" \
    -e DAEMON_P2P_LISTEN_ADDR="/ip4/0.0.0.0/tcp/9000" \
    "$NODE_IMAGE" >/dev/null
  info "Waiting for node to be ready..."
  sleep 5
}

setup() {
  info "--- Setting up benchmark environment ---"

  info "Building node Docker image..."
  # Build arguments for Docker
  BUILD_ARGS="--build-arg BUILD_MODE=release"
  # Add ARM64 build arg if running on ARM64 platform
  if [[ "$(uname -m)" == "arm64" || "$(uname -m)" == "aarch64" ]]; then
    BUILD_ARGS="$BUILD_ARGS --build-arg WORKERD_ARCH=linux-arm64"
  fi
  docker build $BUILD_ARGS -t "$NODE_IMAGE" "$NODE_DIR"

  info "Building test contracts..."
  (cd "$NODE_DIR/tests" && FOUNDRY_PROFILE=release forge build)

  info "Building benchmark runner..."
  (cd "$BENCH_DIR" && cargo build --release)

  info "Building benchmark JS workers..."
  (cd "$BENCH_DIR/workers" && npm install && npm run build)

  if ! docker network inspect "$DOCKER_NETWORK" >/dev/null 2>&1; then
    info "Creating Docker network: $DOCKER_NETWORK"
    docker network create "$DOCKER_NETWORK"
  fi

  info "Starting Anvil container..."
  docker run -d --rm --name anvil-bench -p 8545:8545 --network "$DOCKER_NETWORK" --network-alias anvil ghcr.io/foundry-rs/foundry:latest anvil --host 0.0.0.0 >/dev/null
}

run_benchmarks() {
  info "--- Running benchmarks ---"

  restart_node
  "$BENCH_DIR/target/release/benchmarks" \
    --node-grpc-addr "http://localhost:50051" \
    --anvil-rpc-url "http://localhost:8545" \
    idle

  restart_node
  "$BENCH_DIR/target/release/benchmarks" \
    --node-grpc-addr "http://localhost:50051" \
    --anvil-rpc-url "http://localhost:8545" \
    cpu

  restart_node
  "$BENCH_DIR/target/release/benchmarks" \
    --node-grpc-addr "http://localhost:50051" \
    --anvil-rpc-url "http://localhost:8545" \
    io

  restart_node
  "$BENCH_DIR/target/release/benchmarks" \
    --node-grpc-addr "http://localhost:50051" \
    --anvil-rpc-url "http://localhost:8545" \
    realistic

  restart_node
  "$BENCH_DIR/target/release/benchmarks" \
    --node-grpc-addr "http://localhost:50051" \
    --anvil-rpc-url "http://localhost:8545" \
    web3-throughput

  restart_node
  "$BENCH_DIR/target/release/benchmarks" \
    --node-grpc-addr "http://localhost:50051" \
    --anvil-rpc-url "http://localhost:8545" \
    web3-latency
}

teardown() {
  info "--- Tearing down benchmark environment ---"
  docker logs nxcc-bench-node >&2 || true
  docker stop nxcc-bench-node >/dev/null 2>&1 || true
  docker stop anvil-bench >/dev/null 2>&1 || true
  if docker network inspect "$DOCKER_NETWORK" >/dev/null 2>&1; then
      info "Removing Docker network: $DOCKER_NETWORK"
      docker network rm "$DOCKER_NETWORK" >/dev/null 2>&1 || true
  fi
  info "Teardown complete."
}

# --- Execution ---
trap teardown EXIT INT TERM

setup
run_benchmarks

info "Benchmark run finished successfully."
