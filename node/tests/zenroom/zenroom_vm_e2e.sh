#!/bin/sh

set -e

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
NODE_ROOT=$(cd "$SCRIPT_DIR/../.." && pwd)
REPO_ROOT=$(cd "$NODE_ROOT/.." && pwd)

# shellcheck disable=SC1091
. "$NODE_ROOT/tests/utils.sh"
# shellcheck disable=SC1091
. "$NODE_ROOT/tests/grpcurl_helper.sh"

MODE="debug"
POSTBACK_PORT="9911"

if ! ensure_node_runtime_deps; then
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "Error: python3 is required for the postback test server." >&2
  exit 1
fi

ZENCODE_EXEC_PATH="${NXCC_ZENROOM_VM_ZENCODE_EXEC_PATH:-}"
if [ -z "$ZENCODE_EXEC_PATH" ]; then
  if command -v zencode-exec >/dev/null 2>&1; then
    ZENCODE_EXEC_PATH="$(command -v zencode-exec)"
  else
    echo "Error: zencode-exec not found on PATH. Install Zenroom or set NXCC_ZENROOM_VM_ZENCODE_EXEC_PATH." >&2
    exit 1
  fi
fi

TEST_DIR=$(create_tmp_dir "nxcc-zenroom-e2e")
SOCK_DIR=$(create_tmp_dir "nx-zenroom")

DAEMON_SOCK="$SOCK_DIR/d.sock"
ENCLAVE_SOCK="$SOCK_DIR/e.sock"
WORKERD_VM_SOCK="$SOCK_DIR/w.sock"
ZENROOM_VM_SOCK="$SOCK_DIR/z.sock"

DAEMON_LOG="$TEST_DIR/daemon.log"
ENCLAVE_LOG="$TEST_DIR/enclave.log"
WORKERD_VM_LOG="$TEST_DIR/workerd-vm.log"
ZENROOM_VM_LOG="$TEST_DIR/zenroom-vm.log"
POSTBACK_BODY="$TEST_DIR/postback_body.json"
POSTBACK_BODY_SECRET_1="$TEST_DIR/postback_body_secret_1.json"
POSTBACK_BODY_SECRET_2="$TEST_DIR/postback_body_secret_2.json"
POSTBACK_BODY_SECRET_3="$TEST_DIR/postback_body_secret_3.json"
OPERATOR_KEY_FILE="$TEST_DIR/operator_signing_key.bin"

cleanup() {
  set +e
  [ -n "${POSTBACK_PID:-}" ] && kill "$POSTBACK_PID" 2>/dev/null || true
  [ -n "${ZENROOM_VM_PID:-}" ] && kill "$ZENROOM_VM_PID" 2>/dev/null || true
  [ -n "${WORKERD_VM_PID:-}" ] && kill "$WORKERD_VM_PID" 2>/dev/null || true
  [ -n "${ENCLAVE_PID:-}" ] && kill "$ENCLAVE_PID" 2>/dev/null || true
  [ -n "${DAEMON_PID:-}" ] && kill "$DAEMON_PID" 2>/dev/null || true
  [ -d "$SOCK_DIR" ] && rm -rf "$SOCK_DIR"
  [ -d "$TEST_DIR" ] && rm -rf "$TEST_DIR"
}

trap cleanup EXIT INT TERM

printf "Building binaries...\n"
cargo build --manifest-path "$NODE_ROOT/Cargo.toml" \
  -p nxcc-daemon \
  -p nxcc-platform-enclave \
  -p nxcc-workerd-vm \
  -p nxcc-zenroom-vm

DAEMON_BIN="$NODE_ROOT/target/$MODE/nxcc-daemon"
ENCLAVE_BIN="$NODE_ROOT/target/$MODE/nxcc-platform-enclave"
WORKERD_VM_BIN="$NODE_ROOT/target/$MODE/nxcc-workerd-vm"
ZENROOM_VM_BIN="$NODE_ROOT/target/$MODE/nxcc-zenroom-vm"

if [ ! -x "$DAEMON_BIN" ] || [ ! -x "$ENCLAVE_BIN" ] || [ ! -x "$WORKERD_VM_BIN" ] || [ ! -x "$ZENROOM_VM_BIN" ]; then
  echo "Error: required binaries not found. Build failed?" >&2
  exit 1
