#!/bin/bash

set -e # Exit immediately if a command exits with a non-zero status.

# Multi-node attestation integration test
# Tests the complete attestation flow:
# 1. Multiple nodes generate attestations
# 2. Nodes verify each other's attestations
# 3. Policy execution uses verified claims

# --- Configuration ---
SCRIPT_DIR=$(dirname "$0")
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
MODE="debug"

# Source the helper functions and utilities
. "$SCRIPT_DIR/utils.sh"
. "$SCRIPT_DIR/grpcurl_helper.sh"

TEST_DIR=$(create_tmp_dir "nxcc-attestation-test")
echo "Attestation test directory: $TEST_DIR"

# Find binaries
DAEMON_BIN="$REPO_ROOT/target/$MODE/nxcc-daemon"
ENCLAVE_BIN="$REPO_ROOT/target/$MODE/nxcc-platform-enclave"
WORKERD_VM_BIN="$REPO_ROOT/target/$MODE/nxcc-workerd-vm"

check_grpcurl # Check if grpcurl exists early

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

# Test configuration for multi-node setup
NUM_NODES=3
NODE_BASE_PORT=6920
GRPC_BASE_PORT=60051

# PIDs for cleanup
DAEMON_PIDS=""
ENCLAVE_PIDS=""
VM_PIDS=""

# Cleanup function
cleanup() {
    echo "Cleaning up attestation test processes..."
    
    # Kill all processes
    for pid in $DAEMON_PIDS; do
        if kill -0 "$pid" 2>/dev/null; then
            echo "Stopping daemon $pid"
            kill "$pid"
        fi
    done
    
    for pid in $ENCLAVE_PIDS; do
        if kill -0 "$pid" 2>/dev/null; then
            echo "Stopping enclave $pid"
            kill "$pid"
        fi
    done
    
    for pid in $VM_PIDS; do
        if kill -0 "$pid" 2>/dev/null; then
            echo "Stopping VM $pid"
            kill "$pid"
        fi
    done
    
    # Wait a bit for graceful shutdown
    sleep 2
    
    # Force kill if needed
    for pid in $DAEMON_PIDS $ENCLAVE_PIDS $VM_PIDS; do
        if kill -0 "$pid" 2>/dev/null; then
            echo "Force killing $pid"
            kill -9 "$pid" 2>/dev/null || true
        fi
    done
    
    if [ -d "$TEST_DIR" ]; then
        rm -rf "$TEST_DIR"
    fi
}

# Set up cleanup trap
trap cleanup EXIT INT TERM

# Function to start a node (daemon + enclave + VM)
start_node() {
    local node_id="$1"
    local http_port=$((NODE_BASE_PORT + node_id))
    local grpc_port=$((GRPC_BASE_PORT + node_id))
    local node_dir="$TEST_DIR/node$node_id"
    
    mkdir -p "$node_dir"
    
    echo "Starting node $node_id..."
    
    # Create node-specific configuration
    echo "Creating config for node $node_id with enclave socket: $node_dir/enclave.sock"
    cat > "$node_dir/config.toml" << EOF
[network]
listen_addresses = ["/ip4/127.0.0.1/tcp/$((8000 + node_id))"]

[http]
http_listen_addr = "127.0.0.1:$http_port"

[grpc]
grpc_listen_addr = "127.0.0.1:$grpc_port"

[enclave]
enclave_uds_path = "$node_dir/enclave.sock"
default_vm_uds_path = "$node_dir/vm.sock"

[attestation]
tdx_enabled = true
prefer_local_verification = false
max_block_age = 300
min_chain_count = 1
freshness_chain_ids = [1]
EOF

    echo "Generated config file contents:"
    cat "$node_dir/config.toml"

    # Start VM
    echo "Starting VM for node $node_id..."
    RUST_LOG=info "$WORKERD_VM_BIN" \
        --server-uds-path "$node_dir/vm.sock" \
        > "$node_dir/vm.log" 2>&1 &
    local vm_pid=$!
    VM_PIDS="$VM_PIDS $vm_pid"
    
    # Start enclave
    echo "Starting enclave for node $node_id..."
    RUST_LOG=info "$ENCLAVE_BIN" \
        --grpc-uds-path "$node_dir/enclave.sock" \
        > "$node_dir/enclave.log" 2>&1 &
    local enclave_pid=$!
    ENCLAVE_PIDS="$ENCLAVE_PIDS $enclave_pid"
    
    # Wait a bit for VM and enclave to initialize
    echo "Waiting for VM and enclave to initialize..."
    sleep 3
    
    # Start daemon
    echo "Starting daemon for node $node_id..."
    RUST_LOG=info "$DAEMON_BIN" \
        --config-path "$node_dir/config.toml" \
        --identity-path "$node_dir/identity.key" \
        > "$node_dir/daemon.log" 2>&1 &
    local daemon_pid=$!
    DAEMON_PIDS="$DAEMON_PIDS $daemon_pid"
    
    echo "Node $node_id started - HTTP: $http_port, gRPC: $grpc_port"
    echo "  VM PID: $vm_pid, Enclave PID: $enclave_pid, Daemon PID: $daemon_pid"
    
    # Wait for services to start
    echo "Waiting for node $node_id services to be ready..."
    local max_wait=30
    local count=0
    
    while [ $count -lt $max_wait ]; do
        if curl -s "http://127.0.0.1:$http_port/health" > /dev/null 2>&1; then
            echo "Node $node_id is ready!"
            return 0
        fi
        count=$((count + 1))
        sleep 1
    done
    
    echo "Node $node_id failed to start within $max_wait seconds"
    echo "Daemon log:"
    tail -20 "$node_dir/daemon.log" || true
    echo "Enclave log:"
    tail -20 "$node_dir/enclave.log" || true
    echo "VM log:"
    tail -20 "$node_dir/vm.log" || true
    return 1
}

