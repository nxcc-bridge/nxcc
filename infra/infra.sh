#!/bin/bash
#
# Manages cloud and local Kubernetes resources for the nXCC confidential workload.
#
# This script provides a structured command interface:
# - ci <setup|teardown>: Manages one-time CI/CD resources in GCP (Service Account, WIF, etc.).
# - cluster <create|destroy> <env>: Manages the Kubernetes cluster itself.
#   - <env> can be 'gke' for a Google Kubernetes Engine cluster.
#   - <env> can be 'kind' for a local Kubernetes-in-Docker cluster.
# - k8s <deploy|destroy> <env>: Manages the application deployment via Helm.
#   - <env> can be 'debug' (for kind), 'staging' (for gke), or 'prod' (for gke).
#
# The script is designed to be idempotent and handles cloud provider identity.

set -e
set -o pipefail

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
readonly HELM_CHART_PATH="./charts/nxcc-node"

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
#
# Account Resolution Logic:
# 1. Use `GCP_ACCOUNT` env var if set.
# 2. If multiple gcloud accounts are logged in, prompt the user to select one.
# 3. If one account is logged in, use it automatically.
# 4. If no accounts are logged in, trigger the login flow.
#
# Project ID Resolution Logic:
# 1. Use `GCP_PROJECT_ID` env var if set.
# 2. If not set, infer from the gcloud configuration for the selected account.
# 3. If it cannot be determined, exit with an error.
################################################################################
resolve_gcp_identity() {
  info "Resolving GCP identity..."

  # --- Step 1: Resolve GCP Account ---
  if [[ -n "${GCP_ACCOUNT_OVERRIDE}" ]]; then
    GCP_ACCOUNT="${GCP_ACCOUNT_OVERRIDE}"
    info "Using account from GCP_ACCOUNT environment variable: ${GCP_ACCOUNT}"
  else
    # Use a `while read` loop for compatibility with older bash versions (like on macOS)
    # This safely reads each line of gcloud output into the `accounts` array.
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


# --- CI/CD Functions ---

################################################################################
# Manages CI/CD resources (Service Account, WIF, Artifact Registry).
# Globals:
#   PROJECT_ID, GCP_ACCOUNT, SERVICE_ACCOUNT_NAME, WIF_POOL_ID, WIF_PROVIDER_ID,
#   AR_REPO_NAME, GCP_AR_LOCATION
# Arguments:
#   None
################################################################################
cicd_setup() {
  info "Starting CI/CD resource setup..."

  read -p "Enter the Git repository (OWNER/REPO) [nxcc-bridge/nxcc]: " GIT_REPO
  GIT_REPO="${GIT_REPO:-nxcc-bridge/nxcc}"
  info "Configuring for repository: ${GIT_REPO}"

  info "Enabling required Google Cloud APIs for CI/CD..."
  local apis_to_enable=(
    "iam.googleapis.com"
    "iamcredentials.googleapis.com"
    "artifactregistry.googleapis.com"
    "cloudresourcemanager.googleapis.com" # Needed for WIF pool creation
  )
  for api in "${apis_to_enable[@]}"; do
    if ! gcloud services list --enabled --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" --format="value(config.name)" | grep -q "^${api}$"; then
      gcloud services enable "${api}" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}"
      success "Enabled API: ${api}"
    else
      warn "API ${api} is already enabled."
    fi
  done

  local sa_email="${SERVICE_ACCOUNT_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"
  info "Checking for Service Account: ${sa_email}"
  if ! gcloud iam service-accounts describe "${sa_email}" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" &>/dev/null; then
    gcloud iam service-accounts create "${SERVICE_ACCOUNT_NAME}" \
      --display-name="CI/CD Service Account for nXCC" \
      --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}"
    success "Created Service Account: ${sa_email}"
  else
    warn "Service Account ${sa_email} already exists."
  fi

  info "Checking for Workload Identity Pool: ${WIF_POOL_ID}"
  if ! gcloud iam workload-identity-pools describe "${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" &>/dev/null; then
    gcloud iam workload-identity-pools create "${WIF_POOL_ID}" \
      --location="global" \
      --display-name="nXCC CI/CD Pool" \
      --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}"
    success "Created Workload Identity Pool: ${WIF_POOL_ID}"
  else
    warn "Workload Identity Pool ${WIF_POOL_ID} already exists."
  fi

  local wif_pool_full_name
  wif_pool_full_name=$(gcloud iam workload-identity-pools describe "${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" --format="value(name)")
  info "Checking for Workload Identity Provider: ${WIF_PROVIDER_ID}"
  if ! gcloud iam workload-identity-pools providers describe "${WIF_PROVIDER_ID}" --workload-identity-pool="${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" &>/dev/null; then
    gcloud iam workload-identity-pools providers create-oidc "${WIF_PROVIDER_ID}" \
      --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" \
      --workload-identity-pool="${WIF_POOL_ID}" \
      --location="global" \
      --issuer-uri="https://token.actions.githubusercontent.com" \
      --attribute-mapping="google.subject=assertion.sub,attribute.repository=assertion.repository" \
      --attribute-condition="attribute.repository == '${GIT_REPO}'"
    success "Created Workload Identity Provider."
  else
    warn "Workload Identity Provider ${WIF_PROVIDER_ID} already exists. Updating condition..."
    gcloud iam workload-identity-pools providers update-oidc "${WIF_PROVIDER_ID}" \
      --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" \
      --workload-identity-pool="${WIF_POOL_ID}" \
      --location="global" \
      --attribute-condition="attribute.repository == '${GIT_REPO}'"
    success "Updated provider condition for repo ${GIT_REPO}."
  fi

  info "Granting Workload Identity User role to the Git repository..."
  gcloud iam service-accounts add-iam-policy-binding "${sa_email}" \
    --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" \
    --role="roles/iam.workloadIdentityUser" \
    --member="principalSet://iam.googleapis.com/${wif_pool_full_name}/attribute.repository/${GIT_REPO}" \
    2> >(grep -v "already exists" >&2) || true
  success "Permission granted for ${GIT_REPO} to impersonate ${sa_email}."

  info "Checking for Artifact Registry repository: ${AR_REPO_NAME}"
  if ! gcloud artifacts repositories describe "${AR_REPO_NAME}" --location="${GCP_AR_LOCATION}" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" &>/dev/null; then
    gcloud artifacts repositories create "${AR_REPO_NAME}" \
      --repository-format="docker" \
      --location="${GCP_AR_LOCATION}" \
      --description="Docker images for nXCC" \
      --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}"
    success "Created Artifact Registry repository: ${AR_REPO_NAME}"
  else
    warn "Artifact Registry repository ${AR_REPO_NAME} already exists."
  fi

  info "Granting Artifact Registry Writer role to the Service Account..."
  gcloud artifacts repositories add-iam-policy-binding "${AR_REPO_NAME}" \
    --location="${GCP_AR_LOCATION}" \
    --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" \
    --role="roles/artifactregistry.writer" \
    --member="serviceAccount:${sa_email}" \
    2> >(grep -v "already exists" >&2) || true
  success "Permission granted for ${sa_email} to write to the repository."

  local wif_provider_full_name
  wif_provider_full_name=$(gcloud iam workload-identity-pools providers describe "${WIF_PROVIDER_ID}" --workload-identity-pool="${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" --format="value(name)")

  echo -e "\n\n${C_GREEN}================================================================"
  echo -e "          CI/CD Setup Complete!"
  echo -e "================================================================${C_RESET}"
  echo -e "\nAdd the following as secrets to your ${C_YELLOW}${GIT_REPO}${C_RESET} repository:"
  echo -e "----------------------------------------------------------------\n"
  echo -e "${C_BLUE}GCP_PROJECT_ID:${C_RESET} ${PROJECT_ID}"
  echo -e "${C_BLUE}GCP_WORKLOAD_IDENTITY_PROVIDER:${C_RESET} ${wif_provider_full_name}"
  echo -e "${C_BLUE}GCP_SERVICE_ACCOUNT:${C_RESET} ${sa_email}"
  echo -e "\nYour Artifact Registry host is: ${C_YELLOW}${GCP_AR_LOCATION}-docker.pkg.dev${C_RESET}"
}

