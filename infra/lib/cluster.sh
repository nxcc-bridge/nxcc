#!/bin/bash
#
# Functions for managing GKE cluster.
# This script is intended to be sourced, not executed directly.

################################################################################
# Creates the GKE Autopilot cluster with Confidential Computing.
################################################################################
cluster_create_gke() {
	info "Starting GKE cluster creation..."
	check_deps gcloud
	resolve_gcp_identity

	info "Enabling GKE API..."
	if ! gcloud services list --enabled --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(config.name)" | grep -q "^container.googleapis.com$"; then
		gcloud services enable container.googleapis.com --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}"
		success "Enabled GKE API (container.googleapis.com)."
	else
		warn "GKE API is already enabled."
	fi

	info "Checking for GKE cluster: ${GKE_CLUSTER_NAME}"
	if ! gcloud container clusters describe "${GKE_CLUSTER_NAME}" --region "${GCP_GKE_REGION}" --project "${RESOLVED_PROJECT_ID}" --account "${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		info "Creating GKE Autopilot cluster '${GKE_CLUSTER_NAME}' in '${GCP_GKE_REGION}'..."
		info "This will take several minutes."
		gcloud container clusters create-auto "${GKE_CLUSTER_NAME}" \
			--project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" \
			--region="${GCP_GKE_REGION}" \
			--release-channel="rapid"
		success "Created GKE cluster."
	else
		warn "GKE cluster ${GKE_CLUSTER_NAME} already exists."
	fi

	info "Granting CI/CD Service Account permission to deploy to the cluster..."
	local sa_email="${SERVICE_ACCOUNT_NAME}@${RESOLVED_PROJECT_ID}.iam.gserviceaccount.com"
	gcloud projects add-iam-policy-binding "${RESOLVED_PROJECT_ID}" \
		--account="${RESOLVED_GCP_ACCOUNT}" \
		--member="serviceAccount:${sa_email}" \
		--role="roles/container.developer" \
		2> >(grep -v "already exists" >&2) || true
	success "Granted 'GKE Developer' role to ${sa_email}."

	info "Configuring kubectl to connect to the cluster..."
	gcloud container clusters get-credentials "${GKE_CLUSTER_NAME}" --region "${GCP_GKE_REGION}" --project "${RESOLVED_PROJECT_ID}" --account "${RESOLVED_GCP_ACCOUNT}"
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
	local sa_email="${SERVICE_ACCOUNT_NAME}@${RESOLVED_PROJECT_ID}.iam.gserviceaccount.com"
	gcloud projects remove-iam-policy-binding "${RESOLVED_PROJECT_ID}" \
		--account="${RESOLVED_GCP_ACCOUNT}" \
		--member="serviceAccount:${sa_email}" \
		--role="roles/container.developer" \
		2> >(grep -v "does not exist" >&2) || true
	success "Removed 'GKE Developer' role."

	info "Deleting GKE cluster: ${GKE_CLUSTER_NAME}"
	if gcloud container clusters describe "${GKE_CLUSTER_NAME}" --region "${GCP_GKE_REGION}" --project "${RESOLVED_PROJECT_ID}" --account "${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		info "This will take several minutes."
		gcloud container clusters delete "${GKE_CLUSTER_NAME}" --region "${GCP_GKE_REGION}" --project "${RESOLVED_PROJECT_ID}" --account "${RESOLVED_GCP_ACCOUNT}" --quiet
		success "Deleted GKE cluster."
	else
		warn "GKE cluster ${GKE_CLUSTER_NAME} does not exist."
	fi
}
