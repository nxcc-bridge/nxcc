#!/bin/sh

set -e # Exit immediately if a command exits with a non-zero status.
# set -x # Debugging: print commands

# --- Configuration ---
SCRIPT_DIR=$(dirname "$0")
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
MODE="debug"

# Source the helper functions and utilities
. "$SCRIPT_DIR/utils.sh"
. "$SCRIPT_DIR/grpcurl_helper.sh"

TEST_DIR=$(create_tmp_dir "nxcc-test")
echo "Test directory: $TEST_DIR"

# Find binaries (assuming they are built in target/release)
DAEMON_BIN="$REPO_ROOT/target/$MODE/nxcc-daemon"
ENCLAVE_BIN="$REPO_ROOT/target/$MODE/nxcc-platform-enclave"
WORKERD_VM_BIN="$REPO_ROOT/target/$MODE/nxcc-workerd-vm" # Use the base server binary

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

# Test Parameters
SECRET_CHAIN_ID=0 # Mock chain
SECRET_IDENTITY_ADDR="0x1111111111111111111111111111111111111111"
SECRET_IDENTITY_ID_NUM="555" # Use a simple numeric string for grpcurl

NODE1_NAME="alice"
NODE2_NAME="bob"
NODE1_PORT=9001 # For libp2p TCP listening
NODE2_PORT=9002 # For libp2p TCP listening

# --- Cleanup Function ---
cleanup() {
    echo "Cleaning up..."
    
    # Kill any background grpcurl processes
    if [ -n "$GET_SECRETS_ALICE_PID" ] && ps -p $GET_SECRETS_ALICE_PID >/dev/null 2>&1; then
        echo "Killing Alice grpcurl process (PID: $GET_SECRETS_ALICE_PID)"
        kill -9 $GET_SECRETS_ALICE_PID >/dev/null 2>&1 || true
    fi
    
    if [ -n "$GET_SECRETS_BOB_PID" ] && ps -p $GET_SECRETS_BOB_PID >/dev/null 2>&1; then
        echo "Killing Bob grpcurl process (PID: $GET_SECRETS_BOB_PID)"
        kill -9 $GET_SECRETS_BOB_PID >/dev/null 2>&1 || true
    fi
    
    # Kill any other potential background processes
    pkill -P $$ >/dev/null 2>&1 || true
    
    # Kill node processes
    cleanup_node "$NODE1_NAME"
    cleanup_node "$NODE2_NAME"
    
    # Remove the test directory
    if [ -d "$TEST_DIR" ]; then
        echo "Removing test directory: $TEST_DIR"
        rm -rf "$TEST_DIR"
    fi

    killall nxcc-daemon 2>&1 || true # why?
    
    echo "Cleanup finished."
    
    # Force exit to ensure no lingering processes
    trap - EXIT SIGINT SIGTERM
}

# Set up trap for signals
trap cleanup EXIT INT TERM

# --- Node 1 (Alice) Setup ---
echo "--- Setting up Node 1 (Alice) ---"
setup_node "$NODE1_NAME" "$TEST_DIR" "$NODE1_PORT" "" \
    "$DAEMON_BIN" "$ENCLAVE_BIN" "$WORKERD_VM_BIN"

# Sleep after node setup to ensure everything is ready
sleep 3

# Attach VM to Enclave via Daemon
grpcurl_attach_vm "$alice_DAEMON_SOCK" "policy-vm-0" "$alice_VM_SOCK"

# Sleep after VM attachment
sleep 3

# --- Node 2 (Bob) Setup ---
echo "--- Setting up Node 2 (Bob) ---"
setup_node "$NODE2_NAME" "$TEST_DIR" "$NODE2_PORT" "$alice_MULTIADDR" \
    "$DAEMON_BIN" "$ENCLAVE_BIN" "$WORKERD_VM_BIN"

# Sleep after node setup to ensure everything is ready
sleep 3

# Attach VM to Enclave via Daemon
grpcurl_attach_vm "$bob_DAEMON_SOCK" "policy-vm-0" "$bob_VM_SOCK"

# Sleep after VM attachment and before starting the test workflow
sleep 5

# --- Test Workflow ---
echo "--- Starting Test Workflow ---"

