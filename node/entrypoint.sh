#!/bin/sh
# File: entrypoint.sh
# This script is executed by tini as PID 1 inside the container.
set -e

# --- Configuration ---
# Use environment variables for configuration, with sensible defaults.
APP_VERBOSE="${NXCC_ALL_VERBOSE:-false}"
DAEMON_GRPC_ADDR="${DAEMON_GRPC_TARGET_ADDR:-}"
DAEMON_P2P_ADDR="${DAEMON_P2P_LISTEN_ADDR:-/ip4/0.0.0.0/tcp/9000}"
DAEMON_UDS_PATH="/run/nxcc/daemon.sock"
ENCLAVE_UDS_PATH="${ENCLAVE_UDS_SOCKET:-/run/nxcc/enclave.sock}"
WORKERD_VM_UDS_PATH="${WORKERD_UDS_SOCKET:-/run/nxcc/workerd.sock}"
WORKERD_BIN_PATH="${WORKERD_BIN_PATH_ABS:-/usr/local/bin/workerd}"
IDENTITY_PATH="${NXCC_IDENTITY_PATH:-}"
POLICY_CACHE_DIR="${NXCC_POLICY_CACHE_DIR:-}"
CONFIG_PATH="${NXCC_CONFIG_PATH:-config.toml}"
DAEMON_EXTRA_ARGS="${NXCC_DAEMON_EXTRA_ARGS:-}"

# --- Start VM ---
vm_cli_args="--server-mode uds --server-uds-path $WORKERD_VM_UDS_PATH --workerd-path $WORKERD_BIN_PATH"
if [ "$APP_VERBOSE" = "true" ]; then
	vm_cli_args="$vm_cli_args --verbose"
fi
echo "Starting nxcc-workerd-vm with args: $vm_cli_args"
# shellcheck disable=SC2086
nxcc-workerd-vm $vm_cli_args &

# --- Start Enclave ---
enclave_cli_args="--grpc-mode uds --grpc-uds-path $ENCLAVE_UDS_PATH"
if [ "$APP_VERBOSE" = "true" ]; then
	enclave_cli_args="$enclave_cli_args --verbose"
fi
echo "Starting nxcc-platform-enclave with args: $enclave_cli_args"
# shellcheck disable=SC2086
nxcc-platform-enclave $enclave_cli_args &

# Wait for dependent services to be ready before starting the daemon.
echo "Waiting for VM and enclave sockets..."
while ! [ -S "$WORKERD_VM_UDS_PATH" ] || ! [ -S "$ENCLAVE_UDS_PATH" ]; do
	sleep 0.1
done
echo "VM and enclave are ready."

# --- Start Daemon ---
# It is assumed that the daemon will automatically attach the VM specified via
# --default-vm-uds-path once the socket is available.
daemon_cli_args=""
if [ "$APP_VERBOSE" = "true" ]; then
	daemon_cli_args="$daemon_cli_args --verbose"
fi

if [ -f "$CONFIG_PATH" ]; then
	daemon_cli_args="$daemon_cli_args --config $CONFIG_PATH"
else
	if [ -n "$DAEMON_GRPC_ADDR" ]; then
		daemon_cli_args="$daemon_cli_args --mode tcp --tcp-addr $DAEMON_GRPC_ADDR"
	else
		daemon_cli_args="$daemon_cli_args --mode uds --uds-path $DAEMON_UDS_PATH"
	fi
	daemon_cli_args="$daemon_cli_args --listen-addresses $DAEMON_P2P_ADDR"
	daemon_cli_args="$daemon_cli_args --http-listen-addr 0.0.0.0:6922"
	daemon_cli_args="$daemon_cli_args --enclave-uds-path $ENCLAVE_UDS_PATH"
	daemon_cli_args="$daemon_cli_args --default-vm-uds-path $WORKERD_VM_UDS_PATH"
	if [ -n "$IDENTITY_PATH" ]; then
		daemon_cli_args="$daemon_cli_args --identity-path $IDENTITY_PATH"
	fi
	if [ -n "$POLICY_CACHE_DIR" ]; then
		daemon_cli_args="$daemon_cli_args --policy-cache-dir $POLICY_CACHE_DIR"
	fi
fi

if [ -n "$DAEMON_EXTRA_ARGS" ]; then
	daemon_cli_args="$daemon_cli_args $DAEMON_EXTRA_ARGS"
fi

echo "Starting nxcc-daemon with args:$daemon_cli_args"
# shellcheck disable=SC2086
nxcc-daemon $daemon_cli_args &

echo "All components started. Waiting for processes to exit..."
wait
