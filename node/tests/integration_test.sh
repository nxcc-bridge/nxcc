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
JS_WORKER_DIR="$SCRIPT_DIR/js_workers"
CONTRACTS_DIR="$SCRIPT_DIR/contracts"

ANVIL_PID=""

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

# Anvil configuration
ANVIL_RPC_URL="http://127.0.0.1:8545"
ANVIL_CHAIN_ID=31337                                                                  # Default Anvil chain ID
DEPLOYER_PK="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"      # Anvil default[0]
WORKER_SENDER_PK="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d" # Anvil default[1]

# Path to the JS worker script for the test
TEST_WORKER_JS_PATH="$SCRIPT_DIR/policy/test_worker_integration.js"
DSSE_WORK_ORDER_PAYLOAD_TYPE="application/vnd.nxcc.workorderpayload.v1+json"

NODE1_NAME="alice"
NODE2_NAME="bob"
NODE1_PORT=9001 # For libp2p TCP listening
NODE2_PORT=9002 # For libp2p TCP listening

# --- Helper Functions ---
start_anvil() {
	echo "Starting Anvil..."
	anvil --silent &
	ANVIL_PID=$!
	# Wait for anvil to be ready
	for i in $(seq 1 10); do
		if cast chain-id --rpc-url "$ANVIL_RPC_URL" >/dev/null 2>&1; then
			echo "Anvil started. Chain ID: $(cast chain-id --rpc-url "$ANVIL_RPC_URL")"
			return 0
		fi
		sleep 1
	done
	echo "ERROR: Anvil failed to start or respond in time."
	exit 1
}

stop_anvil() {
	if [ -n "$ANVIL_PID" ]; then
		echo "Stopping Anvil (PID: $ANVIL_PID)..."
		kill "$ANVIL_PID" 2>/dev/null || true
		ANVIL_PID=""
	fi
}

