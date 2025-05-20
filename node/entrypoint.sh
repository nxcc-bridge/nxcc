#!/bin/sh
# File: entrypoint.sh
# This script is executed by tini as PID 1 inside the container.
set -e

APP_VERBOSE="${NXCC_ALL_VERBOSE}"

DAEMON_P2P_LISTEN_EFFECTIVE="${DAEMON_P2P_LISTEN_ADDR}"
DAEMON_OWN_GRPC_UDS_PATH_EFFECTIVE="/run/nxcc/daemon.sock"
ENCLAVE_UDS_PATH_EFFECTIVE="${ENCLAVE_UDS_SOCKET}"
WORKERD_VM_UDS_PATH_EFFECTIVE="${WORKERD_UDS_SOCKET}"
WORKERD_EXECUTABLE_PATH_EFFECTIVE="${WORKERD_BIN_PATH_ABS}"

DAEMON_IDENTITY_PATH_EFFECTIVE="${NXCC_IDENTITY_PATH:-}"
DAEMON_POLICY_CACHE_DIR_EFFECTIVE="${NXCC_POLICY_CACHE_DIR:-}"
DAEMON_CONFIG_PATH_EFFECTIVE="${NXCC_CONFIG_PATH:-config.toml}"
DAEMON_EXTRA_ARGS="${NXCC_DAEMON_EXTRA_ARGS:-}"

daemon_cli_args=""
if [ "$APP_VERBOSE" = "true" ]; then
  daemon_cli_args="$daemon_cli_args --verbose"
fi

if [ -f "$DAEMON_CONFIG_PATH_EFFECTIVE" ]; then
  daemon_cli_args="$daemon_cli_args --config $DAEMON_CONFIG_PATH_EFFECTIVE"
else
  daemon_cli_args="$daemon_cli_args --mode uds --uds-path $DAEMON_OWN_GRPC_UDS_PATH_EFFECTIVE"
  daemon_cli_args="$daemon_cli_args --listen-addresses $DAEMON_P2P_LISTEN_EFFECTIVE"
  daemon_cli_args="$daemon_cli_args --enclave-uds-path $ENCLAVE_UDS_PATH_EFFECTIVE"
  daemon_cli_args="$daemon_cli_args --default-vm-uds-path $WORKERD_VM_UDS_PATH_EFFECTIVE"
  if [ -n "$DAEMON_IDENTITY_PATH_EFFECTIVE" ]; then
    daemon_cli_args="$daemon_cli_args --identity-path $DAEMON_IDENTITY_PATH_EFFECTIVE"
  fi
  if [ -n "$DAEMON_POLICY_CACHE_DIR_EFFECTIVE" ]; then
    daemon_cli_args="$daemon_cli_args --policy-cache-dir $DAEMON_POLICY_CACHE_DIR_EFFECTIVE"
  fi
fi

if [ -n "$DAEMON_EXTRA_ARGS" ]; then
  daemon_cli_args="$daemon_cli_args $DAEMON_EXTRA_ARGS"
fi

echo "Starting nxcc-daemon with args:$daemon_cli_args"
nxcc-daemon $daemon_cli_args &

enclave_cli_args=""
if [ "$APP_VERBOSE" = "true" ]; then
  enclave_cli_args="$enclave_cli_args --verbose"
fi
enclave_cli_args="$enclave_cli_args --grpc-mode uds --grpc-uds-path $ENCLAVE_UDS_PATH_EFFECTIVE"

echo "Starting nxcc-platform-enclave with args:$enclave_cli_args"
nxcc-platform-enclave $enclave_cli_args &

workerd_vm_cli_args=""
if [ "$APP_VERBOSE" = "true" ]; then
  workerd_vm_cli_args="$workerd_vm_cli_args --verbose"
fi
workerd_vm_cli_args="$workerd_vm_cli_args --server-mode uds --server-uds-path $WORKERD_VM_UDS_PATH_EFFECTIVE"
workerd_vm_cli_args="$workerd_vm_cli_args --workerd-path $WORKERD_EXECUTABLE_PATH_EFFECTIVE"

echo "Starting nxcc-workerd-vm with args:$workerd_vm_cli_args"
nxcc-workerd-vm $workerd_vm_cli_args &

wait
