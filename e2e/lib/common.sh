#!/bin/bash
#
# Common functions and utilities for E2E tests
#

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Global configuration
E2E_VERBOSE="${E2E_VERBOSE:-false}"
E2E_TEST_TEXT="${E2E_TEST_TEXT:-Hello from NXCC E2E Test!}"

# Logging functions
log() {
	echo -e "${BLUE}[E2E]${NC} $*"
}

success() {
	echo -e "${GREEN}[E2E SUCCESS]${NC} $*"
}

warn() {
	echo -e "${YELLOW}[E2E WARN]${NC} $*"
}

error() {
	echo -e "${RED}[E2E ERROR]${NC} $*" >&2
	exit 1
}

verbose_log() {
	if [[ "$E2E_VERBOSE" == "true" ]]; then
		echo -e "${BLUE}[E2E VERBOSE]${NC} $*"
	fi
}

# Check if a command exists
command_exists() {
	local cmd="$1"
	local verbose_check="${2:-false}"

	if [[ "$verbose_check" == "true" ]]; then
		verbose_log "Checking if command '$cmd' exists..."
		if command -v "$cmd"; then
			verbose_log "✓ Command '$cmd' found at: $(command -v "$cmd")"
			return 0
		else
			verbose_log "✗ Command '$cmd' not found"
			return 1
		fi
	else
		command -v "$cmd" &>/dev/null
	fi
}

