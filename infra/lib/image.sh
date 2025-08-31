#!/bin/bash
#
# Functions for building and pushing Docker images with multi-registry support.
# This script is intended to be sourced, not executed directly.

################################################################################
# Common Docker build helper function.
# Parameters:
#   $1: image_name - Full image name and tag
#   $2: build_mode - "debug" or "release"
#   $3: action - "load" for local, "push" for registry
#   $4: cache_from - Optional cache source
################################################################################
_docker_build_common() {
	local image_name="$1"
	local build_mode="$2"
	local action="$3"
	local cache_from="${4:-}"

	local dockerfile_path="node/Dockerfile"
	local build_context="node"

	# Prepare build arguments
	local build_args=()
	if [[ "$build_mode" == "debug" ]]; then
		info "Building in debug mode"
		build_args=(--build-arg "BUILD_MODE=")
	else
		build_args=(--build-arg "BUILD_MODE=release")
		info "Building in release mode"
	fi

	if [ ! -f "$dockerfile_path" ]; then
		error "Dockerfile not found at '$dockerfile_path'. Please ensure you're in the project root."
	fi

	# ---------------- Cache config ----------------
	# Picks a sane cache directory automatically:
	# - On GitHub Actions:  $RUNNER_TEMP/buildx-cache
	# - Locally:            ${XDG_CACHE_HOME:-$HOME/.cache}/docker/buildx-cache
	# You can override with DOCKER_BUILD_CACHE_DIR.

	local cache_dir
	if [[ -n "${DOCKER_BUILD_CACHE_DIR:-}" ]]; then
		cache_dir="$DOCKER_BUILD_CACHE_DIR"
	elif [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
		cache_dir="${RUNNER_TEMP:-/tmp}/buildx-cache"
	else
		cache_dir="${XDG_CACHE_HOME:-$HOME/.cache}/docker/buildx-cache"
	fi
	mkdir -p "$cache_dir"

	# Configure cache settings
	local cache_args=()
	if [[ "${FORCE_REBUILD:-false}" == "true" ]]; then
		info "Force rebuild requested - disabling all caches"
		cache_args=(--no-cache)
	elif [[ -n "${cache_from:-}" ]]; then
		info "Using upstream cache from: $cache_from"
		cache_args=(
			--cache-from "type=registry,ref=$cache_from"
			--cache-from "type=local,src=$cache_dir"
			--cache-to "type=local,dest=$cache_dir,mode=max"
		)
	elif [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
		info "Using GitHub Actions cache + local fallback at: $cache_dir"
		cache_args=(
			--cache-from "type=gha"
			--cache-to "type=gha,mode=max"
			--cache-from "type=local,src=$cache_dir"
			--cache-to "type=local,dest=$cache_dir,mode=max"
		)
	else
		info "Using local Docker buildx cache at: $cache_dir"
		cache_args=(
			--cache-from "type=local,src=$cache_dir"
			--cache-to "type=local,dest=$cache_dir,mode=max"
		)
	fi
	# --------------------------------------------------------

	# Configure platform settings - default to amd64 for speed and TEE compatibility
	local build_platforms="${BUILD_PLATFORMS:-linux/amd64}"
	info "Building for platform: $build_platforms"

	# Check if buildx is available and create/use a builder
	if docker buildx version &>/dev/null; then
		# Create or use existing buildx instance
		if ! docker buildx inspect nxcc-builder &>/dev/null; then
			info "Creating buildx builder instance"
			docker buildx create --name nxcc-builder --use
		else
			docker buildx use nxcc-builder
		fi

		# Build with appropriate action (load for local, push for registry)
		local action_arg
		case "$action" in
		"load") action_arg="--load" ;;
		"push") action_arg="--push" ;;
		*) error "Invalid action: $action. Use 'load' or 'push'." ;;
		esac

		docker buildx build \
			--platform "$build_platforms" \
			"$action_arg" \
			--tag "$image_name" \
			"${cache_args[@]:+${cache_args[@]}}" \
			"${build_args[@]:+${build_args[@]}}" \
			--file "$dockerfile_path" \
			"$build_context"
	else
		info "Using standard docker build"

		docker build \
			--tag "$image_name" \
			"${build_args[@]:+${build_args[@]}}" \
			--file "$dockerfile_path" \
			--quiet \
			"$build_context"

		# Push if needed (for registry builds without buildx)
		if [[ "$action" == "push" ]]; then
			info "Pushing image to registry"
			docker push "$image_name"
		fi
	fi
}

