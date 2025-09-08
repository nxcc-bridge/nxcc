#!/bin/sh

# Docker Configuration Test for NXCC Daemon
# Tests configuration loading in Docker containers using the --dump-config flag

set -e

# Test configuration
TEST_DIR="/tmp/nxcc-docker-config-test-$$"
DOCKER_IMAGE="nxcc-node:debug"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log() {
	echo "${GREEN}[TEST]${NC} $*"
}

warn() {
	echo "${YELLOW}[WARN]${NC} $*"
}

error() {
	echo "${RED}[ERROR]${NC} $*"
}

# Cleanup function
cleanup() {
	if [ -d "$TEST_DIR" ]; then
		rm -rf "$TEST_DIR"
	fi
}

# Setup test directory
setup() {
	log "Setting up test directory: $TEST_DIR"
	mkdir -p "$TEST_DIR"
}

# Check if jq is available
check_jq() {
	if ! command -v jq >/dev/null 2>&1; then
		error "jq is required for JSON parsing but not found"
		error "Please install jq: brew install jq (on macOS) or apt-get install jq (on Ubuntu)"
		exit 1
	fi
}

# Check if Docker image exists
check_docker_image() {
	if ! docker image inspect "$DOCKER_IMAGE" >/dev/null 2>&1; then
		error "Docker image '$DOCKER_IMAGE' not found"
		error "Please build it first with: ./infra/infra.sh image build --debug"
		exit 1
	fi
	log "Using Docker image: $DOCKER_IMAGE"
}

# Extract JSON value using jq
extract_json_value() {
	json_file="$1"
	key_path="$2"

	# Use jq to extract the value - dots already work in jq
	jq -r ".$key_path" "$json_file" 2>/dev/null || echo "null"
}

# Extract array values from JSON (for network config arrays)
extract_json_array() {
	json_file="$1"
	key_path="$2"

	case "$key_path" in
	network.listen_addresses)
		jq -r '.network.listen_addresses | join(",")' "$json_file" 2>/dev/null || echo ""
		;;
	network.bootstrap_peers)
		jq -r '.network.bootstrap_peers | join(",")' "$json_file" 2>/dev/null || echo ""
		;;
	*)
		echo ""
		;;
	esac
}

# Run Docker container with config dump and extract JSON
run_docker_with_config() {
	output_file="$1"
	shift

	# Run docker command and capture output (specify platform to avoid warnings)
	temp_output="/tmp/docker_output_$$"
	if ! timeout 30s docker run --rm --platform linux/amd64 "$@" \
		"$DOCKER_IMAGE" >"$temp_output" 2>&1; then
		error "Failed to run Docker container"
		error "Docker container output was:"
		cat "$temp_output" >&2
		rm -f "$temp_output"
		return 1
	fi

	# Extract JSON from output (should be a single line when dump-config is used)
	if ! grep '^{.*}$' "$temp_output" >"$output_file"; then
		error "No JSON output found in Docker container output"
		error "Docker container output was:"
		cat "$temp_output" >&2
		rm -f "$temp_output"
		return 1
	fi

	rm -f "$temp_output"

	if [ ! -s "$output_file" ]; then
		error "No JSON output captured in $output_file"
		return 1
	fi

	return 0
}

# Validate configuration values
validate_config() {
	config_file="$1"
	expected_values="$2"
	test_name="$3"

	if [ ! -f "$config_file" ] || [ ! -s "$config_file" ]; then
		error "Configuration file $config_file not found or empty"
		return 1
	fi

	log "Validating configuration for $test_name..."

	validation_passed=0
	total_checks=0

	# Use a temp file to avoid subshell issues with while loop
	temp_file=$(mktemp)
	printf '%s\n' "$expected_values" >"$temp_file"

	while IFS='=' read -r key expected_value; do
		if [ -z "$key" ] || [ -z "$expected_value" ]; then
			continue
		fi

		total_checks=$((total_checks + 1))
		actual_value=$(extract_json_value "$config_file" "$key")
		expected_clean=$(echo "$expected_value" | tr -d ' ')

		if [ "$actual_value" = "$expected_clean" ]; then
			log "✓ $key: $actual_value"
			validation_passed=$((validation_passed + 1))
		else
			error "✗ $key: got '$actual_value', expected '$expected_clean'"
		fi
	done <"$temp_file"

	rm -f "$temp_file"

	if [ "$validation_passed" -eq "$total_checks" ] && [ "$total_checks" -gt 0 ]; then
		log "✅ All $validation_passed configuration values validated for $test_name"
		return 0
	else
		error "❌ Configuration validation failed for $test_name ($validation_passed/$total_checks passed)"
		return 1
	fi
}

