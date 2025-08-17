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

ANVIL_PID_1=""
ANVIL_PID_2=""

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
ANVIL_RPC_URL_1="http://127.0.0.1:8545"
ANVIL_CHAIN_ID_1=31337
ANVIL_RPC_URL_2="http://127.0.0.1:8546"
ANVIL_CHAIN_ID_2=1338
ANVIL_WS_URL_1="ws://127.0.0.1:8545"
ANVIL_WS_URL_2="ws://127.0.0.1:8546"
DEPLOYER_PK="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"      # Anvil default[0]
WORKER_SENDER_PK="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d" # Anvil default[1]

# Path to the JS worker script for the test
# shellcheck disable=SC2034  # TEST_WORKER_JS_PATH may be used by future tests
TEST_WORKER_JS_PATH="$SCRIPT_DIR/policy/test_worker_integration.js"
DSSE_WORK_ORDER_PAYLOAD_TYPE="application/vnd.nxcc.workorderpayload.v1+json"

NODE1_NAME="alice"
NODE2_NAME="bob"
NODE1_P2P_PORT=9001
NODE2_P2P_PORT=9002
NODE1_HTTP_PORT=6922
NODE2_HTTP_PORT=6923

# --- Helper Functions ---
start_anvils() {
	echo "Starting Anvil instance 1 (chain $ANVIL_CHAIN_ID_1 on port 8545)..."
	anvil --port 8545 --chain-id "$ANVIL_CHAIN_ID_1" --silent &
	ANVIL_PID_1=$!
	# Wait for anvil to be ready
	for i in $(seq 1 10); do
		if cast chain-id --rpc-url "$ANVIL_RPC_URL_1" >/dev/null 2>&1; then
			echo "Anvil 1 started. Chain ID: $(cast chain-id --rpc-url "$ANVIL_RPC_URL_1")"
			break
		fi
		sleep 1
	done
	if ! cast chain-id --rpc-url "$ANVIL_RPC_URL_1" >/dev/null 2>&1; then
		echo "ERROR: Anvil 1 failed to start or respond in time."
		exit 1
	fi

	echo "Starting Anvil instance 2 (chain $ANVIL_CHAIN_ID_2 on port 8546)..."
	anvil --port 8546 --chain-id "$ANVIL_CHAIN_ID_2" --silent &
	ANVIL_PID_2=$!
	# Wait for anvil to be ready
	for i in $(seq 1 10); do
		if cast chain-id --rpc-url "$ANVIL_RPC_URL_2" >/dev/null 2>&1; then
			echo "Anvil 2 started. Chain ID: $(cast chain-id --rpc-url "$ANVIL_RPC_URL_2")"
			break
		fi
		sleep 1
	done
	if ! cast chain-id --rpc-url "$ANVIL_RPC_URL_2" >/dev/null 2>&1; then
		echo "ERROR: Anvil 2 failed to start or respond in time."
		exit 1
	fi
}

