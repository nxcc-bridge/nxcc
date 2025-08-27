#!/bin/bash
#
# Functions for managing local development containers.
# This script is intended to be sourced, not executed directly.

################################################################################
# Runs a local development container with all tools pre-installed.
#
# Arguments:
#   --platform <platform>  Specify the platform (e.g., linux/amd64, linux/arm64)
#   --build                Force rebuild of the container
################################################################################
dev_run_container() {
	info "Starting NXCC development container..."
	check_deps docker

	local project_root platform_arg build_platform run_platform force_build=false
	project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

	# Parse arguments
	while [[ $# -gt 0 ]]; do
		case $1 in
		--platform)
			platform_arg="$2"
			shift 2
			;;
		--build)
			force_build=true
			shift
			;;
		*)
			shift
			;;
		esac
	done

	# Set platform arguments
	if [[ -n "$platform_arg" ]]; then
		build_platform="--platform $platform_arg"
		run_platform="--platform $platform_arg"
		info "Using specified platform: $platform_arg"
	else
		# Auto-detect current platform for running
		local current_arch
		current_arch="$(docker info --format "{{ .Architecture }}")"
		run_platform="--platform linux/$current_arch"
		info "Auto-detected platform: linux/$current_arch"
	fi

	# Build the development container if it doesn't exist or force rebuild
	if [[ "$force_build" == true ]] || ! docker image inspect nxcc-dev &>/dev/null; then
		info "Building development container (this may take a few minutes)..."
		# shellcheck disable=SC2086  # We want word splitting for platform_arg
		docker build $build_platform -f "${project_root}/dev/Dockerfile" -t nxcc-dev "${project_root}"
	fi

	info "Running development container with project mounted at /workspace"
	info "Available tools: rust, node, pnpm, forge, grpcurl"
	info ""
	info "Try these commands inside the container:"
	info "  cd /workspace/node && cargo build          # Build Rust components"
	info "  cd /workspace/contracts/evm && forge build  # Build smart contracts"
	info "  cd /workspace/sdk && pnpm build             # Build CLI and SDK"
	info "  cd /workspace && ./e2e/e2e_test.sh          # Run e2e tests"
	info ""

	# Run the container interactively with the project mounted
	# shellcheck disable=SC2086  # We want word splitting for run_platform
	docker run -it --rm \
		$run_platform \
		-v "${project_root}:/workspace" \
		-v /var/run/docker.sock:/var/run/docker.sock \
		-w /workspace \
		nxcc-dev
}