# Test 1: Basic environment variable configuration in Docker
test_docker_basic_env() {
	log "Test 1: Basic Docker environment variable configuration"

	cd "$TEST_DIR"

	# Test with environment variables using NXCC_DAEMON_ prefix
	if ! run_docker_with_config basic_output.json \
		-e NXCC_DAEMON_VERBOSE=true \
		-e NXCC_DAEMON_MODE=tcp \
		-e NXCC_DAEMON_TCP_ADDR="0.0.0.0:50051" \
		-e NXCC_DAEMON_HTTP_LISTEN_ADDR="0.0.0.0:6922" \
		-e NXCC_DAEMON_API_ENABLED=true \
		-e NXCC_DAEMON_ENCLAVE_UDS_PATH="/run/nxcc/enclave.sock" \
		-e NXCC_DAEMON_TDX_ENABLED=false \
		-e NXCC_DAEMON_GCS_PROJECT_ID="env-project-123" \
		-e NXCC_DAEMON_MAX_BLOCK_AGE=600 \
		-e NXCC_DAEMON_DUMP_CONFIG=true; then
		return 1
	fi

	log "✓ Config dumped successfully from Docker container"

	# Define expected values from our environment variables
	expected_values="verbose=true
grpc.mode=tcp
grpc.tcp_addr=0.0.0.0:50051
http.http_listen_addr=0.0.0.0:6922
http.api_enabled=true
enclave.enclave_uds_path=/run/nxcc/enclave.sock
attestation.tdx_enabled=false
attestation.gcs_project_id=env-project-123
attestation.max_block_age=600"

	# Validate the configuration
	validate_config basic_output.json "$expected_values" "basic environment test"
}

# Test 2: Array environment variables in Docker
test_docker_array_env_vars() {
	log "Test 2: Docker array environment variable configuration"

	cd "$TEST_DIR"

	# Test with environment variables including arrays using comma-separated values
	if ! run_docker_with_config array_output.json \
		-e NXCC_DAEMON_VERBOSE=true \
		-e NXCC_DAEMON_API_ENABLED=true \
		-e NXCC_DAEMON_API_CORS_ALLOWED_ORIGINS="http://env.test,http://another.test" \
		-e NXCC_DAEMON_GCS_PROJECT_ID="docker-env-project-456" \
		-e NXCC_DAEMON_TDX_ENABLED=false \
		-e NXCC_DAEMON_MAX_BLOCK_AGE=1200 \
		-e NXCC_DAEMON_FRESHNESS_CHAIN_IDS="1,56,137" \
		-e NXCC_DAEMON_DUMP_CONFIG=true; then
		return 1
	fi

	log "✓ Config with array environment variables dumped successfully"

	# Define expected values from our environment variables
	expected_values="verbose=true
http.api_enabled=true
attestation.gcs_project_id=docker-env-project-456
attestation.tdx_enabled=false
attestation.max_block_age=1200"

	validate_config array_output.json "$expected_values" "array environment variables test"
}

# Test 3: CLI arguments in Docker
test_docker_cli_args() {
	log "Test 3: Docker CLI argument configuration"

	cd "$TEST_DIR"

	if ! run_docker_with_config cli_output.json \
		-e NXCC_DAEMON_DUMP_CONFIG=true \
		-e NXCC_DAEMON_EXTRA_ARGS="--verbose --tcp-addr 0.0.0.0:52051 --http-listen-addr 0.0.0.0:7922 --min-schedule-interval-ms 25 --gcs-project-id docker-cli-project-789"; then
		return 1
	fi

	log "✓ Config with CLI arguments dumped successfully"

	# Define expected values from our CLI arguments
	expected_values="verbose=true
grpc.tcp_addr=0.0.0.0:52051
http.http_listen_addr=0.0.0.0:7922
scheduler.min_schedule_interval_ms=25
attestation.gcs_project_id=docker-cli-project-789"

	validate_config cli_output.json "$expected_values" "CLI arguments test"
}

