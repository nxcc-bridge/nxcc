#!/bin/bash
#
# Cluster management functions for E2E tests
#

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Setup local kind cluster
setup_local_cluster() {
    local project_root="$1"
    local skip_setup="${2:-false}"
    
    if [[ "$skip_setup" == "true" ]]; then
        log "Skipping local cluster setup as requested"
        return 0
    fi
    
    log "Setting up local kind cluster..."
    
    # Build local image first with timeout
    verbose_log "Building local Docker image..."
    local build_timeout="${E2E_DOCKER_BUILD_TIMEOUT:-900}"
    if ! (cd "$project_root" && timeout "$build_timeout" ./infra/infra.sh build local); then
        if [[ $? -eq 124 ]]; then
            error "Docker build timed out after ${build_timeout} seconds"
        else
            error "Docker build failed"
        fi
    fi
    
    # Create kind cluster
    verbose_log "Creating kind cluster..."
    (cd "$project_root" && ./infra/infra.sh cluster create kind)
    
    # Deploy to debug environment (kind)
    verbose_log "Deploying NXCC to debug environment..."
    (cd "$project_root" && ./infra/infra.sh k8s deploy debug)
    
    # Wait for pods to be ready
    wait_for_pods "debug" 300
    
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