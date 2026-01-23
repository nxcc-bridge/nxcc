#!/bin/sh

set -e # Exit immediately if a command exits with a non-zero status.
# set -x # Debugging: print commands

# =============================================================================
# NXCC Comprehensive Integration Test
# =============================================================================
# This test combines all NXCC functionality into a single comprehensive test:
# - 3-node attestation-based architecture (Alice, Bob trusted; Charlie untrusted)
# - Secret sharing with real attestation policies
# - Cross-chain event handling
# - HTTP worker testing
# - Worker log streaming
# - Scheduled events
# - Security validation and forgery protection
# =============================================================================

# --- Configuration ---
SCRIPT_DIR=$(dirname "$0")
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
MODE="debug"

# Source helpers and phases
. "$SCRIPT_DIR/utils.sh"
. "$SCRIPT_DIR/grpcurl_helper.sh"
. "$SCRIPT_DIR/phases/cleanup.sh"
. "$SCRIPT_DIR/phases/setup.sh"
. "$SCRIPT_DIR/phases/secret_sharing.sh"
. "$SCRIPT_DIR/phases/cross_chain_events.sh"
. "$SCRIPT_DIR/phases/http_worker.sh"
. "$SCRIPT_DIR/phases/advanced_features.sh"

# Set up trap for signals
trap cleanup EXIT INT TERM

# --- Phase Dependencies ---
# Define setup dependencies for each phase.
# shellcheck disable=SC2034  # Used dynamically via eval in phase dependency resolution
PHASE_DEPS_SECRET_SHARING="nodes_3"
# shellcheck disable=SC2034  # Used dynamically via eval in phase dependency resolution
PHASE_DEPS_CROSS_CHAIN_EVENTS="nodes_2 anvil contracts"
# shellcheck disable=SC2034  # Used dynamically via eval in phase dependency resolution
PHASE_DEPS_HTTP_WORKER="nodes_1"
# shellcheck disable=SC2034  # Used dynamically via eval in phase dependency resolution
PHASE_DEPS_ADVANCED_FEATURES="nodes_1"
# shellcheck disable=SC2034  # Used dynamically via eval in phase dependency resolution
PHASE_DEPS_SETUP="nodes_3" # The setup phase itself requires 3 nodes

# --- Global Test Parameters ---
TEST_DIR=""
P2P_RESPONSE_TIMEOUT_SECS="${NXCC_DAEMON_P2P_RESPONSE_TIMEOUT_SECS:-15}"
export NXCC_DAEMON_P2P_RESPONSE_TIMEOUT_SECS="$P2P_RESPONSE_TIMEOUT_SECS"
JS_WORKER_DIR="$SCRIPT_DIR/js_workers"
# shellcheck disable=SC2034  # Used by phase files (phases/setup.sh)
CONTRACTS_DIR="$SCRIPT_DIR/contracts"

# Binaries
DAEMON_BIN=""
ENCLAVE_BIN=""
WORKERD_VM_BIN=""

# Operator Keys
ALICE_OPERATOR_KEY=""
BOB_OPERATOR_KEY=""

# Anvil PIDs - used by utils.sh for process management
# shellcheck disable=SC2034  # Used by utils.sh functions
ANVIL_PID_1=""
# shellcheck disable=SC2034  # Used by utils.sh functions
ANVIL_PID_2=""

# Secret Sharing Parameters - used by phases/secret_sharing.sh
# shellcheck disable=SC2034  # Used by phases/secret_sharing.sh
SECRET_CHAIN_ID=0 # Mock chain
# shellcheck disable=SC2034  # Used by phases/secret_sharing.sh
SECRET_IDENTITY_ADDR="0x1111111111111111111111111111111111111111"
# shellcheck disable=SC2034  # Used by phases/secret_sharing.sh
SECRET_IDENTITY_ID_NUM="555" # Use a simple numeric string for grpcurl
# shellcheck disable=SC2034  # Used by phases/secret_sharing.sh
SECRET_NAME_IN_WORKER="THE_SECRET" # How the secret is named in the worker's env

# Anvil configuration - used by phase files
# shellcheck disable=SC2034  # Used by phase files
ANVIL_RPC_URL_1="http://127.0.0.1:8545"
# shellcheck disable=SC2034  # Used by phase files
ANVIL_CHAIN_ID_1=31337
# shellcheck disable=SC2034  # Used by phase files
ANVIL_RPC_URL_2="http://127.0.0.1:8546"
# shellcheck disable=SC2034  # Used by phase files
ANVIL_CHAIN_ID_2=1338
# shellcheck disable=SC2034  # Used by phase files
ANVIL_WS_URL_1="ws://127.0.0.1:8545"
# shellcheck disable=SC2034  # Used by phase files
ANVIL_WS_URL_2="ws://127.0.0.1:8546"
# shellcheck disable=SC2034  # Used by phase files
DEPLOYER_PK="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80" # Anvil default[0]
# shellcheck disable=SC2034  # Used by phase files
WORKER_SENDER_PK="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d" # Anvil default[1]

