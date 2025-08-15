#!/bin/bash
#
# Worker management functions for E2E tests
#

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Ensure SDK lib is built
ensure_sdk_lib() {
    local project_root="${1:-$E2E_PROJECT_ROOT}"
    
    verbose_log "Ensuring SDK lib is built..."
    
    if ! cd "$project_root/sdk/lib"; then
        error "Failed to change to SDK lib directory: $project_root/sdk/lib"
    fi
    
    # Check if dist directory exists and has files
    if [[ ! -d "dist" ]] || [[ ! -f "dist/index.js" ]] || [[ ! -f "dist/index.d.ts" ]]; then
        verbose_log "SDK lib not built, building now..."
        
        # Run npm install and build for SDK lib
        verbose_log "Running npm install in SDK lib..."
        local npm_install_output
        if ! npm_install_output=$(npm install 2>&1); then
            error "npm install failed in SDK lib:\n$npm_install_output"
        fi
        
        verbose_log "Building SDK lib..."
        local npm_build_output
        if ! npm_build_output=$(npm run build 2>&1); then
            error "npm run build failed in SDK lib:\n$npm_build_output"
        fi
        verbose_log "SDK lib built successfully"
    else
        verbose_log "SDK lib already built"
    fi
}

# Initialize new project in temp directory
init_test_project() {
    local temp_dir="$1"
    local project_root="${2:-$E2E_PROJECT_ROOT}"
    
    log "Initializing test project..."
    
    # Ensure SDK lib is built first
    ensure_sdk_lib "$project_root"
    
    cd "$temp_dir"
    
    # Initialize project using NXCC CLI
    nxcc init .
    
    # Add type: module to package.json for ESM support
    if command_exists jq; then
        jq '. + {"type": "module"}' package.json > package.json.tmp && mv package.json.tmp package.json
    else
        # Fallback if jq is not available
        sed -i.bak 's/"private": true,/"private": true,\n  "type": "module",/' package.json
        rm -f package.json.bak
    fi
    
    # Install SDK locally from the project
    verbose_log "Installing @nxcc/sdk from local path: $project_root/sdk/lib"
    npm install "$project_root/sdk/lib"
    
    # Install other dependencies
    npm install
    
    success "Test project initialized at $temp_dir"
}