################################################################################
# Tears down CI/CD resources.
################################################################################
cicd_teardown() {
  info "Starting CI/CD resource teardown..."
  local sa_email="${SERVICE_ACCOUNT_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"

  info "Removing Artifact Registry IAM policy..."
  gcloud artifacts repositories remove-iam-policy-binding "${AR_REPO_NAME}" \
    --location="${GCP_AR_LOCATION}" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" \
    --role="roles/artifactregistry.writer" --member="serviceAccount:${sa_email}" \
    2> >(grep -v "does not exist" >&2) || true
  success "Artifact Registry policy binding removed."

  info "Deleting Artifact Registry repository: ${AR_REPO_NAME}"
  if gcloud artifacts repositories describe "${AR_REPO_NAME}" --location="${GCP_AR_LOCATION}" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" &>/dev/null; then
    gcloud artifacts repositories delete "${AR_REPO_NAME}" --location="${GCP_AR_LOCATION}" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" --quiet
    success "Deleted Artifact Registry repository."
  else
    warn "Artifact Registry repository does not exist."
  fi

  info "Deleting Workload Identity Provider: ${WIF_PROVIDER_ID}"
  if gcloud iam workload-identity-pools providers describe "${WIF_PROVIDER_ID}" --workload-identity-pool="${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" &>/dev/null; then
    gcloud iam workload-identity-pools providers delete "${WIF_PROVIDER_ID}" \
      --workload-identity-pool="${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" --quiet
    success "Deleted Workload Identity Provider."
  else
    warn "Workload Identity Provider does not exist."
  fi

  info "Deleting Workload Identity Pool: ${WIF_POOL_ID}"
  if gcloud iam workload-identity-pools describe "${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" &>/dev/null; then
    gcloud iam workload-identity-pools delete "${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" --quiet
    success "Deleted Workload Identity Pool."
  else
    warn "Workload Identity Pool does not exist."
  fi

  info "Deleting Service Account: ${sa_email}"
  if gcloud iam service-accounts describe "${sa_email}" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" &>/dev/null; then
    gcloud iam service-accounts delete "${sa_email}" --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" --quiet
    success "Deleted Service Account."
  else
    warn "Service Account does not exist."
  fi

  success "CI/CD teardown complete."
}

