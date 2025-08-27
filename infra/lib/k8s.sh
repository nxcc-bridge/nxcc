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
	staging | prod)
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

	# Handle global operator key configuration
	if [[ "${NXCC_OPERATOR_KEY_ENABLED:-false}" == "true" ]]; then
		helm_set_args+=(--set operatorKey.enabled=true)
		helm_set_args+=(--set operatorKey.secretName="${NXCC_OPERATOR_KEY_SECRET_NAME:-nxcc-operator-key}")

		if [[ "${NXCC_OPERATOR_KEY_CREATE_SECRET:-false}" == "true" ]] && [[ -n "${NXCC_OPERATOR_KEY_DATA:-}" ]]; then
			helm_set_args+=(--set operatorKey.createSecret=true)
			helm_set_args+=(--set operatorKey.privateKeyData="${NXCC_OPERATOR_KEY_DATA}")
		fi
	fi

	case "$env" in
	debug)
		# For local KinD cluster. Use GHCR by default, allow local override.
		local image_repo="${IMAGE_REPO_OVERRIDE:-ghcr.io/nxcc-bridge/node}"
		local image_tag="${IMAGE_TAG_OVERRIDE:-latest}"
		local full_image="${image_repo}:${image_tag}"

		# Only try to load into KinD if using a local image
		if [[ "$image_repo" == *"local"* ]] || [[ "$image_repo" != *"ghcr.io"* && "$image_repo" != *"docker.pkg.dev"* ]]; then
			info "Loading local Docker image '${full_image}' into KinD cluster..."
			check_deps kind
			kind load docker-image --name "nxcc-debug" "${full_image}"
		else
			info "Using remote image: ${full_image}"
		fi

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

		# 2. Resource configuration - use minimal resources for GitHub Actions
		if [[ "${E2E_MINIMAL_RESOURCES:-false}" == "true" ]]; then
			# Ultra-minimal resources for GitHub Actions (<1 CPU total)
			info "Using minimal resource configuration for CI environment"
			helm_set_args+=(--set seed.resources.requests.cpu=25m)
			helm_set_args+=(--set seed.resources.requests.memory=64Mi)
			helm_set_args+=(--set seed.resources.limits.cpu=50m)
			helm_set_args+=(--set seed.resources.limits.memory=128Mi)
			helm_set_args+=(--set worker.resources.requests.cpu=50m)
			helm_set_args+=(--set worker.resources.requests.memory=128Mi)
			helm_set_args+=(--set worker.resources.limits.cpu=100m)
			helm_set_args+=(--set worker.resources.limits.memory=256Mi)
		else
			# Standard KinD resources for local development
			helm_set_args+=(--set seed.resources.requests.cpu=250m)
			helm_set_args+=(--set seed.resources.requests.memory=256Mi)
			helm_set_args+=(--set seed.resources.limits.memory=512Mi)
			helm_set_args+=(--set worker.resources.requests.cpu=500m)
			helm_set_args+=(--set worker.resources.requests.memory=512Mi)
			helm_set_args+=(--set worker.resources.limits.memory=1Gi)
		fi
		;;
	staging | prod)
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
			helm_set_args+=(--set 'ingress.hosts[0].host=staging.nxcc.example.com')
		else # prod
			helm_set_args+=(--set confidential.enabled=true)
			helm_set_args+=(--set seed.replicaCount=3)
			helm_set_args+=(--set worker.replicaCount=1)
			helm_set_args+=(--set ingress.enabled=true)
			helm_set_args+=(--set 'ingress.hosts[0].host=prod.nxcc.example.com')
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

	# Display access information for the main deployment
	info "Main deployment access information:"
	# Wait a moment for ingress to be created
	sleep 2
	local ingress_hosts
	ingress_hosts=$(kubectl get ingress "${helm_release_name}" -n "${namespace}" -o jsonpath='{.spec.rules[*].host}' 2>/dev/null || echo "")

	if [[ -n "$ingress_hosts" ]]; then
		for host in $ingress_hosts; do
			local protocol="https"
			if [[ "$env" == "debug" ]]; then protocol="http"; fi
			info "Main worker URL: ${protocol}://${host}/"
			info "Main seed URL: ${protocol}://${host}/seed"
		done
	else
		info "Ingress not yet available. Use 'kubectl get ingress -n ${namespace}' to check status."
	fi

	info "Use 'kubectl get all -n ${namespace}' to check status."
}

