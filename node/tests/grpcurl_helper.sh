#!/bin/bash

# Common grpcurl helper functions for the integration test

# Absolute path to the proto directory relative to this script's location
# Use pwd for better sh compatibility than realpath
SCRIPT_DIR=$(dirname "$0")
PROTO_DIR=$(cd "$SCRIPT_DIR/../interface/proto" && pwd)
DAEMON_PROTO="daemon.proto"
ENCLAVE_PROTO="enclave.proto" # Needed for CheckSecrets on enclave

# Function to check if grpcurl is available
check_grpcurl() {
	if ! command -v grpcurl >/dev/null 2>&1; then
		echo "Error: grpcurl command not found. Please install it."
		exit 1
	fi
}

# Function to call Daemon's AttachVm
# Args: $1=Daemon UDS Path, $2=VM ID, $3=VM UDS Path
grpcurl_attach_vm() {
	_daemon_sock="$1"
	_vm_id="$2"
	_vm_sock="$3"
	echo "Attempting to attach VM '$_vm_id' ($_vm_sock) to Daemon ($_daemon_sock)..."

	grpcurl \
		-proto "$DAEMON_PROTO" \
		-import-path "$PROTO_DIR" \
		-plaintext -unix \
		-d '{
      "vm_id": "'"${_vm_id}"'",
      "uds_path": "'"${_vm_sock}"'"
    }' \
		"unix://$_daemon_sock" \
		daemon.Debug/AttachVm
	_grp_exit_code=$?
	if [ $_grp_exit_code -ne 0 ]; then
		echo "ERROR: grpcurl AttachVm failed with exit code $_grp_exit_code for $_daemon_sock"
		return 1
	fi
	echo "AttachVm call completed for $_daemon_sock." >&2

	return 0
}

# Function to call Daemon's SubmitWorkOrder
# Args: $1=Daemon UDS Path, $2=Path to file containing the JSON payload for grpcurl (e.g., {"work_order_dsse_bytes": "BASE64_ENCODED_DSSE_JSON_BYTES"})
grpcurl_submit_work_order() {
	_daemon_sock="$1"
	_payload_file="$2"

	echo "Calling SubmitWorkOrder on $_daemon_sock with payload from $_payload_file..." >&2

	cat $_payload_file | grpcurl \
		-proto "$DAEMON_PROTO" \
		-import-path "$PROTO_DIR" \
		-plaintext -unix \
		-d "@" \
		"unix://$_daemon_sock" \
		daemon.WorkOrder/SubmitWorkOrder
	_grp_exit_code=$?
	if [ $_grp_exit_code -ne 0 ]; then
		echo "ERROR: grpcurl SubmitWorkOrder failed with exit code $_grp_exit_code for $_daemon_sock" >&2
		return 1
	fi
	echo "SubmitWorkOrder call completed for $_daemon_sock." >&2
	return 0
}

# Function to call Daemon's GetSecrets
# Args: $1=Daemon UDS Path, $2=Chain ID, $3=Identity Address, $4=Identity ID (numeric)
grpcurl_get_secrets() {
	_daemon_sock="$1"
	_chain_id="$2"
	_identity_addr="$3"
	_identity_id_num="$4"
	_node_id="$5" # Node ID of the *requester* (e.g., Alice's daemon asking for itself)

	echo "Calling GetSecrets on $_daemon_sock for $_identity_addr/$_identity_id_num (Node: $_node_id)..." >&2
	grpcurl \
		-proto "$DAEMON_PROTO" \
		-import-path "$PROTO_DIR" \
		-plaintext -unix \
		-d '{
          "secret_requests": [
            {
              "secret_id": {
                "chain_id": '"$_chain_id"',
                "identity_address": "'"${_identity_addr}"'",
                "identity_id": "'"${_identity_id_num}"'"
              },
              "consumer": {
                 "bundle_hash": "AAECAwQFBgcICQoLDA0ODw==",
                 "signature": "AAECAwQFBgcICQoLDA0ODwECAwQFBgcICQoLDA0ODw=="
               }
            }
          ]
        }' \
		"unix://$_daemon_sock" \
		daemon.Secrets/GetSecrets
	_grp_exit_code=$?
	if [ $_grp_exit_code -ne 0 ]; then # TODO: this is not the daemon secrets service, it's the enclave one.
		echo "ERROR: grpcurl GetSecrets failed with exit code $_grp_exit_code for $_daemon_sock"
		return 1
	fi
	echo "GetSecrets call completed for $_daemon_sock." >&2

	return 0
}

