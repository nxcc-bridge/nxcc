#!/bin/bash
#
# Cluster management functions for E2E tests
#

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Source infra common.sh for LOCAL_IMAGE_NAME and LOCAL_IMAGE_TAG constants
source "$(dirname "${BASH_SOURCE[0]}")/../../infra/lib/common.sh"

# Setup local kind cluster
setup_local_cluster() {
	local project_root="$1"
	local skip_setup="${2:-false}"

	if [[ "$skip_setup" == "true" ]]; then
		log "Skipping local cluster setup as requested"
		return 0
	fi

	log "Setting up local kind cluster..."

	# Handle Docker image preparation based on CI mode
	if [[ "${E2E_CI_MODE:-false}" == "true" && -n "${E2E_PREBUILT_IMAGE:-}" ]]; then
		verbose_log "Using pre-built Docker image from CI: $E2E_PREBUILT_IMAGE"

		# Pull and retag the pre-built image for local use
		verbose_log "Pulling pre-built image..."
		if ! docker pull "$E2E_PREBUILT_IMAGE"; then
			error "Failed to pull pre-built image: $E2E_PREBUILT_IMAGE"
		fi

		# Get the local image name from common.sh or infra/lib/common.sh
		local local_image="${LOCAL_IMAGE_NAME:-nxcc-node-local}:${LOCAL_IMAGE_TAG:-latest}"
		verbose_log "Retagging to local image name: $local_image"
		if ! docker tag "$E2E_PREBUILT_IMAGE" "$local_image"; then
			error "Failed to retag image to: $local_image"
		fi

		success "Pre-built Docker image prepared for local deployment"
	else
		# Build local image first with timeout (existing behavior)
		verbose_log "Building local Docker image..."
		local build_timeout="${E2E_DOCKER_BUILD_TIMEOUT:-900}"

		if ! (cd "$project_root" && timeout "$build_timeout" ./infra/infra.sh build local); then
			if [[ $? -eq 124 ]]; then
				error "Docker build timed out after ${build_timeout} seconds"
			else
				error "Docker build failed"
			fi
		fi
	fi

	# Create kind cluster
	verbose_log "Creating kind cluster..."
	(cd "$project_root" && ./infra/infra.sh cluster create kind)

	# Deploy to debug environment (kind) with node variations for e2e testing
	verbose_log "Deploying NXCC to debug environment with node variations..."
	(cd "$project_root" && ./infra/infra.sh k8s deploy debug)

	# Deploy additional node variations for e2e testing
	verbose_log "Deploying node variations for e2e testing..."
	(cd "$project_root" && ./infra/infra.sh k8s deploy-variations debug "untrusted:nodeVariations.operatorKeys.untrusted.enabled=false" "non-confidential:nodeVariations.confidentialOverrides.test.enabled=false")

	# Wait for pods to be ready (main deployment)
	wait_for_pods "debug" 300

	# Wait for variant pods to be ready
	wait_for_variant_pods "debug" 300

	success "Local cluster setup complete"
}

# Setup GKE staging cluster
setup_staging_cluster() {
	local project_root="$1"
	local skip_setup="${2:-false}"

	if [[ "$skip_setup" == "true" ]]; then
		log "Skipping staging cluster setup as requested"
		return 0
	fi

	log "Setting up GKE staging cluster..."

	# Build and push GCP image with timeout
	verbose_log "Building and pushing GCP image..."
	local build_timeout="${E2E_DOCKER_BUILD_TIMEOUT:-900}"
	if ! (cd "$project_root" && timeout "$build_timeout" ./infra/infra.sh build gcp); then
		if [[ $? -eq 124 ]]; then
			error "Docker build timed out after ${build_timeout} seconds"
		else
			error "Docker build failed"
		fi
	fi

	# Create GKE cluster if needed
	verbose_log "Creating GKE cluster..."
	(cd "$project_root" && ./infra/infra.sh cluster create gke)

	# Deploy to staging environment
	verbose_log "Deploying NXCC to staging environment..."
	(cd "$project_root" && ./infra/infra.sh k8s deploy staging)

	# Wait for pods to be ready
	wait_for_pods "staging" 600

	success "Staging cluster setup complete"
}