################################################################################
# Deploys multiple NXCC nodes with different variations for e2e testing.
# Arguments:
#   $1: The environment to deploy to ('debug', 'staging', 'prod').
#   $2+: Node variation configurations in format "variant-name:key1=value1,key2=value2"
# Examples:
#   k8s_deploy_variations debug "untrusted:operatorKeys.untrusted.enabled=true"
#   k8s_deploy_variations debug "non-confidential:confidentialOverrides.test.enabled=false"
################################################################################
k8s_deploy_variations() {
	local env="$1"
	shift
	local variations=("$@")

	info "Deploying NXCC nodes with variations for e2e testing in '${env}' environment..."

	# First deploy the base node
	k8s_deploy "$env"

	# Then deploy each variation as a separate release
	for variation_spec in "${variations[@]}"; do
		local variant_name
		variant_name=$(echo "$variation_spec" | cut -d':' -f1)
		local variant_overrides
		variant_overrides=$(echo "$variation_spec" | cut -d':' -f2-)

		if [[ -z "$variant_name" ]]; then
			warn "Skipping invalid variation spec: $variation_spec"
			continue
		fi

		info "Deploying node variant: $variant_name"
		k8s_deploy_variant "$env" "$variant_name" "$variant_overrides"
	done
}

################################################################################
# Deploys a single NXCC node variant for e2e testing.
# Arguments:
#   $1: The environment to deploy to ('debug', 'staging', 'prod').
#   $2: The variant name (used for release naming).
#   $3: Comma-separated helm overrides (optional).
################################################################################
k8s_deploy_variant() {
	local env="$1"
	local variant_name="$2"
	local variant_overrides="${3:-}"
	local helm_release_name="nxcc-node-${env}-${variant_name}"
	local namespace="${env}"
	local helm_set_args=()
	local helm_timeout="${HELM_TIMEOUT:-5m}"

	info "Deploying NXCC node variant '${variant_name}' to '${env}' environment..."
	check_deps helm kubectl

	if [ ! -d "${HELM_CHART_PATH}" ]; then
		error "Helm chart not found at '${HELM_CHART_PATH}'. Please create it first."
	fi

	# Set the node variant for template processing
	helm_set_args+=(--set nodeVariant="${variant_name}")

	# Enable variant routing for addressability
	helm_set_args+=(--set variantRouting.enabled=true)

	# Parse and apply variant-specific overrides
	if [[ -n "$variant_overrides" ]]; then
		IFS=',' read -ra overrides <<<"$variant_overrides"
		for override in "${overrides[@]}"; do
			if [[ "$override" == *"="* ]]; then
				helm_set_args+=(--set "$override")
			else
				warn "Skipping invalid override: $override"
			fi
		done
	fi

	# Set path-based routing for the variant
	helm_set_args+=(--set variantRouting.pathPrefix="/variant/${variant_name}")
	helm_set_args+=(--set ingress.enabled=true)

	# Copy base environment configuration
	case "$env" in
	debug)
		local image_repo="${IMAGE_REPO_OVERRIDE:-ghcr.io/nxcc-bridge/node}"
		local image_tag="${IMAGE_TAG_OVERRIDE:-latest}"

		helm_set_args+=(--set seed.replicaCount=1)
		helm_set_args+=(--set worker.replicaCount=1)
		helm_set_args+=(--set image.repository="${image_repo}")
		helm_set_args+=(--set image.tag="${image_tag}")
		helm_set_args+=(--set image.pullPolicy=IfNotPresent)
		helm_set_args+=(--set seed.topologySpread.enabled=false)
		helm_set_args+=(--set ingress.className="nginx") # Use nginx for debug

		if [[ "${E2E_MINIMAL_RESOURCES:-false}" == "true" ]]; then
			helm_set_args+=(--set seed.resources.requests.cpu=25m)
			helm_set_args+=(--set seed.resources.requests.memory=64Mi)
			helm_set_args+=(--set seed.resources.limits.cpu=50m)
			helm_set_args+=(--set seed.resources.limits.memory=128Mi)
			helm_set_args+=(--set worker.resources.requests.cpu=50m)
			helm_set_args+=(--set worker.resources.requests.memory=128Mi)
			helm_set_args+=(--set worker.resources.limits.cpu=100m)
			helm_set_args+=(--set worker.resources.limits.memory=256Mi)
		fi
		;;
	staging | prod)
		# Use default values for staging/prod
		;;
	esac

	# Create namespace if it doesn't exist
	kubectl create namespace "${namespace}" --dry-run=client -o yaml | kubectl apply -f -

	# Deploy the variant
	info "Deploying Helm chart with variant '${variant_name}'..."
	if helm upgrade --install "${helm_release_name}" "${HELM_CHART_PATH}" \
		--namespace="${namespace}" \
		--timeout="${helm_timeout}" \
		--wait \
		"${helm_set_args[@]}"; then
		success "Helm chart '${helm_release_name}' deployed successfully to '${env}' environment."
	else
		error "Failed to deploy Helm chart '${helm_release_name}' to '${env}' environment."
	fi

	# Wait for pods to be ready
	info "Waiting for variant '${variant_name}' pods to be ready in namespace '${namespace}'..."
	if kubectl wait --for=condition=ready pod -l "app.kubernetes.io/instance=${helm_release_name}" -n "${namespace}" --timeout=300s; then
		success "Variant '${variant_name}' pods in namespace '${namespace}' are ready."
	else
		warn "Some variant '${variant_name}' pods in namespace '${namespace}' may not be ready. Check status manually."
	fi

	# Display access information for the variant
	info "Variant '${variant_name}' deployed successfully."

	# Wait a moment for ingress to be created, then show access URLs
	sleep 2
	local ingress_hosts
	ingress_hosts=$(kubectl get ingress "${helm_release_name}-${variant_name}-worker" -n "${namespace}" -o jsonpath='{.spec.rules[*].host}' 2>/dev/null || echo "")

	if [[ -n "$ingress_hosts" ]]; then
		for host in $ingress_hosts; do
			local protocol="https"
			if [[ "$env" == "debug" ]]; then protocol="http"; fi

			info "Worker access URL: ${protocol}://${host}/variant/${variant_name}"
			info "  - Use this URL to send requests directly to the '${variant_name}' worker variant"
			info "Seed access URL: ${protocol}://${host}/variant/${variant_name}/seed"
			info "  - Use this URL to send requests directly to the '${variant_name}' seed variant"
		done
	else
		info "Ingress not yet available. Use 'kubectl get ingress -n ${namespace}' to check status."
	fi

	info "Use 'kubectl get all -l app.kubernetes.io/instance=${helm_release_name} -n ${namespace}' to check status."
}

