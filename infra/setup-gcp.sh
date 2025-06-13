#!/bin/bash
#
# Idempotent setup script for Google Cloud resources required by the CI/CD pipeline.
#
# This script configures:
# 1. A dedicated Service Account for the CI/CD process.
# 2. A Workload Identity Federation pool and provider to allow passwordless
#    authentication from a specific Git repository.
# 3. An Artifact Registry repository to store Docker images.
# 4. The necessary IAM permissions for the Service Account.
#
# It is idempotent and does not modify your global gcloud configuration.

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

# The location for the Artifact Registry. Can be a region (e.g., europe-west1)
# or a multi-region (e.g., europe).
# Override with the GCP_LOCATION environment variable.
readonly GCP_LOCATION="${GCP_LOCATION:-europe}"

# --- Color Codes for Output ---
readonly C_GREEN='\033[0;32m'
readonly C_YELLOW='\033[0;33m'
readonly C_BLUE='\033[0;34m'
readonly C_RESET='\033[0m'

# --- Helper Functions ---
info() {
  echo -e "${C_BLUE}INFO:${C_RESET} $1"
}

success() {
  echo -e "${C_GREEN}SUCCESS:${C_RESET} $1"
}

warn() {
  echo -e "${C_YELLOW}WARN:${C_RESET} $1"
}

# --- Main Logic ---

# 1. Check for dependencies
if ! command -v gcloud &>/dev/null; then
  echo "Error: gcloud CLI is not installed. Please install it to continue." >&2
  exit 1
fi

# 2. Get Project ID
PROJECT_ID="$1"
if [[ -z "$PROJECT_ID" ]]; then
  read -p "Enter your Google Cloud Project ID: " PROJECT_ID
  if [[ -z "$PROJECT_ID" ]]; then
    echo "Error: Project ID is required." >&2
    exit 1
  fi
fi
info "Using project: ${PROJECT_ID}"

# 3. Get Git Repository Name (with default)
readonly DEFAULT_GIT_REPO="nxcc-bridge/nxcc"
read -p "Enter the Git repository (OWNER/REPO) [${DEFAULT_GIT_REPO}]: " GIT_REPO
# If the user just presses Enter, use the default value.
GIT_REPO="${GIT_REPO:-$DEFAULT_GIT_REPO}"
info "Configuring for repository: ${GIT_REPO}"


# 4. Authenticate user if necessary
if ! gcloud auth list --filter=status:ACTIVE --format="value(account)" | grep -q "."; then
  info "You are not logged in. Opening browser for authentication..."
  gcloud auth login
  gcloud auth application-default login
fi
info "Authenticated successfully."

# 5. Enable required APIs
info "Enabling required Google Cloud APIs..."
readonly APIS_TO_ENABLE=(
  "iam.googleapis.com"
  "iamcredentials.googleapis.com"
  "artifactregistry.googleapis.com"
)
for API in "${APIS_TO_ENABLE[@]}"; do
  if ! gcloud services list --enabled --project="${PROJECT_ID}" --format="value(config.name)" | grep -q "^${API}$"; then
    gcloud services enable "${API}" --project="${PROJECT_ID}"
    success "Enabled API: ${API}"
  else
    warn "API ${API} is already enabled."
  fi
done

# 6. Create Service Account
SA_EMAIL="${SERVICE_ACCOUNT_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"
info "Checking for Service Account: ${SA_EMAIL}"
if ! gcloud iam service-accounts describe "${SA_EMAIL}" --project="${PROJECT_ID}" &>/dev/null; then
  info "Creating Service Account..."
  gcloud iam service-accounts create "${SERVICE_ACCOUNT_NAME}" \
    --display-name="CI/CD Service Account for nXCC" \
    --project="${PROJECT_ID}"
  success "Created Service Account: ${SA_EMAIL}"
else
  warn "Service Account ${SA_EMAIL} already exists."
fi

# 7. Create Workload Identity Pool
info "Checking for Workload Identity Pool: ${WIF_POOL_ID}"
if ! gcloud iam workload-identity-pools describe "${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" &>/dev/null; then
  info "Creating Workload Identity Pool..."
  gcloud iam workload-identity-pools create "${WIF_POOL_ID}" \
    --location="global" \
    --display-name="nXCC CI/CD Pool" \
    --project="${PROJECT_ID}"
  success "Created Workload Identity Pool: ${WIF_POOL_ID}"
else
  warn "Workload Identity Pool ${WIF_POOL_ID} already exists."
fi