# Setup production cluster
setup_prod_cluster() {
	local project_root="$1"
	local skip_setup="${2:-false}"

	if [[ "$skip_setup" == "true" ]]; then
		log "Skipping production cluster setup as requested"
		return 0
	fi

	log "Setting up GKE production cluster..."

	# Build and push GCP image with timeout
	verbose_log "Building and pushing GCP image..."
	local build_timeout="${E2E_DOCKER_BUILD_TIMEOUT:-900}"
	if ! (cd "$project_root" && timeout "$build_timeout" ./infra/infra.sh build gcp); then
		if [[ $? -eq 124 ]]; then
			error "Docker build timed out after ${build_timeout} seconds"
		else
			error "Docker build failed"
		fi
	fi

	# Create GKE cluster if needed (same as staging for now)
	verbose_log "Creating GKE cluster..."
	(cd "$project_root" && ./infra/infra.sh cluster create gke)

	# Deploy to production environment
	verbose_log "Deploying NXCC to production environment..."
	(cd "$project_root" && ./infra/infra.sh k8s deploy prod)

	# Wait for pods to be ready
	wait_for_pods "prod" 600

	success "Production cluster setup complete"
}

# Test connectivity using infra test script
test_connectivity() {
	local env="$1"
	local project_root="$2"

	log "Testing connectivity to $env environment..."

	case "$env" in
	local)
		(cd "$project_root" && ./infra/infra.sh test debug)
		;;
	staging)
		(cd "$project_root" && ./infra/infra.sh test staging)
		;;
	prod)
		(cd "$project_root" && ./infra/infra.sh test prod)
		;;
	*)
		error "Unknown environment: $env"
		;;
	esac

	success "Connectivity test completed for $env environment"
}

# Wait for variant pods to be ready
wait_for_variant_pods() {
	local namespace="$1"
	local timeout="${2:-300}"

	log "Waiting for variant pods in namespace '$namespace' to be ready..."

	# Wait for untrusted variant pods
	if kubectl get pods -n "$namespace" -l "app.kubernetes.io/variant=untrusted" --no-headers 2>/dev/null | grep -q .; then
		verbose_log "Waiting for untrusted variant pods..."
		kubectl wait --for=condition=ready pod -l "app.kubernetes.io/variant=untrusted" -n "$namespace" --timeout="${timeout}s" || warn "Untrusted variant pods may not be ready"
	fi

	# Wait for non-confidential variant pods
	if kubectl get pods -n "$namespace" -l "app.kubernetes.io/variant=non-confidential" --no-headers 2>/dev/null | grep -q .; then
		verbose_log "Waiting for non-confidential variant pods..."
		kubectl wait --for=condition=ready pod -l "app.kubernetes.io/variant=non-confidential" -n "$namespace" --timeout="${timeout}s" || warn "Non-confidential variant pods may not be ready"
	fi

	success "Variant pods in namespace '$namespace' are ready"
}

# List deployed variants and their URLs
list_deployed_variants() {
	local env="$1"
	local project_root="$2"

	log "Listing deployed variants in $env environment..."

	(cd "$project_root" && ./infra/infra.sh k8s list-variants "$env")
}

# Test variant routing functionality
test_variant_routing() {
	local env="$1"

	log "Testing variant routing in $env environment..."

	# Test main deployment
	if quick_http_test "$env" "/w/health" "healthy"; then
		success "Main deployment HTTP test passed"
	else
		warn "Main deployment HTTP test failed"
		return 1
	fi

	# Test untrusted variant if available
	if quick_http_test "$env" "/variant/untrusted/w/health" "healthy"; then
		success "Untrusted variant HTTP test passed"
	else
		verbose_log "Untrusted variant HTTP test failed (variant may not be deployed)"
	fi

	# Test non-confidential variant if available
	if quick_http_test "$env" "/variant/non-confidential/w/health" "healthy"; then
		success "Non-confidential variant HTTP test passed"
	else
		verbose_log "Non-confidential variant HTTP test failed (variant may not be deployed)"
	fi

	success "Variant routing tests completed"
}

# Cleanup cluster resources
cleanup_cluster() {
	local env="$1"
	local project_root="$2"
	local force="${3:-false}"

	if [[ "$force" != "true" ]]; then
		log "Skipping cluster cleanup (use --force-cleanup to enable)"
		return 0
	fi

	log "Cleaning up $env cluster..."

	case "$env" in
	local)
		(cd "$project_root" && ./infra/infra.sh k8s destroy debug)
		(cd "$project_root" && ./infra/infra.sh cluster destroy kind)
		;;
	staging)
		(cd "$project_root" && ./infra/infra.sh k8s destroy staging)
		# Don't destroy GKE cluster automatically as it's expensive to recreate
		;;
	prod)
		(cd "$project_root" && ./infra/infra.sh k8s destroy prod)
		# Don't destroy GKE cluster automatically as it's expensive to recreate
		;;
	*)
		error "Unknown environment: $env"
		;;
	esac

	success "Cluster cleanup completed for $env environment"
}
