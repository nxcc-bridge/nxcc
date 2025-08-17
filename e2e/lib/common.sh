#!/bin/bash
#
# Common functions and utilities for E2E tests
#

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Global configuration
E2E_VERBOSE="${E2E_VERBOSE:-false}"
E2E_TEST_TEXT="${E2E_TEST_TEXT:-Hello from NXCC E2E Test!}"

# Logging functions
log() {
    echo -e "${BLUE}[E2E]${NC} $*"
}

success() {
    echo -e "${GREEN}[E2E SUCCESS]${NC} $*"
}

warn() {
    echo -e "${YELLOW}[E2E WARN]${NC} $*"
}

error() {
    echo -e "${RED}[E2E ERROR]${NC} $*" >&2
    exit 1
}

verbose_log() {
    if [[ "$E2E_VERBOSE" == "true" ]]; then
        echo -e "${BLUE}[E2E VERBOSE]${NC} $*"
    fi
}

# Check if a command exists
command_exists() {
    local cmd="$1"
    local verbose_check="${2:-false}"
    
    if [[ "$verbose_check" == "true" ]]; then
        verbose_log "Checking if command '$cmd' exists..."
        if command -v "$cmd"; then
            verbose_log "✓ Command '$cmd' found at: $(command -v "$cmd")"
            return 0
        else
            verbose_log "✗ Command '$cmd' not found"
            return 1
        fi
    else
        command -v "$cmd" &> /dev/null
    fi
}