# Path to the JS worker script for the test
# shellcheck disable=SC2034  # TEST_WORKER_JS_PATH may be used by future tests
TEST_WORKER_JS_PATH="$SCRIPT_DIR/policy/test_worker_integration.js"
# shellcheck disable=SC2034  # Used by phase files
DSSE_WORK_ORDER_PAYLOAD_TYPE="application/vnd.nxcc.workorderpayload.v1+json"
# shellcheck disable=SC2034  # Used by phase files for worker content loading
POLICY_WORKER_JS_BUNDLE_PATH=""
EVENT_HANDLER_WORKER_JS_BUNDLE_PATH=""
HTTP_ECHO_WORKER_JS_BUNDLE_PATH=""

# Node configuration - used by phase files
# shellcheck disable=SC2034  # Used by phases/setup.sh
NODE1_NAME="alice" # Trusted node
# shellcheck disable=SC2034  # Used by phases/setup.sh
NODE2_NAME="bob" # Trusted node
# shellcheck disable=SC2034  # Used by phases/setup.sh
NODE3_NAME="charlie" # Untrusted node (will be blocked by attestation policy)
# shellcheck disable=SC2034  # Used by phases/setup.sh
NODE1_P2P_PORT=9001
# shellcheck disable=SC2034  # Used by phases/setup.sh
NODE2_P2P_PORT=9002
# shellcheck disable=SC2034  # Used by phases/setup.sh
NODE3_P2P_PORT=9003
# shellcheck disable=SC2034  # Used by phases/setup.sh
NODE1_HTTP_PORT=6922
# shellcheck disable=SC2034  # Used by phases/setup.sh
NODE2_HTTP_PORT=6923
# shellcheck disable=SC2034  # Used by phases/setup.sh
NODE3_HTTP_PORT=6924

# --- Test Preparation ---
prepare_test_environment() {
	if ! ensure_node_runtime_deps; then
		exit 1
	fi

	TEST_DIR=$(create_tmp_dir "nxcc-comprehensive-test")
	echo "🚀 === NXCC Comprehensive Integration Test ==="
	echo "📁 Test directory: $TEST_DIR"

	ALICE_OPERATOR_KEY="$TEST_DIR/alice_operator.key"
	BOB_OPERATOR_KEY="$TEST_DIR/bob_operator.key"
	echo "Creating operator signing keys for trusted nodes..."
	generate_operator_key "$ALICE_OPERATOR_KEY"
	generate_operator_key "$BOB_OPERATOR_KEY"
	echo "✅ Operator keys created for Alice and Bob"

	echo "Building NXCC binaries..."
	cargo build

	DAEMON_BIN="$REPO_ROOT/target/$MODE/nxcc-daemon"
	ENCLAVE_BIN="$REPO_ROOT/target/$MODE/nxcc-platform-enclave"
	WORKERD_VM_BIN="$REPO_ROOT/target/$MODE/nxcc-workerd-vm"

	if [ ! -f "$DAEMON_BIN" ] || [ ! -f "$ENCLAVE_BIN" ] || [ ! -f "$WORKERD_VM_BIN" ]; then
		echo "One or more binaries not found. Build first."
		exit 1
	fi

	echo "--- Building JS Workers ---"
	if [ ! -d "$JS_WORKER_DIR/node_modules" ]; then
		echo "Installing JS dependencies..."
		(cd "$JS_WORKER_DIR" && npm install)
	fi
	echo "Building JS bundles..."
	(cd "$JS_WORKER_DIR" && npm run build)

	# These variables are used by phase files for worker content loading
	# shellcheck disable=SC2034  # Used by phases/secret_sharing.sh
	POLICY_WORKER_JS_BUNDLE_PATH="$JS_WORKER_DIR/dist/test_worker_integration.js"
	# shellcheck disable=SC2034  # Used by phases/cross_chain_events.sh
	EVENT_HANDLER_WORKER_JS_BUNDLE_PATH="$JS_WORKER_DIR/dist/event_handler_worker.js"
	# shellcheck disable=SC2034  # Used by phases/advanced_features.sh
	HTTP_ECHO_WORKER_JS_BUNDLE_PATH="$JS_WORKER_DIR/dist/http_echo_worker.js"
}

