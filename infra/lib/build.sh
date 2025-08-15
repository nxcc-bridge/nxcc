#!/bin/bash
#
# Functions for building Docker images with correct architecture.
# This script is intended to be sourced, not executed directly.

################################################################################
# Builds Docker image for local KinD deployment with multi-arch support.
################################################################################
build_local_image() {
  info "Building Docker image for local KinD deployment..."
  check_deps docker

  local dockerfile_path="node/Dockerfile"
  local build_context="node"
  local image_name="${LOCAL_IMAGE_NAME}:${LOCAL_IMAGE_TAG}"

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
    
    # Build for both architectures but load only the current platform
    local current_arch
    current_arch=$(uname -m)
    if [[ "$current_arch" == "arm64" ]]; then
      local platform="linux/arm64"
    else
      local platform="linux/amd64"
    fi
    
    info "Building and loading image for platform: $platform"
    docker buildx build \
      --platform "$platform" \
      --load \
      --tag "$image_name" \
      --file "$dockerfile_path" \
      "$build_context"
  else
    info "Docker buildx not available, using standard docker build..."
    docker build \
      --tag "$image_name" \
      --file "$dockerfile_path" \
      "$build_context"
  fi

  success "Local image built successfully: $image_name"
  info "Image is ready for KinD deployment."
}

################################################################################
# Builds and pushes Docker image to GCP Artifact Registry.
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

  if [ ! -f "$dockerfile_path" ]; then
    error "Dockerfile not found at '$dockerfile_path'. Please ensure you're in the project root."
  fi

  info "Configuring Docker authentication for Artifact Registry..."
  gcloud auth configure-docker "${registry_host}" --account="${RESOLVED_GCP_ACCOUNT}" --quiet

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
    
    # Build for both x86_64 and ARM64 (GKE Autopilot supports both)
    docker buildx build \
      --platform linux/amd64,linux/arm64 \
      --push \
      --tag "$full_image" \
      --file "$dockerfile_path" \
      "$build_context"
  else
    info "Docker buildx not available, building for current architecture only..."
    docker build \
      --tag "$full_image" \
      --file "$dockerfile_path" \
      "$build_context"
    
    info "Pushing image to Artifact Registry..."
    docker push "$full_image"
  fi

  success "GCP image built and pushed successfully: $full_image"
  info "Image is ready for GKE deployment."
}