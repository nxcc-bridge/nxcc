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
SECRET_IDENTITY_ID_NUM="555"       # Use a simple numeric string for grpcurl
SECRET_NAME_IN_WORKER="THE_SECRET" # How the secret is named in the worker's env

# Path to the JS worker script for the test
TEST_WORKER_JS_PATH="$SCRIPT_DIR/policy/test_worker_integration.js"
DSSE_WORK_ORDER_PAYLOAD_TYPE="application/vnd.nxcc.workorderpayload.v1+json"

NODE1_NAME="alice"
NODE2_NAME="bob"
NODE1_PORT=9001 # For libp2p TCP listening
NODE2_PORT=9002 # For libp2p TCP listening

# --- Cleanup Function ---
cleanup() {
	echo "Cleaning up..."

	# Kill any other potential background processes
	# This is a bit aggressive, ensure it doesn't kill unrelated processes if tests run in parallel
	pkill -P $$ >/dev/null 2>&1 || true

	# Kill node processes
	cleanup_node "$NODE1_NAME"
	cleanup_node "$NODE2_NAME"

	# Remove the test directory
	if [ -d "$TEST_DIR" ]; then
		echo "Removing test directory: $TEST_DIR"
		rm -rf "$TEST_DIR"
	fi

	killall nxcc-daemon 2>&1 || true

	echo "Cleanup finished."

	# Force exit to ensure no lingering processes
	trap - EXIT SIGINT SIGTERM
}

# Set up trap for signals
trap cleanup EXIT INT TERM

# --- Prepare Test Worker and Work Order ---
echo "--- Preparing Test Worker and Work Order ---"

# 1. JS Worker Code
TEST_WORKER_JS_CONTENT=$(cat "$TEST_WORKER_JS_PATH")
TEST_WORKER_JS_B64=$(printf "%s" "$TEST_WORKER_JS_CONTENT" | base64 | tr -d '\n')

# 2. WorkerBundlePayload for the JS worker
WORKER_BUNDLE_PAYLOAD_JSON=$(jq -n \
	--arg vm "nxcc/workerd" \
	--arg executable_b64 "$TEST_WORKER_JS_B64" \
	'{vm: $vm, executable: $executable_b64, metadata: {}}')

# 3. DSSE Envelope for the WorkerBundle
WORKER_BUNDLE_DSSE_JSON=$(jq -n \
	--arg payload_b64 "$(printf "%s" "$WORKER_BUNDLE_PAYLOAD_JSON" | base64 | tr -d '\n')" \
	--arg payload_type "application/vnd.nxcc.workerbundlepayload.v1+json" \
	'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}')
WORKER_BUNDLE_DSSE_B64=$(printf "%s" "$WORKER_BUNDLE_DSSE_JSON" | base64 | tr -d '\n')

# 4. WorkerManifest for the WorkOrder
WORKER_MANIFEST_JSON=$(jq -n \
	--arg bundle_source "data:application/json;base64,$WORKER_BUNDLE_DSSE_B64" \
	--argjson chain_id "$SECRET_CHAIN_ID" \
	--arg identity_address "$SECRET_IDENTITY_ADDR" \
	--arg identity_id_str "$SECRET_IDENTITY_ID_NUM" \
	--arg secret_name "$SECRET_NAME_IN_WORKER" \
	'{bundle: {source: $bundle_source, hash: null}, identities: [[{chain_id: $chain_id, identity_address: $identity_address, identity_id: $identity_id_str}, $secret_name]], userdata: {}}')

# 5. WorkOrderPayload
WORK_ORDER_PAYLOAD_JSON=$(jq -n \
	--arg id "test-work-order-$(date +%s%N)" \
	--argjson worker_manifest "$WORKER_MANIFEST_JSON" \
	'{id: $id, worker: $worker_manifest, events: [{kind: "launch"}]}')

# 6. DSSE Envelope for the WorkOrder
WORK_ORDER_DSSE_JSON=$(jq -n \
	--arg payload_b64 "$(printf "%s" "$WORK_ORDER_PAYLOAD_JSON" | base64 | tr -d '\n')" \
	--arg payload_type "$DSSE_WORK_ORDER_PAYLOAD_TYPE" \
	'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}')
WORK_ORDER_DSSE_B64=$(printf "%s" "$WORK_ORDER_DSSE_JSON" | base64 | tr -d '\n')

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

# 1. Alice receives a work order
echo "Step 1: Alice receives work order (triggers secret generation)..."
grpcurl_submit_work_order "$alice_DAEMON_SOCK" "$WORK_ORDER_DSSE_B64"

# Alice's daemon log for the VM output
ALICE_VM_LOG="$alice_DIR/vm.log"

# Wait for Alice's worker to execute and log output
echo "Waiting for Alice's worker to log derived bits..."
ALICE_DERIVED_BITS=""
for i in $( # Poll for up to 20 seconds
	seq 1 20
); do
	if [ -f "$ALICE_VM_LOG" ]; then
		ALICE_DERIVED_BITS=$(grep "DERIVED_BASE64:" "$ALICE_VM_LOG" |
			tail -n 1 |
			sed -E 's/.*DERIVED_BASE64: ([A-Za-z0-9+/=]*).*/\1/')
	fi
	if [ -n "$ALICE_DERIVED_BITS" ]; then
		echo "Alice derived bits: $ALICE_DERIVED_BITS"
		break
	fi
	sleep 1
done

if [ -z "$ALICE_DERIVED_BITS" ]; then
	echo "ERROR: Alice's worker did not log derived bits."
	cat "$ALICE_VM_LOG" # Print log for debugging
	exit 1
fi

# 2. Bob receives the same work order
echo "Step 2: Bob receives the same work order (triggers P2P secret request to Alice)..."
grpcurl_submit_work_order "$bob_DAEMON_SOCK" "$WORK_ORDER_DSSE_B64"

# Bob's daemon log for the VM output
BOB_VM_LOG="$bob_DIR/vm.log"

# Wait for Bob's worker to execute and log output
echo "Waiting for Bob's worker to log derived bits..."
BOB_DERIVED_BITS=""
for i in $( # Poll for up to 30 seconds (longer for P2P)
	seq 1 30
); do
	if [ -f "$BOB_VM_LOG" ]; then
		BOB_DERIVED_BITS=$(grep "DERIVED_BASE64:" "$BOB_VM_LOG" |
			tail -n 1 |
			sed -E 's/.*DERIVED_BASE64: ([A-Za-z0-9+/=]*).*/\1/')
	fi
	if [ -n "$BOB_DERIVED_BITS" ]; then
		echo "Bob derived bits: $BOB_DERIVED_BITS"
		break
	fi
	sleep 1
done

if [ -z "$BOB_DERIVED_BITS" ]; then
	echo "ERROR: Bob's worker did not log derived bits."
	cat "$BOB_VM_LOG" # Print log for debugging
	exit 1
fi

# 3. Compare derived bits
echo "Step 3: Comparing derived bits..."
if [ "$ALICE_DERIVED_BITS" = "$BOB_DERIVED_BITS" ]; then
	echo "SUCCESS: Derived bits match between Alice and Bob."
else
	echo "ERROR: Derived bits DO NOT match!"
	echo "Alice: $ALICE_DERIVED_BITS"
	echo "Bob:   $BOB_DERIVED_BITS"
	exit 1
fi

# Final sleep before declaring success
sleep 5

echo "--- Test Workflow Completed Successfully ---"

# Cleanup is handled by the trap
exit 0