# Create echo worker for HTTP testing
create_echo_worker() {
    local project_dir="$1"
    local test_message="${2:-$E2E_TEST_TEXT}"
    
    log "Creating echo worker for HTTP testing..."
    
    cd "$project_dir"
    
    # Create echo worker with comprehensive HTTP handling
    cat > workers/echo-worker.ts << EOF
import { worker, type WorkerContext } from "@nxcc/sdk";

export default worker({
  async fetch(request: Request, { userdata }: WorkerContext) {
    const url = new URL(request.url);
    const path = url.pathname;
    const method = request.method;
    
    console.log(\`Received \${method} request to \${path}\`);
    
    // Handle different HTTP methods and paths
    if (method === "GET" && path === "/echo") {
      return {
        message: userdata.testMessage || "Hello from NXCC worker!",
        timestamp: new Date().toISOString(),
        method: method,
        path: path,
        headers: {} // Object.fromEntries(request.headers.entries()) - not available in workers
      };
    }
    
    if (method === "POST" && path === "/echo") {
      let body = {};
      try {
        const text = await request.text();
        if (text) {
          body = JSON.parse(text);
        }
      } catch (e) {
        // If not JSON, treat as plain text
        body = { text: "Failed to parse body" };
      }
      
      return {
        message: "Echo received",
        received: body,
        testMessage: userdata.testMessage || "Hello from NXCC worker!",
        timestamp: new Date().toISOString(),
        method: method,
        path: path
      };
    }
    
    if (method === "GET" && path === "/health") {
      return {
        status: "healthy",
        timestamp: new Date().toISOString(),
        uptime: 0 // process.uptime not available in workers
      };
    }
    
    // Default response for other paths
    return {
      message: userdata.testMessage || "Hello from NXCC worker!",
      availableEndpoints: [
        "/echo (GET/POST) - Echo test endpoint",
        "/health (GET) - Health check endpoint"
      ],
      timestamp: new Date().toISOString(),
      method: method,
      path: path,
      note: "This is the default response for unmatched paths"
    };
  },

  async launch(eventPayload: Record<string, unknown>, { userdata }: WorkerContext) {
    console.log("Echo worker launched successfully!");
    console.log("Test message:", userdata.testMessage);
    console.log("Event payload:", JSON.stringify(eventPayload, null, 2));
    
    // Return launch confirmation
    return {
      launched: true,
      timestamp: new Date().toISOString(),
      testMessage: userdata.testMessage
    };
  }
});
EOF

    # Create worker manifest
    cat > workers/echo-manifest.json << EOF
{
  "bundle": {
    "source": "../dist/echo-worker.js"
  },
  "identities": [],
  "userdata": {
    "name": "echo-worker",
    "testMessage": "$test_message",
    "description": "E2E test worker for HTTP echo functionality"
  }
}
EOF

    # Fix type issues in the generated my-worker.ts by replacing the entire file
    cat > workers/my-worker.ts << 'WORKER_EOF'
import { worker, type WorkerContext } from "@nxcc/sdk";

export default worker({
  async launch(eventPayload: Record<string, unknown>, { userdata }: WorkerContext) {
    console.log("Worker launched!", eventPayload, userdata);
  },

  async fetch(request: Request, { userdata }: WorkerContext) {
    return {
      message: "Hello from nXCC worker!",
      path: new URL(request.url).pathname,
    };
  },

  async handleTransfer(eventPayload: Record<string, unknown>, { userdata }: WorkerContext) {
    const args = eventPayload.args as Record<string, unknown>;
    const from = args?.from as string;
    const to = args?.to as string;
    const value = args?.value as string;
    const transactionHash = eventPayload.transactionHash as string;
    const blockNumber = eventPayload.blockNumber as number;

    console.log(`USDC Transfer detected:`);
    console.log(`  From: ${from}`);
    console.log(`  To: ${to}`);
    console.log(`  Amount: ${(Number(value) / 1e6).toFixed(2)} USDC`);
    console.log(`  Tx: ${transactionHash}`);
    console.log(`  Block: ${blockNumber}`);
  },
});
WORKER_EOF
    
    # Update build script to include echo worker
    if command_exists jq; then
        local build_script
        build_script=$(jq -r '.scripts.build' package.json)
        local new_build_script="$build_script && esbuild workers/echo-worker.ts --bundle --outfile=dist/echo-worker.js --format=esm --target=es2022"
        jq --arg script "$new_build_script" '.scripts.build = $script' package.json > package.json.tmp && mv package.json.tmp package.json
    else
        # Fallback if jq is not available
        sed -i.bak 's/--format=esm"/--format=esm --target=es2022 \&\& esbuild workers\/echo-worker.ts --bundle --outfile=dist\/echo-worker.js --format=esm --target=es2022"/' package.json
        rm -f package.json.bak
    fi
    
    # Update tsconfig to include echo worker
    if command_exists jq; then
        jq '.include += ["workers/echo-worker.ts"]' tsconfig.json > tsconfig.tmp.json && mv tsconfig.tmp.json tsconfig.json
    else
        # Fallback if jq is not available
        sed -i.bak 's/"workers\/my-worker.ts"/"workers\/my-worker.ts", "workers\/echo-worker.ts"/' tsconfig.json
        rm -f tsconfig.json.bak
    fi
    
    success "Echo worker created successfully"
}

# Build the project
build_project() {
    local project_dir="$1"
    
    log "Building project..."
    
    cd "$project_dir"
    
    # Install esbuild if not available
    local npm_list_output
    if ! npm_list_output=$(npm list esbuild 2>&1); then
        verbose_log "esbuild not found, installing..."
        verbose_log "npm list output: $npm_list_output"
        npm install --save-dev esbuild@^0.21.4
    else
        verbose_log "esbuild already installed"
    fi
    
    # Build TypeScript
    if ! npm run build; then
        error "Failed to build TypeScript project"
    fi
    
    # Bundle the worker for deployment
    verbose_log "Creating worker bundle..."
    if ! nxcc bundle workers/echo-manifest.json --out workers/echo-worker.bundle.json; then
        error "Failed to create worker bundle"
    fi
    
    success "Project built successfully"
}

# Deploy worker to specified environment
deploy_worker() {
    local project_dir="$1"
    local env="$2"
    local timeout="${E2E_WORKER_DEPLOY_TIMEOUT:-300}"
    
    log "Deploying worker to $env environment..."
    
    cd "$project_dir"
    
    # Deploy using direct nxcc command with port forwarding
    local port="${E2E_TEST_PORT:-6922}"
    local rpc_url="http://localhost:$port"
    
    # Use timeout to prevent hanging and deploy directly with port forwarding
    if with_port_forward "$env" timeout "$timeout" nxcc worker deploy "workers/echo-manifest.json" --rpc-url "$rpc_url" --bundle; then
        success "Worker deployed to $env environment"
        return 0
    else
        local exit_code=$?
        if [[ $exit_code -eq 124 ]]; then
            error "Worker deployment timed out after ${timeout} seconds"
        else
            error "Failed to deploy worker to $env environment (exit code: $exit_code)"
        fi
    fi
}

# Test HTTP requests to worker
test_worker_http() {
    local env="$1"
    local test_message="${2:-$E2E_TEST_TEXT}"
    
    log "Testing HTTP requests to worker in $env environment..."
    
    local success_count=0
    
    # Test health endpoint
    if quick_http_test "$env" "/w/health" "healthy"; then
        ((success_count++))
    fi
    
    # Test GET request to echo endpoint  
    if quick_http_test "$env" "/w/echo" "$test_message"; then
        ((success_count++))
    fi
    
    # Test POST request to echo endpoint
    local timestamp
    timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    local post_data="{\"test\": \"data\", \"number\": 42, \"timestamp\": \"$timestamp\"}"
    if quick_http_test "$env" "/w/echo" "Echo received" "POST" "$post_data"; then
        ((success_count++))
    fi
    
    # Test root path
    if quick_http_test "$env" "/w/" "availableEndpoints"; then
        ((success_count++))
    fi
    
    # Test unknown path (should return default response)
    if quick_http_test "$env" "/w/unknown" "default response"; then
        ((success_count++))
    fi
    
    if [[ $success_count -ge 3 ]]; then
        success "HTTP request testing completed successfully for $env environment ($success_count/5 tests passed)"
        return 0
    else
        warn "HTTP request testing completed with issues for $env environment ($success_count/5 tests passed)"
        return 1
    fi
}

# Test worker deployment and functionality
test_worker_functionality() {
    local project_dir="$1"
    local env="$2"
    local test_message="${3:-$E2E_TEST_TEXT}"
    
    log "Testing worker functionality in $env environment..."
    
    # Deploy worker
    deploy_worker "$project_dir" "$env"
    
    # Wait for worker to start
    sleep 5
    
    # Get logs to verify worker started
    get_worker_logs "$env" 20
    
    # Test HTTP functionality
    test_worker_http "$env" "$test_message"
    
    success "Worker functionality test completed for $env environment"
}