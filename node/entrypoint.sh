#!/bin/sh
# File: entrypoint.sh
# This script is executed by tini as PID 1 inside the container.
set -e

# --- Configuration ---
# Use environment variables for configuration, with sensible defaults.
# Components now use clap-only configuration with component-specific prefixes.
APP_VERBOSE="${NXCC_ALL_VERBOSE:-false}"
DAEMON_UDS_PATH="/run/nxcc/daemon.sock"
ENCLAVE_UDS_PATH="${NXCC_ENCLAVE_GRPC_UDS_PATH:-/run/nxcc/enclave.sock}"
WORKERD_VM_UDS_PATH="${NXCC_WORKERD_SERVER_UDS_PATH:-/run/nxcc/workerd.sock}"
WORKERD_BIN_PATH="${WORKERD_BIN_PATH_ABS:-/usr/local/bin/workerd}"
DAEMON_EXTRA_ARGS="${NXCC_DAEMON_EXTRA_ARGS:-}"
DUMP_CONFIG="${NXCC_DAEMON_DUMP_CONFIG:-false}"

# Export environment variables for the components to use
# Ensure container defaults so the daemon listens on all interfaces unless overridden
if [ -z "${NXCC_DAEMON_HTTP_LISTEN_ADDR:-}" ]; then
	export NXCC_DAEMON_HTTP_LISTEN_ADDR="0.0.0.0:6922"
fi
if [ -z "${NXCC_DAEMON_LISTEN_ADDRESSES:-}" ]; then
	export NXCC_DAEMON_LISTEN_ADDRESSES="/ip4/0.0.0.0/tcp/9000"
fi

# Daemon configuration (NXCC_DAEMON_ prefix)
export NXCC_DAEMON_UDS_PATH="$DAEMON_UDS_PATH"
export NXCC_DAEMON_ENCLAVE_UDS_PATH="$ENCLAVE_UDS_PATH"
export NXCC_DAEMON_DEFAULT_VM_UDS_PATH="$WORKERD_VM_UDS_PATH"
if [ "$APP_VERBOSE" = "true" ]; then
	export NXCC_DAEMON_VERBOSE="true"
fi

# Enclave configuration (NXCC_ENCLAVE_ prefix)
export NXCC_ENCLAVE_GRPC_UDS_PATH="$ENCLAVE_UDS_PATH"
if [ "$APP_VERBOSE" = "true" ]; then
	export NXCC_ENCLAVE_VERBOSE="true"
fi

# VM configuration (NXCC_WORKERD_ prefix for workerd VM)
workerd_vm_env="NXCC_VM_SERVER_MODE=uds NXCC_VM_SERVER_UDS_PATH=$WORKERD_VM_UDS_PATH NXCC_WORKERD_SERVER_UDS_PATH=$WORKERD_VM_UDS_PATH NXCC_WORKERD_WORKERD_PATH=$WORKERD_BIN_PATH"
if [ "$APP_VERBOSE" = "true" ]; then
	workerd_vm_env="$workerd_vm_env NXCC_VM_VERBOSE=true NXCC_WORKERD_VERBOSE=true"
fi

# If dump config mode, skip starting VM and enclave
if [ "$DUMP_CONFIG" != "true" ]; then
	# --- Start VM ---
	# Pass base VM settings alongside the workerd-specific ones so additional VM binaries
	# can be launched with their own bindings in the future.
	echo "Starting nxcc-workerd-vm (configured via environment variables)"
	# shellcheck disable=SC2086
	env $workerd_vm_env nxcc-workerd-vm &

	# --- Start Enclave ---
	# Enclave configuration is now handled via environment variables
	echo "Starting nxcc-platform-enclave (configured via environment variables)"
	nxcc-platform-enclave &

	# Wait for dependent services to be ready before starting the daemon.
	echo "Waiting for VM and enclave sockets..."
	while ! [ -S "$WORKERD_VM_UDS_PATH" ] || ! [ -S "$ENCLAVE_UDS_PATH" ]; do
		sleep 0.1
	done
	echo "VM and enclave are ready."
fi

# --- Start Daemon ---
# Daemon configuration is now handled via environment variables with clap
daemon_cli_args=""

# Pass through extra daemon arguments if specified
if [ -n "$DAEMON_EXTRA_ARGS" ]; then
	daemon_cli_args="$daemon_cli_args $DAEMON_EXTRA_ARGS"
fi

# Add --dump-config if in dump config mode
if [ "$DUMP_CONFIG" = "true" ]; then
	daemon_cli_args="$daemon_cli_args --dump-config"
fi

echo "Starting nxcc-daemon (configured via environment variables)${daemon_cli_args:+ with extra args:$daemon_cli_args}"
# shellcheck disable=SC2086
if [ "$DUMP_CONFIG" = "true" ]; then
	# In dump config mode, run daemon directly and exit
	nxcc-daemon $daemon_cli_args
else
	# Normal mode - run in background and wait
	nxcc-daemon $daemon_cli_args &
	echo "All components started. Waiting for processes to exit..."
	wait
fi