################################################################################
# Build local Docker images with specified mode.
# Parameters:
#   --debug: Build debug image (fast, larger)
#   --release: Build release image (optimized, smaller)
#   --tag=TAG: Custom local tag (overrides default)
################################################################################
image_build() {
	local debug_flag=false
	local release_flag=false
	local custom_tag=""

	# Parse arguments
	while [[ $# -gt 0 ]]; do
		case $1 in
		--debug)
			debug_flag=true
			shift
			;;
		--release)
			release_flag=true
			shift
			;;
		--tag=*)
			custom_tag="${1#*=}"
			shift
			;;
		*)
			error "Unknown option for 'image build': $1"
			;;
		esac
	done

	# Validate arguments
	if [[ "$debug_flag" == true && "$release_flag" == true ]]; then
		error "Cannot specify both --debug and --release. Use one of:
  ./infra.sh image build --debug    # Fast debug build
  ./infra.sh image build --release  # Optimized release build"
	fi

	if [[ "$debug_flag" == false && "$release_flag" == false ]]; then
		error "Must specify either --debug or --release. Use one of:
  ./infra.sh image build --debug    # Fast debug build
  ./infra.sh image build --release  # Optimized release build"
	fi

	# Determine build mode and tag
	local build_mode
	local default_tag
	if [[ "$debug_flag" == true ]]; then
		build_mode="debug"
		default_tag="debug"
	else
		build_mode="release"
		default_tag="latest"
	fi

	local image_tag="${custom_tag:-$default_tag}"
	local image_name="nxcc-node:${image_tag}"

	info "Building Docker image: $image_name ($build_mode mode)"
	check_deps docker

	# Build the image using the common build function
	_docker_build_common "$image_name" "$build_mode" "load" "${BUILD_CACHE_FROM:-}"

	success "Built local image: $image_name"
}

################################################################################
# Push local Docker image to specified target registry.
# Parameters:
#   $1: target (kind, gcp, aws, azure)
#   --source=TAG: Local source tag (default: latest)
#   --tag=TAG: Target tag (default: target-specific)
################################################################################
image_push() {
	local target="$1"
	shift

	if [[ -z "$target" ]]; then
		error "Must specify target. Use one of:
  ./infra.sh image push gcp      # Push to GCP Artifact Registry
  ./infra.sh image push aws      # Push to AWS ECR
  ./infra.sh image push azure    # Push to Azure Container Registry"
	fi

	local source_tag="latest"
	local custom_tag=""

	# Parse remaining arguments
	while [[ $# -gt 0 ]]; do
		case $1 in
		--source=*)
			source_tag="${1#*=}"
			shift
			;;
		--tag=*)
			custom_tag="${1#*=}"
			shift
			;;
		*)
			error "Unknown option for 'image push': $1"
			;;
		esac
	done

	local source_image="nxcc-node:${source_tag}"

	# Verify source image exists
	if ! docker image inspect "$source_image" &>/dev/null; then
		error "Local source image not found: $source_image
Build it first with: ./infra.sh image build --debug|--release [--tag=$source_tag]"
	fi

	case "$target" in
	gcp)
		_image_push_gcp "$source_image" "$source_tag" "$custom_tag"
		;;
	aws)
		_image_push_aws "$source_image" "$source_tag" "$custom_tag"
		;;
	azure)
		_image_push_azure "$source_image" "$source_tag" "$custom_tag"
		;;
	*)
		error "Invalid target: $target. Use one of: gcp, aws, azure"
		;;
	esac
}

################################################################################
# List images in specified target.
# Parameters:
#   $1: target (local, gcp, aws, azure) - default: gcp
################################################################################
image_list() {
	local target="${1:-gcp}"

	case "$target" in
	local)
		_image_list_local
		;;
	gcp)
		_image_list_gcp
		;;
	aws)
		_image_list_aws
		;;
	azure)
		_image_list_azure
		;;
	*)
		error "Invalid target: $target. Use one of: local, gcp, aws, azure"
		;;
	esac
}