# Check dependencies
check_dependencies() {
    log "Checking dependencies..."
    
    local missing_deps=()
    
    # Check for required tools
    for dep in docker kind kubectl curl jq node pnpm; do
        if ! command_exists "$dep"; then
            missing_deps+=("$dep")
        fi
    done
    
    if [[ ${#missing_deps[@]} -gt 0 ]]; then
        error "Missing required dependencies: ${missing_deps[*]}"
    fi
    
    success "All dependencies are available"
}

# Check and build NXCC CLI if needed
ensure_nxcc_cli() {
    local project_root="$1"
    
    if ! command_exists nxcc; then
        log "NXCC CLI not found, attempting to build from source..."
        
        # First, ensure the SDK lib is built since the CLI depends on it
        verbose_log "Building SDK lib dependency..."
        if ! cd "$project_root/sdk/lib"; then
            error "Failed to change to SDK lib directory: $project_root/sdk/lib"
        fi
        
        # Run pnpm install and build for SDK lib
        verbose_log "Running pnpm install in SDK lib..."
        local pnpm_install_output
        if ! pnpm_install_output=$(pnpm install 2>&1); then
            error "pnpm install failed in SDK lib:\n$pnpm_install_output"
        fi
        
        verbose_log "Building SDK lib..."
        local pnpm_build_output
        if ! pnpm_build_output=$(pnpm run build 2>&1); then
            error "pnpm run build failed in SDK lib:\n$pnpm_build_output"
        fi
        verbose_log "SDK lib built successfully"
        
        # Change to CLI directory
        verbose_log "Changing to CLI directory: $project_root/sdk/cli"
        if ! cd "$project_root/sdk/cli"; then
            error "Failed to change to CLI directory: $project_root/sdk/cli"
        fi
        
        # Run pnpm install with error handling
        verbose_log "Running pnpm install..."
        local pnpm_install_output
        if ! pnpm_install_output=$(pnpm install 2>&1); then
            error "pnpm install failed:\n$pnpm_install_output"
        fi
        verbose_log "pnpm install completed successfully"
        
        # Run pnpm build with error handling
        verbose_log "Running pnpm run build..."
        local pnpm_build_output
        if ! pnpm_build_output=$(pnpm run build 2>&1); then
            error "pnpm run build failed:\n$pnpm_build_output"
        fi
        verbose_log "pnpm run build completed successfully"
        
        # Check if dist/index.js was created
        local dist_dir="$project_root/sdk/cli/dist"
        local index_js="$dist_dir/index.js"
        if [[ ! -f "$index_js" ]]; then
            error "Build completed but $index_js was not created. Build output:\n$pnpm_build_output"
        fi
        verbose_log "Build output file exists: $index_js"
        
        # Create symlink to make nxcc command available
        verbose_log "Creating symlink for nxcc command..."
        if [[ ! -f "$dist_dir/nxcc" ]]; then
            if ! ln -sf "$index_js" "$dist_dir/nxcc"; then
                error "Failed to create symlink from $dist_dir/nxcc to $index_js"
            fi
        fi
        verbose_log "Symlink created: $dist_dir/nxcc -> $index_js"
        
        # Add to PATH
        verbose_log "Adding $dist_dir to PATH"
        export PATH="$dist_dir:$PATH"
        
        # Return to project root
        cd "$project_root" || error "Failed to change to project root directory"
        
        # Final verification
        verbose_log "Verifying nxcc command is available..."
        if ! command_exists nxcc "true"; then
            local which_output
            which_output=$(which nxcc 2>&1 || echo "Command not found")
            local path_content="PATH=$PATH"
            local ls_dist
            ls_dist=$(ls -la "$dist_dir" 2>&1 || echo "Directory listing failed")
            error "nxcc command still not available after build.\nDetails:\n- which nxcc: $which_output\n- $path_content\n- dist directory contents:\n$ls_dist"
        fi
        
        local nxcc_location
        nxcc_location=$(which nxcc)
        success "NXCC CLI built successfully and available at: $nxcc_location"
    else
        verbose_log "NXCC CLI found at: $(which nxcc)"
        
        # Check if the logs command is available
        if ! nxcc worker logs --help &>/dev/null; then
            log "NXCC CLI logs command not available, rebuilding CLI..."
            
            # Change to CLI directory
            verbose_log "Changing to CLI directory: $project_root/sdk/cli"
            if ! cd "$project_root/sdk/cli"; then
                error "Failed to change to CLI directory: $project_root/sdk/cli"
            fi
            
            # Run pnpm build to get latest features
            verbose_log "Running pnpm run build..."
            local pnpm_build_output
            if ! pnpm_build_output=$(pnpm run build 2>&1); then
                error "pnpm run build failed:\n$pnpm_build_output"
            fi
            verbose_log "pnpm run build completed successfully"
            
            # Update PATH to use local version
            local dist_dir="$project_root/sdk/cli/dist"
            verbose_log "Adding $dist_dir to PATH"
            export PATH="$dist_dir:$PATH"
            
            # Return to project root
            cd "$project_root" || error "Failed to change to project root directory"
            
            success "NXCC CLI rebuilt with latest features"
        fi
    fi
}

# Wait for pods to be ready
wait_for_pods() {
    local namespace="$1"
    local timeout="${2:-300}" # 5 minutes default
    local label="${3:-app.kubernetes.io/component=worker}"
    
    log "Waiting for pods in namespace '$namespace' to be ready..."
    
    if ! kubectl wait --for=condition=ready pod -l "$label" -n "$namespace" --timeout="${timeout}s"; then
        error "Pods in namespace '$namespace' did not become ready within ${timeout} seconds"
    fi
    
    success "Pods in namespace '$namespace' are ready"
}

# Get worker pod name
get_worker_pod() {
    local namespace="$1"
    local pod_name
    if ! pod_name=$(kubectl get pods -n "$namespace" -l app.kubernetes.io/component=worker -o jsonpath='{.items[0].metadata.name}' 2>&1); then
        verbose_log "Failed to get worker pod in namespace $namespace: $pod_name"
        echo ""
        return 1
    fi
    echo "$pod_name"
}

# Setup port forwarding
setup_port_forward() {
    local env="$1"
    local test_port="${2:-8080}"
    local daemon_port="${3:-6922}"
    local namespace="$env"
    
    if [[ "$env" == "local" ]]; then
        # For local, map debug namespace
        namespace="debug"
    fi
    
    local worker_pod
    worker_pod=$(get_worker_pod "$namespace")
    
    if [[ -z "$worker_pod" ]]; then
        error "No worker pods found in $namespace namespace"
    fi
    
    log "Setting up port-forward for $env environment..."
    verbose_log "Port-forwarding $worker_pod:$daemon_port -> localhost:$test_port"
    
    # Start port-forward in background
    verbose_log "Starting port-forward: kubectl port-forward -n $namespace pod/$worker_pod $test_port:$daemon_port"
    kubectl port-forward -n "$namespace" pod/"$worker_pod" "$test_port:$daemon_port" &
    local pf_pid=$!
    
    # Store PID for cleanup
    local pid_file="/tmp/e2e_port_forward_$env.pid"
    echo "$pf_pid" > "$pid_file"
    
    # Wait for port-forward to be ready
    sleep 3
    
    verbose_log "Port-forward established (PID: $pf_pid)"
    return 0
}

# Cleanup port forwarding
cleanup_port_forward() {
    local env="$1"
    local pid_file="/tmp/e2e_port_forward_$env.pid"
    
    if [[ -f "$pid_file" ]]; then
        local pf_pid
        pf_pid=$(cat "$pid_file")
        if kill -0 "$pf_pid" 2>/dev/null; then
            verbose_log "Stopping port-forward for $env (PID: $pf_pid)..."
            kill "$pf_pid"
            wait "$pf_pid" || verbose_log "Port-forward process $pf_pid already exited"
        fi
        rm -f "$pid_file"
    fi
}

# Test HTTP endpoint with retries
test_http_endpoint() {
    local url="$1"
    local expected_pattern="$2"
    local method="${3:-GET}"
    local data="$4"
    local retries="${5:-3}"
    local delay="${6:-2}"
    
    for i in $(seq 1 "$retries"); do
        verbose_log "Testing $method $url (attempt $i/$retries)..."
        
        local response
        local curl_output
        if [[ "$method" == "POST" && -n "$data" ]]; then
            if ! curl_output=$(curl -s -f -X POST -H "Content-Type: application/json" -d "$data" "$url" 2>&1); then
                response="FAILED"
                verbose_log "curl failed: $curl_output"
            else
                response="$curl_output"
            fi
        else
            if ! curl_output=$(curl -s -f "$url" 2>&1); then
                response="FAILED"
                verbose_log "curl failed: $curl_output"
            else
                response="$curl_output"
            fi
        fi
        
        if [[ "$response" == "FAILED" ]]; then
            warn "$method $url failed (attempt $i/$retries)"
            if [[ $i -lt $retries ]]; then
                sleep "$delay"
                continue
            fi
            return 1
        fi
        
        verbose_log "Response: $response"
        
        if [[ -n "$expected_pattern" ]]; then
            if echo "$response" | grep -q "$expected_pattern"; then
                success "✓ $method $url returned expected pattern: $expected_pattern"
                return 0
            else
                warn "$method $url response did not match expected pattern: $expected_pattern"
                if [[ $i -lt $retries ]]; then
                    sleep "$delay"
                    continue
                fi
                return 1
            fi
        else
            success "✓ $method $url succeeded"
            return 0
        fi
    done
    
    return 1
}


# Execute a command with port forwarding, automatically cleaning up
# Usage: with_port_forward <env> <command>
with_port_forward() {
    local env="$1"
    shift
    local namespace="$env"
    
    if [[ "$env" == "local" ]]; then
        namespace="debug"
    fi
    
    local worker_pod
    worker_pod=$(get_worker_pod "$namespace")
    
    if [[ -z "$worker_pod" ]]; then
        error "No worker pods found in $namespace namespace"
    fi
    
    local port="${E2E_TEST_PORT:-6922}"
    verbose_log "Setting up port-forward for command execution..."
    verbose_log "Port-forwarding $worker_pod:6922 -> localhost:$port"
    
    # Start port-forward in background
    verbose_log "Starting port-forward: kubectl port-forward -n $namespace pod/$worker_pod $port:6922"
    kubectl port-forward -n "$namespace" pod/"$worker_pod" "$port:6922" &
    local pf_pid=$!
    
    # Wait for port-forward to be ready
    sleep 3
    
    # Execute the command
    local exit_code=0
    if ! "$@"; then
        exit_code=$?
    fi
    
    # Cleanup port-forward
    if kill -0 "$pf_pid" 2>/dev/null; then
        verbose_log "Stopping port-forward (PID: $pf_pid)..."
        kill "$pf_pid"
        wait "$pf_pid" || verbose_log "Port-forward process $pf_pid already exited"
    fi
    
    return $exit_code
}

# Quick test HTTP endpoint with proper environment routing
quick_http_test() {
    local env="$1"
    local endpoint="$2"
    local expected_pattern="${3:-}"
    local method="${4:-GET}"
    local data="$5"
    
    case "$env" in
        local|debug)
            # Use port-forwarding for local development
            local port="${E2E_TEST_PORT:-6922}"
            local url="http://localhost:$port$endpoint"
            with_port_forward "$env" test_http_endpoint "$url" "$expected_pattern" "$method" "$data"
            ;;
        staging)
            # Get the actual ingress IP for staging
            log "Getting ingress IP for staging environment..."
            local ingress_ip
            local ingress_output
            if ! ingress_output=$(kubectl get ingress -n staging -o jsonpath='{.items[0].status.loadBalancer.ingress[0].ip}' 2>&1); then
                verbose_log "Failed to get ingress IP: $ingress_output"
                ingress_ip=""
            else
                ingress_ip="$ingress_output"
            fi
            if [[ -z "$ingress_ip" ]]; then
                # Wait a bit and try again - ingress IPs can take time to provision
                verbose_log "Ingress IP not ready, waiting 10 seconds..."
                sleep 10
                if ! ingress_output=$(kubectl get ingress -n staging -o jsonpath='{.items[0].status.loadBalancer.ingress[0].ip}' 2>&1); then
                    verbose_log "Failed to get ingress IP on retry: $ingress_output"
                    ingress_ip=""
                else
                    ingress_ip="$ingress_output"
                fi
            fi
            if [[ -z "$ingress_ip" ]]; then
                warn "No ingress IP found for staging, falling back to port-forward"
                local port="${E2E_TEST_PORT:-6922}"
                local url="http://localhost:$port$endpoint"
                with_port_forward "$env" test_http_endpoint "$url" "$expected_pattern" "$method" "$data"
            else
                verbose_log "Using ingress IP: $ingress_ip"
                local url="http://$ingress_ip$endpoint"
                test_http_endpoint "$url" "$expected_pattern" "$method" "$data"
            fi
            ;;
        prod)
            # Get the actual ingress IP for production
            log "Getting ingress IP for production environment..."
            local ingress_ip
            local ingress_output
            if ! ingress_output=$(kubectl get ingress -n prod -o jsonpath='{.items[0].status.loadBalancer.ingress[0].ip}' 2>&1); then
                verbose_log "Failed to get ingress IP: $ingress_output"
                ingress_ip=""
            else
                ingress_ip="$ingress_output"
            fi
            if [[ -z "$ingress_ip" ]]; then
                # Wait a bit and try again - ingress IPs can take time to provision
                verbose_log "Ingress IP not ready, waiting 10 seconds..."
                sleep 10
                if ! ingress_output=$(kubectl get ingress -n prod -o jsonpath='{.items[0].status.loadBalancer.ingress[0].ip}' 2>&1); then
                    verbose_log "Failed to get ingress IP on retry: $ingress_output"
                    ingress_ip=""
                else
                    ingress_ip="$ingress_output"
                fi
            fi
            if [[ -z "$ingress_ip" ]]; then
                warn "No ingress IP found for prod, falling back to port-forward"
                local port="${E2E_TEST_PORT:-6922}"
                local url="http://localhost:$port$endpoint"
                with_port_forward "$env" test_http_endpoint "$url" "$expected_pattern" "$method" "$data"
            else
                verbose_log "Using ingress IP: $ingress_ip"
                local url="http://$ingress_ip$endpoint"
                test_http_endpoint "$url" "$expected_pattern" "$method" "$data"
            fi
            ;;
        *)
            error "Unknown environment: $env"
            ;;
    esac
}