# Function to call Enclave's CheckSecrets (via Daemon for simplicity, assuming passthrough or dedicated endpoint)
# For this test, we'll call the *enclave* directly, assuming we know its socket path.
# Args: $1=Enclave UDS Path, $2=Chain ID, $3=Identity Address, $4=Identity ID (numeric)
grpcurl_check_secrets_enclave() {
	_enclave_sock="$1"
	_chain_id="$2"
	_identity_addr="$3"
	_identity_id_num="$4"

	# Sleep before making the gRPC call
	sleep 1 >&2

	echo "Calling CheckSecrets on Enclave ($_enclave_sock) for $_identity_addr/$_identity_id_num..." >&2
	grpcurl \
		-proto "$ENCLAVE_PROTO" \
		-import-path "$PROTO_DIR" \
		-plaintext -unix \
		-d '{
          "ids": [
            {
              "chain_id": '"$_chain_id"',
              "identity_address": "'"${_identity_addr}"'",
              "identity_id": "'"${_identity_id_num}"'"
            }
          ]
        }' \
		"unix://$_enclave_sock" \
		enclave.Secrets/CheckSecrets
	_grp_exit_code=$?
	if [ $_grp_exit_code -ne 0 ]; then
		echo "ERROR: grpcurl CheckSecrets failed with exit code $_grp_exit_code for $_enclave_sock" >&2
		return 1
	fi
	echo "CheckSecrets call completed for $_enclave_sock." >&2

	# Sleep after the call
	sleep 1 >&2

	return 0
}

# Function to poll CheckSecrets until found=true or timeout
# Args: $1=Enclave UDS Path, $2=Chain ID, $3=Identity Address, $4=Identity ID (numeric), $5=Timeout (secs), $6=Interval (secs)
poll_until_secret_found() {
	_enclave_sock="$1"
	_chain_id="$2"
	_identity_addr="$3"
	_identity_id_num="$4"
	_timeout_secs="$5"
	_interval_secs="$6"
	_start_time=$(date +%s)
	_end_time=$((_start_time + _timeout_secs))

	echo "Polling CheckSecrets on $_enclave_sock for $_identity_addr/$_identity_id_num (Timeout: ${_timeout_secs}s)..." >&2

	# Initial sleep before starting to poll
	sleep 3 >&2

	while [ "$(date +%s)" -lt $_end_time ]; do
		_output=$(grpcurl_check_secrets_enclave "$_enclave_sock" "$_chain_id" "$_identity_addr" "$_identity_id_num" 2>&1)
		_grpcurl_exit=$?

		if [ $_grpcurl_exit -eq 0 ]; then
			# Check if "found": true is in the output
			if echo "$_output" | grep '"found": *true' >/dev/null; then
				echo "Secret $_identity_addr/$_identity_id_num found on $_enclave_sock." >&2
				# Sleep a bit after finding the secret to ensure stability
				sleep 1 >&2
				return 0 # Success
			else
				printf "." >&2 # Progress indicator
			fi
		else
			echo "Warning: CheckSecrets call failed during polling (Exit: $_grpcurl_exit): $_output" >&2
			# Continue polling, maybe the service wasn't ready yet
		fi

		# Use the provided interval for polling
		sleep "$_interval_secs" >&2
	done

	echo "ERROR: Timeout waiting for secret $_identity_addr/$_identity_id_num on $_enclave_sock." >&2
	return 1 # Failure (timeout)
}

# Make functions available to sourcing scripts
# Note: export -f is a bashism, but often works in modern sh.
# If it fails, the main script needs to define these directly or use "." to source.
export PROTO_DIR DAEMON_PROTO ENCLAVE_PROTO # Added export for new function
export -f check_grpcurl grpcurl_attach_vm grpcurl_submit_work_order grpcurl_get_secrets grpcurl_check_secrets_enclave poll_until_secret_found