fi

python3 - "$OPERATOR_KEY_FILE" <<'PY'
import sys
from pathlib import Path

Path(sys.argv[1]).write_bytes(bytes(range(32)))
PY

printf "Starting postback server on %s...\n" "$POSTBACK_PORT"
python3 "$SCRIPT_DIR/postback_server.py" --port "$POSTBACK_PORT" --output "$POSTBACK_BODY" \
  >"$TEST_DIR/postback.log" 2>&1 &
POSTBACK_PID=$!

printf "Starting nxcc-workerd-vm...\n"
"$WORKERD_VM_BIN" --server-mode uds --server-uds-path "$WORKERD_VM_SOCK" --verbose \
  >"$WORKERD_VM_LOG" 2>&1 &
WORKERD_VM_PID=$!

printf "Starting nxcc-zenroom-vm...\n"
NXCC_VM_SERVER_MODE=uds \
NXCC_VM_SERVER_UDS_PATH="$ZENROOM_VM_SOCK" \
NXCC_ZENROOM_VM_ZENCODE_EXEC_PATH="$ZENCODE_EXEC_PATH" \
NXCC_ZENROOM_VM_POSTBACK_ENABLED="true" \
NXCC_ZENROOM_VM_POSTBACK_ALLOWED_HOST_SUFFIXES="127.0.0.1,localhost" \
NXCC_ZENROOM_VM_POSTBACK_ALLOWED_SCHEMES="http" \
NXCC_ZENROOM_VM_POSTBACK_ALLOWED_PORTS="$POSTBACK_PORT" \
NXCC_ZENROOM_VM_POSTBACK_BLOCK_PRIVATE_IPS="false" \
NXCC_ZENROOM_VM_MAX_STDOUT_BYTES="1048576" \
NXCC_ZENROOM_VM_MAX_STDERR_BYTES="1048576" \
NXCC_ZENROOM_VM_MAX_SCRIPT_BYTES="1048576" \
"$ZENROOM_VM_BIN" --server-mode uds --server-uds-path "$ZENROOM_VM_SOCK" --verbose \
  >"$ZENROOM_VM_LOG" 2>&1 &
ZENROOM_VM_PID=$!

printf "Starting nxcc-platform-enclave...\n"
"$ENCLAVE_BIN" --grpc-mode uds --grpc-uds-path "$ENCLAVE_SOCK" --verbose \
  >"$ENCLAVE_LOG" 2>&1 &
ENCLAVE_PID=$!

printf "Starting nxcc-daemon...\n"
NXCC_DAEMON_VM_ATTACHMENTS="nxcc/workerd=$WORKERD_VM_SOCK,nxcc/zenroom=$ZENROOM_VM_SOCK" \
NXCC_DAEMON_P2P_RESPONSE_TIMEOUT_SECS="1" \
NXCC_DAEMON_OPERATOR_SIGNING_KEY_PATH="$OPERATOR_KEY_FILE" \
RUST_LOG="nxcc_daemon=debug,nxcc_vm_base=info,nxcc_zenroom_vm=debug" \
"$DAEMON_BIN" \
  --uds-path "$DAEMON_SOCK" \
  --enclave-uds-path "$ENCLAVE_SOCK" \
  --default-vm-uds-path "$WORKERD_VM_SOCK" \
  --default-vm-id "nxcc/workerd" \
  --identity-path "$TEST_DIR/identity.key" \
  --policy-cache-dir "$TEST_DIR/policy_cache" \
  --listen-addresses "/ip4/127.0.0.1/tcp/9001" \
  --http-listen-addr "127.0.0.1:6922" \
  --verbose \
  >"$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!

printf "Waiting for daemon socket...\n"
for _i in $(seq 1 50); do
  if [ -S "$DAEMON_SOCK" ]; then
    break
  fi
  sleep 0.1
done

