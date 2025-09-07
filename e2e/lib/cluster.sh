#!/bin/bash
#
# Cluster management functions for E2E tests
#

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Source infra common.sh for constants
source "$(dirname "${BASH_SOURCE[0]}")/../../infra/lib/common.sh"

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
	(cd "$project_root" && E2E_BUILD_MODE="${E2E_BUILD_MODE:-debug}" ./infra/infra.sh deploy create staging)

	# Wait for deployment to be ready using terraform readiness check
	verbose_log "Waiting for staging deployment to be ready..."
	if ! wait_for_deployment_ready "staging" 300 "$project_root"; then
		error "Staging deployment failed to become ready"
	fi

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
	(cd "$project_root" && E2E_BUILD_MODE="${E2E_BUILD_MODE:-release}" ./infra/infra.sh deploy create production)

	# Wait for deployment to be ready using terraform readiness check
	verbose_log "Waiting for production deployment to be ready..."
	if ! wait_for_deployment_ready "production" 300 "$project_root"; then
		error "Production deployment failed to become ready"
	fi

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

# Wait for deployment to be ready using terraform outputs and HTTP health checks
wait_for_deployment_ready() {
	local env="$1"
	local timeout="${2:-300}"
	local project_root="${3:-$E2E_PROJECT_ROOT}"

	# Use the enhanced terraform readiness check from common.sh
	test_terraform_deployment_ready "$env" "$timeout" "$project_root"
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
