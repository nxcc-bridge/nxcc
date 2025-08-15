#!/bin/bash
#
# Functions for building Docker images with correct architecture.
# This script is intended to be sourced, not executed directly.

################################################################################
# Builds Docker image for local KinD deployment.
# Defaults to amd64 architecture for Intel TDX TEE compatibility.
# Set BUILD_SINGLE_ARCH=true for faster single-arch builds.
# Set BUILD_PLATFORMS to override default platforms.
################################################################################
build_local_image() {
  info "Building Docker image for local KinD deployment..."
  check_deps docker

  local dockerfile_path="node/Dockerfile"
  local build_context="node"
  local image_name="${LOCAL_IMAGE_NAME}:${LOCAL_IMAGE_TAG}"
  
  # Support debug builds for faster development
  local build_mode="${BUILD_MODE:-debug}"
  local build_args=""
  if [[ "$build_mode" == "debug" ]]; then
    build_args=""  # Don't pass BUILD_MODE arg, let Dockerfile default to debug
    info "Building in debug mode for faster development builds"
  else
    build_args="--build-arg BUILD_MODE=release"
    info "Building in release mode"
  fi

  if [ ! -f "$dockerfile_path" ]; then
    error "Dockerfile not found at '$dockerfile_path'. Please ensure you're in the project root."
  fi

  info "Building multi-architecture image: $image_name"
  info "This ensures compatibility with both x86_64 and ARM64 architectures."
  
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
    
    # Build for Intel TDX TEE (amd64) by default, with multi-arch support available
    local build_platforms="${BUILD_PLATFORMS:-linux/amd64}"
    if [[ "$BUILD_SINGLE_ARCH" == "true" ]]; then
      build_platforms="linux/amd64"
      info "Single architecture build requested: amd64"
    else
      # Default to amd64 for Intel TDX TEE, but allow override
      info "Building for platform: $build_platforms"
    fi
    
    if [[ -n "$build_args" ]]; then
      docker buildx build \
        --platform "$build_platforms" \
        --load \
        --tag "$image_name" \
        $build_args \
        --file "$dockerfile_path" \
        --quiet \
        "$build_context"
    else
      docker buildx build \
        --platform "$build_platforms" \
        --load \
        --tag "$image_name" \
        --file "$dockerfile_path" \
        --quiet \
        "$build_context"
    fi
  else
    info "Docker buildx not available, using standard docker build..."
    if [[ -n "$build_args" ]]; then
      docker build \
        --tag "$image_name" \
        $build_args \
        --file "$dockerfile_path" \
        --quiet \
        "$build_context"
    else
      docker build \
        --tag "$image_name" \
        --file "$dockerfile_path" \
        --quiet \
        "$build_context"
    fi
  fi

  success "Local image built successfully: $image_name"
  info "Image is ready for KinD deployment."
}

################################################################################
# Builds and pushes Docker image to GCP Artifact Registry.
# Defaults to amd64 architecture for Intel TDX TEE compatibility.
# Set BUILD_SINGLE_ARCH=true for faster single-arch builds.
# Set BUILD_MODE=debug for debug builds (defaults to release for GCP).
################################################################################
build_gcp_image() {
  info "Building and pushing Docker image to GCP Artifact Registry..."
  check_deps docker gcloud
  resolve_gcp_identity

  local dockerfile_path="node/Dockerfile"
  local build_context="node"
  local registry_host="${GCP_AR_LOCATION}-docker.pkg.dev"
  local image_repo="${registry_host}/${RESOLVED_PROJECT_ID}/${AR_REPO_NAME}/node"
  local image_tag="${IMAGE_TAG_OVERRIDE:-latest}"
  local full_image="${image_repo}:${image_tag}"
  
  # Support debug builds for faster development
  local build_mode="${BUILD_MODE:-release}"
  local build_args=""
  if [[ "$build_mode" == "debug" ]]; then
    build_args=""  # Don't pass BUILD_MODE arg, let Dockerfile default to debug
    info "Building in debug mode for faster development builds"
  else
    build_args="--build-arg BUILD_MODE=release"
    info "Building in release mode"
  fi

  if [ ! -f "$dockerfile_path" ]; then
    error "Dockerfile not found at '$dockerfile_path'. Please ensure you're in the project root."
  fi

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
  
  # Check if buildx is available for multi-arch builds
  if docker buildx version &>/dev/null; then
    info "Using docker buildx for multi-architecture build and push..."
    
    # Create or use existing buildx instance
    if ! docker buildx inspect nxcc-builder &>/dev/null; then
      info "Creating new buildx builder instance..."
      docker buildx create --name nxcc-builder --use
    else
      info "Using existing buildx builder..."
      docker buildx use nxcc-builder
    fi
    
    # Build for Intel TDX TEE (amd64) by default, with multi-arch support available
    local build_platforms="${BUILD_PLATFORMS:-linux/amd64}"
    if [[ "$BUILD_SINGLE_ARCH" == "true" ]]; then
      build_platforms="linux/amd64"
      info "Single architecture build requested: amd64"
    else
      info "Multi-architecture build: $build_platforms"
    fi
    
    if [[ -n "$build_args" ]]; then
      docker buildx build \
        --platform "$build_platforms" \
        --push \
        --tag "$full_image" \
        $build_args \
        --file "$dockerfile_path" \
        "$build_context"
    else
      docker buildx build \
        --platform "$build_platforms" \
        --push \
        --tag "$full_image" \
        --file "$dockerfile_path" \
        "$build_context"
    fi
  else
    info "Docker buildx not available, building for current architecture only..."
    if [[ -n "$build_args" ]]; then
      docker build \
        --tag "$full_image" \
        $build_args \
        --file "$dockerfile_path" \
        "$build_context"
    else
      docker build \
        --tag "$full_image" \
        --file "$dockerfile_path" \
        "$build_context"
    fi
    
    info "Pushing image to Artifact Registry..."
    docker push "$full_image"
  fi

  success "GCP image built and pushed successfully: $full_image"
  info "Image is ready for GKE deployment."
}