if [ ! -S "$DAEMON_SOCK" ]; then
  echo "Error: daemon socket not ready." >&2
  echo "Daemon log:" >&2
  cat "$DAEMON_LOG" >&2 || true
  echo "Enclave log:" >&2
  cat "$ENCLAVE_LOG" >&2 || true
  echo "Workerd VM log:" >&2
  cat "$WORKERD_VM_LOG" >&2 || true
  echo "Zenroom VM log:" >&2
  cat "$ZENROOM_VM_LOG" >&2 || true
  exit 1
fi

DSSE_WORKER_BUNDLE_PAYLOAD_TYPE="application/vnd.nxcc.workerbundlepayload.v1+json"
DSSE_WORK_ORDER_PAYLOAD_TYPE="application/vnd.nxcc.workorderpayload.v1+json"

SCRIPT_B64_FILE="$TEST_DIR/hello_world_b64.txt"
base64 <"$SCRIPT_DIR/hello_world.zen" | tr -d '\n' >"$SCRIPT_B64_FILE"

BUNDLE_PAYLOAD_FILE="$TEST_DIR/zenroom_bundle_payload.json"
jq -n \
  --arg vm "nxcc/zenroom" \
  --rawfile executable_b64 "$SCRIPT_B64_FILE" \
  '{vm: $vm, executable: $executable_b64, metadata: {}}' >"$BUNDLE_PAYLOAD_FILE"

BUNDLE_PAYLOAD_B64_FILE="$TEST_DIR/zenroom_bundle_payload_b64.txt"
base64 <"$BUNDLE_PAYLOAD_FILE" | tr -d '\n' >"$BUNDLE_PAYLOAD_B64_FILE"

MOCK_SIG=$(printf "%s" "mocksig" | base64 | tr -d '\n')

BUNDLE_DSSE_FILE="$TEST_DIR/zenroom_bundle_dsse.json"
jq -n \
  --rawfile payload_b64 "$BUNDLE_PAYLOAD_B64_FILE" \
  --arg payload_type "$DSSE_WORKER_BUNDLE_PAYLOAD_TYPE" \
  --arg sig "$MOCK_SIG" \
  '{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: $sig}]}' \
  >"$BUNDLE_DSSE_FILE"

BUNDLE_DSSE_B64_FILE="$TEST_DIR/zenroom_bundle_dsse_b64.txt"
base64 <"$BUNDLE_DSSE_FILE" | tr -d '\n' >"$BUNDLE_DSSE_B64_FILE"

MANIFEST_FILE="$TEST_DIR/zenroom_worker_manifest.json"
jq --arg bundle_source "data:application/json;base64,$(cat "$BUNDLE_DSSE_B64_FILE")" \
  '.bundle.source = $bundle_source' \
  "$SCRIPT_DIR/worker.manifest.json" >"$MANIFEST_FILE"

WORK_ORDER_PAYLOAD_FILE="$TEST_DIR/zenroom_work_order_payload.json"
jq -n \
  --arg id "zenroom-hello-$(date +%s%N)" \
  --slurpfile worker_manifest "$MANIFEST_FILE" \
  '{id: $id, worker: $worker_manifest[0], events: [{handler: "run", kind: "launch"}]}' \
  >"$WORK_ORDER_PAYLOAD_FILE"

WORK_ORDER_PAYLOAD_B64_FILE="$TEST_DIR/zenroom_work_order_payload_b64.txt"
base64 <"$WORK_ORDER_PAYLOAD_FILE" | tr -d '\n' >"$WORK_ORDER_PAYLOAD_B64_FILE"

WORK_ORDER_DSSE_FILE="$TEST_DIR/zenroom_work_order_dsse.json"
jq -n \
  --rawfile payload_b64 "$WORK_ORDER_PAYLOAD_B64_FILE" \
  --arg payload_type "$DSSE_WORK_ORDER_PAYLOAD_TYPE" \
  --arg sig "$MOCK_SIG" \
  '{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: $sig}]}' \
  >"$WORK_ORDER_DSSE_FILE"

WORK_ORDER_DSSE_B64_FILE="$TEST_DIR/zenroom_work_order_dsse_b64.txt"
base64 <"$WORK_ORDER_DSSE_FILE" | tr -d '\n' >"$WORK_ORDER_DSSE_B64_FILE"

