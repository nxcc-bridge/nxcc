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

# Local KinD Cluster Config
readonly KIND_CLUSTER_NAME="nxcc-debug"
export KIND_CLUSTER_NAME
readonly LOCAL_IMAGE_NAME="nxcc-node-local" # Expected name of the local image for 'debug'
export LOCAL_IMAGE_NAME
readonly LOCAL_IMAGE_TAG="latest"
export LOCAL_IMAGE_TAG

# Helm Chart Config
# Split declaration and assignment to avoid masking return values
HELM_CHART_PATH="$(dirname "$(realpath "$0")")/charts/nxcc-node"
readonly HELM_CHART_PATH
export HELM_CHART_PATH

# TDX Development VM Config
readonly TDX_VM_NAME="${TDX_VM_NAME:-nxcc-tdx-dev}"
export TDX_VM_NAME
readonly TDX_VM_ZONE="${TDX_VM_ZONE:-us-central1-a}"
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
readonly GCP_AR_LOCATION="${GCP_AR_LOCATION:-europe}"
readonly GCP_GKE_REGION="${GCP_GKE_REGION:-europe-west1}"

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
# Creates a Kubernetes secret with operator signing key from raw key data
# Arguments:
#   $1: Base64-encoded private key data OR path to key file (for backward compatibility)
#   $2: Secret name (optional, defaults to 'nxcc-operator-key')
#   $3: Namespace (optional, defaults to current namespace)
################################################################################
create_operator_key_secret() {
	local key_input="$1"
	local secret_name="${2:-nxcc-operator-key}"
	local namespace="${3:-}"

	if [[ -z "$key_input" ]]; then
		error "Operator key data or file path is required"
	fi

	local key_data
	# Check if input is a file path (backward compatibility)
	if [[ -f "$key_input" ]]; then
		info "Reading key from file: $key_input"
		key_data=$(base64 -w 0 <"$key_input")
	else
		# Assume it's already base64-encoded key data
		key_data="$key_input"
	fi

	local kubectl_args=(
		create secret generic "$secret_name"
		--from-literal=private-key="$(echo "$key_data" | base64 -d | base64 -w 0)"
	)

	if [[ -n "$namespace" ]]; then
		kubectl_args+=(--namespace="$namespace")
	fi

	info "Creating Kubernetes secret '$secret_name' with operator key..."

	if kubectl "${kubectl_args[@]}"; then
		success "Operator key secret '$secret_name' created successfully"
	else
		# Check if secret already exists (which is fine for persistent keys)
		if kubectl get secret "$secret_name" ${namespace:+--namespace="$namespace"} >/dev/null 2>&1; then
			info "Operator key secret '$secret_name' already exists - using existing persistent key"
			success "Using existing operator key secret '$secret_name'"
		else
			error "Failed to create operator key secret '$secret_name' and it doesn't exist"
		fi
	fi
}

################################################################################
# Sets up operator key for the current deployment environment
# Arguments:
#   $1: Environment (debug, staging, prod)
#   $2: Path to operator key file (optional, for backward compatibility)
################################################################################
setup_operator_keys() {
	local env="$1"
	local key_file="$2"

	if [[ -z "$env" ]]; then
		error "Environment is required for setup_operator_keys"
	fi

	# Check if secret already exists first
	local secret_name="nxcc-operator-key"
	if kubectl get secret "$secret_name" --namespace="$env" >/dev/null 2>&1; then
		info "Using existing persistent operator key secret '$secret_name'"
	else
		# Generate key data directly without materializing to disk
		local key_data
		if [[ -n "$key_file" ]] && [[ -f "$key_file" ]]; then
			info "Using provided key file: $key_file"
			key_data=$(base64 -w 0 <"$key_file")
		else
			info "Generating new operator key directly in memory"
			# shellcheck disable=SC2119 # Intentionally called without arguments
			key_data=$(generate_operator_key)
		fi

		# Create the Kubernetes secret directly from key data
		create_operator_key_secret "$key_data" "$secret_name" "$env"
	fi

	# Set environment variables for deployment
	export NXCC_OPERATOR_KEY_ENABLED=true
	export NXCC_OPERATOR_KEY_SECRET_NAME="nxcc-operator-key"

	success "Operator key configured for environment: $env"
}