# Test 4: Mixed configuration sources in Docker - demonstrates priority order
test_docker_mixed_config() {
	log "Test 4: Docker mixed configuration sources (env + CLI) - priority test"

	cd "$TEST_DIR"

	# Use env vars + CLI args to test priority (CLI > env)
	if ! run_docker_with_config mixed_output.json \
		-e NXCC_DAEMON_VERBOSE=false \
		-e NXCC_DAEMON_TCP_ADDR="0.0.0.0:50000" \
		-e NXCC_DAEMON_GCS_PROJECT_ID="env-override-project" \
		-e NXCC_DAEMON_MAX_BLOCK_AGE=900 \
		-e NXCC_DAEMON_TDX_ENABLED=true \
		-e NXCC_DAEMON_DUMP_CONFIG=true \
		-e NXCC_DAEMON_EXTRA_ARGS="--verbose --gcs-project-id cli-final-project --tcp-addr 0.0.0.0:53051"; then
		return 1
	fi

	log "✓ Mixed config dumped successfully"

	# Expected values should show CLI > env priority
	expected_values="verbose=true
grpc.tcp_addr=0.0.0.0:53051
attestation.gcs_project_id=cli-final-project
attestation.max_block_age=900
attestation.tdx_enabled=true"

	if validate_config mixed_output.json "$expected_values" "mixed sources priority test"; then
		log "Configuration priority validated: CLI args override env vars"
		return 0
	else
		error "Mixed configuration priority test failed"
		return 1
	fi
}

# Test 5: Network configuration via environment variables
test_network_env_config() {
	log "Test 5: Network configuration via environment variables"

	cd "$TEST_DIR"

	# Test with network environment variables using comma-separated format
	if ! run_docker_with_config network_env_output.json \
		-e NXCC_DAEMON_LISTEN_ADDRESSES="/ip4/127.0.0.1/tcp/19000,/ip4/127.0.0.1/tcp/19001" \
		-e NXCC_DAEMON_BOOTSTRAP_PEERS="/ip4/10.0.0.1/tcp/9000,/ip4/10.0.0.2/tcp/9000" \
		-e NXCC_DAEMON_DUMP_CONFIG=true; then
		return 1
	fi

	# Validate network configuration
	listen_addrs=$(extract_json_array network_env_output.json "network.listen_addresses")
	bootstrap_peers=$(extract_json_array network_env_output.json "network.bootstrap_peers")

	log "Found listen_addresses: '$listen_addrs'"
	log "Found bootstrap_peers: '$bootstrap_peers'"

	# Check if our env vars were applied
	if echo "$listen_addrs" | grep -q "/ip4/127.0.0.1/tcp/19000" &&
		echo "$listen_addrs" | grep -q "/ip4/127.0.0.1/tcp/19001"; then
		log "✓ listen_addresses loaded correctly from environment variables"
	else
		error "✗ listen_addresses not loaded correctly from environment variables"
		error "Expected: /ip4/127.0.0.1/tcp/19000,/ip4/127.0.0.1/tcp/19001"
		error "Got: $listen_addrs"
		return 1
	fi

	if echo "$bootstrap_peers" | grep -q "/ip4/10.0.0.1/tcp/9000" &&
		echo "$bootstrap_peers" | grep -q "/ip4/10.0.0.2/tcp/9000"; then
		log "✓ bootstrap_peers loaded correctly from environment variables"
	else
		error "✗ bootstrap_peers not loaded correctly from environment variables"
		error "Expected: /ip4/10.0.0.1/tcp/9000,/ip4/10.0.0.2/tcp/9000"
		error "Got: $bootstrap_peers"
		return 1
	fi

	log "Environment variable network configuration test passed"
	return 0
}

# Test 6: Alternative network configuration format
test_network_alt_config() {
	log "Test 6: Alternative network configuration format"

	cd "$TEST_DIR"

	# Test with different network addresses
	if ! run_docker_with_config network_alt_output.json \
		-e NXCC_DAEMON_LISTEN_ADDRESSES="/ip4/0.0.0.0/tcp/20000,/ip4/0.0.0.0/tcp/20001" \
		-e NXCC_DAEMON_BOOTSTRAP_PEERS="/ip4/192.168.1.10/tcp/9000,/ip4/192.168.1.11/tcp/9000" \
		-e NXCC_DAEMON_DUMP_CONFIG=true; then
		return 1
	fi

	# Validate network environment variable configuration
	listen_addrs=$(extract_json_array network_alt_output.json "network.listen_addresses")
	bootstrap_peers=$(extract_json_array network_alt_output.json "network.bootstrap_peers")

	log "Found listen_addresses: '$listen_addrs'"
	log "Found bootstrap_peers: '$bootstrap_peers'"

	# Check if our env vars were applied
	if echo "$listen_addrs" | grep -q "/ip4/0.0.0.0/tcp/20000" &&
		echo "$listen_addrs" | grep -q "/ip4/0.0.0.0/tcp/20001"; then
		log "✓ listen_addresses loaded correctly from environment variables"
	else
		error "✗ listen_addresses not loaded correctly from environment variables"
		error "Expected: /ip4/0.0.0.0/tcp/20000,/ip4/0.0.0.0/tcp/20001"
		error "Got: $listen_addrs"
		return 1
	fi

	if echo "$bootstrap_peers" | grep -q "/ip4/192.168.1.10/tcp/9000" &&
		echo "$bootstrap_peers" | grep -q "/ip4/192.168.1.11/tcp/9000"; then
		log "✓ bootstrap_peers loaded correctly from environment variables"
	else
		error "✗ bootstrap_peers not loaded correctly from environment variables"
		error "Expected: /ip4/192.168.1.10/tcp/9000,/ip4/192.168.1.11/tcp/9000"
		error "Got: $bootstrap_peers"
		return 1
	fi

	log "Alternative network configuration test passed"
	return 0
}