SUBMIT_PAYLOAD_FILE="$TEST_DIR/submit_zenroom_work_order.json"
jq -n \
  --rawfile work_order_dsse_bytes "$WORK_ORDER_DSSE_B64_FILE" \
  '{work_order_dsse_bytes: $work_order_dsse_bytes}' \
  >"$SUBMIT_PAYLOAD_FILE"

printf "Submitting Zenroom work order...\n"
submit_response=$(grpcurl_submit_work_order "$DAEMON_SOCK" "$SUBMIT_PAYLOAD_FILE")
submit_success=$(echo "$submit_response" | jq -r .success)
if [ "$submit_success" != "true" ]; then
  echo "Error: work order submission failed: $submit_response" >&2
  exit 1
fi

printf "Waiting for postback...\n"
for _i in $(seq 1 50); do
  if [ -s "$POSTBACK_BODY" ]; then
    break
  fi
  sleep 0.2
done

if [ ! -s "$POSTBACK_BODY" ]; then
  echo "Error: postback was not received." >&2
  echo "Daemon log:" >&2
  cat "$DAEMON_LOG" >&2 || true
  echo "Zenroom VM log:" >&2
  cat "$ZENROOM_VM_LOG" >&2 || true
  exit 1
fi

POSTBACK_VALUE=$(jq -r '.' "$POSTBACK_BODY")
if [ "$POSTBACK_VALUE" != "Hello_World" ]; then
  echo "Error: unexpected postback payload: $POSTBACK_VALUE" >&2
  exit 1
fi

printf "SUCCESS: Zenroom postback received: %s\n" "$POSTBACK_VALUE"

base64_len() {
  python3 - "$1" <<'PY'
import base64
import sys

value = sys.argv[1]
decoded = base64.b64decode(value, validate=True)
print(len(decoded))
PY
}

run_secret_work_order() {
  _identity_id="$1"
  _output_file="$2"
  _log_file="$3"

  printf "Starting postback server for secret test on %s (identity %s)...\n" "$POSTBACK_PORT" "$_identity_id" >&2
  sleep 0.2
  python3 "$SCRIPT_DIR/postback_server.py" --port "$POSTBACK_PORT" --output "$_output_file" \
    >"$_log_file" 2>&1 &
  POSTBACK_PID=$!

  _manifest_file="$TEST_DIR/zenroom_worker_secret_manifest_${_identity_id}.json"
  jq --arg bundle_source "data:application/json;base64,$(cat "$SECRET_BUNDLE_DSSE_B64_FILE")" \
    --arg identity_id "$_identity_id" \
    '.bundle.source = $bundle_source | .identities[0][0].identity_id = $identity_id' \
    "$SCRIPT_DIR/worker.secret.manifest.json" >"$_manifest_file"

  _payload_file="$TEST_DIR/zenroom_secret_work_order_payload_${_identity_id}.json"
  jq -n \
    --arg id "zenroom-secret-${_identity_id}-$(date +%s%N)" \
    --slurpfile worker_manifest "$_manifest_file" \
    '{id: $id, worker: $worker_manifest[0], events: [{handler: "run", kind: "launch"}]}' \
    >"$_payload_file"

  _payload_b64_file="${_payload_file%.json}_b64.txt"
  base64 <"$_payload_file" | tr -d '\n' >"$_payload_b64_file"

  _dsse_file="${_payload_file%.json}_dsse.json"
  jq -n \
    --rawfile payload_b64 "$_payload_b64_file" \
    --arg payload_type "$DSSE_WORK_ORDER_PAYLOAD_TYPE" \
    --arg sig "$MOCK_SIG" \
    '{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: $sig}]}' \
    >"$_dsse_file"

  _dsse_b64_file="${_payload_file%.json}_dsse_b64.txt"
  base64 <"$_dsse_file" | tr -d '\n' >"$_dsse_b64_file"

  _submit_file="${_payload_file%.json}_submit.json"
  jq -n \
    --rawfile work_order_dsse_bytes "$_dsse_b64_file" \
    '{work_order_dsse_bytes: $work_order_dsse_bytes}' \
    >"$_submit_file"

  printf "Submitting Zenroom secret work order (identity %s)...\n" "$_identity_id" >&2
  _submit_response=$(grpcurl_submit_work_order "$DAEMON_SOCK" "$_submit_file")
  _submit_success=$(echo "$_submit_response" | jq -r .success)
  if [ "$_submit_success" != "true" ]; then
    echo "Error: secret work order submission failed: $_submit_response" >&2
    exit 1
  fi

  printf "Waiting for secret postback (identity %s)...\n" "$_identity_id" >&2
  for _i in $(seq 1 50); do
    if [ -s "$_output_file" ]; then
      break
    fi
    sleep 0.2
  done

  if [ ! -s "$_output_file" ]; then
    echo "Error: secret postback was not received." >&2
    echo "Daemon log:" >&2
    cat "$DAEMON_LOG" >&2 || true
    echo "Zenroom VM log:" >&2
    cat "$ZENROOM_VM_LOG" >&2 || true
    exit 1
  fi

  _secret_value=$(jq -r '.' "$_output_file")
  if [ -z "$_secret_value" ] || [ "$_secret_value" = "null" ]; then
    echo "Error: secret postback payload was empty" >&2
    exit 1
  fi

  _secret_byte_len=$(base64_len "$_secret_value")
  if [ "$_secret_byte_len" -ne 16 ]; then
    echo "Error: expected derived secret length 16, got $_secret_byte_len" >&2
    exit 1
  fi

  printf "%s" "$_secret_value"
}