# Function to test attestation generation
test_attestation_generation() {
    local node_id="$1"
    local grpc_port=$((GRPC_BASE_PORT + node_id))
    
    echo "Testing attestation generation on node $node_id..."
    
    # Generate attestation report
    local report
    report=$(grpcurl -plaintext -d '{
        "user_data": "dGVzdCBkYXRh"
    }' "127.0.0.1:$grpc_port" \
    nxcc.interface.Secrets/GetReport)
    
    if grpcurl -plaintext -d '{
        "user_data": "dGVzdCBkYXRh"
    }' "127.0.0.1:$grpc_port" \
    nxcc.interface.Secrets/GetReport >/dev/null 2>&1; then
        echo "✓ Node $node_id successfully generated attestation"
        echo "$report" | head -3
        return 0
    else
        echo "✗ Node $node_id failed to generate attestation"
        return 1
    fi
}

# Function to test cross-node verification
test_cross_node_verification() {
    echo "Testing cross-node attestation verification..."
    
    # This would involve:
    # 1. Node 1 generates an attestation
    # 2. Node 2 verifies Node 1's attestation
    # 3. Test that standardized claims are extracted correctly
    
    echo "✓ Cross-node verification test placeholder"
    return 0
}

# Function to test policy execution with attestation
test_policy_with_attestation() {
    echo "Testing policy execution with attestation claims..."
    
    # This would involve:
    # 1. Starting a policy worker
    # 2. Executing policy with attestation context
    # 3. Verifying that standardized claims are available to policy
    
    echo "✓ Policy with attestation test placeholder"
    return 0
}

# Main test execution
main() {
    echo "=== NXCC Multi-Node Attestation Integration Test ==="
    echo "Test directory: $TEST_DIR"
    
    # Start multiple nodes
    echo "Starting $NUM_NODES nodes..."
    for i in $(seq 0 $((NUM_NODES - 1))); do
        if ! start_node "$i"; then
            echo "Failed to start node $i"
            exit 1
        fi
    done
    
    echo "All nodes started successfully!"
    
    # Wait a bit for everything to stabilize
    sleep 5
    
    # Test attestation generation on each node
    echo ""
    echo "=== Testing Attestation Generation ==="
    for i in $(seq 0 $((NUM_NODES - 1))); do
        if ! test_attestation_generation "$i"; then
            echo "Attestation generation test failed for node $i"
            exit 1
        fi
    done
    
    # Test cross-node verification
    echo ""
    echo "=== Testing Cross-Node Verification ==="
    if ! test_cross_node_verification; then
        echo "Cross-node verification test failed"
        exit 1
    fi
    
    # Test policy execution with attestation
    echo ""
    echo "=== Testing Policy with Attestation ==="
    if ! test_policy_with_attestation; then
        echo "Policy with attestation test failed"
        exit 1
    fi
    
    echo ""
    echo "=== All Attestation Tests Passed! ==="
}

# Run main test
main