stop_anvils() {
	if [ -n "$ANVIL_PID_1" ]; then
		echo "Stopping Anvil 1 (PID: $ANVIL_PID_1)..."
		kill "$ANVIL_PID_1" 2>/dev/null || true
		ANVIL_PID_1=""
	fi
	if [ -n "$ANVIL_PID_2" ]; then
		echo "Stopping Anvil 2 (PID: $ANVIL_PID_2)..."
		kill "$ANVIL_PID_2" 2>/dev/null || true
		ANVIL_PID_2=""
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

	stop_anvils

	# Remove the test directory
	if [ -d "$TEST_DIR" ]; then
		echo "Removing test directory: $TEST_DIR"
		rm -rf "$TEST_DIR"
	fi

	killall nxcc-daemon 2>&1 || true

	echo "Cleanup finished."

	# Force exit to ensure no lingering processes
	# shellcheck disable=SC3048  # SIGINT/SIGTERM prefixes are widely supported
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
echo "--- Setting up Node 1 (Alice) on P2P port $NODE1_P2P_PORT and HTTP port $NODE1_HTTP_PORT ---"
# setup_node sets dynamic variables like alice_DAEMON_SOCK, alice_VM_ID, alice_VM_SOCK, etc.
# shellcheck disable=SC2034  # Variables are set dynamically by setup_node
setup_node "$NODE1_NAME" "$TEST_DIR" "$NODE1_P2P_PORT" "" \
	"$DAEMON_BIN" "$ENCLAVE_BIN" "$WORKERD_VM_BIN" "127.0.0.1:$NODE1_HTTP_PORT"

# Sleep after node setup to ensure everything is ready
sleep 2

# Attach VM to Enclave via Daemon
# shellcheck disable=SC2154  # alice_* variables are set by setup_node
grpcurl_attach_vm "$alice_DAEMON_SOCK" "$alice_VM_ID" "$alice_VM_SOCK"

# --- Node 2 (Bob) Setup ---
echo "--- Setting up Node 2 (Bob) on P2P port $NODE2_P2P_PORT and HTTP port $NODE2_HTTP_PORT ---"
# setup_node sets dynamic variables like bob_DAEMON_SOCK, bob_VM_ID, bob_VM_SOCK, etc.
# alice_MULTIADDR was set by the previous setup_node call
# shellcheck disable=SC2154  # alice_MULTIADDR is set by setup_node function
setup_node "$NODE2_NAME" "$TEST_DIR" "$NODE2_P2P_PORT" "$alice_MULTIADDR" \
	"$DAEMON_BIN" "$ENCLAVE_BIN" "$WORKERD_VM_BIN" "127.0.0.1:$NODE2_HTTP_PORT"

# Sleep after node setup to ensure everything is ready
sleep 1

# Attach VM to Enclave via Daemon
# shellcheck disable=SC2154  # bob_* variables are set by setup_node
grpcurl_attach_vm "$bob_DAEMON_SOCK" "$bob_VM_ID" "$bob_VM_SOCK"

# Sleep after VM attachment and before starting the test workflow
sleep 2

# --- Original Test Workflow ---
echo "--- Starting Original Secret Sharing Test Workflow ---"

# 1. Alice receives a work order
echo "Step 1: Alice receives work order (triggers secret generation)..."
grpcurl_submit_work_order "$alice_DAEMON_SOCK" "$GRPCURL_SUBMIT_ORIG_WO_PAYLOAD_FILE"

# Wait for Alice's worker to execute and log output
echo "Waiting for Alice's worker to log derived bits..."
ALICE_DERIVED_BITS=""
# shellcheck disable=SC2034,SC2154  # i is unused, alice_* vars set by setup_node
for i in $( # Poll for up to 5 seconds
	seq 1 5
); do
	if [ -f "$alice_VM_LOG" ]; then
		ALICE_DERIVED_BITS=$(grep "DERIVED_BASE64:" "$alice_VM_LOG" |
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
	cat "$alice_VM_LOG" # Print log for debugging
	exit 1
fi

# 2. Bob receives the same work order
echo "Step 2: Bob receives the same work order (triggers P2P secret request to Alice)..."
# shellcheck disable=SC2154  # bob_DAEMON_SOCK is set by setup_node
grpcurl_submit_work_order "$bob_DAEMON_SOCK" "$GRPCURL_SUBMIT_ORIG_WO_PAYLOAD_FILE"

# Wait for Bob's worker to execute and log output
echo "Waiting for Bob's worker to log derived bits..."
BOB_DERIVED_BITS=""
# shellcheck disable=SC2034,SC2154  # i is unused, bob_* vars set by setup_node
for i in $( # Poll for up to 20 seconds (longer for P2P)
	seq 1 20
); do
	if [ -f "$bob_VM_LOG" ]; then
		BOB_DERIVED_BITS=$(grep "DERIVED_BASE64:" "$bob_VM_LOG" |
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
	cat "$bob_VM_LOG" # Print log for debugging
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
echo "--- Starting New Cross-Chain Data Movement via Events Test ---"

# 4a. Start Anvils
start_anvils

# 4b. Deploy TestEvents.sol contract to both chains
echo "Compiling and deploying TestEvents contract..."
if ! command -v forge >/dev/null 2>&1; then
	echo "ERROR: forge (foundry) command not found. Please install foundry."
	exit 1
fi
(cd "$CONTRACTS_DIR" && forge build --force --via-ir) # Use via-ir for potentially smaller bytecode
TEST_EVENTS_BYTECODE=$(jq -r .bytecode.object <"$SCRIPT_DIR/out/TestEvents.sol/TestEvents.json")
# shellcheck disable=SC2034  # TEST_EVENTS_ABI is extracted but may be used later in tests
TEST_EVENTS_ABI=$(jq .abi <"$SCRIPT_DIR/out/TestEvents.sol/TestEvents.json")
TEST_EVENTS_ABI_STRING=$(jq -c .abi <"$SCRIPT_DIR/out/TestEvents.sol/TestEvents.json")
echo "Deploying contract to chain 1 ($ANVIL_CHAIN_ID_1)..."
DEPLOY_OUTPUT_1=$(cast send --json --rpc-url "$ANVIL_RPC_URL_1" --private-key "$DEPLOYER_PK" \
	--create "$TEST_EVENTS_BYTECODE")
CONTRACT_ADDRESS_1=$(echo "$DEPLOY_OUTPUT_1" | jq -r .contractAddress)
if [ -z "$CONTRACT_ADDRESS_1" ] || [ "$CONTRACT_ADDRESS_1" = "null" ]; then
	echo "ERROR: Failed to deploy TestEvents contract on chain 1."
	echo "$DEPLOY_OUTPUT_1"
	exit 1
fi
echo "TestEvents contract deployed on chain 1 at: $CONTRACT_ADDRESS_1"

echo "Deploying contract to chain 2 ($ANVIL_CHAIN_ID_2)..."
DEPLOY_OUTPUT_2=$(cast send --json --rpc-url "$ANVIL_RPC_URL_2" --private-key "$DEPLOYER_PK" \
	--create "$TEST_EVENTS_BYTECODE")
CONTRACT_ADDRESS_2=$(echo "$DEPLOY_OUTPUT_2" | jq -r .contractAddress)
if [ -z "$CONTRACT_ADDRESS_2" ] || [ "$CONTRACT_ADDRESS_2" = "null" ]; then
	echo "ERROR: Failed to deploy TestEvents contract on chain 2."
	echo "$DEPLOY_OUTPUT_2"
	exit 1
fi
echo "TestEvents contract deployed on chain 2 at: $CONTRACT_ADDRESS_2"

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
	--arg rpc_url_1 "$ANVIL_RPC_URL_1" \
	--arg contract_addr_1 "$CONTRACT_ADDRESS_1" \
	--arg rpc_url_2 "$ANVIL_RPC_URL_2" \
	--arg contract_addr_2 "$CONTRACT_ADDRESS_2" \
	--arg pk "$WORKER_SENDER_PK" \
	--arg abi_string "$TEST_EVENTS_ABI_STRING" \
	"{
        bundle: {source: (\"data:application/json;base64,\" + \$bundle_source_b64), hash: null},
        identities: [],
        userdata: {
						chain1: { rpcUrl: \$rpc_url_1, contractAddress: \$contract_addr_1 },
						chain2: { rpcUrl: \$rpc_url_2, contractAddress: \$contract_addr_2 },
						contractAbi: \$abi_string,
						ethereumPrivateKey: \$pk
        }
    }" >"$EVENT_WORKER_MANIFEST_FILE"

EVENT_VALUE_CHANGED_SIGNATURE=$(cast sig-event "ValueChanged(uint256,uint256,bytes)")
OTHER_EVENT_SIGNATURE=$(cast sig-event "OtherEvent(uint256)")

EVENT_WORK_ORDER_PAYLOAD_FILE="$TEST_DIR/event_work_order_payload.json"
jq -n \
	--arg id "cross-chain-work-order-$(date +%s%N)" \
	--slurpfile worker_manifest "$EVENT_WORKER_MANIFEST_FILE" \
	--argjson chain_id_1 "$ANVIL_CHAIN_ID_1" \
	--arg contract_address_1 "$CONTRACT_ADDRESS_1" \
	--arg value_changed_sig "$EVENT_VALUE_CHANGED_SIGNATURE" \
	--argjson chain_id_2 "$ANVIL_CHAIN_ID_2" \
	--arg contract_address_2 "$CONTRACT_ADDRESS_2" \
	--arg other_event_sig "$OTHER_EVENT_SIGNATURE" \
 	--arg anvil_ws_url_1 "$ANVIL_WS_URL_1" \
 	--arg anvil_ws_url_2 "$ANVIL_WS_URL_2" \
	'{
 id: $id,
 worker: $worker_manifest[0],
 events: [
            {"handler": "launch", "kind": "launch"},
            {
                "handler": "valueChanged",
                "kind": "web3_event",
                "chain": $chain_id_1,
                "address": [$contract_address_1],
                "topics": [[$value_changed_sig]],
                "gateways": [$anvil_ws_url_1]
            },
            {
                "handler": "otherEvent",
                "kind": "web3_event",
                "chain": $chain_id_2,
                "address": [$contract_address_2],
                "topics": [[$other_event_sig]],
                "gateways": [$anvil_ws_url_2]
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
# shellcheck disable=SC2154  # alice_DAEMON_SOCK and bob_DAEMON_SOCK are set by setup_node
grpcurl_submit_work_order "$alice_DAEMON_SOCK" "$GRPCURL_SUBMIT_EVENT_WO_PAYLOAD_FILE"
grpcurl_submit_work_order "$bob_DAEMON_SOCK" "$GRPCURL_SUBMIT_EVENT_WO_PAYLOAD_FILE"

# 4c. Trigger ValueChanged on chain 1, check update on chain 2
echo "Triggering ValueChanged on chain 1..."
NEW_VALUE_1=42
DATA_1="0xdeadbeef"
cast send --rpc-url "$ANVIL_RPC_URL_1" --private-key "$DEPLOYER_PK" "$CONTRACT_ADDRESS_1" \
	"triggerEvent(uint256,bytes)" "$NEW_VALUE_1" "$DATA_1" >/dev/null

echo "Polling contract on chain 2 for update..."
CONTRACT_UPDATED_SUCCESSFULLY=false
for i in $( # Poll for updates
	seq 1 5
); do
	CURRENT_VALUE=$(cast call --rpc-url "$ANVIL_RPC_URL_2" "$CONTRACT_ADDRESS_2" "value()(uint256)")
	CURRENT_DATA=$(cast call --rpc-url "$ANVIL_RPC_URL_2" "$CONTRACT_ADDRESS_2" "eventDataPayload()(bytes)")

	if [ "$CURRENT_VALUE" -eq "$NEW_VALUE_1" ] && [ "$CURRENT_DATA" = "$DATA_1" ]; then
		echo "SUCCESS (Cross-Chain Test Part 1): Contract on chain 2 updated as expected."
		CONTRACT_UPDATED_SUCCESSFULLY=true
		break
	fi
	printf "."
	sleep 1
done

if [ "$CONTRACT_UPDATED_SUCCESSFULLY" != "true" ]; then
	echo "ERROR (Cross-Chain Test Part 1): Contract on chain 2 not updated as expected after timeout."
	echo "Expected value: $NEW_VALUE_1, Got: $CURRENT_VALUE"
	echo "Expected data: $DATA_1, Got: $CURRENT_DATA"
	echo "Alice daemon log:"
	# shellcheck disable=SC2154  # alice_* and bob_* vars are set by setup_node
	cat "$alice_DAEMON_LOG" || true
	echo "Alice VM log:"
	cat "$alice_VM_LOG" || true
	echo "Bob daemon log:"
	# shellcheck disable=SC2154  # bob_DAEMON_LOG is set by setup_node function
	cat "$bob_DAEMON_LOG" || true
	echo "Bob VM log:"
	cat "$bob_VM_LOG" || true
	exit 1
fi

# 4d. Trigger OtherEvent on chain 2, check update on chain 1
echo "Triggering OtherEvent on chain 2..."
NEW_VALUE_2=99
DATA_2="0x" # The worker should submit empty bytes for OtherEvent
cast send --rpc-url "$ANVIL_RPC_URL_2" --private-key "$DEPLOYER_PK" "$CONTRACT_ADDRESS_2" \
	"triggerOtherEvent(uint256)" "$NEW_VALUE_2" >/dev/null

echo "Polling contract on chain 1 for update..."
CONTRACT_UPDATED_SUCCESSFULLY=false
# shellcheck disable=SC2034  # i is intentionally unused in polling loop
for i in $( # Poll for updates
	seq 1 10
); do
	CURRENT_VALUE=$(cast call --rpc-url "$ANVIL_RPC_URL_1" "$CONTRACT_ADDRESS_1" "value()(uint256)")
	CURRENT_DATA=$(cast call --rpc-url "$ANVIL_RPC_URL_1" "$CONTRACT_ADDRESS_1" "eventDataPayload()(bytes)")

	if [ "$CURRENT_VALUE" -eq "$NEW_VALUE_2" ] && [ "$CURRENT_DATA" = "$DATA_2" ]; then
		echo "SUCCESS (Cross-Chain Test Part 2): Contract on chain 1 updated as expected."
		CONTRACT_UPDATED_SUCCESSFULLY=true
		break
	fi
	printf "."
	sleep 1
done

if [ "$CONTRACT_UPDATED_SUCCESSFULLY" != "true" ]; then
	echo "ERROR (Cross-Chain Test Part 2): Contract on chain 1 not updated as expected after timeout."
	echo "Expected value: $NEW_VALUE_2, Got: $CURRENT_VALUE"
	echo "Expected data: $DATA_2, Got: $CURRENT_DATA"
	echo "Alice daemon log:"
	# shellcheck disable=SC2154  # alice_* and bob_* vars are set by setup_node
	cat "$alice_DAEMON_LOG" || true
	echo "Alice VM log:"
	cat "$alice_VM_LOG" || true
	echo "Bob daemon log:"
	# shellcheck disable=SC2154  # bob_DAEMON_LOG is set by setup_node function
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
            {"handler": "fetch", "kind": "http_request"}
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
	"http://127.0.0.1:${NODE1_HTTP_PORT}/w/${HTTP_ECHO_WORK_ORDER_ID}/test/path?queryArg=testVal" \
	-o "$HTTP_RESPONSE_FILE")

echo "HTTP Echo Worker Response Status Code: $HTTP_STATUS_CODE"
echo "HTTP Echo Worker Response Body:"
cat "$HTTP_RESPONSE_FILE"

if [ "$HTTP_STATUS_CODE" -ne 200 ]; then
	echo "ERROR (HTTP Worker Test): Worker returned status $HTTP_STATUS_CODE, expected 200."
	echo "Alice daemon log:"
	# shellcheck disable=SC2154  # alice_* and bob_* vars are set by setup_node
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

# ==============================================================================
# Log Streaming Test Workflow (Step 6: Worker Log Streaming)
# ==============================================================================
echo "--- Starting Worker Log Streaming Test Workflow ---"

# 6a. Test worker log streaming via HTTP API
echo "Testing worker log streaming via HTTP API..."

# Use the existing HTTP echo worker for log streaming tests
HTTP_LOGS_URL="http://127.0.0.1:${NODE1_HTTP_PORT}/api/workers/${HTTP_ECHO_WORK_ORDER_ID}/logs"

# Test 1: Get logs without streaming (should fail as not implemented)
echo "Testing non-streaming logs (expected to fail)..."
HTTP_LOGS_RESPONSE=$(curl -s -w "%{http_code}" -o /dev/null "$HTTP_LOGS_URL?follow=false" || echo "000")
if [ "$HTTP_LOGS_RESPONSE" = "500" ]; then
	echo "SUCCESS (Log Streaming Test 1): Non-streaming correctly returns 500 (not implemented)."
else
	echo "ERROR (Log Streaming Test 1): Expected 500 for non-streaming, got $HTTP_LOGS_RESPONSE"
	exit 1
fi

# Test 2: Test streaming logs with follow=true (should succeed)
echo "Testing streaming logs with follow=true..."
LOGS_STREAM_PID=""
LOGS_OUTPUT_FILE="$TEST_DIR/worker_logs_stream.txt"

# Start streaming in background and capture a few lines
timeout 5s curl -s -H "Accept: text/event-stream" "$HTTP_LOGS_URL?follow=true&tail=5" > "$LOGS_OUTPUT_FILE" &
LOGS_STREAM_PID=$!

# Wait a moment for the stream to start
sleep 2

# Make additional HTTP requests to the worker to generate more logs
echo "Generating additional logs by invoking worker..."
for i in 1 2 3; do
	curl -s -X POST "http://127.0.0.1:${NODE1_HTTP_PORT}/w/${HTTP_ECHO_WORK_ORDER_ID}/echo-test" \
		-H "Content-Type: application/json" \
		-d "{\"test\": \"log-stream-test-$i\"}" > /dev/null
	sleep 0.5
done

# Wait for stream to complete or timeout
wait $LOGS_STREAM_PID 2>/dev/null || true

# Check if we received SSE-formatted logs
if [ -f "$LOGS_OUTPUT_FILE" ] && [ -s "$LOGS_OUTPUT_FILE" ]; then
	echo "SUCCESS (Log Streaming Test 2): Received log stream data."
	
	# Verify SSE format (should contain "data: " lines)
	if grep -q "data: " "$LOGS_OUTPUT_FILE"; then
		echo "SUCCESS (Log Streaming Test 3): Log stream contains properly formatted SSE data."
		
		# Count the number of log entries (each "data: " line is a log entry)
		LOG_ENTRY_COUNT=$(grep -c "data: " "$LOGS_OUTPUT_FILE")
		echo "SUCCESS (Log Streaming Test 4): Received $LOG_ENTRY_COUNT log entries from stream."
		
		if [ "$LOG_ENTRY_COUNT" -ge 1 ]; then
			echo "SUCCESS (Log Streaming Test 5): Adequate number of log entries received."
		else
			echo "ERROR (Log Streaming Test 5): Expected at least 1 log entry, got $LOG_ENTRY_COUNT"
			exit 1
		fi
	else
		echo "ERROR (Log Streaming Test 3): Log stream does not contain SSE data format."
		echo "Log stream contents:"
		cat "$LOGS_OUTPUT_FILE" || true
		exit 1
	fi
else
	echo "ERROR (Log Streaming Test 2): No log stream data received."
	exit 1
fi

# Test 3: Test streaming with tail parameter
echo "Testing streaming logs with tail parameter..."
LOGS_TAIL_OUTPUT_FILE="$TEST_DIR/worker_logs_tail.txt"

timeout 3s curl -s -H "Accept: text/event-stream" "$HTTP_LOGS_URL?follow=true&tail=2" > "$LOGS_TAIL_OUTPUT_FILE" &
LOGS_TAIL_PID=$!

# Wait for stream to complete or timeout
wait $LOGS_TAIL_PID 2>/dev/null || true

if [ -f "$LOGS_TAIL_OUTPUT_FILE" ] && [ -s "$LOGS_TAIL_OUTPUT_FILE" ]; then
	echo "SUCCESS (Log Streaming Test 6): Tail parameter streaming works."
else
	echo "ERROR (Log Streaming Test 6): Tail parameter streaming failed."
	exit 1
fi

# Test 4: Test invalid worker ID (should return error)
echo "Testing log streaming with invalid worker ID..."
INVALID_LOGS_URL="http://127.0.0.1:${NODE1_HTTP_PORT}/api/workers/invalid-worker-id/logs"
INVALID_LOGS_RESPONSE=$(curl -s -w "%{http_code}" -o /dev/null "$INVALID_LOGS_URL?follow=true" || echo "000")

if [ "$INVALID_LOGS_RESPONSE" = "500" ] || [ "$INVALID_LOGS_RESPONSE" = "404" ]; then
	echo "SUCCESS (Log Streaming Test 7): Invalid worker ID correctly returns error ($INVALID_LOGS_RESPONSE)."
else
	echo "ERROR (Log Streaming Test 7): Expected error for invalid worker ID, got $INVALID_LOGS_RESPONSE"
	exit 1
fi

echo "SUCCESS: All worker log streaming tests passed."

# ==============================================================================
# New Test Workflow (Step 7: Scheduled Events)
# ==============================================================================
echo "--- Starting Scheduled Events Test Workflow ---"

# 7a. Prepare Work Order for scheduled events worker
echo "Preparing scheduled events work order..."
SCHEDULED_WORKER_JS_CONTENT=$(cat "$HTTP_ECHO_WORKER_JS_BUNDLE_PATH") # Reuse the echo worker for simplicity
SCHEDULED_WORKER_JS_B64=$(printf "%s" "$SCHEDULED_WORKER_JS_CONTENT" | base64 | tr -d '\n')

SCHEDULED_WORKER_BUNDLE_PAYLOAD_FILE="$TEST_DIR/scheduled_worker_bundle_payload.json"
jq -n \
	--arg vm "nxcc/workerd" \
	--arg executable_b64 "$SCHEDULED_WORKER_JS_B64" \
	'{vm: $vm, executable: $executable_b64, metadata: {}}' >"$SCHEDULED_WORKER_BUNDLE_PAYLOAD_FILE"

SCHEDULED_WORKER_BUNDLE_PAYLOAD_B64_FILE="$TEST_DIR/scheduled_worker_bundle_payload_b64.txt"
base64 <"$SCHEDULED_WORKER_BUNDLE_PAYLOAD_FILE" | tr -d '\n' >"$SCHEDULED_WORKER_BUNDLE_PAYLOAD_B64_FILE"

SCHEDULED_WORKER_BUNDLE_DSSE_FILE="$TEST_DIR/scheduled_worker_bundle_dsse.json"
jq -n \
	--rawfile payload_b64 "$SCHEDULED_WORKER_BUNDLE_PAYLOAD_B64_FILE" \
	--arg payload_type "application/vnd.nxcc.workerbundlepayload.v1+json" \
	'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$SCHEDULED_WORKER_BUNDLE_DSSE_FILE"

SCHEDULED_WORKER_BUNDLE_DSSE_B64_FILE="$TEST_DIR/scheduled_worker_bundle_dsse_b64.txt"
base64 <"$SCHEDULED_WORKER_BUNDLE_DSSE_FILE" | tr -d '\n' >"$SCHEDULED_WORKER_BUNDLE_DSSE_B64_FILE"

SCHEDULED_WORKER_MANIFEST_FILE="$TEST_DIR/scheduled_worker_manifest.json"
jq -n \
	--rawfile bundle_source_b64 "$SCHEDULED_WORKER_BUNDLE_DSSE_B64_FILE" \
	"{
        bundle: {source: (\"data:application/json;base64,\" + \$bundle_source_b64), hash: null},
        identities: [],
        userdata: {testMessage: \"scheduled-test\"}
    }" >"$SCHEDULED_WORKER_MANIFEST_FILE"

SCHEDULED_WORK_ORDER_PAYLOAD_FILE="$TEST_DIR/scheduled_work_order_payload.json"
jq -n \
	--arg id "scheduled-test-work-order-$(date +%s%N)" \
	--slurpfile worker_manifest "$SCHEDULED_WORKER_MANIFEST_FILE" \
	'{
        id: $id,
        worker: $worker_manifest[0],
        events: [
            {"handler": "launch", "kind": "launch"},
            {"handler": "fetch", "kind": "scheduled", "period_ms": 2000}
        ]
    }' >"$SCHEDULED_WORK_ORDER_PAYLOAD_FILE"

SCHEDULED_WORK_ORDER_PAYLOAD_B64_FILE="$TEST_DIR/scheduled_work_order_payload_b64.txt"
base64 <"$SCHEDULED_WORK_ORDER_PAYLOAD_FILE" | tr -d '\n' >"$SCHEDULED_WORK_ORDER_PAYLOAD_B64_FILE"

SCHEDULED_WORK_ORDER_DSSE_FILE="$TEST_DIR/scheduled_work_order_dsse.json"
jq -n \
	--rawfile payload_b64 "$SCHEDULED_WORK_ORDER_PAYLOAD_B64_FILE" \
	--arg payload_type "$DSSE_WORK_ORDER_PAYLOAD_TYPE" \
	'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$SCHEDULED_WORK_ORDER_DSSE_FILE"

SCHEDULED_WORK_ORDER_DSSE_B64_FILE="$TEST_DIR/scheduled_work_order_dsse_b64.txt"
base64 <"$SCHEDULED_WORK_ORDER_DSSE_FILE" | tr -d '\n' >"$SCHEDULED_WORK_ORDER_DSSE_B64_FILE"

GRPCURL_SUBMIT_SCHEDULED_WO_PAYLOAD_FILE="$TEST_DIR/submit_scheduled_wo_payload.json"
jq -n \
	--rawfile work_order_dsse_bytes "$SCHEDULED_WORK_ORDER_DSSE_B64_FILE" \
	'{work_order_dsse_bytes: $work_order_dsse_bytes}' >"$GRPCURL_SUBMIT_SCHEDULED_WO_PAYLOAD_FILE"

# 7b. Submit scheduled events work order to Alice
echo "Submitting scheduled events work order to Alice..."
SCHEDULED_WO_SUBMIT_RESPONSE=$(grpcurl_submit_work_order "$alice_DAEMON_SOCK" "$GRPCURL_SUBMIT_SCHEDULED_WO_PAYLOAD_FILE")
echo "Scheduled Work Order Submit Response: $SCHEDULED_WO_SUBMIT_RESPONSE"

SCHEDULED_WO_SUBMIT_SUCCESS=$(echo "$SCHEDULED_WO_SUBMIT_RESPONSE" | jq -r .success)
if [ "$SCHEDULED_WO_SUBMIT_SUCCESS" != "true" ]; then
	echo "ERROR: Submitting scheduled work order was not successful."
	exit 1
fi

SCHEDULED_WORK_ORDER_ID=$(echo "$SCHEDULED_WO_SUBMIT_RESPONSE" | jq -r .workOrderId)
if [ -z "$SCHEDULED_WORK_ORDER_ID" ] || [ "$SCHEDULED_WORK_ORDER_ID" = "null" ]; then
	echo "ERROR: Failed to get workOrderId from scheduled work order submission."
	exit 1
fi
echo "Scheduled Work Order ID: $SCHEDULED_WORK_ORDER_ID"

# 7c. Wait for scheduled events to fire and check logs
echo "Waiting for scheduled events to fire (8 seconds to catch multiple events)..."
sleep 8

# 7d. Check for scheduled event execution in logs
echo "Checking for scheduled event execution in Alice's daemon logs..."
if grep -q "Firing scheduled event" "$alice_DAEMON_LOG"; then
	SCHEDULED_EVENT_COUNT=$(grep -c "Firing scheduled event" "$alice_DAEMON_LOG")
	echo "SUCCESS (Scheduled Events Test): Found $SCHEDULED_EVENT_COUNT scheduled event(s) in daemon logs."
	
	# Verify we got multiple events (should be at least 3 in 8 seconds with 2-second interval)
	if [ "$SCHEDULED_EVENT_COUNT" -ge 3 ]; then
		echo "SUCCESS (Scheduled Events Test): Multiple scheduled events detected ($SCHEDULED_EVENT_COUNT events)."
	else
		echo "WARNING (Scheduled Events Test): Only $SCHEDULED_EVENT_COUNT scheduled events detected, expected at least 3."
	fi
else
	echo "ERROR (Scheduled Events Test): No scheduled events found in daemon logs."
	echo "Alice daemon log contents:"
	cat "$alice_DAEMON_LOG" || true
	exit 1
fi

echo "--- Test Workflow Completed Successfully ---"

# Cleanup is handled by the trap
exit 0