SECRET_SCRIPT_B64_FILE="$TEST_DIR/secret_injection_b64.txt"
base64 <"$SCRIPT_DIR/secret_injection.zen" | tr -d '\n' >"$SECRET_SCRIPT_B64_FILE"

SECRET_BUNDLE_PAYLOAD_FILE="$TEST_DIR/zenroom_secret_bundle_payload.json"
jq -n \
  --arg vm "nxcc/zenroom" \
  --rawfile executable_b64 "$SECRET_SCRIPT_B64_FILE" \
  '{vm: $vm, executable: $executable_b64, metadata: {}}' >"$SECRET_BUNDLE_PAYLOAD_FILE"

SECRET_BUNDLE_PAYLOAD_B64_FILE="$TEST_DIR/zenroom_secret_bundle_payload_b64.txt"
base64 <"$SECRET_BUNDLE_PAYLOAD_FILE" | tr -d '\n' >"$SECRET_BUNDLE_PAYLOAD_B64_FILE"

SECRET_BUNDLE_DSSE_FILE="$TEST_DIR/zenroom_secret_bundle_dsse.json"
jq -n \
  --rawfile payload_b64 "$SECRET_BUNDLE_PAYLOAD_B64_FILE" \
  --arg payload_type "$DSSE_WORKER_BUNDLE_PAYLOAD_TYPE" \
  --arg sig "$MOCK_SIG" \
  '{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: $sig}]}' \
  >"$SECRET_BUNDLE_DSSE_FILE"

SECRET_BUNDLE_DSSE_B64_FILE="$TEST_DIR/zenroom_secret_bundle_dsse_b64.txt"
base64 <"$SECRET_BUNDLE_DSSE_FILE" | tr -d '\n' >"$SECRET_BUNDLE_DSSE_B64_FILE"

SECRET_OUTPUT_1=$(run_secret_work_order "777" "$POSTBACK_BODY_SECRET_1" "$TEST_DIR/postback_secret_1.log")
SECRET_OUTPUT_2=$(run_secret_work_order "777" "$POSTBACK_BODY_SECRET_2" "$TEST_DIR/postback_secret_2.log")
if [ "$SECRET_OUTPUT_1" != "$SECRET_OUTPUT_2" ]; then
  echo "Error: derived secret for identity 777 was not stable across runs" >&2
  exit 1
fi

SECRET_OUTPUT_3=$(run_secret_work_order "778" "$POSTBACK_BODY_SECRET_3" "$TEST_DIR/postback_secret_3.log")
if [ "$SECRET_OUTPUT_1" = "$SECRET_OUTPUT_3" ]; then
  echo "Error: derived secrets for identities 777 and 778 unexpectedly matched" >&2
  exit 1
fi

printf "SUCCESS: Zenroom secret injection verified (stable + identity-dependent)\n"