# --- Cluster Management Functions ---

################################################################################
# Creates the GKE Autopilot cluster with Confidential Computing.
################################################################################
cluster_create_gke() {
  info "Starting GKE cluster creation..."
  check_deps gcloud kubectl
  resolve_gcp_identity

  info "Enabling GKE API..."
  if ! gcloud services list --enabled --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" --format="value(config.name)" | grep -q "^container.googleapis.com$"; then
    gcloud services enable container.googleapis.com --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}"
    success "Enabled GKE API (container.googleapis.com)."
  else
    warn "GKE API is already enabled."
  fi

  info "Checking for GKE cluster: ${GKE_CLUSTER_NAME}"
  if ! gcloud container clusters describe "${GKE_CLUSTER_NAME}" --region "${GCP_GKE_REGION}" --project "${PROJECT_ID}" --account "${GCP_ACCOUNT}" &>/dev/null; then
    info "Creating GKE Autopilot cluster '${GKE_CLUSTER_NAME}' in '${GCP_GKE_REGION}'..."
    info "This will take several minutes."
    gcloud container clusters create-auto "${GKE_CLUSTER_NAME}" \
      --project="${PROJECT_ID}" --account="${GCP_ACCOUNT}" \
      --region="${GCP_GKE_REGION}" \
      --release-channel="rapid"
    success "Created GKE cluster."
  else
    warn "GKE cluster ${GKE_CLUSTER_NAME} already exists."
  fi

  info "Granting CI/CD Service Account permission to deploy to the cluster..."
  local sa_email="${SERVICE_ACCOUNT_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"
  gcloud projects add-iam-policy-binding "${PROJECT_ID}" \
    --account="${GCP_ACCOUNT}" \
    --member="serviceAccount:${sa_email}" \
    --role="roles/container.developer" \
    2> >(grep -v "already exists" >&2) || true
  success "Granted 'GKE Developer' role to ${sa_email}."

  info "Configuring kubectl to connect to the cluster..."
  gcloud container clusters get-credentials "${GKE_CLUSTER_NAME}" --region "${GCP_GKE_REGION}" --project "${PROJECT_ID}" --account "${GCP_ACCOUNT}"
  success "kubectl is configured for ${GKE_CLUSTER_NAME}."
}