# 8. Create or Update Workload Identity Provider for the Pool
WIF_POOL_FULL_NAME=$(gcloud iam workload-identity-pools describe "${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" --format="value(name)")
info "Checking for Workload Identity Provider: ${WIF_PROVIDER_ID}"
if ! gcloud iam workload-identity-pools providers describe "${WIF_PROVIDER_ID}" --workload-identity-pool="${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" &>/dev/null; then
  info "Creating Workload Identity Provider for repo ${GIT_REPO}..."
  gcloud iam workload-identity-pools providers create-oidc "${WIF_PROVIDER_ID}" \
    --project="${PROJECT_ID}" \
    --workload-identity-pool="${WIF_POOL_ID}" \
    --location="global" \
    --issuer-uri="https://token.actions.githubusercontent.com" \
    --attribute-mapping="google.subject=assertion.sub,attribute.repository=assertion.repository" \
    --attribute-condition="attribute.repository == '${GIT_REPO}'"
  success "Created Workload Identity Provider."
else
  warn "Workload Identity Provider ${WIF_PROVIDER_ID} already exists. Updating condition..."
  # If the provider exists, we update it to ensure the condition matches the selected repo.
  gcloud iam workload-identity-pools providers update-oidc "${WIF_PROVIDER_ID}" \
    --project="${PROJECT_ID}" \
    --workload-identity-pool="${WIF_POOL_ID}" \
    --location="global" \
    --attribute-condition="attribute.repository == '${GIT_REPO}'"
  success "Updated provider condition for repo ${GIT_REPO}."
fi

# 9. Grant Service Account permission to be impersonated by the WIF provider
info "Granting Workload Identity User role to the Git repository..."
# This command is additive and idempotent.
gcloud iam service-accounts add-iam-policy-binding "${SA_EMAIL}" \
  --project="${PROJECT_ID}" \
  --role="roles/iam.workloadIdentityUser" \
  --member="principalSet://iam.googleapis.com/${WIF_POOL_FULL_NAME}/attribute.repository/${GIT_REPO}" \
  2> >(grep -v "already exists" >&2) || true
success "Permission granted for ${GIT_REPO} to impersonate ${SA_EMAIL}."

# 10. Create Artifact Registry repository
info "Checking for Artifact Registry repository: ${AR_REPO_NAME}"
if ! gcloud artifacts repositories describe "${AR_REPO_NAME}" --location="${GCP_LOCATION}" --project="${PROJECT_ID}" &>/dev/null; then
  info "Creating Artifact Registry repository in ${GCP_LOCATION}..."
  gcloud artifacts repositories create "${AR_REPO_NAME}" \
    --repository-format="docker" \
    --location="${GCP_LOCATION}" \
    --description="Docker images for nXCC" \
    --project="${PROJECT_ID}"
  success "Created Artifact Registry repository: ${AR_REPO_NAME}"
else
  warn "Artifact Registry repository ${AR_REPO_NAME} already exists."
fi

# 11. Grant Service Account permission to push to Artifact Registry
info "Granting Artifact Registry Writer role to the Service Account..."
gcloud artifacts repositories add-iam-policy-binding "${AR_REPO_NAME}" \
  --location="${GCP_LOCATION}" \
  --project="${PROJECT_ID}" \
  --role="roles/artifactregistry.writer" \
  --member="serviceAccount:${SA_EMAIL}" \
  2> >(grep -v "already exists" >&2) || true
success "Permission granted for ${SA_EMAIL} to write to the repository."

# --- Final Output ---
WIF_PROVIDER_FULL_NAME=$(gcloud iam workload-identity-pools providers describe "${WIF_PROVIDER_ID}" --workload-identity-pool="${WIF_POOL_ID}" --location="global" --project="${PROJECT_ID}" --format="value(name)")

echo -e "\n\n${C_GREEN}================================================================"
echo -e "          Google Cloud Setup Complete!"
echo -e "================================================================${C_RESET}"
echo -e "\nAdd the following as secrets to your ${C_YELLOW}${GIT_REPO}${C_RESET} repository:"
echo -e "----------------------------------------------------------------\n"
echo -e "${C_BLUE}GCP_PROJECT_ID:${C_RESET}"
echo -e "${PROJECT_ID}\n"
echo -e "${C_BLUE}GCP_WORKLOAD_IDENTITY_PROVIDER:${C_RESET}"
echo -e "${WIF_PROVIDER_FULL_NAME}\n"
echo -e "${C_BLUE}GCP_SERVICE_ACCOUNT:${C_RESET}"
echo -e "${SA_EMAIL}\n"
echo -e "----------------------------------------------------------------"
echo -e "Your Artifact Registry host will be: ${C_YELLOW}${GCP_LOCATION}-docker.pkg.dev${C_RESET}"
echo -e "The full image name prefix will be: ${C_YELLOW}${GCP_LOCATION}-docker.pkg.dev/${PROJECT_ID}/${AR_REPO_NAME}${C_RESET}"
echo -e "\n"