################################################################################
# Push image to GCP Artifact Registry.
# Parameters:
#   $1: source_image - Local Docker image name
#   $2: source_tag - Source tag name (for default tag logic)
#   $3: custom_tag - Custom target tag (optional)
################################################################################
_image_push_gcp() {
	local source_image="$1"
	local source_tag="$2"
	local custom_tag="$3"

	check_deps docker gcloud
	resolve_gcp_identity

	# Determine target tag
	local target_tag
	if [[ -n "$custom_tag" ]]; then
		target_tag="$custom_tag"
	elif [[ "$source_tag" == "debug" ]]; then
		target_tag="debug"
	else
		target_tag="latest"
	fi

	local registry_host="${GCP_AR_LOCATION}-docker.pkg.dev"
	local target_image="${registry_host}/${RESOLVED_PROJECT_ID}/${AR_REPO_NAME}/node:${target_tag}"

	info "Pushing to GCP Artifact Registry: $source_image → $target_image"

	# Configure Docker authentication
	gcloud auth configure-docker "${registry_host}" --account="${RESOLVED_GCP_ACCOUNT}" --quiet

	# Tag and push
	docker tag "$source_image" "$target_image"
	docker push "$target_image"

	success "Pushed to GCP: $target_image"
}

################################################################################
# Push image to AWS ECR (placeholder implementation).
# Parameters:
#   $1: source_image - Local Docker image name
#   $2: source_tag - Source tag name (for default tag logic)
#   $3: custom_tag - Custom target tag (optional)
################################################################################
_image_push_aws() {
	local source_image="$1"
	local source_tag="$2"
	local custom_tag="$3"

	# Determine target tag
	local target_tag
	if [[ -n "$custom_tag" ]]; then
		target_tag="$custom_tag"
	elif [[ "$source_tag" == "debug" ]]; then
		target_tag="staging-debug"
	else
		target_tag="production"
	fi

	error "AWS ECR support not yet implemented. Target tag would be: $target_tag
To implement: configure AWS CLI, create ECR repository, and add push logic."
}

################################################################################
# Push image to Azure Container Registry (placeholder implementation).
# Parameters:
#   $1: source_image - Local Docker image name
#   $2: source_tag - Source tag name (for default tag logic)
#   $3: custom_tag - Custom target tag (optional)
################################################################################
_image_push_azure() {
	local source_image="$1"
	local source_tag="$2"
	local custom_tag="$3"

	# Determine target tag
	local target_tag
	if [[ -n "$custom_tag" ]]; then
		target_tag="$custom_tag"
	elif [[ "$source_tag" == "debug" ]]; then
		target_tag="debug-$(date +%Y%m%d)"
	else
		target_tag="latest"
	fi

	error "Azure Container Registry support not yet implemented. Target tag would be: $target_tag
To implement: configure Azure CLI, create ACR, and add push logic."
}

################################################################################
# List local Docker images.
################################################################################
_image_list_local() {
	check_deps docker

	info "Local nXCC images:"
	docker images --filter "reference=nxcc-node*" --format "table {{.Repository}}:{{.Tag}}\t{{.CreatedSince}}\t{{.Size}}"
}

################################################################################
# List images in GCP Artifact Registry.
################################################################################
_image_list_gcp() {
	check_deps gcloud
	resolve_gcp_identity

	local registry_path="${GCP_AR_LOCATION}-docker.pkg.dev/${RESOLVED_PROJECT_ID}/${AR_REPO_NAME}/node"

	info "GCP Artifact Registry images:"
	if gcloud container images list-tags "$registry_path" --account="${RESOLVED_GCP_ACCOUNT}" --format="table(tags[0]:label=TAG,timestamp.date():label=CREATED,digest.short():label=DIGEST)" 2>/dev/null; then
		success "Listed images from: $registry_path"
	else
		warn "No images found or unable to access: $registry_path"
	fi
}

################################################################################
# List images in AWS ECR (placeholder implementation).
################################################################################
_image_list_aws() {
	error "AWS ECR support not yet implemented.
To implement: configure AWS CLI and add ECR list logic."
}

################################################################################
# List images in Azure Container Registry (placeholder implementation).
################################################################################
_image_list_azure() {
	error "Azure Container Registry support not yet implemented.
To implement: configure Azure CLI and add ACR list logic."
}