################################################################################
# Destroys the GKE cluster.
################################################################################
cluster_destroy_gke() {
  info "Starting GKE cluster destruction..."
  check_deps gcloud
  resolve_gcp_identity

  info "Removing GKE Developer role from CI/CD Service Account..."
  local sa_email="${SERVICE_ACCOUNT_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"
  gcloud projects remove-iam-policy-binding "${PROJECT_ID}" \
    --account="${GCP_ACCOUNT}" \
    --member="serviceAccount:${sa_email}" \
    --role="roles/container.developer" \
    2> >(grep -v "does not exist" >&2) || true
  success "Removed 'GKE Developer' role."

  info "Deleting GKE cluster: ${GKE_CLUSTER_NAME}"
  if gcloud container clusters describe "${GKE_CLUSTER_NAME}" --region "${GCP_GKE_REGION}" --project "${PROJECT_ID}" --account "${GCP_ACCOUNT}" &>/dev/null; then
    info "This will take several minutes."
    gcloud container clusters delete "${GKE_CLUSTER_NAME}" --region "${GCP_GKE_REGION}" --project "${PROJECT_ID}" --account "${GCP_ACCOUNT}" --quiet
    success "Deleted GKE cluster."
  else
    warn "GKE cluster ${GKE_CLUSTER_NAME} does not exist."
  fi
}

################################################################################
# Creates a local KinD cluster for debugging.
################################################################################
cluster_create_kind() {
  info "Starting KinD cluster creation..."
  check_deps kind docker

  if kind get clusters | grep -q "^${KIND_CLUSTER_NAME}$"; then
    warn "KinD cluster '${KIND_CLUSTER_NAME}' already exists."
  else
    info "Creating KinD cluster '${KIND_CLUSTER_NAME}'..."
    kind create cluster --name "${KIND_CLUSTER_NAME}"
    success "KinD cluster created."
  fi

  info "Attempting to load local Docker image '${LOCAL_IMAGE_NAME}:${LOCAL_IMAGE_TAG}' into the cluster..."
  info "Note: This assumes you have already built the image (e.g., 'docker build -t ${LOCAL_IMAGE_NAME}:${LOCAL_IMAGE_TAG} .')."
  if ! docker image inspect "${LOCAL_IMAGE_NAME}:${LOCAL_IMAGE_TAG}" &>/dev/null; then
      warn "Local image '${LOCAL_IMAGE_NAME}:${LOCAL_IMAGE_TAG}' not found. Skipping image load."
      warn "Deployment may fail if the image is not available in the cluster."
  else
      kind load docker-image "${LOCAL_IMAGE_NAME}:${LOCAL_IMAGE_TAG}" --name "${KIND_CLUSTER_NAME}"
      success "Image loaded into KinD cluster."
  fi

  success "KinD cluster setup is complete. Current context is '$(kubectl config current-context)'."
}

################################################################################
# Destroys the local KinD cluster.
################################################################################
cluster_destroy_kind() {
  info "Starting KinD cluster destruction..."
  check_deps kind

  if kind get clusters | grep -q "^${KIND_CLUSTER_NAME}$"; then
    info "Deleting KinD cluster '${KIND_CLUSTER_NAME}'..."
    kind delete cluster --name "${KIND_CLUSTER_NAME}"
    success "KinD cluster deleted."
  else
    warn "KinD cluster '${KIND_CLUSTER_NAME}' does not exist. Nothing to do."
  fi
}

# --- Helm Chart Management Functions ---

