#!/bin/sh
# shellcheck disable=SC3043  # 'local' is not POSIX but widely supported

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
	# Enable test attestation providers for integration tests (both env var and CLI flag supported)
	NXCC_ENABLE_TEST_PROVIDERS=true RUST_LOG=nxcc_platform_enclave=debug "$ENCLAVE_BIN" --grpc-mode uds --grpc-uds-path "$NODE_ENCLAVE_SOCK" --enable-test-providers --verbose &
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
