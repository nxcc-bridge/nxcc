#!/bin/sh

# run.sh - start one or more nxcc nodes locally
# Usage: ./run.sh [num_nodes]

set -e

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
TESTS_DIR="$SCRIPT_DIR/tests"

# Source helper utilities used by integration tests
# shellcheck disable=SC1091  # utils.sh is sourced dynamically
. "$TESTS_DIR/utils.sh"

# Minimal grpcurl helpers
PROTO_DIR="$SCRIPT_DIR/interface/proto"
DAEMON_PROTO="daemon.proto"

check_grpcurl() {
  if ! command -v grpcurl >/dev/null 2>&1; then
    echo "Error: grpcurl command not found. Please install it." >&2
    exit 1
  fi
}

check_cargo() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo command not found. Please install Rust and Cargo." >&2
    exit 1
  fi
}

build_binaries() {
  echo "Building nxcc binaries..."
  cargo build --manifest-path "$SCRIPT_DIR/Cargo.toml" --target-dir "$SCRIPT_DIR/target" \
    -p nxcc-daemon -p nxcc-platform-enclave -p nxcc-workerd-vm >/dev/null
}

grpcurl_attach_vm() {
  _daemon_sock="$1"
  _vm_id="$2"
  _vm_sock="$3"
  for _ in $(seq 1 10); do
    if grpcurl \
      -proto "$DAEMON_PROTO" \
      -import-path "$PROTO_DIR" \
      -plaintext -unix \
      -d '{"vm_id":"'"${_vm_id}"'","uds_path":"'"${_vm_sock}"'"}' \
      "unix://$_daemon_sock" \
      daemon.Debug/AttachVm >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "Failed to attach VM via grpcurl after retries." >&2
  return 1
}

NUM_NODES="${1:-1}"
MODE="debug"

RUN_DIR=$(create_tmp_dir "nxcc-run")
echo "Run directory: $RUN_DIR"

daemon_bin="$SCRIPT_DIR/target/$MODE/nxcc-daemon"
enclave_bin="$SCRIPT_DIR/target/$MODE/nxcc-platform-enclave"
vm_bin="$SCRIPT_DIR/target/$MODE/nxcc-workerd-vm"

check_grpcurl
check_cargo
build_binaries

if [ ! -f "$daemon_bin" ] || [ ! -f "$enclave_bin" ] || [ ! -f "$vm_bin" ]; then
  echo "Required binaries not found. Build failed?" >&2
  exit 1
fi

cleanup() {
  set +e
  echo "Cleaning up..."
  for i in $(seq 1 "$NUM_NODES"); do
    cleanup_node "node$i"
  done
  [ -d "$RUN_DIR" ] && rm -rf "$RUN_DIR"
  echo "Cleanup complete."
}

trap cleanup EXIT INT TERM

BOOTSTRAP=""
for i in $(seq 1 "$NUM_NODES"); do
  NAME="node$i"
  P2P_PORT=$((9000 + i))
  HTTP_PORT=$((6921 + i))
  echo "--- Starting $NAME on ports $P2P_PORT/$HTTP_PORT ---"
  setup_node "$NAME" "$RUN_DIR" "$P2P_PORT" "$BOOTSTRAP" \
    "$daemon_bin" "$enclave_bin" "$vm_bin" "127.0.0.1:$HTTP_PORT"
  eval DAEMON_SOCK="\$${NAME}_DAEMON_SOCK"
  eval VM_ID="\$${NAME}_VM_ID"
  eval VM_SOCK="\$${NAME}_VM_SOCK"
  sleep 1
  if ! grpcurl_attach_vm "$DAEMON_SOCK" "$VM_ID" "$VM_SOCK"; then
    echo "Warning: failed to attach VM for $NAME" >&2
  fi
  if [ "$i" -eq 1 ]; then
    eval BOOTSTRAP="\$${NAME}_MULTIADDR"
  fi
  sleep 1
done

echo "All $NUM_NODES node(s) started. Press Ctrl+C to stop."
wait