################################################################################
# Deploys the application to a specific environment using Helm.
# Arguments:
#   $1: The environment to deploy to ('debug', 'staging', 'prod').
################################################################################
k8s_deploy() {
  local env="$1"
  local helm_release_name="nxcc-node-${env}"
  local namespace="${env}"
  local helm_set_args=()

  info "Starting application deployment to '${env}' environment..."
  check_deps helm kubectl

  if [ ! -d "${HELM_CHART_PATH}" ]; then
    error "Helm chart not found at '${HELM_CHART_PATH}'. Please create it first."
  fi

  # --- Environment-specific configurations ---
  info "Configuring for '${env}'..."
  case "$env" in
    debug)
      # For local KinD cluster
      helm_set_args+=(--set confidential.enabled=false)
      helm_set_args+=(--set seed.replicaCount=1)
      helm_set_args+=(--set worker.replicaCount=1)
      helm_set_args+=(--set ingress.enabled=false)
      helm_set_args+=(--set worker.service.type=NodePort)
      helm_set_args+=(--set image.repository="${LOCAL_IMAGE_NAME}")
      helm_set_args+=(--set image.tag="${LOCAL_IMAGE_TAG}")
      helm_set_args+=(--set image.pullPolicy=IfNotPresent)
      ;;
    staging|prod)
      # For GKE cluster
      resolve_gcp_identity
      helm_set_args+=(--set image.repository="${GCP_AR_LOCATION}-docker.pkg.dev/${PROJECT_ID}/${AR_REPO_NAME}/node")
      helm_set_args+=(--set image.tag="latest") # Replace with your actual image tag in CI/CD

      if [ "$env" == "staging" ]; then
        helm_set_args+=(--set confidential.enabled=false)
        helm_set_args+=(--set seed.replicaCount=1)
        helm_set_args+=(--set worker.replicaCount=1)
        helm_set_args+=(--set ingress.hosts[0].host="staging.nxcc.example.com")
      else # prod
        helm_set_args+=(--set confidential.enabled=true)
        helm_set_args+=(--set seed.replicaCount=3)
        helm_set_args+=(--set worker.replicaCount=1)
        helm_set_args+=(--set ingress.hosts[0].host="prod.nxcc.example.com")
      fi
      ;;
    *)
      error "Invalid environment '${env}' specified for deployment. Must be 'debug', 'staging', or 'prod'."
      ;;
  esac

  info "Deploying/upgrading Helm release '${helm_release_name}' in namespace '${namespace}'."
  helm upgrade "${helm_release_name}" "${HELM_CHART_PATH}" \
    --install \
    --create-namespace \
    --atomic \
    --timeout 5m \
    --wait \
    --namespace "${namespace}" \
    "${helm_set_args[@]}"

  success "Application deployment to '${env}' complete."
  info "Use 'kubectl get all -n ${namespace}' to check status."
}

################################################################################
# Uninstalls the application from a specific environment.
# Arguments:
#   $1: The environment to destroy ('debug', 'staging', 'prod').
################################################################################
k8s_destroy() {
  local env="$1"
  local helm_release_name="nxcc-node-${env}"
  local namespace="${env}"

  info "Starting application uninstall from '${env}' environment..."
  check_deps helm kubectl

  if helm status "${helm_release_name}" --namespace "${namespace}" &>/dev/null; then
    info "Uninstalling Helm release '${helm_release_name}' from namespace '${namespace}'."
    helm uninstall "${helm_release_name}" --namespace "${namespace}"
    success "Helm release uninstalled."
  else
    warn "Helm release '${helm_release_name}' in namespace '${namespace}' not found. Nothing to do."
  fi
}

# --- Usage and Main ---