print_help() {
	echo "Usage: $0 [phase1 phase2 ...]"
	echo ""
	echo "Runs the NXCC comprehensive integration test."
	echo "If no phases are specified, all phases are run in order."
	echo ""
	echo "Available phases:"
	echo "  setup                - Sets up a 3-node network."
	echo "  secret_sharing       - Tests attestation-based secret sharing."
	echo "  cross_chain_events   - Tests cross-chain event handling."
	echo "  http_worker          - Tests HTTP worker functionality."
	echo "  advanced_features    - Tests advanced features like log streaming and scheduled events."
	echo "  --help, -h           - Show this help message."
}

main() {
	if [ "$1" = "--help" ] || [ "$1" = "-h" ]; then
		print_help
		exit 0
	fi

	phases_to_run="$*"
	run_all=false
	if [ -z "$phases_to_run" ]; then
		phases_to_run="setup secret_sharing cross_chain_events http_worker advanced_features"
		run_all=true
	fi

	prepare_test_environment

	# --- Determine and run setup for the union of all phase dependencies ---
	req_nodes=0
	req_anvil=false
	req_contracts=false

	for phase in $phases_to_run; do
		# Dynamically get dependency string from variable, e.g., PHASE_DEPS_SECRET_SHARING
		phase_deps_var="PHASE_DEPS_$(echo "$phase" | tr '[:lower:]' '[:upper:]')"
		eval "phase_deps=\$${phase_deps_var}"

		# Check node requirements
		# shellcheck disable=SC2154  # phase_deps is set by eval from dynamic variable name
		if echo "$phase_deps" | grep -q "nodes_3"; then
			if [ "$req_nodes" -lt 3 ]; then req_nodes=3; fi
		elif echo "$phase_deps" | grep -q "nodes_2"; then
			if [ "$req_nodes" -lt 2 ]; then req_nodes=2; fi
		elif echo "$phase_deps" | grep -q "nodes_1"; then
			if [ "$req_nodes" -lt 1 ]; then req_nodes=1; fi
		fi

		# Check anvil requirement
		if echo "$phase_deps" | grep -q "anvil"; then
			req_anvil=true
		fi

		# Check contracts requirement
		if echo "$phase_deps" | grep -q "contracts"; then
			req_contracts=true
		fi
	done

	# Execute setup based on combined requirements
	if [ "$req_nodes" -gt 0 ]; then
		setup_nodes "$req_nodes"
	fi
	if [ "$req_anvil" = "true" ]; then
		start_anvils
	fi
	if [ "$req_contracts" = "true" ]; then
		setup_contracts
	fi

	for phase in $phases_to_run; do
		case "$phase" in
		setup)
			# The 'setup' phase is now implicitly handled by the dependency system.
			# If specified, we ensure it's logged as complete.
			echo "✅ Phase 'setup' completed (nodes are already set up based on requirements)."
			;;
		secret_sharing)
			phase_secret_sharing
			;;
		cross_chain_events)
			phase_cross_chain_events
			;;
		http_worker)
			phase_http_worker
			;;
		advanced_features)
			phase_advanced_features
			;;
		*)
			echo "Unknown phase: $phase"
			exit 1
			;;
		esac
	done

	if [ "$run_all" = "true" ]; then
		echo ""
		echo "🎉 === COMPREHENSIVE TEST RESULTS SUMMARY ==="
		echo "✅ PHASE 1: 3-node attestation architecture established"
		echo "✅ PHASE 2: Attestation-based secret sharing and access control validated"
		echo "   • Trusted nodes (Alice ↔ Bob) successfully share secrets"
		echo "   • Untrusted nodes (Charlie) blocked by attestation policy"
		echo "✅ PHASE 3: Cross-chain event handling with security policies"
		echo "✅ PHASE 4: HTTP worker functionality with attestation protection"
		echo "✅ PHASE 5: Advanced features (streaming, scheduling) validated"
		echo ""
		echo "🔐 Key Security Properties Demonstrated:"
		echo "   • Cryptographic attestation verification before policy execution"
		echo "   • EAT-compliant claims extraction and validation"
		echo "   • Policy-based access control using verified claims"
		echo "   • Protection against forged attestations"
		echo "   • Secure P2P secret sharing between authorized nodes"
		echo "   • Multi-layer security architecture validation"
		echo ""
		echo "🚀 NXCC comprehensive integration test completed successfully!"
	else
		echo ""
		echo "✅ All specified phases completed successfully!"
	fi

	exit 0
}

main "$@"
