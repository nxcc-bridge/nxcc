#!/bin/sh

set -e # Exit immediately if a command exits with a non-zero status.

# --- Find script directory and load utilities ---
SCRIPT_DIR=$(dirname "$0")
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
MODE="debug"

# Source utility functions
. "$SCRIPT_DIR/utils.sh"
. "$SCRIPT_DIR/grpcurl_helper.sh"

# --- Configuration ---
TEST_DIR=$(create_tmp_dir "nxcc-dev")
echo "Node directory: $TEST_DIR"

# Find binaries
DAEMON_BIN="$REPO_ROOT/target/$MODE/nxcc-daemon"
ENCLAVE_BIN="$REPO_ROOT/target/$MODE/nxcc-platform-enclave"
WORKERD_VM_BIN="$REPO_ROOT/target/$MODE/nxcc-workerd-vm"

# Check if binaries exist
if [ ! -f "$DAEMON_BIN" ]; then
    echo "Daemon binary not found at $DAEMON_BIN. Build first."
    exit 1
fi
if [ ! -f "$ENCLAVE_BIN" ]; then
    echo "Enclave binary not found at $ENCLAVE_BIN. Build first."
    exit 1
fi
if [ ! -f "$WORKERD_VM_BIN" ]; then
    echo "Workerd VM binary not found at $WORKERD_VM_BIN. Build first."
    exit 1
fi

# Node parameters
NODE_NAME="dev"
NODE_PORT=9000
BOOTSTRAP_PEERS="" # No bootstrap peers for dev node

# --- Cleanup Function ---
cleanup() {
    echo "Cleaning up..."
    # Kill node processes
    cleanup_node "$NODE_NAME"
    # Remove the test directory
    if [ -d "$TEST_DIR" ]; then
        echo "Removing test directory: $TEST_DIR"
        rm -rf "$TEST_DIR"
    fi
    echo "Cleanup finished."
}
trap cleanup EXIT INT TERM

# --- Setup Node ---
echo "=== Setting up $NODE_NAME node ==="
setup_node "$NODE_NAME" "$TEST_DIR" "$NODE_PORT" "$BOOTSTRAP_PEERS" \
    "$DAEMON_BIN" "$ENCLAVE_BIN" "$WORKERD_VM_BIN"

# Attach VM to Enclave via Daemon
eval "DAEMON_SOCK=\$${NODE_NAME}_DAEMON_SOCK"
eval "VM_SOCK=\$${NODE_NAME}_VM_SOCK"
grpcurl_attach_vm "$DAEMON_SOCK" "policy-vm-0" "$VM_SOCK"

# --- Print Node Information ---
echo "=== Node Information ==="
eval "echo \"Peer ID: \$${NODE_NAME}_PEER_ID\""
eval "echo \"Multiaddr: \$${NODE_NAME}_MULTIADDR\""
eval "echo \"Daemon Socket: \$${NODE_NAME}_DAEMON_SOCK\""
eval "echo \"Enclave Socket: \$${NODE_NAME}_ENCLAVE_SOCK\""
eval "echo \"VM Socket: \$${NODE_NAME}_VM_SOCK\""
eval "echo \"Identity Path: \$${NODE_NAME}_IDENTITY\""

# --- Keep Running Until Interrupted ---
echo "=== Node is running ==="
echo "Press Ctrl+C to stop the node"

# Wait indefinitely (until Ctrl+C)
while true; do
    sleep 1
done
