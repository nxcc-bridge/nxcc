#!/bin/bash
#
# End-to-End Test Script for NXCC
#
# This script tests the complete NXCC workflow:
# 1. Sets up a kind cluster locally
# 2. Uses the CLI to init a new project in a temp dir
# 3. Modifies the project to have an HTTP handler that echoes text
# 4. Builds and deploys the project to the local node
# 5. Gets logs and makes HTTP requests to verify functionality
# 6. Optionally deploys to GKE staging environment
#
# Usage:
#   ./e2e/e2e_test.sh [options]
#   cd e2e && ./e2e_test.sh [options]
#
# Options:
#   --env local|staging|prod    Environment to test (default: local)
#   --skip-cluster-setup        Skip cluster creation (assumes cluster exists)
#   --skip-cleanup              Skip cleanup at the end
#   --test-staging              Also test staging deployment after local
#   --verbose                   Enable verbose logging
#   --force-cleanup             Force cleanup of cluster resources
#   --cache-from REPO           Use upstream cache from specified repository
#   --help                      Show this help message

set -e
set -o pipefail

# Script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
E2E_LIB_DIR="$SCRIPT_DIR/lib"

# Source helper libraries
source "$E2E_LIB_DIR/common.sh"
source "$E2E_LIB_DIR/cluster.sh"
source "$E2E_LIB_DIR/worker.sh"

# Default configuration
ENVIRONMENT="local"
SKIP_CLUSTER_SETUP="false"
SKIP_CLEANUP="false"
FORCE_CLEANUP="false"
TEST_STAGING="false"
# Debug build flag is handled via BUILD_MODE export
TEMP_PROJECT_DIR=""

# Export configuration for helper scripts
export E2E_VERBOSE="false"
export E2E_TEST_TEXT="Hello from NXCC E2E Test!"
export E2E_PROJECT_ROOT="$PROJECT_ROOT"
export BUILD_MODE="debug"            # Always use debug builds for faster e2e testing
export BUILD_PLATFORMS="linux/amd64" # Use single arch builds for faster e2e testing

# Timeout configurations (in seconds)
export E2E_WORKER_DEPLOY_TIMEOUT="300" # 5 minutes for worker deployment
export E2E_HTTP_TEST_TIMEOUT="180"     # 3 minutes for HTTP tests
export E2E_DOCKER_BUILD_TIMEOUT="900"  # 15 minutes for docker builds
export HELM_TIMEOUT="10m"              # 10 minutes for helm operations

# Help function
show_help() {
	cat <<EOF
NXCC End-to-End Test Script

This script tests the complete NXCC workflow from cluster setup to worker deployment and verification.

Usage: $0 [options]

Options:
    --env local|staging|prod    Environment to test (default: local)
    --skip-cluster-setup        Skip cluster creation (assumes cluster exists)
    --skip-cleanup              Skip cleanup at the end
    --test-staging              Also test staging deployment after local
    --verbose                   Enable verbose logging
    --force-cleanup             Force cleanup of cluster resources
    --debug                     Use debug builds for faster development (default)
    --release                   Use release builds for production testing
    --cache-from REPO           Use upstream cache from specified repository
    --help                      Show this help message

Examples:
    # Run complete local test with debug builds (default)
    $0

    # Run with release builds for performance testing
    $0 --release

    # Use upstream cache for faster builds
    $0 --cache-from ghcr.io/my-org/node:latest

    # Test existing local cluster without setup/cleanup
    $0 --skip-cluster-setup --skip-cleanup

    # Test local then staging
    $0 --test-staging

    # Test only staging environment
    $0 --env staging --skip-cluster-setup

    # Force cleanup after test
    $0 --force-cleanup

EOF
}

# Parse command line arguments
parse_args() {
	while [[ $# -gt 0 ]]; do
		case $1 in
		--env)
			ENVIRONMENT="$2"
			shift 2
			;;
		--skip-cluster-setup)
			SKIP_CLUSTER_SETUP="true"
			shift
			;;
		--skip-cleanup)
			SKIP_CLEANUP="true"
			shift
			;;
		--force-cleanup)
			FORCE_CLEANUP="true"
			shift
			;;
		--test-staging)
			TEST_STAGING="true"
			shift
			;;
		--verbose)
			export E2E_VERBOSE="true"
			shift
			;;
		--debug)
			# Set debug mode via BUILD_MODE
			export BUILD_MODE=""
			shift
			;;
		--release)
			# Set release mode via BUILD_MODE
			export BUILD_MODE="release"
			shift
			;;
		--cache-from)
			export BUILD_CACHE_FROM="$2"
			shift 2
			;;
		--help)
			show_help
			exit 0
			;;
		*)
			error "Unknown option: $1. Use --help for usage information."
			;;
		esac
	done

	# Validate environment
	case "$ENVIRONMENT" in
	local | staging | prod) ;;
	*)
		error "Invalid environment: $ENVIRONMENT. Must be one of: local, staging, prod"
		;;
	esac
}