# --- Cleanup Function ---
cleanup() {
	set +x # Don't debug log cleanup commands
	echo "Cleaning up..."

	# Kill any other potential background processes
	# This is a bit aggressive, ensure it doesn't kill unrelated processes if tests run in parallel
	pkill -P $$ >/dev/null 2>&1 || true

	# Kill node processes
	cleanup_node "$NODE1_NAME"
	cleanup_node "$NODE2_NAME"

	stop_anvil

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

# --- Build JS Workers ---
echo "--- Building JS Workers ---"
if [ ! -d "$JS_WORKER_DIR/node_modules" ]; then
	echo "Installing JS dependencies..."
	(cd "$JS_WORKER_DIR" && npm install)
fi
echo "Building JS bundles..."
(cd "$JS_WORKER_DIR" && npm run build)

POLICY_WORKER_JS_BUNDLE_PATH="$JS_WORKER_DIR/dist/test_worker_integration.js"
EVENT_HANDLER_WORKER_JS_BUNDLE_PATH="$JS_WORKER_DIR/dist/event_handler_worker.js"
HTTP_ECHO_WORKER_JS_BUNDLE_PATH="$JS_WORKER_DIR/dist/http_echo_worker.js"

# --- Prepare Test Worker and Work Order ---
echo "--- Preparing Work Order for Original Secret Sharing Test ---"

# 1. JS Worker Code
TEST_WORKER_JS_CONTENT=$(cat "$POLICY_WORKER_JS_BUNDLE_PATH")
TEST_WORKER_JS_B64=$(printf "%s" "$TEST_WORKER_JS_CONTENT" | base64 | tr -d '\n')

# 2. WorkerBundlePayload for the JS worker (using file to avoid argument list too long)
WORKER_BUNDLE_PAYLOAD_FILE="$TEST_DIR/worker_bundle_payload.json"
jq -n \
	--arg vm "nxcc/workerd" \
	--arg executable_b64 "$TEST_WORKER_JS_B64" \
	'{vm: $vm, executable: $executable_b64, metadata: {}}' >"$WORKER_BUNDLE_PAYLOAD_FILE"

# 3. DSSE Envelope for the WorkerBundle (using files to avoid argument list too long)
WORKER_BUNDLE_PAYLOAD_B64_FILE="$TEST_DIR/worker_bundle_payload_b64.txt"
base64 <"$WORKER_BUNDLE_PAYLOAD_FILE" | tr -d '\n' >"$WORKER_BUNDLE_PAYLOAD_B64_FILE"

WORKER_BUNDLE_DSSE_FILE="$TEST_DIR/worker_bundle_dsse.json"
jq -n \
	--rawfile payload_b64 "$WORKER_BUNDLE_PAYLOAD_B64_FILE" \
	--arg payload_type "application/vnd.nxcc.workerbundlepayload.v1+json" \
	'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$WORKER_BUNDLE_DSSE_FILE"

WORKER_BUNDLE_DSSE_B64_FILE="$TEST_DIR/worker_bundle_dsse_b64.txt"
base64 <"$WORKER_BUNDLE_DSSE_FILE" | tr -d '\n' >"$WORKER_BUNDLE_DSSE_B64_FILE"

# 4. WorkerManifest for the WorkOrder (using files to avoid argument list too long)
WORKER_MANIFEST_FILE="$TEST_DIR/worker_manifest.json"
jq -n \
	--rawfile bundle_source_b64 "$WORKER_BUNDLE_DSSE_B64_FILE" \
	--argjson chain_id "$SECRET_CHAIN_ID" \
	--arg identity_address "$SECRET_IDENTITY_ADDR" \
	--arg identity_id_str "$SECRET_IDENTITY_ID_NUM" \
	--arg secret_name "$SECRET_NAME_IN_WORKER" \
	'{bundle: {source: ("data:application/json;base64," + $bundle_source_b64), hash: null}, identities: [[{chain_id: $chain_id, identity_address: $identity_address, identity_id: $identity_id_str}, $secret_name]], userdata: {}}' >"$WORKER_MANIFEST_FILE"

# 5. WorkOrderPayload (using files to avoid argument list too long)
WORK_ORDER_PAYLOAD_FILE="$TEST_DIR/work_order_payload.json"
jq -n \
	--arg id "test-work-order-$(date +%s%N)" \
	--slurpfile worker_manifest "$WORKER_MANIFEST_FILE" \
	'{id: $id, worker: $worker_manifest[0], events: [{"handler": "launch", "kind": "launch"}]}' >"$WORK_ORDER_PAYLOAD_FILE"

# 6. DSSE Envelope for the WorkOrder (using files to avoid argument list too long)
WORK_ORDER_PAYLOAD_B64_FILE="$TEST_DIR/work_order_payload_b64.txt"
base64 <"$WORK_ORDER_PAYLOAD_FILE" | tr -d '\n' >"$WORK_ORDER_PAYLOAD_B64_FILE"

ORIG_WORK_ORDER_DSSE_FILE="$TEST_DIR/orig_work_order_dsse.json"
jq -n \
	--rawfile payload_b64 "$WORK_ORDER_PAYLOAD_B64_FILE" \
	--arg payload_type "$DSSE_WORK_ORDER_PAYLOAD_TYPE" \
	'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$ORIG_WORK_ORDER_DSSE_FILE"

# Prepare the payload file for grpcurl
ORIG_WORK_ORDER_DSSE_B64_FILE="$TEST_DIR/orig_work_order_dsse_b64.txt"
base64 <"$ORIG_WORK_ORDER_DSSE_FILE" | tr -d '\n' >"$ORIG_WORK_ORDER_DSSE_B64_FILE"

GRPCURL_SUBMIT_ORIG_WO_PAYLOAD_FILE="$TEST_DIR/submit_orig_wo_payload.json"
jq -n \
	--rawfile work_order_dsse_bytes "$ORIG_WORK_ORDER_DSSE_B64_FILE" \
	'{work_order_dsse_bytes: $work_order_dsse_bytes}' >"$GRPCURL_SUBMIT_ORIG_WO_PAYLOAD_FILE"

# ==============================================================================
# Original Test Workflow (Steps 1-3: Secret Sharing)
# ==============================================================================
# --- Node 1 (Alice) Setup ---
echo "--- Setting up Node 1 (Alice) ---"
setup_node "$NODE1_NAME" "$TEST_DIR" "$NODE1_PORT" "" \
	"$DAEMON_BIN" "$ENCLAVE_BIN" "$WORKERD_VM_BIN"

# Sleep after node setup to ensure everything is ready
sleep 2

# Attach VM to Enclave via Daemon
grpcurl_attach_vm "$alice_DAEMON_SOCK" "policy-vm-0" "$alice_VM_SOCK"

# --- Node 2 (Bob) Setup ---
echo "--- Setting up Node 2 (Bob) ---"
setup_node "$NODE2_NAME" "$TEST_DIR" "$NODE2_PORT" "$alice_MULTIADDR" \
	"$DAEMON_BIN" "$ENCLAVE_BIN" "$WORKERD_VM_BIN"

# Sleep after node setup to ensure everything is ready
sleep 1

# Attach VM to Enclave via Daemon
grpcurl_attach_vm "$bob_DAEMON_SOCK" "policy-vm-0" "$bob_VM_SOCK"

# Sleep after VM attachment and before starting the test workflow
sleep 2

# --- Original Test Workflow ---
echo "--- Starting Original Secret Sharing Test Workflow ---"

# 1. Alice receives a work order
echo "Step 1: Alice receives work order (triggers secret generation)..."
grpcurl_submit_work_order "$alice_DAEMON_SOCK" "$GRPCURL_SUBMIT_ORIG_WO_PAYLOAD_FILE"

# Alice's daemon log for the VM output
ALICE_VM_LOG="$alice_DIR/vm.log"

# Wait for Alice's worker to execute and log output
echo "Waiting for Alice's worker to log derived bits..."
ALICE_DERIVED_BITS=""
for i in $( # Poll for up to 5 seconds
	seq 1 5
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
grpcurl_submit_work_order "$bob_DAEMON_SOCK" "$GRPCURL_SUBMIT_ORIG_WO_PAYLOAD_FILE"

# Bob's daemon log for the VM output
BOB_VM_LOG="$bob_DIR/vm.log"

# Wait for Bob's worker to execute and log output
echo "Waiting for Bob's worker to log derived bits..."
BOB_DERIVED_BITS=""
for i in $( # Poll for up to 20 seconds (longer for P2P)
	seq 1 20
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
	echo "SUCCESS (Original Test): Derived bits match between Alice and Bob."
else
	echo "ERROR (Original Test): Derived bits DO NOT match!"
	echo "Alice: $ALICE_DERIVED_BITS"
	echo "Bob:   $BOB_DERIVED_BITS"
	exit 1
fi

# ==============================================================================
# New Test Workflow (Step 4: Web3 Event Subscription)
# ==============================================================================
echo "--- Starting New Web3 Event Subscription Test Workflow ---"

# 4a. Start Anvil
start_anvil

# 4b. Deploy TestEvents.sol contract
echo "Compiling and deploying TestEvents contract..."
if ! command -v forge >/dev/null 2>&1; then
	echo "ERROR: forge (foundry) command not found. Please install foundry."
	exit 1
fi
(cd "$CONTRACTS_DIR" && forge build --force --via-ir) # Use via-ir for potentially smaller bytecode
TEST_EVENTS_BYTECODE=$(jq -r .bytecode.object <"$SCRIPT_DIR/out/TestEvents.sol/TestEvents.json")
TEST_EVENTS_ABI=$(jq .abi <"$SCRIPT_DIR/out/TestEvents.sol/TestEvents.json")
TEST_EVENTS_ABI_ESCAPED=$(echo "$TEST_EVENTS_ABI" | jq -c . | sed 's/"/\\"/g') # For embedding in JSON string

DEPLOY_OUTPUT=$(cast send --json --rpc-url "$ANVIL_RPC_URL" --private-key "$DEPLOYER_PK" \
	--create "$TEST_EVENTS_BYTECODE")
CONTRACT_ADDRESS=$(echo "$DEPLOY_OUTPUT" | jq -r .contractAddress)
if [ -z "$CONTRACT_ADDRESS" ] || [ "$CONTRACT_ADDRESS" = "null" ]; then
	echo "ERROR: Failed to deploy TestEvents contract."
	echo "$DEPLOY_OUTPUT"
	exit 1
fi
echo "TestEvents contract deployed at: $CONTRACT_ADDRESS"

# Prepare Work Order for the event handling worker (using files to avoid argument list too long)
EVENT_HANDLER_WORKER_JS_CONTENT=$(cat "$EVENT_HANDLER_WORKER_JS_BUNDLE_PATH")
EVENT_HANDLER_WORKER_JS_B64=$(printf "%s" "$EVENT_HANDLER_WORKER_JS_CONTENT" | base64 | tr -d '\n')

EVENT_WORKER_BUNDLE_PAYLOAD_FILE="$TEST_DIR/event_worker_bundle_payload.json"
jq -n \
	--arg vm "nxcc/workerd" \
	--arg executable_b64 "$EVENT_HANDLER_WORKER_JS_B64" \
	'{vm: $vm, executable: $executable_b64, metadata: {}}' >"$EVENT_WORKER_BUNDLE_PAYLOAD_FILE"

EVENT_WORKER_BUNDLE_PAYLOAD_B64_FILE="$TEST_DIR/event_worker_bundle_payload_b64.txt"
base64 <"$EVENT_WORKER_BUNDLE_PAYLOAD_FILE" | tr -d '\n' >"$EVENT_WORKER_BUNDLE_PAYLOAD_B64_FILE"

EVENT_WORKER_BUNDLE_DSSE_FILE="$TEST_DIR/event_worker_bundle_dsse.json"
jq -n \
	--rawfile payload_b64 "$EVENT_WORKER_BUNDLE_PAYLOAD_B64_FILE" \
	--arg payload_type "application/vnd.nxcc.workerbundlepayload.v1+json" \
	'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$EVENT_WORKER_BUNDLE_DSSE_FILE"

EVENT_WORKER_BUNDLE_DSSE_B64_FILE="$TEST_DIR/event_worker_bundle_dsse_b64.txt"
base64 <"$EVENT_WORKER_BUNDLE_DSSE_FILE" | tr -d '\n' >"$EVENT_WORKER_BUNDLE_DSSE_B64_FILE"

EVENT_WORKER_MANIFEST_FILE="$TEST_DIR/event_worker_manifest.json"
jq -n \
	--rawfile bundle_source_b64 "$EVENT_WORKER_BUNDLE_DSSE_B64_FILE" \
	--arg rpc_url "$ANVIL_RPC_URL" \
	--arg contract_addr "$CONTRACT_ADDRESS" \
	--arg pk "$WORKER_SENDER_PK" \
	"{
        bundle: {source: (\"data:application/json;base64,\" + \$bundle_source_b64), hash: null},
        identities: [],
        userdata: {
            rpcUrl: \$rpc_url,
            contractAddress: \$contract_addr,
            contractAbi: \"$TEST_EVENTS_ABI_ESCAPED\",
            ethereumPrivateKey: \$pk
        }
    }" >"$EVENT_WORKER_MANIFEST_FILE"

EVENT_VALUE_CHANGED_SIGNATURE=$(cast sig-event "ValueChanged(uint256,uint256,bytes)")
OTHER_EVENT_SIGNATURE=$(cast sig-event "OtherEvent(uint256)")

EVENT_WORK_ORDER_PAYLOAD_FILE="$TEST_DIR/event_work_order_payload.json"
jq -n \
	--arg id "event-work-order-$(date +%s%N)" \
	--slurpfile worker_manifest "$EVENT_WORKER_MANIFEST_FILE" \
	--argjson chain_id "$ANVIL_CHAIN_ID" \
	--arg contract_address "$CONTRACT_ADDRESS" \
	--arg event_sig "$EVENT_VALUE_CHANGED_SIGNATURE" \
	'{
 id: $id,
 worker: $worker_manifest[0],
 events: [
            {"handler": "launch", "kind": "launch"},
            {
                "handler": "valueChanged",
                "kind": "web3_event",
                "chain": $chain_id,
                "address": [$contract_address],
                "topics": [[$event_sig]]
            }
        ]
    }' >"$EVENT_WORK_ORDER_PAYLOAD_FILE"

EVENT_WORK_ORDER_PAYLOAD_B64_FILE="$TEST_DIR/event_work_order_payload_b64.txt"
base64 <"$EVENT_WORK_ORDER_PAYLOAD_FILE" | tr -d '\n' >"$EVENT_WORK_ORDER_PAYLOAD_B64_FILE"

EVENT_WORK_ORDER_DSSE_FILE="$TEST_DIR/event_work_order_dsse.json"
jq -n \
	--rawfile payload_b64 "$EVENT_WORK_ORDER_PAYLOAD_B64_FILE" \
	--arg payload_type "$DSSE_WORK_ORDER_PAYLOAD_TYPE" \
	'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$EVENT_WORK_ORDER_DSSE_FILE"

EVENT_WORK_ORDER_DSSE_B64_FILE="$TEST_DIR/event_work_order_dsse_b64.txt"
base64 <"$EVENT_WORK_ORDER_DSSE_FILE" | tr -d '\n' >"$EVENT_WORK_ORDER_DSSE_B64_FILE"

GRPCURL_SUBMIT_EVENT_WO_PAYLOAD_FILE="$TEST_DIR/submit_event_wo_payload.json"
jq -n \
	--rawfile work_order_dsse_bytes "$EVENT_WORK_ORDER_DSSE_B64_FILE" \
	'{work_order_dsse_bytes: $work_order_dsse_bytes}' >"$GRPCURL_SUBMIT_EVENT_WO_PAYLOAD_FILE"

echo "Submitting event-listening work order to Alice and Bob..."
grpcurl_submit_work_order "$alice_DAEMON_SOCK" "$GRPCURL_SUBMIT_EVENT_WO_PAYLOAD_FILE"
grpcurl_submit_work_order "$bob_DAEMON_SOCK" "$GRPCURL_SUBMIT_EVENT_WO_PAYLOAD_FILE"

# 4c. Trigger events
echo "Triggering OtherEvent (should be ignored by workers)..."
cast send --rpc-url "$ANVIL_RPC_URL" --private-key "$DEPLOYER_PK" "$CONTRACT_ADDRESS" \
	"triggerOtherEvent(uint256)" 789 >/dev/null
sleep 2 # Give time for potential (unwanted) processing

INITIAL_VALUE_ON_CHAIN=$(cast call --rpc-url "$ANVIL_RPC_URL" "$CONTRACT_ADDRESS" "value()(uint256)")
if [ "$INITIAL_VALUE_ON_CHAIN" -ne 0 ]; then # Assuming initial value is 0
	echo "ERROR: Contract value changed after OtherEvent, expected no change. Value: $INITIAL_VALUE_ON_CHAIN"
	exit 1
fi
echo "Contract state unchanged after OtherEvent, as expected."

echo "Triggering ValueChanged event..."
NEW_VALUE_FOR_EVENT=42
EVENT_PAYLOAD_DATA_HEX="0xdeadbeefcafe"
cast send --rpc-url "$ANVIL_RPC_URL" --private-key "$DEPLOYER_PK" "$CONTRACT_ADDRESS" \
	"triggerEvent(uint256,bytes)" "$NEW_VALUE_FOR_EVENT" "$EVENT_PAYLOAD_DATA_HEX" >/dev/null

# 4d & 4e. Verify contract update
echo "Polling contract state for update from ValueChanged event..."
CONTRACT_UPDATED_SUCCESSFULLY=false
for i in $( # Poll for updates
	seq 1 10
); do
	CURRENT_VALUE=$(cast call --rpc-url "$ANVIL_RPC_URL" "$CONTRACT_ADDRESS" "value()(uint256)")
	CURRENT_DATA=$(cast call --rpc-url "$ANVIL_RPC_URL" "$CONTRACT_ADDRESS" "eventDataPayload()(bytes)")

	if [ "$CURRENT_VALUE" -eq "$NEW_VALUE_FOR_EVENT" ] && [ "$CURRENT_DATA" = "$EVENT_PAYLOAD_DATA_HEX" ]; then
		echo "SUCCESS (Event Test): Contract state updated as expected."
		CONTRACT_UPDATED_SUCCESSFULLY=true
		break
	fi
	printf "."
	sleep 1
done

if [ "$CONTRACT_UPDATED_SUCCESSFULLY" != "true" ]; then
	echo "ERROR (Event Test): Contract state not updated as expected after timeout."
	echo "Expected value: $NEW_VALUE_FOR_EVENT, Got: $CURRENT_VALUE"
	echo "Expected data: $EVENT_PAYLOAD_DATA_HEX, Got: $CURRENT_DATA"
	echo "Alice daemon log:"
	cat "$alice_DAEMON_LOG" || true
	echo "Alice VM log:"
	cat "$alice_VM_LOG" || true
	echo "Bob daemon log:"
	cat "$bob_DAEMON_LOG" || true
	echo "Bob VM log:"
	cat "$bob_VM_LOG" || true
	exit 1
fi

# ==============================================================================
# New Test Workflow (Step 5: HTTP Worker Request/Response)
# ==============================================================================
echo "--- Starting New HTTP Worker Test Workflow ---"

# 5a. Prepare Work Order for the HTTP echo worker
echo "Preparing HTTP echo work order..."
HTTP_ECHO_WORKER_JS_CONTENT=$(cat "$HTTP_ECHO_WORKER_JS_BUNDLE_PATH")
HTTP_ECHO_WORKER_JS_B64=$(printf "%s" "$HTTP_ECHO_WORKER_JS_CONTENT" | base64 | tr -d '\n')

HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_FILE="$TEST_DIR/http_echo_worker_bundle_payload.json"
jq -n \
	--arg vm "nxcc/workerd" \
	--arg executable_b64 "$HTTP_ECHO_WORKER_JS_B64" \
	'{vm: $vm, executable: $executable_b64, metadata: {}}' >"$HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_FILE"

HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_B64_FILE="$TEST_DIR/http_echo_worker_bundle_payload_b64.txt"
base64 <"$HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_FILE" | tr -d '\n' >"$HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_B64_FILE"

HTTP_ECHO_WORKER_BUNDLE_DSSE_FILE="$TEST_DIR/http_echo_worker_bundle_dsse.json"
jq -n \
	--rawfile payload_b64 "$HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_B64_FILE" \
	--arg payload_type "application/vnd.nxcc.workerbundlepayload.v1+json" \
	'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$HTTP_ECHO_WORKER_BUNDLE_DSSE_FILE"

HTTP_ECHO_WORKER_BUNDLE_DSSE_B64_FILE="$TEST_DIR/http_echo_worker_bundle_dsse_b64.txt"
base64 <"$HTTP_ECHO_WORKER_BUNDLE_DSSE_FILE" | tr -d '\n' >"$HTTP_ECHO_WORKER_BUNDLE_DSSE_B64_FILE"

HTTP_ECHO_WORKER_MANIFEST_FILE="$TEST_DIR/http_echo_worker_manifest.json"
jq -n \
	--rawfile bundle_source_b64 "$HTTP_ECHO_WORKER_BUNDLE_DSSE_B64_FILE" \
	"{
        bundle: {source: (\"data:application/json;base64,\" + \$bundle_source_b64), hash: null},
        identities: [],
        userdata: {}
    }" >"$HTTP_ECHO_WORKER_MANIFEST_FILE"

HTTP_ECHO_WORK_ORDER_PAYLOAD_FILE="$TEST_DIR/http_echo_work_order_payload.json"
jq -n \
	--arg id "http-echo-work-order-$(date +%s%N)" \
	--slurpfile worker_manifest "$HTTP_ECHO_WORKER_MANIFEST_FILE" \
	'{
        id: $id,
        worker: $worker_manifest[0],
        events: [
            {"handler": "fetch", "kind": "http_request_trigger"}
        ]
    }' >"$HTTP_ECHO_WORK_ORDER_PAYLOAD_FILE"

HTTP_ECHO_WORK_ORDER_PAYLOAD_B64_FILE="$TEST_DIR/http_echo_work_order_payload_b64.txt"
base64 <"$HTTP_ECHO_WORK_ORDER_PAYLOAD_FILE" | tr -d '\n' >"$HTTP_ECHO_WORK_ORDER_PAYLOAD_B64_FILE"

HTTP_ECHO_WORK_ORDER_DSSE_FILE="$TEST_DIR/http_echo_work_order_dsse.json"
jq -n \
	--rawfile payload_b64 "$HTTP_ECHO_WORK_ORDER_PAYLOAD_B64_FILE" \
	--arg payload_type "$DSSE_WORK_ORDER_PAYLOAD_TYPE" \
	'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$HTTP_ECHO_WORK_ORDER_DSSE_FILE"

HTTP_ECHO_WORK_ORDER_DSSE_B64_FILE="$TEST_DIR/http_echo_work_order_dsse_b64.txt"
base64 <"$HTTP_ECHO_WORK_ORDER_DSSE_FILE" | tr -d '\n' >"$HTTP_ECHO_WORK_ORDER_DSSE_B64_FILE"

GRPCURL_SUBMIT_HTTP_ECHO_WO_PAYLOAD_FILE="$TEST_DIR/submit_http_echo_wo_payload.json"
jq -n \
	--rawfile work_order_dsse_bytes "$HTTP_ECHO_WORK_ORDER_DSSE_B64_FILE" \
	'{work_order_dsse_bytes: $work_order_dsse_bytes}' >"$GRPCURL_SUBMIT_HTTP_ECHO_WO_PAYLOAD_FILE"

# 5b. Submit HTTP echo work order to Alice
echo "Submitting HTTP echo work order to Alice..."
HTTP_ECHO_WO_SUBMIT_RESPONSE=$(grpcurl_submit_work_order "$alice_DAEMON_SOCK" "$GRPCURL_SUBMIT_HTTP_ECHO_WO_PAYLOAD_FILE")
echo "HTTP Echo Work Order Submit Response: $HTTP_ECHO_WO_SUBMIT_RESPONSE"

HTTP_ECHO_WO_SUBMIT_SUCCESS=$(echo "$HTTP_ECHO_WO_SUBMIT_RESPONSE" | jq -r .success)
if [ "$HTTP_ECHO_WO_SUBMIT_SUCCESS" != "true" ]; then
	echo "ERROR: Submitting HTTP echo work order was not successful."
	exit 1
fi

HTTP_ECHO_WORK_ORDER_ID=$(echo "$HTTP_ECHO_WO_SUBMIT_RESPONSE" | jq -r .workOrderId)
if [ -z "$HTTP_ECHO_WORK_ORDER_ID" ] || [ "$HTTP_ECHO_WORK_ORDER_ID" = "null" ]; then
	echo "ERROR: Failed to get workOrderId from HTTP echo work order submission."
	exit 1
fi
echo "HTTP Echo Work Order ID (mount segment): $HTTP_ECHO_WORK_ORDER_ID"

# Give some time for the worker to be mounted and ready
sleep 2

# 5c. Send HTTP request to the worker via Alice's daemon
echo "Sending HTTP POST request to the echo worker..."
HTTP_REQUEST_BODY="Hello From Test Script"
HTTP_RESPONSE_FILE="$TEST_DIR/http_echo_worker_response.json"
HTTP_STATUS_CODE=$(curl -s -w "%{http_code}" -X POST \
	-H "Content-Type: text/plain" \
	-H "X-Custom-Test-Header: custom-value" \
	-d "$HTTP_REQUEST_BODY" \
	"http://127.0.0.1:6922/w/${HTTP_ECHO_WORK_ORDER_ID}/test/path?queryArg=testVal" \
	-o "$HTTP_RESPONSE_FILE")

echo "HTTP Echo Worker Response Status Code: $HTTP_STATUS_CODE"
echo "HTTP Echo Worker Response Body:"
cat "$HTTP_RESPONSE_FILE"

if [ "$HTTP_STATUS_CODE" -ne 200 ]; then
	echo "ERROR (HTTP Worker Test): Worker returned status $HTTP_STATUS_CODE, expected 200."
	echo "Alice daemon log:"
	cat "$alice_DAEMON_LOG" || true
	echo "Alice VM log:"
	cat "$alice_VM_LOG" || true
	exit 1
fi

# 5d. Verify the response
jq -e '.message == "HTTP Echo Worker Response"' "$HTTP_RESPONSE_FILE" >/dev/null || {
	echo "ERROR (HTTP Worker Test): Incorrect message"
	exit 1
}
jq -e '.method == "POST"' "$HTTP_RESPONSE_FILE" >/dev/null || {
	echo "ERROR (HTTP Worker Test): Incorrect method"
	exit 1
}
jq -e '.pathname == "/test/path"' "$HTTP_RESPONSE_FILE" >/dev/null || {
	echo "ERROR (HTTP Worker Test): Incorrect pathname"
	exit 1
}
jq -e '.searchParams.queryArg == "testVal"' "$HTTP_RESPONSE_FILE" >/dev/null || {
	echo "ERROR (HTTP Worker Test): Incorrect queryArg"
	exit 1
}
jq -e '.headers["content-type"] == "text/plain"' "$HTTP_RESPONSE_FILE" >/dev/null || {
	echo "ERROR (HTTP Worker Test): Incorrect content-type header"
	exit 1
}
jq -e '.headers["x-custom-test-header"] == "custom-value"' "$HTTP_RESPONSE_FILE" >/dev/null || {
	echo "ERROR (HTTP Worker Test): Incorrect x-custom-test-header"
	exit 1
}
jq -e ".body == \"$HTTP_REQUEST_BODY\"" "$HTTP_RESPONSE_FILE" >/dev/null || {
	echo "ERROR (HTTP Worker Test): Incorrect body echo"
	exit 1
}

echo "SUCCESS (HTTP Worker Test): HTTP echo worker responded correctly."

echo "--- Test Workflow Completed Successfully ---"

# Cleanup is handled by the trap
exit 0