# Test 7: Network configuration via CLI arguments
test_network_cli_args() {
	log "Test 7: Network configuration via CLI arguments"

	cd "$TEST_DIR"

	# Test with CLI network arguments
	if ! run_docker_with_config network_cli_output.json \
		-e NXCC_DAEMON_DUMP_CONFIG=true \
		-e NXCC_DAEMON_EXTRA_ARGS="--listen-addresses /ip4/0.0.0.0/tcp/30000,/ip4/0.0.0.0/tcp/30001 --bootstrap-peers /ip4/172.16.1.100/tcp/9000,/ip4/172.16.1.101/tcp/9000"; then
		return 1
	fi

	# Validate CLI network configuration
	listen_addrs=$(extract_json_array network_cli_output.json "network.listen_addresses")
	bootstrap_peers=$(extract_json_array network_cli_output.json "network.bootstrap_peers")

	log "Found listen_addresses: '$listen_addrs'"
	log "Found bootstrap_peers: '$bootstrap_peers'"

	# Check if our CLI args were applied
	if echo "$listen_addrs" | grep -q "/ip4/0.0.0.0/tcp/30000" &&
		echo "$listen_addrs" | grep -q "/ip4/0.0.0.0/tcp/30001"; then
		log "✓ listen_addresses loaded correctly from CLI arguments"
	else
		error "✗ listen_addresses not loaded correctly from CLI arguments"
		error "Expected: /ip4/0.0.0.0/tcp/30000,/ip4/0.0.0.0/tcp/30001"
		error "Got: $listen_addrs"
		return 1
	fi

	if echo "$bootstrap_peers" | grep -q "/ip4/172.16.1.100/tcp/9000" &&
		echo "$bootstrap_peers" | grep -q "/ip4/172.16.1.101/tcp/9000"; then
		log "✓ bootstrap_peers loaded correctly from CLI arguments"
	else
		error "✗ bootstrap_peers not loaded correctly from CLI arguments"
		error "Expected: /ip4/172.16.1.100/tcp/9000,/ip4/172.16.1.101/tcp/9000"
		error "Got: $bootstrap_peers"
		return 1
	fi

	log "CLI arguments network configuration test passed"
	return 0
}

# Test 8: Network configuration priority (CLI > env)
test_network_priority() {
	log "Test 8: Network configuration priority (CLI > env)"

	cd "$TEST_DIR"

	# Test priority: env < CLI
	if ! run_docker_with_config network_priority_output.json \
		-e NXCC_DAEMON_LISTEN_ADDRESSES="/ip4/0.0.0.0/tcp/9000" \
		-e NXCC_DAEMON_BOOTSTRAP_PEERS="/ip4/0.0.0.0/tcp/9001" \
		-e NXCC_DAEMON_DUMP_CONFIG=true \
		-e NXCC_DAEMON_EXTRA_ARGS="--listen-addresses /ip4/0.0.0.0/tcp/10000 --bootstrap-peers /ip4/0.0.0.0/tcp/10001"; then
		return 1
	fi

	# Validate priority - CLI should win
	listen_addrs=$(extract_json_array network_priority_output.json "network.listen_addresses")
	bootstrap_peers=$(extract_json_array network_priority_output.json "network.bootstrap_peers")

	log "Found listen_addresses: '$listen_addrs'"
	log "Found bootstrap_peers: '$bootstrap_peers'"

	# CLI args should override env vars
	if echo "$listen_addrs" | grep -q "/ip4/0.0.0.0/tcp/10000"; then
		log "✓ CLI listen_addresses correctly override env vars"
	else
		error "✗ CLI arguments did not override env vars"
		error "Expected CLI value: /ip4/0.0.0.0/tcp/10000"
		error "Got: $listen_addrs"
		return 1
	fi

	if echo "$bootstrap_peers" | grep -q "/ip4/0.0.0.0/tcp/10001"; then
		log "✓ CLI bootstrap_peers correctly override env vars"
	else
		error "✗ CLI arguments did not override env vars"
		error "Expected CLI value: /ip4/0.0.0.0/tcp/10001"
		error "Got: $bootstrap_peers"
		return 1
	fi

	log "Network configuration priority test passed"
	return 0
}