# Check dependencies for terraform-based deployments
check_dependencies() {
	log "Checking dependencies..."

	local missing_deps=()
	local required_deps=("curl" "jq" "node" "pnpm")

	# Check for required tools
	for dep in "${required_deps[@]}"; do
		if ! command_exists "$dep"; then
			missing_deps+=("$dep")
		fi
	done

	if [[ ${#missing_deps[@]} -gt 0 ]]; then
		error "Missing required dependencies: ${missing_deps[*]}"
	fi

	success "All dependencies are available"
}

# Check and build NXCC CLI if needed
ensure_nxcc_cli() {
	local project_root="$1"

	if ! command_exists nxcc; then
		log "NXCC CLI not found, attempting to build from source..."

		# First, ensure the SDK lib is built since the CLI depends on it
		verbose_log "Building SDK lib dependency..."
		if ! cd "$project_root/sdk/lib"; then
			error "Failed to change to SDK lib directory: $project_root/sdk/lib"
		fi

		# Run pnpm install and build for SDK lib
		verbose_log "Running pnpm install in SDK lib..."
		local pnpm_install_output
		if ! pnpm_install_output=$(pnpm install 2>&1); then
			error "pnpm install failed in SDK lib:\n$pnpm_install_output"
		fi

		verbose_log "Building SDK lib..."
		local pnpm_build_output
		if ! pnpm_build_output=$(pnpm run build 2>&1); then
			error "pnpm run build failed in SDK lib:\n$pnpm_build_output"
		fi
		verbose_log "SDK lib built successfully"

		# Change to CLI directory
		verbose_log "Changing to CLI directory: $project_root/sdk/cli"
		if ! cd "$project_root/sdk/cli"; then
			error "Failed to change to CLI directory: $project_root/sdk/cli"
		fi

		# Run pnpm install with error handling
		verbose_log "Running pnpm install..."
		local pnpm_install_output
		if ! pnpm_install_output=$(pnpm install 2>&1); then
			error "pnpm install failed:\n$pnpm_install_output"
		fi
		verbose_log "pnpm install completed successfully"

		# Run pnpm build with error handling
		verbose_log "Running pnpm run build..."
		local pnpm_build_output
		if ! pnpm_build_output=$(pnpm run build 2>&1); then
			error "pnpm run build failed:\n$pnpm_build_output"
		fi
		verbose_log "pnpm run build completed successfully"

		# Check if dist/index.js was created
		local dist_dir="$project_root/sdk/cli/dist"
		local index_js="$dist_dir/index.js"
		if [[ ! -f "$index_js" ]]; then
			error "Build completed but $index_js was not created. Build output:\n$pnpm_build_output"
		fi
		verbose_log "Build output file exists: $index_js"

		# Create symlink to make nxcc command available
		verbose_log "Creating symlink for nxcc command..."
		if [[ ! -f "$dist_dir/nxcc" ]]; then
			if ! ln -sf "$index_js" "$dist_dir/nxcc"; then
				error "Failed to create symlink from $dist_dir/nxcc to $index_js"
			fi
		fi
		verbose_log "Symlink created: $dist_dir/nxcc -> $index_js"

		# Add to PATH
		verbose_log "Adding $dist_dir to PATH"
		export PATH="$dist_dir:$PATH"

		# Return to project root
		cd "$project_root" || error "Failed to change to project root directory"

		# Final verification
		verbose_log "Verifying nxcc command is available..."
		if ! command_exists nxcc "true"; then
			local which_output
			which_output=$(which nxcc 2>&1 || echo "Command not found")
			local path_content="PATH=$PATH"
			local ls_dist
			ls_dist=$(ls -la "$dist_dir" 2>&1 || echo "Directory listing failed")
			error "nxcc command still not available after build.\nDetails:\n- which nxcc: $which_output\n- $path_content\n- dist directory contents:\n$ls_dist"
		fi

		local nxcc_location
		nxcc_location=$(which nxcc)
		success "NXCC CLI built successfully and available at: $nxcc_location"
	else
		verbose_log "NXCC CLI found at: $(which nxcc)"

		# Check if the logs command is available
		if ! nxcc worker logs --help &>/dev/null; then
			log "NXCC CLI logs command not available, rebuilding CLI..."

			# Change to CLI directory
			verbose_log "Changing to CLI directory: $project_root/sdk/cli"
			if ! cd "$project_root/sdk/cli"; then
				error "Failed to change to CLI directory: $project_root/sdk/cli"
			fi

			# Run pnpm build to get latest features
			verbose_log "Running pnpm run build..."
			local pnpm_build_output
			if ! pnpm_build_output=$(pnpm run build 2>&1); then
				error "pnpm run build failed:\n$pnpm_build_output"
			fi
			verbose_log "pnpm run build completed successfully"

			# Update PATH to use local version
			local dist_dir="$project_root/sdk/cli/dist"
			verbose_log "Adding $dist_dir to PATH"
			export PATH="$dist_dir:$PATH"

			# Return to project root
			cd "$project_root" || error "Failed to change to project root directory"

			success "NXCC CLI rebuilt with latest features"
		fi
	fi
}

# Test HTTP endpoint with retries
test_http_endpoint() {
	local url="$1"
	local expected_pattern="$2"
	local method="${3:-GET}"
	local data="$4"
	local retries="${5:-3}"
	local delay="${6:-2}"

	for i in $(seq 1 "$retries"); do
		verbose_log "Testing $method $url (attempt $i/$retries)..."

		local response
		local curl_output
		if [[ "$method" == "POST" && -n "$data" ]]; then
			if ! curl_output=$(curl -s -f -X POST -H "Content-Type: application/json" -d "$data" "$url" 2>&1); then
				response="FAILED"
				verbose_log "curl failed: $curl_output"
			else
				response="$curl_output"
			fi
		else
			if ! curl_output=$(curl -s -f "$url" 2>&1); then
				response="FAILED"
				verbose_log "curl failed: $curl_output"
			else
				response="$curl_output"
			fi
		fi

		if [[ "$response" == "FAILED" ]]; then
			warn "$method $url failed (attempt $i/$retries)"
			if [[ $i -lt $retries ]]; then
				sleep "$delay"
				continue
			fi
			return 1
		fi

		verbose_log "Response: $response"

		if [[ -n "$expected_pattern" ]]; then
			if echo "$response" | grep -q "$expected_pattern"; then
				success "✓ $method $url returned expected pattern: $expected_pattern"
				return 0
			else
				warn "$method $url response did not match expected pattern: $expected_pattern"
				if [[ $i -lt $retries ]]; then
					sleep "$delay"
					continue
				fi
				return 1
			fi
		else
			success "✓ $method $url succeeded"
			return 0
		fi
	done

	return 1
}

# Quick test HTTP endpoint with proper environment routing
quick_http_test() {
	local env="$1"
	local endpoint="$2"
	local expected_pattern="${3:-}"
	local method="${4:-GET}"
	local data="${5:-}"

	# Get worker URL directly from terraform outputs
	local worker_url
	worker_url=$(get_primary_worker_url "$env")

	if [[ -z "$worker_url" ]]; then
		error "No worker URL found for environment: $env"
	fi

	# Use direct HTTP calls to worker endpoints
	local url="$worker_url$endpoint"
	verbose_log "Testing direct HTTP endpoint: $url"
	test_http_endpoint "$url" "$expected_pattern" "$method" "$data"
}

# Quick worker deployment test
quick_worker_deploy() {
	local env="$1"
	local project_dir="$2"
	local manifest_file="${3:-workers/manifest.template.json}"

	cd "$project_dir" || error "Failed to change to project directory"

	# Get worker URL directly from terraform outputs
	local worker_url
	worker_url=$(get_primary_worker_url "$env")

	if [[ -z "$worker_url" ]]; then
		error "No worker URL found for environment: $env"
	fi

	# Use direct RPC calls to worker endpoints
	verbose_log "Using direct RPC endpoint: $worker_url"
	nxcc worker deploy "$manifest_file" --rpc-url "$worker_url" --bundle
}

# Get worker endpoint URLs from terraform outputs
get_worker_endpoints() {
	local env="$1"
	local project_root="${2:-$E2E_PROJECT_ROOT}"

	cd "$project_root" || error "Failed to change to project root"

	# Get worker endpoints from terraform output
	./infra/infra.sh deploy status "$env" >/dev/null 2>&1 || return 1

	# Navigate to the terraform environment directory to run tofu commands
	local env_dir="infra/environments"
	local target_dir=""

	if [[ "$env" == "staging" ]]; then
		target_dir="$env_dir/staging"
	elif [[ "$env" == "production" ]]; then
		target_dir="$env_dir/production"
	elif [[ "$env" =~ ^dev-.+ ]]; then
		target_dir="$env_dir/dev"
	elif [[ "$env" =~ ^e2e-.+ ]]; then
		target_dir="$env_dir/e2e"
	else
		error "Unknown environment type: $env"
	fi

	cd "$target_dir" || error "Failed to change to terraform directory: $target_dir"

	# Extract worker endpoints from monitoring_targets as JSON
	tofu output -json monitoring_targets 2>/dev/null | jq -r '.worker_endpoints // {}' 2>/dev/null || echo "{}"
}

# Get first available worker URL from terraform outputs
get_primary_worker_url() {
	local env="$1"
	local project_root="${2:-$E2E_PROJECT_ROOT}"

	local endpoints_json
	endpoints_json=$(get_worker_endpoints "$env" "$project_root")

	# Return the first worker's HTTP URL
	echo "$endpoints_json" | jq -r 'to_entries[0].value.http_url // empty' 2>/dev/null
}

# Get all worker URLs from terraform outputs
get_all_worker_urls() {
	local env="$1"
	local project_root="${2:-$E2E_PROJECT_ROOT}"

	local endpoints_json
	endpoints_json=$(get_worker_endpoints "$env" "$project_root")

	# Return all worker HTTP URLs, one per line
	echo "$endpoints_json" | jq -r 'to_entries[].value.http_url // empty' 2>/dev/null
}

# Test if terraform deployment is ready by checking worker endpoint availability
test_terraform_deployment_ready() {
	local env="$1"
	local timeout="${2:-300}"
	local project_root="${3:-$E2E_PROJECT_ROOT}"

	log "Waiting for terraform deployment in environment '$env' to be ready..."

	local elapsed=0
	while [[ $elapsed -lt $timeout ]]; do
		local worker_url
		worker_url=$(get_primary_worker_url "$env" "$project_root")

		if [[ -n "$worker_url" ]]; then
			# Test if the worker is actually responding
			if curl -s --max-time 5 "$worker_url/w/health" >/dev/null 2>&1; then
				success "Terraform deployment in environment '$env' is ready"
				return 0
			fi
		fi

		sleep 10
		elapsed=$((elapsed + 10))
	done

	warn "Terraform deployment readiness check timed out after ${timeout}s"
	return 1
}

# Cleanup function for temp directories and processes
cleanup_temp_resources() {
	local temp_dir="$1"

	# Remove temp directory if provided
	if [[ -n "$temp_dir" && -d "$temp_dir" ]]; then
		verbose_log "Removing temp directory: $temp_dir"
		rm -rf "$temp_dir"
	fi
}
