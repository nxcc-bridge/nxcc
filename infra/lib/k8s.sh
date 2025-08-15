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
  # Use HELM_TIMEOUT if set, otherwise default to 5m.
  local helm_timeout="${HELM_TIMEOUT:-5m}"
  
  # Verify kubectl context is correct for the environment
  local current_context
  current_context=$(kubectl config current-context)
  case "$env" in
    debug)
      if [[ "$current_context" != "kind-nxcc-debug" ]]; then
        warn "Kubectl context is '$current_context', expected 'kind-nxcc-debug'. Switching context..."
        kubectl config use-context "kind-nxcc-debug"
      fi
      ;;
    staging|prod)
      if [[ "$current_context" != *"nxcc"* ]]; then
        warn "Kubectl context is '$current_context', may not be correct for GKE deployment"
      fi
      ;;
  esac

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
      local full_image="${image_repo}:${image_tag}"

      # For KinD, load the local Docker image directly into the cluster to avoid pull credential issues.
      info "Loading Docker image '${full_image}' into KinD cluster..."
      check_deps kind
      kind load docker-image --name "nxcc-debug" "${full_image}"

      helm_set_args+=(--set confidential.enabled=false)
      helm_set_args+=(--set seed.replicaCount=1)
      helm_set_args+=(--set worker.replicaCount=1)
      helm_set_args+=(--set ingress.enabled=false)
      helm_set_args+=(--set worker.service.type=NodePort)
      helm_set_args+=(--set image.repository="${image_repo}")
      helm_set_args+=(--set image.tag="${image_tag}")
      # Use IfNotPresent so Kubernetes uses the pre-loaded image. 'Always' would ignore it.
      helm_set_args+=(--set image.pullPolicy=IfNotPresent)

      # CI (KinD) specific overrides
      # 1. Disable topology spread as KinD runs on a single node without zone labels.
      helm_set_args+=(--set seed.topologySpread.enabled=false)
      # 2. Reduce resource requests/limits to fit within typical CI runner constraints.
      helm_set_args+=(--set seed.resources.requests.cpu=250m)
      helm_set_args+=(--set seed.resources.requests.memory=256Mi)
      helm_set_args+=(--set seed.resources.limits.memory=512Mi)
      helm_set_args+=(--set worker.resources.requests.cpu=500m)
      helm_set_args+=(--set worker.resources.requests.memory=512Mi)
      helm_set_args+=(--set worker.resources.limits.memory=1Gi)
      ;;
    staging|prod)
      # For GKE cluster. Identity must be resolved.
      resolve_gcp_identity

      # Default to 'latest' for staging, but allow override for prod tags.
      local image_tag="${IMAGE_TAG_OVERRIDE:-latest}"

      helm_set_args+=(--set image.repository="${GCP_AR_LOCATION}-docker.pkg.dev/${RESOLVED_PROJECT_ID}/${AR_REPO_NAME}/node")
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

  info "Deploying/upgrading Helm release '${helm_release_name}' in namespace '${namespace}' with a timeout of ${helm_timeout}."
  helm upgrade "${helm_release_name}" "${HELM_CHART_PATH}" \
    --install \
    --create-namespace \
    --atomic \
    --timeout "${helm_timeout}" \
    --wait \
    --namespace "${namespace}" \
    "${helm_set_args[@]}"

  success "Application deployment to '${env}' complete."
  
  # Wait for pods to be ready
  info "Waiting for pods to be ready in namespace '${namespace}'..."
  if kubectl wait --for=condition=ready pod --all -n "${namespace}" --timeout=300s; then
    success "All pods in namespace '${namespace}' are ready."
  else
    warn "Some pods in namespace '${namespace}' may not be ready. Check status manually."
  fi
  
  info "Use 'kubectl get all -n ${namespace}' to check status."
}

################################################################################
# Dumps diagnostic information from a Kubernetes namespace for debugging.
# Arguments:
#   $1: The environment to get info from ('debug', 'staging', 'prod').
################################################################################
k8s_dump_debug_info() {
  local env="$1"
  local namespace="${env}"
  local helm_release_name="nxcc-node-${env}"

  info "--- Dumping debug information for environment '${env}' ---"

  # Helper to run a command and print a title, continuing if it fails.
  run_and_report() {
    local title="$1"
    shift
    info "--- ${title} ---"
    # Execute command and capture output. Continue on error.
    if ! output=$("$@" 2>&1); then
      warn "Command failed: $*"
      echo "${output}"
    else
      echo "${output}"
    fi
    echo # Add a newline for readability
  }

  run_and_report "Helm Status for ${helm_release_name}" \
    helm status "${helm_release_name}" -n "${namespace}"

  run_and_report "All Resources in Namespace ${namespace}" \
    kubectl get all -n "${namespace}" -o wide

  run_and_report "Describe Pods in Namespace ${namespace}" \
    kubectl describe pods -n "${namespace}"

  run_and_report "Events in Namespace ${namespace} (sorted by time)" \
    kubectl get events -n "${namespace}" --sort-by='.lastTimestamp'

  info "--- Logs from all containers in Namespace ${namespace} ---"
  local pods
  # Redirect stderr to /dev/null to avoid error message if no pods are found
  if ! pods=$(kubectl get pods -n "${namespace}" -o jsonpath='{.items[*].metadata.name}' 2>/dev/null); then
    warn "Could not list pods in namespace ${namespace}."
    return
  fi

  if [ -z "$pods" ]; then
    info "No pods found in namespace ${namespace}."
    return
  fi

  for pod in $pods; do
    run_and_report "Logs for pod: ${pod}" \
      kubectl logs "${pod}" -n "${namespace}" --all-containers=true --tail=200
    run_and_report "Logs for previous instance of pod: ${pod}" \
      kubectl logs "${pod}" -n "${namespace}" --all-containers=true --previous --tail=200
  done

  success "--- Debug information dump complete. ---"
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