# Check dependencies (uses helper function)
check_all_dependencies() {
	check_dependencies
	ensure_nxcc_cli "$PROJECT_ROOT"
}

# Setup cluster for specified environment
setup_environment_cluster() {
	local env="$1"

	case "$env" in
	local)
		setup_local_cluster "$PROJECT_ROOT" "$SKIP_CLUSTER_SETUP"
		;;
	staging)
		setup_staging_cluster "$PROJECT_ROOT" "$SKIP_CLUSTER_SETUP"
		;;
	prod)
		setup_prod_cluster "$PROJECT_ROOT" "$SKIP_CLUSTER_SETUP"
		;;
	*)
		error "Unknown environment: $env"
		;;
	esac
}

# Initialize and prepare test project
prepare_test_project() {
	# Create temporary directory
	TEMP_PROJECT_DIR=$(mktemp -d)
	verbose_log "Created temp project directory: $TEMP_PROJECT_DIR"

	# Initialize project using helper functions
	init_test_project "$TEMP_PROJECT_DIR" "$PROJECT_ROOT"
	create_echo_worker "$TEMP_PROJECT_DIR" "$E2E_TEST_TEXT"
	build_project "$TEMP_PROJECT_DIR"

	success "Test project prepared at $TEMP_PROJECT_DIR"
}

# Cleanup function
cleanup() {
	if [[ "$SKIP_CLEANUP" == "true" ]]; then
		log "Skipping cleanup as requested"
		log "Temp project directory: $TEMP_PROJECT_DIR"
		return
	fi

	log "Cleaning up..."

	# Use helper cleanup function
	cleanup_temp_resources "$TEMP_PROJECT_DIR"

	# Cleanup cluster resources if requested
	if [[ "$FORCE_CLEANUP" == "true" ]]; then
		cleanup_cluster "$ENVIRONMENT" "$PROJECT_ROOT" "true"
	fi

	success "Cleanup completed"
}

# Test a specific environment
test_environment() {
	local env="$1"
	log "Starting E2E test for $env environment..."

	# Setup cluster if needed
	if ! setup_environment_cluster "$env"; then
		error "Failed to setup $env environment cluster"
	fi

	# Wait for deployment to be ready
	sleep 10

	# Test basic connectivity first
	if ! test_connectivity "$env" "$PROJECT_ROOT"; then
		error "Connectivity test failed for $env environment"
	fi

	# List deployed variants for visibility
	list_deployed_variants "$env" "$PROJECT_ROOT"

	# Test variant routing functionality (new in 5b8433b)
	if ! test_variant_routing "$env"; then
		warn "Variant routing test failed for $env environment"
	fi

	# Setup port forwarding for remote environments
	if [[ "$env" != "local" ]]; then
		if ! setup_port_forward "$env"; then
			error "Failed to setup port forwarding for $env environment"
		fi
	fi

	# Test worker functionality (deploy, logs, HTTP tests)
	local test_result=0
	if ! test_worker_functionality "$TEMP_PROJECT_DIR" "$env" "$E2E_TEST_TEXT"; then
		test_result=1
		warn "Worker functionality test failed for $env environment"
	fi

	# Test worker functionality on variants if available
	if ! test_worker_functionality_variants "$TEMP_PROJECT_DIR" "$env" "$E2E_TEST_TEXT"; then
		warn "Worker functionality test failed for variants in $env environment"
		# Don't fail the entire test for variant issues
	fi

	# Cleanup port forwarding for this environment
	if [[ "$env" != "local" ]]; then
		cleanup_port_forward "$env"
	fi

	if [[ $test_result -eq 0 ]]; then
		success "E2E test completed successfully for $env environment"
	else
		error "E2E test failed for $env environment"
	fi
}

# Main execution
main() {
	log "Starting NXCC End-to-End Test..."

	parse_args "$@"
	check_all_dependencies

	# Initialize and prepare test project
	prepare_test_project

	# Test specified environment
	test_environment "$ENVIRONMENT"

	# Test staging if requested
	if [[ "$TEST_STAGING" == "true" && "$ENVIRONMENT" == "local" ]]; then
		log "Also testing staging environment as requested..."
		test_environment "staging"
	fi

	success "All E2E tests completed successfully!"
}

# Set up cleanup trap
trap cleanup EXIT

# Run main function
main "$@"
