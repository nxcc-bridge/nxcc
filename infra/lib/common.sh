#!/bin/bash
#
# Common variables, helpers, and identity functions for the infra scripts.
# This script is intended to be sourced, not executed directly.

# --- Configuration ---
# GCP CI/CD and GKE Cluster Config
readonly SERVICE_ACCOUNT_NAME="nxcc-ci-cd-runner"
readonly WIF_POOL_ID="nxcc-ci-pool"
readonly WIF_PROVIDER_ID="nxcc-git-provider"
readonly AR_REPO_NAME="nxcc-images"
readonly GKE_CLUSTER_NAME="nxcc"

# Local KinD Cluster Config
readonly KIND_CLUSTER_NAME="nxcc-debug"
readonly LOCAL_IMAGE_NAME="nxcc-node-local" # Expected name of the local image for 'debug'
readonly LOCAL_IMAGE_TAG="latest"

# Helm Chart Config
readonly HELM_CHART_PATH="$(dirname "$(realpath "$0")")/charts/nxcc-node"

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
GCP_ACCOUNT=""
PROJECT_ID=""

# --- Helper Functions ---
info() { echo -e "${C_BLUE}INFO:${C_RESET} $1"; }
success() { echo -e "${C_GREEN}SUCCESS:${C_RESET} $1"; }
warn() { echo -e "${C_YELLOW}WARN:${C_RESET} $1"; }
error() { echo -e "${C_RED}ERROR:${C_RESET} $1" >&2; exit 1; }

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
# Populates the global GCP_ACCOUNT and PROJECT_ID variables.
################################################################################
resolve_gcp_identity() {
  # If already resolved, do nothing.
  if [[ -n "${GCP_ACCOUNT}" && -n "${PROJECT_ID}" ]]; then
    return 0
  fi

  # If running in a CI environment, assume auth is handled externally (e.g., WIF).
  if [[ -n "${CI}" ]]; then
    info "CI environment detected. Assuming pre-configured GCP identity."
    PROJECT_ID=$(gcloud config get-value project 2>/dev/null)
    if [[ -z "${PROJECT_ID}" ]]; then
      error "GCP Project ID not found in gcloud config. Ensure the 'google-github-actions/auth' step runs first."
    fi
    GCP_ACCOUNT="[Service Account via WIF]"
    success "Using project from CI environment: ${C_YELLOW}${PROJECT_ID}${C_RESET}"
    return
  fi

  info "Resolving GCP identity..."

  # --- Step 1: Resolve GCP Account ---
  if [[ -n "${GCP_ACCOUNT_OVERRIDE}" ]]; then
    GCP_ACCOUNT="${GCP_ACCOUNT_OVERRIDE}"
    info "Using account from GCP_ACCOUNT environment variable: ${GCP_ACCOUNT}"
  else
    local accounts=()
    while IFS= read -r line; do
        accounts+=("$line")
    done < <(gcloud auth list --filter=status:ACTIVE --format="value(account)")

    if [[ ${#accounts[@]} -eq 0 ]]; then
      info "No active GCP account found. Opening browser for authentication..."
      gcloud auth login
      gcloud auth application-default login
      GCP_ACCOUNT=$(gcloud config get-value account)
      success "Logged in successfully as: ${GCP_ACCOUNT}"
    elif [[ ${#accounts[@]} -eq 1 ]]; then
      GCP_ACCOUNT="${accounts[0]}"
      info "Automatically selected the only available GCP account: ${GCP_ACCOUNT}"
    else
      warn "Multiple GCP accounts found. Please choose which one to use:"
      select account in "${accounts[@]}"; do
        if [[ -n "$account" ]]; then
          GCP_ACCOUNT="$account"
          break
        else
          echo "Invalid selection. Please try again."
        fi
      done
    fi
  fi
  success "Using account: ${C_YELLOW}${GCP_ACCOUNT}${C_RESET}"

  # --- Step 2: Resolve Project ID ---
  if [[ -n "${GCP_PROJECT_ID}" ]]; then
    PROJECT_ID="${GCP_PROJECT_ID}"
    info "Using project ID from GCP_PROJECT_ID environment variable: ${PROJECT_ID}"
  else
    PROJECT_ID=$(gcloud config get-value project --account="${GCP_ACCOUNT}" 2>/dev/null)
    if [[ -n "${PROJECT_ID}" ]]; then
      info "Inferred project ID from gcloud config for account ${GCP_ACCOUNT}: ${PROJECT_ID}"
    else
      error "Could not determine GCP Project ID.
Please set it by one of the following methods:
1. Export the environment variable: export GCP_PROJECT_ID=\"your-project-id\"
2. Set it in your gcloud config: gcloud config set project \"your-project-id\" --account=\"${GCP_ACCOUNT}\""
    fi
  fi
  success "Using project: ${C_YELLOW}${PROJECT_ID}${C_RESET}"
}
