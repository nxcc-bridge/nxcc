#!/bin/bash
#
# Functions for managing Kubernetes clusters (GKE and KinD).
# This script is intended to be sourced, not executed directly.

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
