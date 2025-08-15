#!/bin/bash
#
# Functions for building Docker images with correct architecture.
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
    info "Building in debug mode for faster development builds"
  else
    build_args=(--build-arg "BUILD_MODE=release")
    info "Building in release mode"
  fi

  if [ ! -f "$dockerfile_path" ]; then
    error "Dockerfile not found at '$dockerfile_path'. Please ensure you're in the project root."
  fi
  
  # Configure cache settings
  local cache_args=()
  if [[ -n "$cache_from" ]]; then
    info "Using upstream cache from: $cache_from"
    cache_args=(--cache-from "type=registry,ref=$cache_from")
  elif [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
    info "Using GitHub Actions cache"
    cache_args=(--cache-from "type=gha")
  fi
  
  # Configure platform settings - default to single arch for speed
  local build_platforms="${BUILD_PLATFORMS:-linux/amd64}"
  info "Building for platform: $build_platforms"
  
  # Check if buildx is available and create/use a multi-arch builder
  if docker buildx version &>/dev/null; then
    info "Using docker buildx for multi-architecture build..."
    
    # Create or use existing buildx instance
    if ! docker buildx inspect nxcc-builder &>/dev/null; then
      info "Creating new buildx builder instance..."
      docker buildx create --name nxcc-builder --use
    else
      info "Using existing buildx builder..."
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
      --quiet \
      "$build_context"
  else
    info "Docker buildx not available, using standard docker build..."
    
    docker build \
      --tag "$image_name" \
      "${build_args[@]:+${build_args[@]}}" \
      --file "$dockerfile_path" \
      --quiet \
      "$build_context"
    
    # Push if needed (for registry builds without buildx)
    if [[ "$action" == "push" ]]; then
      info "Pushing image to registry..."
      docker push "$image_name"
    fi
  fi
}

################################################################################
# Builds Docker image for local KinD deployment.
# Defaults to amd64 architecture for Intel TDX TEE compatibility.
# Set BUILD_PLATFORMS to override target platforms (defaults to linux/amd64).
# Use BUILD_PLATFORMS=linux/amd64,linux/arm64 for multi-architecture builds.
# Set BUILD_CACHE_FROM to specify upstream cache repository.
################################################################################
build_local_image() {
  info "Building Docker image for local KinD deployment..."
  check_deps docker

  local image_name="${LOCAL_IMAGE_NAME}:${LOCAL_IMAGE_TAG}"
  local build_mode="${BUILD_MODE:-debug}"
  
  info "Building multi-architecture image: $image_name"
  info "This ensures compatibility with both x86_64 and ARM64 architectures."
  
  _docker_build_common "$image_name" "$build_mode" "load" "${BUILD_CACHE_FROM:-}"

  success "Local image built successfully: $image_name"
  info "Image is ready for KinD deployment."
}

################################################################################
# Builds and pushes Docker image to GCP Artifact Registry.
# Defaults to amd64 architecture for Intel TDX TEE compatibility.
# Set BUILD_PLATFORMS to override target platforms (defaults to linux/amd64).
# Use BUILD_PLATFORMS=linux/amd64,linux/arm64 for multi-architecture builds.
# Set BUILD_MODE=debug for debug builds (defaults to release for GCP).
################################################################################
build_gcp_image() {
  info "Building and pushing Docker image to GCP Artifact Registry..."
  check_deps docker gcloud
  resolve_gcp_identity

  local registry_host="${GCP_AR_LOCATION}-docker.pkg.dev"
  local image_repo="${registry_host}/${RESOLVED_PROJECT_ID}/${AR_REPO_NAME}/node"
  local image_tag="${IMAGE_TAG_OVERRIDE:-latest}"
  local full_image="${image_repo}:${image_tag}"
  local build_mode="${BUILD_MODE:-release}"

  info "Configuring Docker authentication for Artifact Registry..."
  gcloud auth configure-docker "${registry_host}" --account="${RESOLVED_GCP_ACCOUNT}" --quiet

  # Check if image already exists for debug builds (to speed up e2e tests)
  if [[ "$build_mode" == "debug" ]]; then
    if gcloud container images describe "$full_image" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
      info "Debug image already exists, skipping build: $full_image"
      return 0
    fi
  fi

  info "Building multi-architecture image for GCP: $full_image"
  
  _docker_build_common "$full_image" "$build_mode" "push"

  success "GCP image built and pushed successfully: $full_image"
  info "Image is ready for GKE deployment."
}
