#!/bin/bash
#
# Manages Google Cloud and Kubernetes resources for the nXCC confidential workload.
#
# This script provides subcommands to handle different lifecycle stages:
# - cicd-setup / cicd-teardown: One-time setup for CI/CD (Service Account, WIF, Artifact Registry).
# - cluster-create / cluster-destroy: Infrequent setup for the GKE cluster.
# - app-deploy / app-destroy: Frequent deployment of the application using Helm.
#
# The script is designed to be idempotent and explicitly handles GCP identity.

set -e
set -o pipefail

# --- Configuration ---
# The name for the Service Account to be created.
readonly SERVICE_ACCOUNT_NAME="nxcc-ci-cd-runner"
# The ID for the Workload Identity Pool.
readonly WIF_POOL_ID="nxcc-ci-pool"
# The ID for the Workload Identity Provider.
readonly WIF_PROVIDER_ID="nxcc-git-provider"
# The name of the Artifact Registry repository to create.
readonly AR_REPO_NAME="nxcc-images"
# The name of the GKE cluster.
readonly GKE_CLUSTER_NAME="nxcc-confidential-cluster"
# The Helm release name for the application.
readonly HELM_RELEASE_NAME="nxcc-app"
# The path to your Helm chart.
readonly HELM_CHART_PATH="./helm/nxcc-app"

# --- GCP Locations ---
# Override with environment variables if needed.
# The location for the Artifact Registry. Can be a region or multi-region.
readonly GCP_AR_LOCATION="${GCP_AR_LOCATION:-europe}"
# The region for the GKE cluster. Must be a specific region that supports TDX.
# See: https://cloud.google.com/compute/docs/regions-zones
readonly GCP_GKE_REGION="${GCP_GKE_REGION:-europe-west1}"

# --- Color Codes for Output ---
readonly C_GREEN='\033[0;32m'
readonly C_YELLOW='\033[0;33m'
readonly C_BLUE='\033[0;34m'
readonly C_RED='\033[0;31m'
readonly C_RESET='\033[0m'

# --- Global Variables for Resolved Identity ---
# These will be populated by resolve_gcp_identity()
GCP_ACCOUNT=""
PROJECT_ID=""

# --- Helper Functions ---
info() { echo -e "${C_BLUE}INFO:${C_RESET} $1"; }
success() { echo -e "${C_GREEN}SUCCESS:${C_RESET} $1"; }
warn() { echo -e "${C_YELLOW}WARN:${C_RESET} $1"; }
error() { echo -e "${C_RED}ERROR:${C_RESET} $1" >&2; exit 1; }

# --- Prerequisite Checks ---
check_deps() {
  for cmd in gcloud kubectl helm; do
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


# --- Main Logic Functions ---

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

################################################################################
# Creates the GKE Autopilot cluster with Confidential Computing.
################################################################################
cluster_create() {
  info "Starting GKE cluster creation..."

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
cluster_destroy() {
  info "Starting GKE cluster destruction..."

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
# Deploys the application using Helm.
################################################################################
app_deploy() {
  info "Starting application deployment with Helm..."

  if [ ! -d "${HELM_CHART_PATH}" ]; then
    error "Helm chart not found at '${HELM_CHART_PATH}'. Please create it first."
  fi

  info "Deploying/upgrading Helm release '${HELM_RELEASE_NAME}' from chart '${HELM_CHART_PATH}'."
  helm upgrade "${HELM_RELEASE_NAME}" "${HELM_CHART_PATH}" \
    --install \
    --atomic \
    --wait \
    --namespace default \
    --set image.repository="${GCP_AR_LOCATION}-docker.pkg.dev/${PROJECT_ID}/${AR_REPO_NAME}/my-app" \
    --set image.tag="latest" # Replace with your actual image tag in CI/CD

  success "Application deployment complete. Use 'kubectl get pods -l app.kubernetes.io/name=nxcc-app' to check status."
}

################################################################################
# Uninstalls the application using Helm.
################################################################################
app_destroy() {
  info "Starting application uninstall..."

  if helm status "${HELM_RELEASE_NAME}" &>/dev/null; then
    info "Uninstalling Helm release '${HELM_RELEASE_NAME}'."
    helm uninstall "${HELM_RELEASE_NAME}"
    success "Helm release uninstalled."
  else
    warn "Helm release '${HELM_RELEASE_NAME}' not found. Nothing to do."
  fi
}

################################################################################
# Displays usage information.
################################################################################
usage() {
  echo "Usage: $0 <command>"
  echo
  echo "Manages GCP and GKE resources for a confidential workload."
  echo
  echo "Commands:"
  echo "  cicd-setup        Sets up Service Account, WIF, and Artifact Registry for CI/CD."
  echo "  cicd-teardown     Tears down all CI/CD resources."
  echo "  cluster-create    Creates the confidential GKE Autopilot cluster."
  echo "  cluster-destroy   Deletes the GKE cluster."
  echo "  app-deploy        Deploys the application to GKE using Helm."
  echo "  app-destroy       Uninstalls the application from GKE."
  echo
  echo "Identity and Project Configuration:"
  echo "  The script determines which GCP account and project to use as follows:"
  echo "  1. Account: Uses the GCP_ACCOUNT env var. If not set, it prompts you to choose"
  echo "     from your logged-in accounts if you have more than one."
  echo "  2. Project: Uses the GCP_PROJECT_ID env var. If not set, it infers the project"
  echo "     from your gcloud configuration for the selected account."
}

# --- Main Execution ---
main() {
  check_deps

  if [[ -z "$1" ]]; then
    usage
    exit 1
  fi

  # Resolve GCP account and project ID before doing anything else.
  resolve_gcp_identity

  local COMMAND="$1"
  case "$COMMAND" in
    cicd-setup)
      cicd_setup
      ;;
    cicd-teardown)
      read -p "Are you sure you want to delete all CI/CD resources in project ${PROJECT_ID}? [y/N] " -n 1 -r
      echo
      if [[ $REPLY =~ ^[Yy]$ ]]; then
        cicd_teardown
      else
        info "Teardown cancelled."
      fi
      ;;
    cluster-create)
      cluster_create
      ;;
    cluster-destroy)
      read -p "Are you sure you want to delete the GKE cluster '${GKE_CLUSTER_NAME}' in project ${PROJECT_ID}? [y/N] " -n 1 -r
      echo
      if [[ $REPLY =~ ^[Yy]$ ]]; then
        cluster_destroy
      else
        info "Cluster deletion cancelled."
      fi
      ;;
    app-deploy)
      app_deploy
      ;;
    app-destroy)
      read -p "Are you sure you want to uninstall the application '${HELM_RELEASE_NAME}'? [y/N] " -n 1 -r
      echo
      if [[ $REPLY =~ ^[Yy]$ ]]; then
        app_destroy
      else
        info "Application uninstall cancelled."
      fi
      ;;
    *)
      usage
      exit 1
      ;;
  esac
}

main "$@"