# 0. Initial state check (optional)
echo "Initial Check: Checking for secret on Alice..."
# Use simple grep for sh compatibility
grpcurl_check_secrets_enclave "$alice_ENCLAVE_SOCK" "$SECRET_CHAIN_ID" "$SECRET_IDENTITY_ADDR" "$SECRET_IDENTITY_ID_NUM" | grep -v '"found": *true' >/dev/null || (
    echo "ERROR: Secret found initially on Alice!"
    exit 1
)

# Sleep between checks
sleep 2

echo "Initial Check: Checking for secret on Bob..."
grpcurl_check_secrets_enclave "$bob_ENCLAVE_SOCK" "$SECRET_CHAIN_ID" "$SECRET_IDENTITY_ADDR" "$SECRET_IDENTITY_ID_NUM" | grep -v '"found": *true' >/dev/null || (
    echo "ERROR: Secret found initially on Bob!"
    exit 1
)

# Sleep before starting the main test steps
sleep 3

# 1. Alice receives GetSecrets request
echo "Step 1 & 2 & 3: Triggering GetSecrets on Alice (will initiate P2P ask & generation)..."
# Run in background as it might involve waiting/generation
grpcurl_get_secrets "$alice_DAEMON_SOCK" "$SECRET_CHAIN_ID" "$SECRET_IDENTITY_ADDR" "$SECRET_IDENTITY_ID_NUM" &
GET_SECRETS_ALICE_PID=$!
# We don't wait for this specific call to finish, as the important part is the side effect (generation)

# Sleep to allow Alice time to process the request
sleep 5

# 3a. Wait for Alice to generate the secret
echo "Step 3a: Waiting for Alice to generate the secret..."
poll_until_secret_found "$alice_ENCLAVE_SOCK" "$SECRET_CHAIN_ID" "$SECRET_IDENTITY_ADDR" "$SECRET_IDENTITY_ID_NUM" 60 3 || exit 1 # 60s timeout, check every 3s (increased from 2s)

# Sleep after Alice generates the secret
sleep 5

# 4. Bob receives GetSecrets request
echo "Step 4 & 5: Triggering GetSecrets on Bob (will initiate P2P ask to Alice)..."
# Run in background
grpcurl_get_secrets "$bob_DAEMON_SOCK" "$SECRET_CHAIN_ID" "$SECRET_IDENTITY_ADDR" "$SECRET_IDENTITY_ID_NUM" &
GET_SECRETS_BOB_PID=$!

# Sleep to allow Bob time to process the request
sleep 5

# 5a. Wait for Bob to receive the secret from Alice
echo "Step 5a, 6, 7: Waiting for Bob to receive the secret..."
poll_until_secret_found "$bob_ENCLAVE_SOCK" "$SECRET_CHAIN_ID" "$SECRET_IDENTITY_ADDR" "$SECRET_IDENTITY_ID_NUM" 60 3 || exit 1 # Increased interval from 2s to 3s

# Sleep after Bob receives the secret
sleep 5

# 8. Final state check
echo "Step 8: Final check..."
echo "Final Check: Checking for secret on Alice..."
grpcurl_check_secrets_enclave "$alice_ENCLAVE_SOCK" "$SECRET_CHAIN_ID" "$SECRET_IDENTITY_ADDR" "$SECRET_IDENTITY_ID_NUM" | grep '"found": *true' >/dev/null || (
    echo "ERROR: Secret NOT found finally on Alice!"
    exit 1
)

# Sleep between checks
sleep 2

echo "Final Check: Checking for secret on Bob..."
grpcurl_check_secrets_enclave "$bob_ENCLAVE_SOCK" "$SECRET_CHAIN_ID" "$SECRET_IDENTITY_ADDR" "$SECRET_IDENTITY_ID_NUM" | grep '"found": *true' >/dev/null || (
    echo "ERROR: Secret NOT found finally on Bob!"
    exit 1
)

# Wait for background grpcurl calls to finish (optional, mostly for cleaner logs)
if [ -n "$GET_SECRETS_ALICE_PID" ]; then
    wait $GET_SECRETS_ALICE_PID 2>/dev/null || true
fi
if [ -n "$GET_SECRETS_BOB_PID" ]; then
    wait $GET_SECRETS_BOB_PID 2>/dev/null || true
fi

# Final sleep before declaring success
sleep 3

echo "--- Test Workflow Completed Successfully ---"

# Cleanup is handled by the trap
exit 0