# Test 9: Docker entrypoint environment variable handling
test_docker_entrypoint_env() {
	log "Test 9: Docker entrypoint environment variable handling"

	cd "$TEST_DIR"

	# Test the environment variables that entrypoint.sh recognizes
	# The entrypoint should pass these through as CLI args
	if ! timeout 30s docker run --rm \
		-e NXCC_DAEMON_VERBOSE=true \
		-e NXCC_DAEMON_EXTRA_ARGS="--gcs-project-id entrypoint-test-project" \
		-e NXCC_DAEMON_DUMP_CONFIG=true \
		"$DOCKER_IMAGE" >entrypoint_output.json 2>&1; then
		error "Failed to run container with entrypoint environment variables"
		return 1
	fi

	# Extract JSON from the output (may have other logs mixed in)
	grep '^{.*}$' entrypoint_output.json >entrypoint_config.json 2>/dev/null || {
		error "No JSON output found in entrypoint test"
		return 1
	}

	# Check if we got JSON output (entrypoint should have passed --dump-config)
	if grep -q "gcs_project_id" entrypoint_config.json 2>/dev/null; then
		log "✓ Entrypoint environment variables processed successfully"

		# Validate the entrypoint passed our values
		actual_project_id="$(extract_json_value entrypoint_config.json "attestation.gcs_project_id")"
		if [ "$actual_project_id" = "entrypoint-test-project" ]; then
			log "✓ Entrypoint correctly passed extra arguments (gcs_project_id: $actual_project_id)"
			return 0
		else
			error "Entrypoint did not pass expected arguments"
			error "Expected gcs_project_id: entrypoint-test-project, got: $actual_project_id"
			return 1
		fi
	else
		error "Failed to get JSON config from entrypoint test"
		cat entrypoint_output.json
		return 1
	fi
}

# Main test runner
main() {
	log "Starting NXCC Docker Configuration Validation Tests"

	# Trap for cleanup
	trap cleanup EXIT

	setup
	check_jq
	check_docker_image

	tests_passed=0
	total_tests=9

	# Run basic configuration tests
	if test_docker_basic_env; then
		tests_passed=$((tests_passed + 1))
	fi

	if test_docker_array_env_vars; then
		tests_passed=$((tests_passed + 1))
	fi

	if test_docker_cli_args; then
		tests_passed=$((tests_passed + 1))
	fi

	if test_docker_mixed_config; then
		tests_passed=$((tests_passed + 1))
	fi

	# Run network-specific configuration tests
	if test_network_env_config; then
		tests_passed=$((tests_passed + 1))
	fi

	if test_network_alt_config; then
		tests_passed=$((tests_passed + 1))
	fi

	if test_network_cli_args; then
		tests_passed=$((tests_passed + 1))
	fi

	if test_network_priority; then
		tests_passed=$((tests_passed + 1))
	fi

	if test_docker_entrypoint_env; then
		tests_passed=$((tests_passed + 1))
	fi

	# Final results
	echo
	if [ $tests_passed -eq $total_tests ]; then
		log "✅ ALL DOCKER CONFIGURATION TESTS PASSED! ($tests_passed/$total_tests)"
	else
		error "❌ Some Docker tests failed ($tests_passed/$total_tests passed)"
		exit 1
	fi
}

# Show usage if help requested
if [ "$1" = "--help" ] || [ "$1" = "-h" ]; then
	echo "Usage: $0"
	echo
	echo "Tests NXCC daemon configuration loading in Docker containers using --dump-config:"
	echo "  - Environment variables (NXCC_DAEMON_ prefix)"
	echo "  - CLI arguments"
	echo "  - Mixed configuration scenarios with priority testing"
	echo "  - Network config arrays (listen-addresses, bootstrap-peers)"
	echo "  - Array configuration via comma-separated values"
	echo "  - Entrypoint script environment variables"
	echo
	echo "Requires:"
	echo "  - Docker daemon running"
	echo "  - nxcc-node:debug image with updated daemon binary"
	echo "  - Rebuild image after code changes: ./infra/infra.sh image build --debug"
	exit 0
fi

main "$@"