################################################################################
# Lists all deployed node variants and their access URLs.
# Arguments:
#   $1: The environment to list variants for ('debug', 'staging', 'prod').
################################################################################
k8s_list_variants() {
	local env="$1"
	local namespace="${env}"

	info "Listing all deployed NXCC nodes in '${env}' environment..."

	# First show the main deployment
	local base_release="nxcc-node-${env}"
	if helm status "${base_release}" -n "${namespace}" &>/dev/null; then
		echo "Main deployment:"
		local ingress_hosts
		ingress_hosts=$(kubectl get ingress "${base_release}" -n "${namespace}" -o jsonpath='{.spec.rules[*].host}' 2>/dev/null || echo "")
		if [[ -n "$ingress_hosts" ]]; then
			for host in $ingress_hosts; do
				local protocol="https"
				if [[ "$env" == "debug" ]]; then protocol="http"; fi
				echo "  Worker URL: ${protocol}://${host}/"
				echo "  Seed URL: ${protocol}://${host}/seed"
			done
		else
			echo "  No ingress configured - check service configuration"
		fi
		echo ""
	fi

	# Find all helm releases that match the variant pattern
	local variant_releases
	variant_releases=$(helm list -n "${namespace}" -o json 2>/dev/null | jq -r ".[] | select(.name | startswith(\"${base_release}-\")) | .name" 2>/dev/null || echo "")

	if [[ -z "$variant_releases" ]]; then
		info "No node variants found in '${env}' environment."
		return 0
	fi

	echo "Deployed variants:"
	for release in $variant_releases; do
		local variant_name="${release#"${base_release}"-}"
		echo "  - Variant: ${variant_name}"

		local worker_ingress_hosts
		worker_ingress_hosts=$(kubectl get ingress "${release}-worker" -n "${namespace}" -o jsonpath='{.spec.rules[*].host}' 2>/dev/null || echo "")
		if [[ -n "$worker_ingress_hosts" ]]; then
			for host in $worker_ingress_hosts; do
				local protocol="https"
				if [[ "$env" == "debug" ]]; then protocol="http"; fi
				echo "    Worker URL: ${protocol}://${host}/variant/${variant_name}"
				echo "    Seed URL: ${protocol}://${host}/variant/${variant_name}/seed"
			done
		else
			echo "    No ingress configured - check deployment status"
		fi
	done
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