# Quick worker deployment test
quick_worker_deploy() {
    local env="$1"
    local project_dir="$2" 
    local manifest_file="${3:-workers/manifest.template.json}"
    
    cd "$project_dir" || error "Failed to change to project directory"
    
    case "$env" in
        local|debug)
            # Use port-forwarding for local development
            local port="${E2E_TEST_PORT:-6922}"
            local rpc_url="http://localhost:$port"
            with_port_forward "$env" nxcc worker deploy "$manifest_file" --rpc-url "$rpc_url" --bundle
            ;;
        staging|prod)
            # For remote environments, always use port-forward for deployment since the RPC endpoint isn't exposed via ingress
            local port="${E2E_TEST_PORT:-6922}"
            local rpc_url="http://localhost:$port"
            with_port_forward "$env" nxcc worker deploy "$manifest_file" --rpc-url "$rpc_url" --bundle
            ;;
        *)
            error "Unknown environment: $env"
            ;;
    esac
}

# Cleanup function for temp directories and processes
cleanup_temp_resources() {
    local temp_dir="$1"
    
    # Cleanup port forwards
    cleanup_port_forward "local"
    cleanup_port_forward "staging"
    cleanup_port_forward "prod"
    
    # Remove temp directory if provided
    if [[ -n "$temp_dir" && -d "$temp_dir" ]]; then
        verbose_log "Removing temp directory: $temp_dir"
        rm -rf "$temp_dir"
    fi
}