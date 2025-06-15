#!/bin/bash
#
# Functions for managing the application deployment via Helm.
# This script is intended to be sourced, not executed directly.

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
      # For local KinD cluster. Overridable for CI.
      local image_repo="${IMAGE_REPO_OVERRIDE:-${LOCAL_IMAGE_NAME}}"
      local image_tag="${IMAGE_TAG_OVERRIDE:-${LOCAL_IMAGE_TAG}}"

      helm_set_args+=(--set confidential.enabled=false)
      helm_set_args+=(--set seed.replicaCount=1)
      helm_set_args+=(--set worker.replicaCount=1)
      helm_set_args+=(--set ingress.enabled=false)
      helm_set_args+=(--set worker.service.type=NodePort)
      helm_set_args+=(--set image.repository="${image_repo}")
      helm_set_args+=(--set image.tag="${image_tag}")
      helm_set_args+=(--set image.pullPolicy=Always)
      ;;
    staging|prod)
      # For GKE cluster. Identity must be resolved.
      resolve_gcp_identity

      # Default to 'latest' for staging, but allow override for prod tags.
      local image_tag="${IMAGE_TAG_OVERRIDE:-latest}"

      helm_set_args+=(--set image.repository="${GCP_AR_LOCATION}-docker.pkg.dev/${PROJECT_ID}/${AR_REPO_NAME}/node")
      helm_set_args+=(--set image.tag="${image_tag}")

      if [ "$env" == "staging" ]; then
        helm_set_args+=(--set confidential.enabled=false)
        helm_set_args+=(--set seed.replicaCount=1)
        helm_set_args+=(--set worker.replicaCount=1)
        helm_set_args+=(--set ingress.enabled=true)
        helm_set_args+=(--set ingress.hosts[0].host="staging.nxcc.example.com")
      else # prod
        helm_set_args+=(--set confidential.enabled=true)
        helm_set_args+=(--set seed.replicaCount=3)
        helm_set_args+=(--set worker.replicaCount=1)
        helm_set_args+=(--set ingress.enabled=true)
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
