#!/bin/bash
#
# Functions for managing CI/CD resources (Service Account, WIF, Artifact Registry).
# This script is intended to be sourced, not executed directly.

################################################################################
# Manages CI/CD resources (Service Account, WIF, Artifact Registry).
# Globals:
#   RESOLVED_PROJECT_ID, RESOLVED_GCP_ACCOUNT, SERVICE_ACCOUNT_NAME, WIF_POOL_ID, WIF_PROVIDER_ID,
#   AR_REPO_NAME, GCP_AR_LOCATION
# Arguments:
#   None
################################################################################
cicd_setup() {
	info "Starting CI/CD resource setup..."

	if [[ "${AUTO_YES:-false}" == "true" ]]; then
		GIT_REPO="nxcc-bridge/nxcc"
		info "Using default repository: ${GIT_REPO}"
	else
		read -r -p "Enter the Git repository (OWNER/REPO) [nxcc-bridge/nxcc]: " GIT_REPO
		GIT_REPO="${GIT_REPO:-nxcc-bridge/nxcc}"
		info "Configuring for repository: ${GIT_REPO}"
	fi

	info "Enabling required Google Cloud APIs for CI/CD..."
	local apis_to_enable=(
		"iam.googleapis.com"
		"iamcredentials.googleapis.com"
		"artifactregistry.googleapis.com"
		"cloudresourcemanager.googleapis.com" # Needed for WIF pool creation
	)
	for api in "${apis_to_enable[@]}"; do
		if ! gcloud services list --enabled --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(config.name)" | grep -q "^${api}$"; then
			gcloud services enable "${api}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}"
			success "Enabled API: ${api}"
		else
			warn "API ${api} is already enabled."
		fi
	done

	local sa_email="${SERVICE_ACCOUNT_NAME}@${RESOLVED_PROJECT_ID}.iam.gserviceaccount.com"
	info "Checking for Service Account: ${sa_email}"
	if ! gcloud iam service-accounts describe "${sa_email}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		gcloud iam service-accounts create "${SERVICE_ACCOUNT_NAME}" \
			--display-name="CI/CD Service Account for nXCC" \
			--project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}"
		success "Created Service Account: ${sa_email}"
	else
		warn "Service Account ${sa_email} already exists."
	fi

	info "Checking for Workload Identity Pool: ${WIF_POOL_ID}"
	if ! gcloud iam workload-identity-pools describe "${WIF_POOL_ID}" --location="global" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		gcloud iam workload-identity-pools create "${WIF_POOL_ID}" \
			--location="global" \
			--display-name="nXCC CI/CD Pool" \
			--project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}"
		success "Created Workload Identity Pool: ${WIF_POOL_ID}"
	else
		warn "Workload Identity Pool ${WIF_POOL_ID} already exists."
	fi

	local wif_pool_full_name
	wif_pool_full_name=$(gcloud iam workload-identity-pools describe "${WIF_POOL_ID}" --location="global" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(name)")
	info "Checking for Workload Identity Provider: ${WIF_PROVIDER_ID}"
	if ! gcloud iam workload-identity-pools providers describe "${WIF_PROVIDER_ID}" --workload-identity-pool="${WIF_POOL_ID}" --location="global" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		gcloud iam workload-identity-pools providers create-oidc "${WIF_PROVIDER_ID}" \
			--project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" \
			--workload-identity-pool="${WIF_POOL_ID}" \
			--location="global" \
			--issuer-uri="https://token.actions.githubusercontent.com" \
			--attribute-mapping="google.subject=assertion.sub,attribute.repository=assertion.repository" \
			--attribute-condition="attribute.repository == '${GIT_REPO}'"
		success "Created Workload Identity Provider."
	else
		warn "Workload Identity Provider ${WIF_PROVIDER_ID} already exists. Updating condition..."
		gcloud iam workload-identity-pools providers update-oidc "${WIF_PROVIDER_ID}" \
			--project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" \
			--workload-identity-pool="${WIF_POOL_ID}" \
			--location="global" \
			--attribute-condition="attribute.repository == '${GIT_REPO}'"
		success "Updated provider condition for repo ${GIT_REPO}."
	fi

	info "Granting Workload Identity User role to the Git repository..."
	gcloud iam service-accounts add-iam-policy-binding "${sa_email}" \
		--project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" \
		--role="roles/iam.workloadIdentityUser" \
		--member="principalSet://iam.googleapis.com/${wif_pool_full_name}/attribute.repository/${GIT_REPO}" \
		2> >(grep -v "already exists" >&2) || true
	success "Permission granted for ${GIT_REPO} to impersonate ${sa_email}."

	info "Checking for Artifact Registry repository: ${AR_REPO_NAME}"
	if ! gcloud artifacts repositories describe "${AR_REPO_NAME}" --location="${GCP_AR_LOCATION}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		gcloud artifacts repositories create "${AR_REPO_NAME}" \
			--repository-format="docker" \
			--location="${GCP_AR_LOCATION}" \
			--description="Docker images for nXCC" \
			--project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}"
		success "Created Artifact Registry repository: ${AR_REPO_NAME}"
	else
		warn "Artifact Registry repository ${AR_REPO_NAME} already exists."
	fi

	info "Granting Artifact Registry Writer role to the Service Account..."
	gcloud artifacts repositories add-iam-policy-binding "${AR_REPO_NAME}" \
		--location="${GCP_AR_LOCATION}" \
		--project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" \
		--role="roles/artifactregistry.writer" \
		--member="serviceAccount:${sa_email}" \
		2> >(grep -v "already exists" >&2) || true
	success "Permission granted for ${sa_email} to write to the repository."

	local wif_provider_full_name
	wif_provider_full_name=$(gcloud iam workload-identity-pools providers describe "${WIF_PROVIDER_ID}" --workload-identity-pool="${WIF_POOL_ID}" --location="global" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(name)")

	echo -e "\n\n${C_GREEN}================================================================"
	echo -e "          CI/CD Setup Complete!"
	echo -e "================================================================${C_RESET}"
	echo -e "\nAdd the following as secrets to your ${C_YELLOW}${GIT_REPO}${C_RESET} repository:"
	echo -e "----------------------------------------------------------------\n"
	echo -e "${C_BLUE}GCP_RESOLVED_PROJECT_ID:${C_RESET} ${RESOLVED_PROJECT_ID}"
	echo -e "${C_BLUE}GCP_WORKLOAD_IDENTITY_PROVIDER:${C_RESET} ${wif_provider_full_name}"
	echo -e "${C_BLUE}GCP_SERVICE_ACCOUNT:${C_RESET} ${sa_email}"
	echo -e "\nYour Artifact Registry host is: ${C_YELLOW}${GCP_AR_LOCATION}-docker.pkg.dev${C_RESET}"
}

################################################################################
# Tears down CI/CD resources.
################################################################################
cicd_teardown() {
	info "Starting CI/CD resource teardown..."
	local sa_email="${SERVICE_ACCOUNT_NAME}@${RESOLVED_PROJECT_ID}.iam.gserviceaccount.com"

	info "Removing Artifact Registry IAM policy..."
	gcloud artifacts repositories remove-iam-policy-binding "${AR_REPO_NAME}" \
		--location="${GCP_AR_LOCATION}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" \
		--role="roles/artifactregistry.writer" --member="serviceAccount:${sa_email}" \
		2> >(grep -v "does not exist" >&2) || true
	success "Artifact Registry policy binding removed."

	info "Deleting Artifact Registry repository: ${AR_REPO_NAME}"
	if gcloud artifacts repositories describe "${AR_REPO_NAME}" --location="${GCP_AR_LOCATION}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		gcloud artifacts repositories delete "${AR_REPO_NAME}" --location="${GCP_AR_LOCATION}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --quiet
		success "Deleted Artifact Registry repository."
	else
		warn "Artifact Registry repository does not exist."
	fi

	info "Deleting Workload Identity Provider: ${WIF_PROVIDER_ID}"
	if gcloud iam workload-identity-pools providers describe "${WIF_PROVIDER_ID}" --workload-identity-pool="${WIF_POOL_ID}" --location="global" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		gcloud iam workload-identity-pools providers delete "${WIF_PROVIDER_ID}" \
			--workload-identity-pool="${WIF_POOL_ID}" --location="global" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --quiet
		success "Deleted Workload Identity Provider."
	else
		warn "Workload Identity Provider does not exist."
	fi

	info "Deleting Workload Identity Pool: ${WIF_POOL_ID}"
	if gcloud iam workload-identity-pools describe "${WIF_POOL_ID}" --location="global" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		gcloud iam workload-identity-pools delete "${WIF_POOL_ID}" --location="global" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --quiet
		success "Deleted Workload Identity Pool."
	else
		warn "Workload Identity Pool does not exist."
	fi

	info "Deleting Service Account: ${sa_email}"
	if gcloud iam service-accounts describe "${sa_email}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		gcloud iam service-accounts delete "${sa_email}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --quiet
		success "Deleted Service Account."
	else
		warn "Service Account does not exist."
	fi

	success "CI/CD teardown complete."
}
