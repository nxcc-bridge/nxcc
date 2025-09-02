#!/bin/bash
#
# Cluster management functions for E2E tests
#

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Source infra common.sh for KIND_CLUSTER_NAME and other constants
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

		# Retag to our standard naming scheme
		local local_image="nxcc-node:${E2E_BUILD_MODE:-debug}"
		verbose_log "Retagging to local image name: $local_image"
		if ! docker tag "$E2E_PREBUILT_IMAGE" "$local_image"; then
			error "Failed to retag image to: $local_image"
		fi

		success "Pre-built Docker image prepared for local deployment"
	else
		# Build local image first with timeout (existing behavior)
		verbose_log "Building local Docker image..."
		local build_timeout="${E2E_DOCKER_BUILD_TIMEOUT:-900}"

		local build_mode="--${E2E_BUILD_MODE:-debug}"
		if ! (cd "$project_root" && timeout "$build_timeout" ./infra/infra.sh image build "$build_mode"); then
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

	# Push image to KinD cluster
	verbose_log "Loading Docker image into KinD cluster..."
	local source_tag="${E2E_BUILD_MODE:-debug}"
	(cd "$project_root" && ./infra/infra.sh image push kind --source="$source_tag")

	# Deploy to debug environment using Terraform
	verbose_log "Deploying NXCC to debug environment..."
	# Use local image for deployment
	export IMAGE_REPO_OVERRIDE="nxcc-node-local"
	export IMAGE_TAG_OVERRIDE="latest"
	(cd "$project_root" && ./infra/infra.sh deploy create e2e-debug)

	# Note: Node variations testing will be handled through Terraform deployment configurations
	verbose_log "Node variations will be configured via Terraform..."

	# Give deployment time to complete
	verbose_log "Giving deployment 30 seconds to complete..."
	sleep 30

	success "Local cluster setup complete"
}

# Setup staging deployment using Terraform
setup_staging_cluster() {
	local project_root="$1"
	local skip_setup="${2:-false}"

	if [[ "$skip_setup" == "true" ]]; then
		log "Skipping staging deployment setup as requested"
		return 0
	fi

	log "Setting up staging deployment using Terraform..."

	# Build and push GCP image with timeout
	local build_mode="--${E2E_BUILD_MODE:-debug}"
	verbose_log "Building ${E2E_BUILD_MODE:-debug} image..."
	local build_timeout="${E2E_DOCKER_BUILD_TIMEOUT:-900}"
	if ! (cd "$project_root" && timeout "$build_timeout" ./infra/infra.sh image build "$build_mode"); then
		if [[ $? -eq 124 ]]; then
			error "Docker build timed out after ${build_timeout} seconds"
		else
			error "Docker build failed"
		fi
	fi

	verbose_log "Pushing ${E2E_BUILD_MODE:-debug} image to GCP..."
	local source_tag="${E2E_BUILD_MODE:-debug}"
	if ! (cd "$project_root" && ./infra/infra.sh image push gcp --source="$source_tag"); then
		error "Docker push failed"
	fi

	# Deploy to staging environment using Terraform
	verbose_log "Deploying NXCC to staging environment..."
	(cd "$project_root" && ./infra/infra.sh deploy create staging)

	# Wait for deployment to be ready
	sleep 60

	success "Staging deployment setup complete"
}

# Setup production deployment using Terraform
setup_prod_cluster() {
	local project_root="$1"
	local skip_setup="${2:-false}"

	if [[ "$skip_setup" == "true" ]]; then
		log "Skipping production deployment setup as requested"
		return 0
	fi

	log "Setting up production deployment using Terraform..."

	# Build and push GCP image with timeout
	local build_mode="--${E2E_BUILD_MODE:-release}" # Production defaults to release
	verbose_log "Building ${E2E_BUILD_MODE:-release} image..."
	local build_timeout="${E2E_DOCKER_BUILD_TIMEOUT:-900}"
	if ! (cd "$project_root" && timeout "$build_timeout" ./infra/infra.sh image build "$build_mode"); then
		if [[ $? -eq 124 ]]; then
			error "Docker build timed out after ${build_timeout} seconds"
		else
			error "Docker build failed"
		fi
	fi

	verbose_log "Pushing ${E2E_BUILD_MODE:-release} image to GCP..."
	local source_tag="${E2E_BUILD_MODE:-latest}" # Production uses latest by default
	if [[ "${E2E_BUILD_MODE:-release}" == "release" ]]; then
		source_tag="latest"
	else
		source_tag="${E2E_BUILD_MODE}"
	fi
	if ! (cd "$project_root" && ./infra/infra.sh image push gcp --source="$source_tag"); then
		error "Docker push failed"
	fi

	# Deploy to production environment using Terraform
	verbose_log "Deploying NXCC to production environment..."
	(cd "$project_root" && ./infra/infra.sh deploy create production)

	# Wait for deployment to be ready
	sleep 60

	success "Production deployment setup complete"
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

# Wait for deployment to be ready (replaced kubectl-based waiting)
wait_for_deployment_ready() {
	local env="$1"
	local timeout="${2:-300}"

	log "Waiting for deployment in environment '$env' to be ready..."

	# Use deployment status check instead of kubectl
	local elapsed=0
	while [[ $elapsed -lt $timeout ]]; do
		if (cd "$project_root" && ./infra/infra.sh deploy status "$env" >/dev/null 2>&1); then
			success "Deployment in environment '$env' is ready"
			return 0
		fi
		sleep 10
		elapsed=$((elapsed + 10))
	done

	warn "Deployment readiness check timed out after ${timeout}s"
	return 1
}

# List deployed variants and their URLs
list_deployed_variants() {
	local env="$1"
	local project_root="$2"

	log "Listing deployed variants in $env environment..."

	# List deployed resources using Terraform
	(cd "$project_root" && ./infra/infra.sh deploy status "$env")
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
		(cd "$project_root" && ./infra/infra.sh deploy destroy e2e-debug --auto-approve)
		(cd "$project_root" && ./infra/infra.sh cluster destroy kind)
		;;
	staging)
		(cd "$project_root" && ./infra/infra.sh deploy destroy staging --auto-approve)
		;;
	prod)
		(cd "$project_root" && ./infra/infra.sh deploy destroy production --auto-approve)
		;;
	*)
		error "Unknown environment: $env"
		;;
	esac

	success "Cluster cleanup completed for $env environment"
}
