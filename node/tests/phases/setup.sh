#!/bin/sh

setup_nodes() {
	num_nodes="$1"
	echo "🚀 === Setting up $num_nodes-Node Attestation Architecture ==="

	if [ "$num_nodes" -ge 1 ]; then
		# --- Node 1 (Alice - Trusted) Setup ---
		echo "🟢 Setting up Alice (trusted node) on P2P port $NODE1_P2P_PORT and HTTP port $NODE1_HTTP_PORT"
		# setup_node sets dynamic variables like alice_DAEMON_SOCK, alice_VM_ID, alice_VM_SOCK, etc.
		# shellcheck disable=SC2034  # Variables are set dynamically by setup_node
		setup_node "$NODE1_NAME" "$TEST_DIR" "$NODE1_P2P_PORT" "" \
			"$DAEMON_BIN" "$ENCLAVE_BIN" "$WORKERD_VM_BIN" "127.0.0.1:$NODE1_HTTP_PORT" "$ALICE_OPERATOR_KEY"

		sleep 2

		# Attach VM to Enclave via Daemon
		# shellcheck disable=SC2154  # alice_* variables are set by setup_node
		grpcurl_attach_vm "$alice_DAEMON_SOCK" "$alice_VM_ID" "$alice_VM_SOCK"
	fi

	if [ "$num_nodes" -ge 2 ]; then
		# --- Node 2 (Bob - Trusted) Setup ---
		echo "🔵 Setting up Bob (trusted node) on P2P port $NODE2_P2P_PORT and HTTP port $NODE2_HTTP_PORT"
		# setup_node sets dynamic variables like bob_DAEMON_SOCK, bob_VM_ID, bob_VM_SOCK, etc.
		# alice_MULTIADDR was set by the previous setup_node call
		# shellcheck disable=SC2154  # alice_MULTIADDR is set by setup_node function
		setup_node "$NODE2_NAME" "$TEST_DIR" "$NODE2_P2P_PORT" "$alice_MULTIADDR" \
			"$DAEMON_BIN" "$ENCLAVE_BIN" "$WORKERD_VM_BIN" "127.0.0.1:$NODE2_HTTP_PORT" "$BOB_OPERATOR_KEY"

		sleep 1

		# Attach VM to Enclave via Daemon
		# shellcheck disable=SC2154  # bob_* variables are set by setup_node
		grpcurl_attach_vm "$bob_DAEMON_SOCK" "$bob_VM_ID" "$bob_VM_SOCK"
	fi

	if [ "$num_nodes" -ge 3 ]; then
		# --- Node 3 (Charlie - Untrusted) Setup ---
		echo "🔴 Setting up Charlie (untrusted node) on P2P port $NODE3_P2P_PORT and HTTP port $NODE3_HTTP_PORT"
		# setup_node sets dynamic variables like charlie_DAEMON_SOCK, charlie_VM_ID, charlie_VM_SOCK, etc.
		# shellcheck disable=SC2154  # charlie_* variables are set by setup_node
		# Note: Charlie gets NO operator key - this is what makes him untrusted
		setup_node "$NODE3_NAME" "$TEST_DIR" "$NODE3_P2P_PORT" "$alice_MULTIADDR" \
			"$DAEMON_BIN" "$ENCLAVE_BIN" "$WORKERD_VM_BIN" "127.0.0.1:$NODE3_HTTP_PORT"

		sleep 1

		# Attach VM to Enclave via Daemon
		# shellcheck disable=SC2154  # charlie_* variables are set by setup_node
		grpcurl_attach_vm "$charlie_DAEMON_SOCK" "$charlie_VM_ID" "$charlie_VM_SOCK"
	fi

	if [ "$num_nodes" -gt 1 ]; then
		expected_peers=$((num_nodes - 1))
		http_ports=""
		if [ "$num_nodes" -ge 1 ]; then http_ports="$http_ports $NODE1_HTTP_PORT"; fi
		if [ "$num_nodes" -ge 2 ]; then http_ports="$http_ports $NODE2_HTTP_PORT"; fi
		if [ "$num_nodes" -ge 3 ]; then http_ports="$http_ports $NODE3_HTTP_PORT"; fi
		wait_for_peer_connections "$http_ports" "$expected_peers" 30
	fi

	echo "✅ All nodes started and connected to P2P network"
}

setup_contracts() {
	echo "🔧 === Setting up Smart Contracts ==="
	echo "Compiling and deploying TestEvents contract..."
	if ! command -v forge >/dev/null 2>&1; then
		echo "ERROR: forge (foundry) command not found. Please install foundry."
		exit 1
	fi
	(cd "$CONTRACTS_DIR" && forge build --force --via-ir) # Use via-ir for potentially smaller bytecode
	TEST_EVENTS_BYTECODE=$(jq -r .bytecode.object <"$SCRIPT_DIR/out/TestEvents.sol/TestEvents.json")
	# shellcheck disable=SC2034  # TEST_EVENTS_ABI is extracted but may be used later in tests
	TEST_EVENTS_ABI=$(jq .abi <"$SCRIPT_DIR/out/TestEvents.sol/TestEvents.json")
	# shellcheck disable=SC2034  # Variable used by cross_chain_events for ABI file input
	TEST_EVENTS_ABI_FILE="$SCRIPT_DIR/out/TestEvents.sol/TestEvents.json"
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
}

phase_setup() {
	# ==============================================================================
	# PHASE 1: Multi-Node Setup with Attestation-Based Architecture
	# ==============================================================================
	# This phase is now a wrapper to maintain compatibility if called directly.
	# The main script's dependency system is the preferred way to set up nodes.
	setup_nodes 3
}
