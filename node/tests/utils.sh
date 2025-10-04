#!/bin/sh
# shellcheck disable=SC3043  # 'local' is not POSIX but widely supported

command_exists() {
	command -v "$1" >/dev/null 2>&1
}

ensure_grpcurl() {
	if command_exists grpcurl; then
		return 0
	fi
	echo "Error: grpcurl command not found. Install it from https://github.com/fullstorydev/grpcurl or your package manager." >&2
	return 1
}

resolve_path() {
	case "$1" in
	~/*)
		echo "$HOME/${1#~/}"
		;;
	*)
		echo "$1"
		;;
	esac
}

ensure_workerd() {
	configured_path="${NXCC_WORKERD_BINARY_PATH:-${WORKERD_BIN_PATH:-}}"
	if [ -n "$configured_path" ]; then
		resolved_configured_path=$(resolve_path "$configured_path")
		if [ -x "$resolved_configured_path" ]; then
			export NXCC_WORKERD_BINARY_PATH="$resolved_configured_path"
			if [ -z "${WORKERD_BIN_PATH:-}" ]; then
				export WORKERD_BIN_PATH="$resolved_configured_path"
			fi
			return 0
		fi
		echo "Error: workerd binary configured at '$configured_path' is not executable." >&2
		return 1
	fi

	resolved_workerd=$(command -v workerd 2>/dev/null || true)
	if [ -n "$resolved_workerd" ]; then
		export NXCC_WORKERD_BINARY_PATH="$resolved_workerd"
		if [ -z "${WORKERD_BIN_PATH:-}" ]; then
			export WORKERD_BIN_PATH="$resolved_workerd"
		fi
		return 0
	fi

	os_hint="See https://github.com/cloudflare/workerd/releases for installation instructions."
	case "$(uname -s)" in
	Darwin)
		os_hint="macOS via Homebrew: brew install cloudflare/workers/workerd"
		;;
	Linux)
		os_hint="Linux: curl -fsSLo workerd.gz https://github.com/cloudflare/workerd/releases/latest/download/workerd-linux-64.gz; gunzip workerd.gz; chmod +x workerd; sudo mv workerd /usr/local/bin/ (use workerd-linux-arm64.gz on arm64)"
		;;
	esac

	cat >&2 <<EOF
Error: Cloudflare's workerd runtime is required but was not found on PATH.

Install options:
  - $os_hint
  - Prefer containers? Use the packaged image: docker build -t nxcc-node node && docker run --rm nxcc-node

To use a custom workerd binary, set NXCC_WORKERD_BINARY_PATH=/path/to/workerd (or WORKERD_BIN_PATH) before running this script.
EOF
	return 1
}

ensure_rust_toolchain() {
	missing_tools=""
	if ! command_exists cargo; then
		missing_tools="cargo"
	fi
	if ! command_exists rustc; then
		if [ -n "$missing_tools" ]; then
			missing_tools="$missing_tools, rustc"
		else
			missing_tools="rustc"
		fi
	fi

	if [ -n "$missing_tools" ]; then
		echo "Warning: Rust toolchain not detected (missing: $missing_tools)." >&2
		echo "Hint: install Rust via rustup (https://rustup.rs/) to provide cargo and rustc on PATH." >&2
		return 1
	fi

	return 0
}

ensure_node_runtime_deps() {
	missing=0
	if ! ensure_rust_toolchain; then
		missing=1
	fi
	if ! ensure_grpcurl; then
		missing=1
	fi
	if ! ensure_workerd; then
		missing=1
	fi
	return $missing
}

# Generate an Ed25519 private key file for operator signing
generate_operator_key() {
	key_path="$1"
	# Generate a 32-byte random key file for Ed25519 private key
	# The NXCC daemon will interpret this as raw key bytes
	head -c 32 /dev/urandom >"$key_path"
}

start_anvils() {
	echo "Starting Anvil instance 1 (chain $ANVIL_CHAIN_ID_1 on port 8545)..."
	anvil --port 8545 --chain-id "$ANVIL_CHAIN_ID_1" --silent &
	ANVIL_PID_1=$!
	# Wait for anvil to be ready
	# shellcheck disable=SC2034  # i is used for timing loop iterations
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
	# shellcheck disable=SC2034  # i is used for timing loop iterations
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

# Function to create a temporary directory with proper OS handling
create_tmp_dir() {
	local prefix="$1"
	if [ "$(uname)" = "Darwin" ]; then
		mktemp -d -t "${prefix}"
	else
		# Linux works with or without -t
		mktemp -d "/tmp/${prefix}-XXXX"
	fi
}

# Function to clean up resources for a node
# Args:
#   $1 - Node name (e.g., "alice")
cleanup_node() {
	NODE_NAME="$1"

	# Get PIDs using eval to access dynamically named variables
	eval "DAEMON_PID=\$${NODE_NAME}_DAEMON_PID"
	eval "ENCLAVE_PID=\$${NODE_NAME}_ENCLAVE_PID"
	eval "VM_PID=\$${NODE_NAME}_VM_PID"
	eval "SOCK_DIR=\$${NODE_NAME}_SOCK_DIR"

	# Kill processes if PIDs exist
	if [ -n "$DAEMON_PID" ]; then kill "$DAEMON_PID" 2>/dev/null || true; fi
	if [ -n "$ENCLAVE_PID" ]; then kill "$ENCLAVE_PID" 2>/dev/null || true; fi
	if [ -n "$VM_PID" ]; then kill "$VM_PID" 2>/dev/null || true; fi

	# Clean up socket directory
	if [ -n "$SOCK_DIR" ] && [ -d "$SOCK_DIR" ]; then
		rm -rf "$SOCK_DIR" 2>/dev/null || true
	fi
}

# Function to wait for all nodes to be properly connected to each other
# Args:
#   $1 - Space-separated list of HTTP ports (e.g., "6922 6923 6924")
#   $2 - Expected number of peer connections per node (e.g., 2 for 3-node setup)
#   $3 - Timeout in seconds (default: 30)
wait_for_peer_connections() {
	HTTP_PORTS="$1"
	EXPECTED_PEER_COUNT="$2"
	TIMEOUT="${3:-30}"

	echo "⏳ Waiting for all nodes to establish peer connections..."
	echo "   Expected connections per node: $EXPECTED_PEER_COUNT"

	# Convert space-separated ports to a list we can iterate over
	PORT_LIST=""
	for port in $HTTP_PORTS; do
		PORT_LIST="$PORT_LIST $port"
	done

	start_time=$(date +%s)
	while true; do
		current_time=$(date +%s)
		elapsed=$((current_time - start_time))

		if [ $elapsed -ge "$TIMEOUT" ]; then
			echo "❌ ERROR: Timeout waiting for peer connections after ${TIMEOUT}s"
			echo "   Checking final connection status..."
			for port in $PORT_LIST; do
				echo "   Node on port $port:"
				curl -s "http://127.0.0.1:$port/api/status" | jq -r '.connected_peers | to_entries | map("     " + .key + " -> " + (.value | tostring)) | .[]' 2>/dev/null || echo "     Failed to get status"
			done
			return 1
		fi

		# Check all nodes have the expected number of connections
		all_connected=true
		for port in $PORT_LIST; do
			# Get connected_peers count for this node
			peer_count=$(curl -s "http://127.0.0.1:$port/api/status" 2>/dev/null | jq -r '.connected_peers | length' 2>/dev/null || echo "0")

			if [ "$peer_count" -ne "$EXPECTED_PEER_COUNT" ]; then
				all_connected=false
				break
			fi
		done

		if [ "$all_connected" = "true" ]; then
			echo "✅ All nodes have established expected peer connections"
			# Show final connection summary
			for port in $PORT_LIST; do
				peer_count=$(curl -s "http://127.0.0.1:$port/api/status" 2>/dev/null | jq -r '.connected_peers | length' 2>/dev/null || echo "0")
				echo "   Node on port $port: $peer_count connected peers"
			done
			return 0
		fi

		printf "."
		sleep 1
	done
}

# Function to set up and start a node
# Args:
#   $1 - Node name (e.g., "alice")
#   $2 - Test directory (base directory for all nodes)
#   $3 - Node port (for libp2p TCP listening)
#   $4 - Bootstrap peers (optional, comma-separated multiaddrs)
#   $5 - Daemon binary path
#   $6 - Enclave binary path
#   $7 - Workerd VM binary path
#   $8 - HTTP listen address (e.g., "127.0.0.1:6922")
#   $9 - Operator signing key path (optional)
setup_node() {
	NODE_NAME="$1"
	TEST_DIR="$2"
	NODE_PORT="$3"
	BOOTSTRAP_PEERS="$4"
	DAEMON_BIN="$5"
	ENCLAVE_BIN="$6"
	WORKERD_VM_BIN="$7"
	HTTP_LISTEN_ADDR="$8"
	OPERATOR_KEY_PATH="$9"

	# Create node directory
	NODE_DIR="$TEST_DIR/$NODE_NAME"
	mkdir -p "$NODE_DIR"

	# Create a temporary directory with shorter paths for sockets
	SOCK_DIR=$(create_tmp_dir "nx-${NODE_NAME}")

	# Define socket paths and other node-specific paths
	NODE_DAEMON_SOCK="$SOCK_DIR/d.sock"
	NODE_ENCLAVE_SOCK="$SOCK_DIR/e.sock"
	NODE_VM_SOCK="$SOCK_DIR/v.sock"
	NODE_VM_ID="policy-vm-${NODE_NAME}"
	NODE_VM_LOG="$NODE_DIR/vm.log"
	NODE_DAEMON_LOG="$NODE_DIR/daemon.log"
	NODE_IDENTITY="$NODE_DIR/identity.key"
	NODE_POLICY_CACHE="$NODE_DIR/policy_cache"

	# Get peer ID
	NODE_PEER_ID=$(RUST_LOG=error "$DAEMON_BIN" --identity-path "$NODE_IDENTITY" --print-peer-id)
	if [ -z "$NODE_PEER_ID" ]; then
		echo "Failed to get Peer ID for $NODE_NAME"
		return 1
	fi

	NODE_MULTIADDR="/ip4/127.0.0.1/tcp/$NODE_PORT/p2p/$NODE_PEER_ID"
	echo "$NODE_NAME Peer ID: $NODE_PEER_ID"
	echo "$NODE_NAME Multiaddr: $NODE_MULTIADDR"

	# Start VM
	echo "Starting $NODE_NAME VM (nxcc-workerd-vm)..."
	"$WORKERD_VM_BIN" --server-mode uds --server-uds-path "$NODE_VM_SOCK" --verbose 2>&1 | tee "$NODE_VM_LOG" &
	NODE_VM_PID=$!

	# Start Enclave
	echo "Starting $NODE_NAME Enclave..."
	RUST_LOG=nxcc_platform_enclave=debug "$ENCLAVE_BIN" --grpc-mode uds --grpc-uds-path "$NODE_ENCLAVE_SOCK" --verbose &
	NODE_ENCLAVE_PID=$!
	sleep 1

	# Start Daemon
	echo "Starting $NODE_NAME Daemon..."
	# Build daemon arguments using POSIX-compliant approach
	DAEMON_ARGS="--uds-path '$NODE_DAEMON_SOCK' --enclave-uds-path '$NODE_ENCLAVE_SOCK' --default-vm-uds-path '$NODE_VM_SOCK' --identity-path '$NODE_IDENTITY' --policy-cache-dir '$NODE_POLICY_CACHE' --listen-addresses '/ip4/127.0.0.1/tcp/$NODE_PORT' --http-listen-addr '$HTTP_LISTEN_ADDR' --verbose"

	# Add bootstrap peers if provided
	if [ -n "$BOOTSTRAP_PEERS" ]; then
		DAEMON_ARGS="$DAEMON_ARGS --bootstrap-peers '$BOOTSTRAP_PEERS'"
	fi

	# Add operator signing key if provided
	if [ -n "$OPERATOR_KEY_PATH" ]; then
		DAEMON_ARGS="$DAEMON_ARGS --operator-signing-key-path '$OPERATOR_KEY_PATH'"
		echo "  Using operator signing key: $OPERATOR_KEY_PATH"
	fi

	# Use eval to properly handle quoted arguments
	# shellcheck disable=SC2086  # We want word splitting here for the eval
	eval "RUST_LOG=info,nxcc_daemon=debug,nxcc_lib=debug '$DAEMON_BIN' $DAEMON_ARGS" 2>&1 | tee "$NODE_DAEMON_LOG" &
	NODE_DAEMON_PID=$!
	sleep 1

	# Return values by setting variables in the parent scope
	# These variables will be available after calling the function
	eval "${NODE_NAME}_DIR=\"$NODE_DIR\""
	eval "${NODE_NAME}_SOCK_DIR=\"$SOCK_DIR\""
	eval "${NODE_NAME}_DAEMON_SOCK=\"$NODE_DAEMON_SOCK\""
	eval "${NODE_NAME}_ENCLAVE_SOCK=\"$NODE_ENCLAVE_SOCK\""
	eval "${NODE_NAME}_VM_SOCK=\"$NODE_VM_SOCK\""
	eval "${NODE_NAME}_VM_ID=\"$NODE_VM_ID\""
	eval "${NODE_NAME}_VM_LOG=\"$NODE_VM_LOG\""
	eval "${NODE_NAME}_DAEMON_LOG=\"$NODE_DAEMON_LOG\""
	eval "${NODE_NAME}_IDENTITY=\"$NODE_IDENTITY\""
	eval "${NODE_NAME}_POLICY_CACHE=\"$NODE_POLICY_CACHE\""
	eval "${NODE_NAME}_PEER_ID=\"$NODE_PEER_ID\""
	eval "${NODE_NAME}_MULTIADDR=\"$NODE_MULTIADDR\""
	eval "${NODE_NAME}_VM_PID=$NODE_VM_PID"
	eval "${NODE_NAME}_ENCLAVE_PID=$NODE_ENCLAVE_PID"
	eval "${NODE_NAME}_DAEMON_PID=$NODE_DAEMON_PID"

	return 0
}

# Function to deploy the HTTP echo worker and return its Work Order ID
# Args:
#   $1 - Daemon UDS Path
#   $2 - A unique suffix for file names to avoid collisions
deploy_http_echo_worker() {
	local daemon_sock="$1"
	local suffix="$2"

	echo "Preparing HTTP echo work order with suffix '$suffix'..." >&2
	local HTTP_ECHO_WORKER_JS_CONTENT
	HTTP_ECHO_WORKER_JS_CONTENT=$(cat "$HTTP_ECHO_WORKER_JS_BUNDLE_PATH")
	local HTTP_ECHO_WORKER_JS_B64
	HTTP_ECHO_WORKER_JS_B64=$(printf "%s" "$HTTP_ECHO_WORKER_JS_CONTENT" | base64 | tr -d '\n')

	local HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_FILE="$TEST_DIR/http_echo_worker_bundle_payload_${suffix}.json"
	jq -n \
		--arg vm "nxcc/workerd" \
		--arg executable_b64 "$HTTP_ECHO_WORKER_JS_B64" \
		'{vm: $vm, executable: $executable_b64, metadata: {}}' >"$HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_FILE"

	local HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_B64_FILE="$TEST_DIR/http_echo_worker_bundle_payload_b64_${suffix}.txt"
	base64 <"$HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_FILE" | tr -d '\n' >"$HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_B64_FILE"

	local HTTP_ECHO_WORKER_BUNDLE_DSSE_FILE="$TEST_DIR/http_echo_worker_bundle_dsse_${suffix}.json"
	jq -n \
		--rawfile payload_b64 "$HTTP_ECHO_WORKER_BUNDLE_PAYLOAD_B64_FILE" \
		--arg payload_type "application/vnd.nxcc.workerbundlepayload.v1+json" \
		'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$HTTP_ECHO_WORKER_BUNDLE_DSSE_FILE"

	local HTTP_ECHO_WORKER_BUNDLE_DSSE_B64_FILE="$TEST_DIR/http_echo_worker_bundle_dsse_b64_${suffix}.txt"
	base64 <"$HTTP_ECHO_WORKER_BUNDLE_DSSE_FILE" | tr -d '\n' >"$HTTP_ECHO_WORKER_BUNDLE_DSSE_B64_FILE"

	local HTTP_ECHO_WORKER_MANIFEST_FILE="$TEST_DIR/http_echo_worker_manifest_${suffix}.json"
	jq -n \
		--rawfile bundle_source_b64 "$HTTP_ECHO_WORKER_BUNDLE_DSSE_B64_FILE" \
		"{
        bundle: {source: (\"data:application/json;base64,\" + \$bundle_source_b64), hash: null},
        identities: [],
        userdata: {}
    }" >"$HTTP_ECHO_WORKER_MANIFEST_FILE"

	local HTTP_ECHO_WORK_ORDER_PAYLOAD_FILE="$TEST_DIR/http_echo_work_order_payload_${suffix}.json"
	jq -n \
		--arg id "http-echo-work-order-${suffix}-$(date +%s%N)" \
		--slurpfile worker_manifest "$HTTP_ECHO_WORKER_MANIFEST_FILE" \
		'{
        id: $id,
        worker: $worker_manifest[0],
        events: [
            {"handler": "fetch", "kind": "http_request"}
        ]
    }' >"$HTTP_ECHO_WORK_ORDER_PAYLOAD_FILE"

	local HTTP_ECHO_WORK_ORDER_PAYLOAD_B64_FILE="$TEST_DIR/http_echo_work_order_payload_b64_${suffix}.txt"
	base64 <"$HTTP_ECHO_WORK_ORDER_PAYLOAD_FILE" | tr -d '\n' >"$HTTP_ECHO_WORK_ORDER_PAYLOAD_B64_FILE"

	local HTTP_ECHO_WORK_ORDER_DSSE_FILE="$TEST_DIR/http_echo_work_order_dsse_${suffix}.json"
	jq -n \
		--rawfile payload_b64 "$HTTP_ECHO_WORK_ORDER_PAYLOAD_B64_FILE" \
		--arg payload_type "$DSSE_WORK_ORDER_PAYLOAD_TYPE" \
		'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$HTTP_ECHO_WORK_ORDER_DSSE_FILE"

	local HTTP_ECHO_WORK_ORDER_DSSE_B64_FILE="$TEST_DIR/http_echo_work_order_dsse_b64_${suffix}.txt"
	base64 <"$HTTP_ECHO_WORK_ORDER_DSSE_FILE" | tr -d '\n' >"$HTTP_ECHO_WORK_ORDER_DSSE_B64_FILE"

	local GRPCURL_SUBMIT_HTTP_ECHO_WO_PAYLOAD_FILE="$TEST_DIR/submit_http_echo_wo_payload_${suffix}.json"
	jq -n \
		--rawfile work_order_dsse_bytes "$HTTP_ECHO_WORK_ORDER_DSSE_B64_FILE" \
		'{work_order_dsse_bytes: $work_order_dsse_bytes}' >"$GRPCURL_SUBMIT_HTTP_ECHO_WO_PAYLOAD_FILE"

	echo "Submitting HTTP echo work order to $daemon_sock..." >&2
	local submit_response
	submit_response=$(grpcurl_submit_work_order "$daemon_sock" "$GRPCURL_SUBMIT_HTTP_ECHO_WO_PAYLOAD_FILE")

	local submit_success
	submit_success=$(echo "$submit_response" | jq -r .success)
	if [ "$submit_success" != "true" ]; then
		echo "ERROR: Submitting HTTP echo work order was not successful: $submit_response"
		exit 1
	fi

	local work_order_id
	work_order_id=$(echo "$submit_response" | jq -r .workOrderId)
	if [ -z "$work_order_id" ] || [ "$work_order_id" = "null" ]; then
		echo "ERROR: Failed to get workOrderId from HTTP echo work order submission."
		exit 1
	fi

	echo "$work_order_id"
}