################################################################################
# Displays usage information.
################################################################################
usage() {
  echo "Usage: $0 <command> <subcommand> [args]"
  echo
  echo "Manages cloud (GCP) and local (KinD) resources for the nXCC application."
  echo
  echo "Commands:"
  echo "  ci <setup|teardown>"
  echo "    Manages GCP resources for CI/CD (Service Account, WIF, Artifact Registry)."
  echo "      setup:    Creates and configures all CI/CD resources."
  echo "      teardown: Deletes all CI/CD resources."
  echo
  echo "  cluster <create|destroy> <env>"
  echo "    Manages the Kubernetes cluster."
  echo "      <env>: gke | kind"
  echo "      create:   Creates the specified cluster."
  echo "      destroy:  Deletes the specified cluster."
  echo
  echo "  k8s <deploy|destroy> <env>"
  echo "    Manages the application deployment via Helm chart."
  echo "      <env>: debug | staging | prod"
  echo "      deploy:   Deploys or upgrades the application to the specified environment."
  echo "      destroy:  Uninstalls the application from the specified environment."
  echo
  echo "Environment Notes:"
  echo "  - 'debug' environment is intended for the 'kind' cluster."
  echo "  - 'staging' and 'prod' environments are intended for the 'gke' cluster."
  echo
  echo "GCP Identity:"
  echo "  For 'ci' and 'gke' commands, the script will resolve your GCP identity automatically."
  echo "  You can override this by setting GCP_ACCOUNT and GCP_PROJECT_ID environment variables."
}

################################################################################
# Main execution block.
################################################################################
main() {
  local command="${1-}"
  local subcommand="${2-}"
  local env="${3-}"

  if [[ -z "$command" ]]; then
    usage
    exit 1
  fi

  case "$command" in
    ci)
      check_deps gcloud
      resolve_gcp_identity # CI commands always need GCP identity
      case "$subcommand" in
        setup)
          cicd_setup
          ;;
        teardown)
          read -p "Are you sure you want to delete all CI/CD resources in project ${PROJECT_ID}? [y/N] " -n 1 -r; echo
          if [[ $REPLY =~ ^[Yy]$ ]]; then cicd_teardown; else info "Teardown cancelled."; fi
          ;;
        *)
          error "Invalid subcommand for 'ci'. Use 'setup' or 'teardown'."
          ;;
      esac
      ;;

    cluster)
      case "$subcommand" in
        create)
          case "$env" in
            gke) cluster_create_gke ;;
            kind) cluster_create_kind ;;
            "") error "Missing environment for 'cluster create'. Use 'gke' or 'kind'." ;;
            *) error "Invalid environment for 'cluster create'. Use 'gke' or 'kind'." ;;
          esac
          ;;
        destroy)
          case "$env" in
            gke)
              read -p "Are you sure you want to delete the GKE cluster '${GKE_CLUSTER_NAME}'? [y/N] " -n 1 -r; echo
              if [[ $REPLY =~ ^[Yy]$ ]]; then cluster_destroy_gke; else info "Cluster deletion cancelled."; fi
              ;;
            kind)
              read -p "Are you sure you want to delete the KinD cluster '${KIND_CLUSTER_NAME}'? [y/N] " -n 1 -r; echo
              if [[ $REPLY =~ ^[Yy]$ ]]; then cluster_destroy_kind; else info "Cluster deletion cancelled."; fi
              ;;
            "") error "Missing environment for 'cluster destroy'. Use 'gke' or 'kind'." ;;
            *) error "Invalid environment for 'cluster destroy'. Use 'gke' or 'kind'." ;;
          esac
          ;;
        *)
          error "Invalid subcommand for 'cluster'. Use 'create' or 'destroy'."
          ;;
      esac
      ;;

    k8s)
      case "$subcommand" in
        deploy)
          if [[ -z "$env" ]]; then error "Missing environment for 'k8s deploy'. Use 'debug', 'staging', or 'prod'."; fi
          k8s_deploy "$env"
          ;;
        destroy)
          if [[ -z "$env" ]]; then error "Missing environment for 'k8s destroy'. Use 'debug', 'staging', or 'prod'."; fi
          local release_to_destroy="nxcc-node-${env}"
          read -p "Are you sure you want to uninstall the application '${release_to_destroy}' from the '${env}' environment? [y/N] " -n 1 -r; echo
          if [[ $REPLY =~ ^[Yy]$ ]]; then k8s_destroy "$env"; else info "Application uninstall cancelled."; fi
          ;;
        *)
          error "Invalid subcommand for 'k8s'. Use 'deploy' or 'destroy'."
          ;;
      esac
      ;;

    *)
      usage
      exit 1
      ;;
  esac
}

main "$@"
