#!/bin/bash
#
# Common variables, helpers, and identity functions for the infra scripts.
# This script is intended to be sourced, not executed directly.

# --- Configuration ---
# GCP CI/CD and GKE Cluster Config
readonly SERVICE_ACCOUNT_NAME="nxcc-ci-cd-runner"
export SERVICE_ACCOUNT_NAME
readonly WIF_POOL_ID="nxcc-ci-pool"
export WIF_POOL_ID
readonly WIF_PROVIDER_ID="nxcc-git-provider"
export WIF_PROVIDER_ID
readonly AR_REPO_NAME="nxcc-images"
export AR_REPO_NAME
readonly GKE_CLUSTER_NAME="nxcc"
export GKE_CLUSTER_NAME

# TDX Development VM Config
readonly TDX_VM_NAME="${TDX_VM_NAME:-nxcc-tdx-dev}"
export TDX_VM_NAME
readonly TDX_VM_ZONE="${TDX_VM_ZONE:-europe-west4-a}"
export TDX_VM_ZONE
readonly TDX_VM_MACHINE_TYPE="${TDX_VM_MACHINE_TYPE:-c3-standard-4}"
export TDX_VM_MACHINE_TYPE
readonly TDX_VM_IMAGE_FAMILY="${TDX_VM_IMAGE_FAMILY:-ubuntu-2404-lts-amd64}"
export TDX_VM_IMAGE_FAMILY
readonly TDX_VM_IMAGE_PROJECT="${TDX_VM_IMAGE_PROJECT:-ubuntu-os-cloud}"
export TDX_VM_IMAGE_PROJECT
readonly NXCC_DEV_IMAGE="${NXCC_DEV_IMAGE:-ghcr.io/nxcc-bridge/dev:latest}"
export NXCC_DEV_IMAGE
readonly TDX_VM_PREEMPTIBLE="${TDX_VM_PREEMPTIBLE:-true}"
export TDX_VM_PREEMPTIBLE

# --- GCP Locations ---
# Override with environment variables if needed.
readonly GCP_AR_LOCATION="${GCP_AR_LOCATION:-europe-west4}"
readonly GCP_GKE_REGION="${GCP_GKE_REGION:-europe-west4}"

# --- Color Codes for Output ---
readonly C_GREEN='\033[0;32m'
readonly C_YELLOW='\033[0;33m'
readonly C_BLUE='\033[0;34m'
readonly C_RED='\033[0;31m'
readonly C_RESET='\033[0m'

# --- Global Variables for Resolved Identity ---
RESOLVED_GCP_ACCOUNT=""
RESOLVED_PROJECT_ID=""

# --- Helper Functions ---
info() { echo -e "${C_BLUE}INFO:${C_RESET} $1"; }
success() { echo -e "${C_GREEN}SUCCESS:${C_RESET} $1"; }
warn() { echo -e "${C_YELLOW}WARN:${C_RESET} $1"; }
error() {
	echo -e "${C_RED}ERROR:${C_RESET} $1" >&2
	exit 1
}

# --- Prerequisite Checks ---
check_deps() {
	for cmd in "$@"; do
		if ! command -v "$cmd" &>/dev/null; then
			error "'$cmd' CLI is not installed. Please install it to continue."
		fi
	done
}

# --- Identity Resolution ---

################################################################################
# Resolves the GCP Account and Project ID to use for all subsequent commands.
# Populates the global RESOLVED_GCP_ACCOUNT and RESOLVED_PROJECT_ID variables.
################################################################################
resolve_gcp_identity() {
	# If already resolved, do nothing.
	if [[ -n "${RESOLVED_GCP_ACCOUNT}" && -n "${RESOLVED_PROJECT_ID}" ]]; then
		return 0
	fi

	# If running in a CI environment, assume auth is handled externally (e.g., WIF).
	if [[ -n "${CI}" ]]; then
		info "CI environment detected. Assuming pre-configured GCP identity."
		RESOLVED_PROJECT_ID=$(gcloud config get-value project 2>/dev/null)
		if [[ -z "${RESOLVED_PROJECT_ID}" ]]; then
			error "GCP Project ID not found in gcloud config. Ensure the 'google-github-actions/auth' step runs first."
		fi
		RESOLVED_GCP_ACCOUNT="[Service Account via WIF]"
		success "Using project from CI environment: ${C_YELLOW}${RESOLVED_PROJECT_ID}${C_RESET}"
		return
	fi

	info "Resolving GCP identity..."

	# --- Step 1: Resolve GCP Account ---
	# Check if account is provided via environment variable
	if [[ -n "${GCP_ACCOUNT:-}" ]]; then
		info "Using account from GCP_ACCOUNT environment variable: ${GCP_ACCOUNT}"
		# Verify the account is authenticated and set it as active
		if gcloud auth list --format="value(account)" | grep -q "^${GCP_ACCOUNT}$"; then
			gcloud config set account "${GCP_ACCOUNT}"
			info "Set ${GCP_ACCOUNT} as active account"
			RESOLVED_GCP_ACCOUNT="${GCP_ACCOUNT}"
		else
			error "Account ${GCP_ACCOUNT} is not authenticated. Please run 'gcloud auth login ${GCP_ACCOUNT}' first."
		fi
	else
		local accounts=()
		while IFS= read -r line; do
			accounts+=("$line")
		done < <(gcloud auth list --filter=status:ACTIVE --format="value(account)")

		if [[ ${#accounts[@]} -eq 0 ]]; then
			info "No active GCP account found. Opening browser for authentication..."
			gcloud auth login
			gcloud auth application-default login
			RESOLVED_GCP_ACCOUNT=$(gcloud config get-value account)
			success "Logged in successfully as: ${RESOLVED_GCP_ACCOUNT}"
		elif [[ ${#accounts[@]} -eq 1 ]]; then
			RESOLVED_GCP_ACCOUNT="${accounts[0]}"
			info "Automatically selected the only available GCP account: ${RESOLVED_GCP_ACCOUNT}"
		else
			warn "Multiple GCP accounts found. Please choose which one to use:"
			select account in "${accounts[@]}"; do
				if [[ -n "$account" ]]; then
					RESOLVED_GCP_ACCOUNT="$account"
					break
				else
					echo "Invalid selection. Please try again."
				fi
			done
		fi
	fi
	success "Using account: ${C_YELLOW}${RESOLVED_GCP_ACCOUNT}${C_RESET}"

	# --- Step 2: Resolve Project ID ---
	if [[ -n "${GCP_PROJECT_ID}" ]]; then
		RESOLVED_PROJECT_ID="${GCP_PROJECT_ID}"
		info "Using project ID from GCP_PROJECT_ID environment variable: ${RESOLVED_PROJECT_ID}"
	else
		RESOLVED_PROJECT_ID=$(gcloud config get-value project --account="${RESOLVED_GCP_ACCOUNT}" 2>/dev/null)
		if [[ -n "${RESOLVED_PROJECT_ID}" ]]; then
			info "Inferred project ID from gcloud config for account ${RESOLVED_GCP_ACCOUNT}: ${RESOLVED_PROJECT_ID}"
		else
			error "Could not determine GCP Project ID.
Please set it by one of the following methods:
1. Export the environment variable: export GCP_PROJECT_ID=\"your-project-id\"
2. Set it in your gcloud config: gcloud config set project \"your-project-id\" --account=\"${RESOLVED_GCP_ACCOUNT}\""
		fi
	fi
	success "Using project: ${C_YELLOW}${RESOLVED_PROJECT_ID}${C_RESET}"
}

################################################################################
# Generates a new Ed25519 private key for operator signing
# Arguments:
#   $1: Output file path for the private key (optional, for backward compatibility)
# Returns: Base64-encoded key data to stdout
################################################################################
# shellcheck disable=SC2120 # Function intentionally supports optional arguments
generate_operator_key() {
	local output_file="${1:-}"

	info "Generating new Ed25519 operator signing key..."

	# Generate 32 bytes of random data for Ed25519 private key
	local key_data
	if command -v openssl >/dev/null 2>&1; then
		key_data=$(openssl rand 32 | base64 -w 0)
	elif command -v dd >/dev/null 2>&1; then
		key_data=$(dd if=/dev/urandom bs=32 count=1 2>/dev/null | base64 -w 0)
	else
		error "Cannot generate random key: neither openssl nor dd is available"
	fi

	# For backward compatibility, write to file if specified
	if [[ -n "$output_file" ]]; then
		echo "$key_data" | base64 -d >"$output_file"
		chmod 600 "$output_file"
		success "Operator key generated at: $output_file"
	fi

	# Return base64-encoded key data
	echo "$key_data"
}

################################################################################
# Modern Terraform Module-Based Deployment Functions
################################################################################

################################################################################
# Ensures application default credentials are available for Terraform
################################################################################
ensure_adc_credentials() {
	# Check if ADC are already available
	if gcloud auth application-default print-access-token >/dev/null 2>&1; then
		return 0
	fi

	info "Terraform requires Application Default Credentials (ADC)"
	info "Opening browser for authentication..."
	echo

	# Run gcloud auth with proper handling
	if ! gcloud auth application-default login; then
		error "Failed to set up Application Default Credentials. Please run: gcloud auth application-default login"
	fi

	success "Application Default Credentials configured"
}

################################################################################
# Determines GCS bucket name for Terraform state
# Returns the bucket name to stdout
################################################################################
get_gcs_state_bucket() {
	# Allow override via environment variable
	if [[ -n "${GCS_STATE_BUCKET:-}" ]]; then
		echo "$GCS_STATE_BUCKET"
		return 0
	fi

	# Allow override via command line option (set by caller)
	if [[ -n "${OVERRIDE_GCS_BUCKET:-}" ]]; then
		echo "$OVERRIDE_GCS_BUCKET"
		return 0
	fi

	# Auto-generate based on project ID
	if [[ -z "$RESOLVED_PROJECT_ID" ]]; then
		error "Cannot determine GCS bucket: no project ID available. Set GCS_STATE_BUCKET environment variable or provide --bucket option"
	fi

	echo "${RESOLVED_PROJECT_ID}-terraform-state"
}

################################################################################
# Determines the correct Docker image to use for deployment
# Returns the image URL to stdout
################################################################################
get_deployment_docker_image() {
	local env="$1"

	# Allow explicit override via environment variable
	if [[ -n "${DOCKER_IMAGE_OVERRIDE:-}" ]]; then
		echo "$DOCKER_IMAGE_OVERRIDE"
		return 0
	fi

	# Check if we have a GCP Artifact Registry setup
	if [[ -n "$RESOLVED_PROJECT_ID" ]]; then
		local gcp_image=""

		# Determine tag based on build mode or environment
		if [[ -n "${E2E_BUILD_MODE:-}" ]]; then
			# E2E test mode - use the build mode tag
			gcp_image="${GCP_AR_LOCATION:-europe-west4}-docker.pkg.dev/${RESOLVED_PROJECT_ID}/${AR_REPO_NAME}/node:${E2E_BUILD_MODE}"
		elif [[ "$env" == "production" ]]; then
			# Production uses latest
			gcp_image="${GCP_AR_LOCATION:-europe-west4}-docker.pkg.dev/${RESOLVED_PROJECT_ID}/${AR_REPO_NAME}/node:latest"
		else
			# Staging and dev use debug by default
			gcp_image="${GCP_AR_LOCATION:-europe-west4}-docker.pkg.dev/${RESOLVED_PROJECT_ID}/${AR_REPO_NAME}/node:debug"
		fi

		# For E2E mode, always prefer the GCP image without checking manifest
		# since we just built and pushed it
		if [[ -n "${E2E_BUILD_MODE:-}" ]]; then
			echo "$gcp_image"
			return 0
		fi

		# For non-E2E mode, check if the GCP image exists
		if docker manifest inspect "$gcp_image" >/dev/null 2>&1; then
			echo "$gcp_image"
			return 0
		fi
	fi

	# Fall back to default GitHub Container Registry
	echo "ghcr.io/nxcc-bridge/node:latest"
}

################################################################################
# Generates Terraform backend configuration arguments
# Arguments:
#   $1: Environment name (staging, production, dev-username, e2e-testid)
# Returns: Array of backend config arguments via stdout
################################################################################
get_backend_configs() {
	local env="$1"
	local bucket_name
	bucket_name=$(get_gcs_state_bucket)

	local configs=("-backend-config=bucket=$bucket_name")

	# Add dynamic prefix based on environment type
	if [[ "$env" =~ ^dev-.+ ]]; then
		local developer_name="${env#dev-}"
		configs+=("-backend-config=prefix=dev/$developer_name")
	elif [[ "$env" =~ ^e2e-.+ ]]; then
		local test_id="${env#e2e-}"
		configs+=("-backend-config=prefix=e2e/$test_id")
	elif [[ "$env" == "staging" ]]; then
		configs+=("-backend-config=prefix=environments/staging")
	elif [[ "$env" == "production" ]]; then
		configs+=("-backend-config=prefix=environments/production")
	fi

	printf '%s\n' "${configs[@]}"
}

################################################################################
# Deploy NXCC infrastructure using Terraform modules
# Arguments:
#   $1: Environment name (staging, production, dev-username, e2e-testid)
################################################################################
deploy_create() {
	local env="$1"
	local env_dir="infra/environments"
	local target_dir=""

	info "Deploying NXCC infrastructure for environment: $env"

	# Determine environment directory based on name pattern
	if [[ "$env" == "staging" ]]; then
		target_dir="$env_dir/staging"
	elif [[ "$env" == "production" ]]; then
		target_dir="$env_dir/production"
	elif [[ "$env" =~ ^dev-.+ ]]; then
		target_dir="$env_dir/dev"
		local developer_name="${env#dev-}"
	elif [[ "$env" =~ ^e2e-.+ ]]; then
		target_dir="$env_dir/e2e"
		local test_id="${env#e2e-}"
	else
		error "Unknown environment type: $env. Use staging, production, dev-username, or e2e-testid"
	fi

	if [[ ! -d "$target_dir" ]]; then
		error "Environment directory not found: $target_dir"
	fi

	info "Using environment template: $target_dir"

	# Ensure ADC credentials are available for Terraform
	ensure_adc_credentials

	cd "$target_dir" || error "Failed to change to directory: $target_dir"

	# Initialize Terraform with dynamic backend config
	info "Initializing Terraform..."
	local bucket_name
	bucket_name=$(get_gcs_state_bucket)
	info "Using GCS state bucket: $bucket_name"

	# Get backend configs and build array manually for POSIX compatibility
	local backend_configs=()
	while IFS= read -r config; do
		backend_configs+=("$config")
	done < <(get_backend_configs "$env")
	tofu init "${backend_configs[@]}"

	# Prepare variables based on environment type
	local tf_vars=()
	if [[ "$env" =~ ^dev-.+ ]]; then
		tf_vars+=("-var=developer_name=$developer_name")
	elif [[ "$env" =~ ^e2e-.+ ]]; then
		tf_vars+=("-var=test_id=$test_id")
	fi

	# Add project ID if available
	if [[ -n "$RESOLVED_PROJECT_ID" ]]; then
		tf_vars+=("-var=project_id=$RESOLVED_PROJECT_ID")
	fi

	# Determine and add Docker image
	local docker_image
	docker_image=$(get_deployment_docker_image "$env")
	tf_vars+=("-var=docker_image=$docker_image")
	info "Using Docker image: $docker_image"

	# Plan and apply
	info "Planning deployment..."
	tofu plan "${tf_vars[@]}"

	info "Applying deployment..."
	tofu apply "${tf_vars[@]}" -auto-approve

	success "Environment $env deployed successfully!"

	# Show helpful outputs
	info "Getting deployment information..."
	deploy_status "$env"
}

################################################################################
# Destroy NXCC infrastructure
# Arguments:
#   $1: Environment name
#   $2: Optional --auto-approve flag
################################################################################
deploy_destroy() {
	local env="$1"
	local auto_approve="${2:-}"
	local env_dir="infra/environments"
	local target_dir=""

	# Determine environment directory (same logic as deploy_create)
	if [[ "$env" == "staging" ]]; then
		target_dir="$env_dir/staging"
	elif [[ "$env" == "production" ]]; then
		target_dir="$env_dir/production"
	elif [[ "$env" =~ ^dev-.+ ]]; then
		target_dir="$env_dir/dev"
		local developer_name="${env#dev-}"
	elif [[ "$env" =~ ^e2e-.+ ]]; then
		target_dir="$env_dir/e2e"
		local test_id="${env#e2e-}"
	else
		error "Unknown environment type: $env"
	fi

	if [[ ! -d "$target_dir" ]]; then
		error "Environment directory not found: $target_dir"
	fi

	# Ensure ADC credentials are available for Terraform
	ensure_adc_credentials

	cd "$target_dir" || error "Failed to change to directory: $target_dir"

	# Prepare variables
	local tf_vars=()
	if [[ "$env" =~ ^dev-.+ ]]; then
		tf_vars+=("-var=developer_name=$developer_name")
	elif [[ "$env" =~ ^e2e-.+ ]]; then
		tf_vars+=("-var=test_id=$test_id")
	fi

	if [[ -n "$RESOLVED_PROJECT_ID" ]]; then
		tf_vars+=("-var=project_id=$RESOLVED_PROJECT_ID")
	fi

	# Add Docker image for consistency
	local docker_image
	docker_image=$(get_deployment_docker_image "$env")
	tf_vars+=("-var=docker_image=$docker_image")

	info "Destroying environment: $env"

	if [[ "$auto_approve" == "--auto-approve" ]]; then
		tofu destroy "${tf_vars[@]}" -auto-approve
	else
		tofu destroy "${tf_vars[@]}"
	fi

	success "Environment $env destroyed successfully!"
}

################################################################################
# Show deployment status and connection information
# Arguments:
#   $1: Environment name
################################################################################
deploy_status() {
	local env="$1"
	local env_dir="infra/environments"
	local target_dir=""

	# Determine environment directory (same logic as deploy_create)
	if [[ "$env" == "staging" ]]; then
		target_dir="$env_dir/staging"
	elif [[ "$env" == "production" ]]; then
		target_dir="$env_dir/production"
	elif [[ "$env" =~ ^dev-.+ ]]; then
		target_dir="$env_dir/dev"
		local developer_name="${env#dev-}"
	elif [[ "$env" =~ ^e2e-.+ ]]; then
		target_dir="$env_dir/e2e"
		local test_id="${env#e2e-}"
	else
		error "Unknown environment type: $env"
	fi

	if [[ ! -d "$target_dir" ]]; then
		error "Environment directory not found: $target_dir"
	fi

	# Ensure ADC credentials are available for Terraform
	ensure_adc_credentials

	cd "$target_dir" || error "Failed to change to directory: $target_dir"

	# Prepare variables
	local tf_vars=()
	if [[ "$env" =~ ^dev-.+ ]]; then
		tf_vars+=("-var=developer_name=$developer_name")
	elif [[ "$env" =~ ^e2e-.+ ]]; then
		tf_vars+=("-var=test_id=$test_id")
	fi

	if [[ -n "$RESOLVED_PROJECT_ID" ]]; then
		tf_vars+=("-var=project_id=$RESOLVED_PROJECT_ID")
	fi

	# Add Docker image for consistency
	local docker_image
	docker_image=$(get_deployment_docker_image "$env")
	tf_vars+=("-var=docker_image=$docker_image")

	info "Environment: $env"
	info "Directory: $target_dir"
	echo

	# Check if state exists
	if ! tofu show -json >/dev/null 2>&1; then
		info "No infrastructure deployed for environment: $env"
		return 0
	fi

	# Show key outputs
	info "=== Deployment Summary ==="
	tofu output -json deployment_summary 2>/dev/null | jq -r '
		if . then
			"Environment: " + (.environment // "unknown") + 
			"\nNamespace: " + (.namespace // "default") + 
			"\nWorkers: " + (.total_workers | tostring) + 
			"\nSeeds: " + (.total_seeds | tostring) + 
			"\nRegions: " + (.regions_used | tostring) + 
			"\nTEE Enabled: " + (.tee_enabled | tostring)
		else
			"No deployment summary available"
		end
	' 2>/dev/null || echo "Deployment summary not available"

	echo
	info "=== Worker Endpoints ==="
	tofu output -json worker_endpoints 2>/dev/null | jq -r '
		if . then
			to_entries[] | "  " + .key + ": " + .value.http_url
		else
			"  No worker endpoints available"
		end
	' 2>/dev/null || echo "  Worker endpoints not available"

	echo
	info "=== Operator Key Information ==="
	tofu output -json operator_key_info 2>/dev/null | jq -r '
		if . then
			"  Secret: " + .secret_name + 
			"\n  Algorithm: " + .algorithm + 
			"\n  Purpose: " + .purpose +
			"\n  Extract Command: " + .extract_command
		else
			"  Operator key information not available"
		end
	' 2>/dev/null || echo "  Operator key information not available"

	echo
	info "=== SSH Commands ==="
	tofu output -json ssh_commands 2>/dev/null | jq -r '
		if . then
			to_entries[] | "  " + .key + ": " + .value
		else
			"  No SSH commands available"
		end
	' 2>/dev/null || echo "  SSH commands not available"
}

################################################################################
# Show deployment plan
# Arguments:
#   $1: Environment name
################################################################################
deploy_plan() {
	local env="$1"
	local env_dir="infra/environments"
	local target_dir=""

	# Determine environment directory (same logic as deploy_create)
	if [[ "$env" == "staging" ]]; then
		target_dir="$env_dir/staging"
	elif [[ "$env" == "production" ]]; then
		target_dir="$env_dir/production"
	elif [[ "$env" =~ ^dev-.+ ]]; then
		target_dir="$env_dir/dev"
		local developer_name="${env#dev-}"
	elif [[ "$env" =~ ^e2e-.+ ]]; then
		target_dir="$env_dir/e2e"
		local test_id="${env#e2e-}"
	else
		error "Unknown environment type: $env"
	fi

	if [[ ! -d "$target_dir" ]]; then
		error "Environment directory not found: $target_dir"
	fi

	cd "$target_dir" || error "Failed to change to directory: $target_dir"

	# Ensure ADC credentials are available for Terraform
	ensure_adc_credentials

	# Initialize Terraform if needed with dynamic backend config
	if [[ ! -d ".terraform" ]]; then
		info "Initializing Terraform..."
		local bucket_name
		bucket_name=$(get_gcs_state_bucket)
		info "Using GCS state bucket: $bucket_name"

		# Get backend configs and build array manually for POSIX compatibility
		local backend_configs=()
		while IFS= read -r config; do
			backend_configs+=("$config")
		done < <(get_backend_configs "$env")
		tofu init "${backend_configs[@]}"
	fi

	# Prepare variables
	local tf_vars=()
	if [[ "$env" =~ ^dev-.+ ]]; then
		tf_vars+=("-var=developer_name=$developer_name")
	elif [[ "$env" =~ ^e2e-.+ ]]; then
		tf_vars+=("-var=test_id=$test_id")
	fi

	if [[ -n "$RESOLVED_PROJECT_ID" ]]; then
		tf_vars+=("-var=project_id=$RESOLVED_PROJECT_ID")
	fi

	# Add Docker image for consistency
	local docker_image
	docker_image=$(get_deployment_docker_image "$env")
	tf_vars+=("-var=docker_image=$docker_image")

	info "Planning deployment for environment: $env"
	tofu plan "${tf_vars[@]}"
}
