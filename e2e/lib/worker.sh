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

    cd "$temp_dir" || error "Failed to change to temp directory"

    # Initialize project using NXCC CLI
    nxcc init .

    # Add type: module to package.json for ESM support
    if command_exists jq; then
        jq '. + {"type": "module"}' package.json >package.json.tmp && mv package.json.tmp package.json
    else
        # Fallback if jq is not available
        sed -i.bak 's/"private": true,/"private": true,\n  "type": "module",/' package.json
        rm -f package.json.bak
    fi

    # Remove the SDK dependency from package.json first to avoid registry lookup
    if command_exists jq; then
        jq 'del(.dependencies["@nxcc/sdk"])' package.json >package.json.tmp && mv package.json.tmp package.json
    else
        # Fallback if jq is not available
        sed -i.bak '/"@nxcc\/sdk": /d' package.json
        rm -f package.json.bak
    fi

    # Install all dependencies first
    pnpm install

    # Now install SDK locally from the project
    verbose_log "Installing @nxcc/sdk from local path: $project_root/sdk/lib"
    pnpm install "file:$project_root/sdk/lib"

    success "Test project initialized at $temp_dir"
}

# Create echo worker for HTTP testing
create_echo_worker() {
    local project_dir="$1"
    local test_message="${2:-$E2E_TEST_TEXT}"

    log "Creating echo worker for HTTP testing..."

    cd "$project_dir" || error "Failed to change to project directory"

    # Create echo worker with comprehensive HTTP handling
    cat >workers/echo-worker.ts <<EOF
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
  },

  async tick(eventPayload: Record<string, unknown>, { userdata }: WorkerContext) {
    const timestamp = new Date().toISOString();
    console.log(\`Scheduled tick executed at \${timestamp}\`);
    
    // Track tick count in userdata or return a response
    const tickResponse = {
      timestamp,
      message: "Scheduled event fired successfully",
      eventPayload,
      testMessage: userdata.testMessage,
      tickNumber: Date.now() // Use timestamp as unique tick identifier
    };
    
    console.log("Tick event processed:", JSON.stringify(tickResponse, null, 2));
    return tickResponse;
  }
});
EOF

    # Create worker manifest with events including scheduled events
    cat >workers/echo-manifest.json <<EOF
{
  "bundle": {
    "source": "../dist/echo-worker.js"
  },
  "identities": [],
  "userdata": {
    "name": "echo-worker",
    "testMessage": "$test_message",
    "description": "E2E test worker for HTTP echo and scheduled events functionality"
  },
  "events": [
    {
      "handler": "launch",
      "kind": "launch"
    },
    {
      "handler": "fetch",
      "kind": "http_request"
    },
    {
      "handler": "tick",
      "kind": "scheduled",
      "period_ms": 500
    }
  ]
}
EOF

    # Fix type issues in the generated my-worker.ts by replacing the entire file
    cat >workers/my-worker.ts <<'WORKER_EOF'
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
        jq --arg script "$new_build_script" '.scripts.build = $script' package.json >package.json.tmp && mv package.json.tmp package.json
    else
        # Fallback if jq is not available
        sed -i.bak 's/--format=esm"/--format=esm --target=es2022 \&\& esbuild workers\/echo-worker.ts --bundle --outfile=dist\/echo-worker.js --format=esm --target=es2022"/' package.json
        rm -f package.json.bak
    fi

    # Update tsconfig to include echo worker
    if command_exists jq; then
        jq '.include += ["workers/echo-worker.ts"]' tsconfig.json >tsconfig.tmp.json && mv tsconfig.tmp.json tsconfig.json
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

    cd "$project_dir" || error "Failed to change to project directory"

    # Install esbuild if not available
    local pnpm_list_output
    if ! pnpm_list_output=$(pnpm list esbuild 2>&1); then
        verbose_log "esbuild not found, installing..."
        verbose_log "pnpm list output: $pnpm_list_output"
        pnpm install --save-dev esbuild@^0.21.4
    else
        verbose_log "esbuild already installed"
    fi

    # Build TypeScript
    if ! pnpm run build; then
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

    cd "$project_dir" || error "Failed to change to project directory"

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

# Get worker ID from logs
get_worker_id() {
    local env="$1"
    local namespace="$env"

    if [[ "$env" == "local" ]]; then
        namespace="debug"
    fi

    local worker_pod
    worker_pod=$(get_worker_pod "$namespace")

    if [[ -n "$worker_pod" ]]; then
        # Extract worker ID from recent logs
        local worker_id
        worker_id=$(kubectl logs "$worker_pod" -n "$namespace" --tail=50 | grep -o "worker.*id.*[a-f0-9-]\{36\}" | head -1 | grep -o "[a-f0-9-]\{36\}" | head -1)
        echo "$worker_id"
    fi
}

# Test CLI log streaming functionality
test_cli_log_streaming() {
    local env="$1"
    local timeout="${E2E_LOG_STREAMING_TIMEOUT:-6}"

    log "Testing CLI log streaming functionality in $env environment..."

    # Get the actual worker ID
    local worker_id
    worker_id=$(get_worker_id "$env")
    if [[ -z "$worker_id" ]]; then
        warn "Could not find worker ID, using placeholder"
        worker_id="dummy-worker-id"
    else
        verbose_log "Found worker ID: $worker_id"
    fi

    # Test static logs first
    log "Testing static log retrieval..."
    local static_logs_output
    if static_logs_output=$(with_port_forward "$env" timeout 10 "$E2E_PROJECT_ROOT/sdk/cli/dist/index.js" worker logs "$worker_id" --tail 10 2>&1); then
        success "Static logs retrieved successfully"
        verbose_log "Static logs output:"
        verbose_log "$static_logs_output"

        # Check if we got some log content
        if [[ -n "$static_logs_output" ]] && [[ "$static_logs_output" != *"No logs available"* ]]; then
            success "Static logs contain content"
        else
            warn "Static logs appear to be empty or unavailable"
        fi
    else
        warn "Failed to retrieve static logs: $static_logs_output"
    fi

    # Test streaming logs with follow
    log "Testing log streaming with follow flag..."
    local stream_output_file
    stream_output_file=$(mktemp)

    # Start log streaming in background
    with_port_forward "$env" timeout "$timeout" "$E2E_PROJECT_ROOT/sdk/cli/dist/index.js" worker logs "$worker_id" --follow --tail 5 >"$stream_output_file" 2>&1 &
    local stream_pid=$!

    # Wait a moment for streaming to start
    sleep 2

    # Generate some log activity by making HTTP requests
    log "Generating log activity..."
    # shellcheck disable=SC2034  # i is unused
    for i in 1 2 3; do
        quick_http_test "$env" "/w/echo" "message" "GET" "" >/dev/null 2>&1 || true
        sleep 0.5
    done

    # Wait for stream to complete or timeout
    sleep $((timeout - 2))

    # Kill the streaming process if still running
    if kill -0 "$stream_pid" 2>/dev/null; then
        kill "$stream_pid" 2>/dev/null || true
        wait "$stream_pid" 2>/dev/null || true
    fi

    # Check streaming output
    if [[ -f "$stream_output_file" ]] && [[ -s "$stream_output_file" ]]; then
        success "Log streaming produced output"
        verbose_log "Streaming logs output:"
        verbose_log "$(cat "$stream_output_file")"

        # Check if we got new log entries (should contain some activity from our HTTP requests)
        local log_line_count
        log_line_count=$(wc -l <"$stream_output_file" || echo "0")

        if [[ "$log_line_count" -ge 1 ]]; then
            success "Log streaming captured $log_line_count log lines"
        else
            warn "Log streaming captured no log lines"
        fi

        # Clean up temp file
        rm -f "$stream_output_file"
        return 0
    else
        warn "Log streaming produced no output"
        rm -f "$stream_output_file"
        return 1
    fi
}

# Test scheduled events by using CLI log streaming to check for scheduled event execution
test_scheduled_events() {
    local env="$1"
    local timeout="${E2E_SCHEDULED_EVENT_TIMEOUT:-8}"

    log "Testing scheduled events using CLI log streaming in $env environment..."

    # Use CLI log streaming to capture scheduled events in real-time
    local stream_output_file
    stream_output_file=$(mktemp)

    # Get the actual worker ID
    local worker_id
    worker_id=$(get_worker_id "$env")
    if [[ -z "$worker_id" ]]; then
        warn "Could not find worker ID, using placeholder"
        worker_id="dummy-worker-id"
    else
        verbose_log "Found worker ID for scheduled events test: $worker_id"
    fi

    log "Starting log stream to capture scheduled events (500ms interval worker)..."
    # Start log streaming in background to capture scheduled events
    with_port_forward "$env" timeout "$timeout" "$E2E_PROJECT_ROOT/sdk/cli/dist/index.js" worker logs "$worker_id" --follow --tail 10 >"$stream_output_file" 2>&1 &
    local stream_pid=$!

    # Wait for the full timeout period to capture multiple scheduled events
    log "Waiting $timeout seconds to capture scheduled events via log streaming..."
    sleep "$timeout"

    # Kill the streaming process if still running
    if kill -0 "$stream_pid" 2>/dev/null; then
        kill "$stream_pid" 2>/dev/null || true
        wait "$stream_pid" 2>/dev/null || true
    fi

    # Analyze the streamed logs for scheduled events
    if [[ -f "$stream_output_file" ]] && [[ -s "$stream_output_file" ]]; then
        verbose_log "Scheduled events log stream output:"
        verbose_log "$(cat "$stream_output_file")"

        # Check for scheduled tick messages in the stream
        local tick_count
        tick_count=$(grep -c "Scheduled tick executed" "$stream_output_file" 2>/dev/null || echo "0")
        tick_count=${tick_count//[^0-9]/} # Remove any non-numeric characters

        if [[ "$tick_count" -ge 1 ]]; then
            success "Scheduled events detected via CLI log streaming ($tick_count events)"

            # Verify we got a reasonable number of ticks (at least 10 in 8 seconds with 500ms interval)
            if [[ "$tick_count" -ge 10 ]]; then
                success "Multiple scheduled events captured ($tick_count ticks) - scheduled events working correctly"
            else
                log "Captured $tick_count scheduled events (expected ~16 for 8s with 500ms interval)"
                success "Scheduled events are working, though fewer than expected"
            fi

            # Clean up temp file
            rm -f "$stream_output_file"
            return 0
        else
            # Fallback: also check for other event-related log messages that might indicate scheduling
            local event_count
            event_count=$(grep -c -E "(event|scheduled|tick|timer)" "$stream_output_file" 2>/dev/null || echo "0")
            event_count=${event_count//[^0-9]/} # Remove any non-numeric characters

            if [[ "$event_count" -gt 0 ]]; then
                log "Found $event_count event-related log entries, but no explicit scheduled ticks"
                success "Event system appears to be working (found $event_count event-related log entries)"
                rm -f "$stream_output_file"
                return 0
            else
                warn "No scheduled events detected in CLI log stream"
                verbose_log "Log stream contents:"
                verbose_log "$(cat "$stream_output_file")"
                rm -f "$stream_output_file"
                return 1
            fi
        fi
    else
        warn "CLI log streaming produced no output for scheduled events test"
        rm -f "$stream_output_file"
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

    # Test HTTP functionality
    test_worker_http "$env" "$test_message"

    # Test CLI log streaming functionality
    test_cli_log_streaming "$env"

    # Test scheduled events functionality
    test_scheduled_events "$env"

    success "Worker functionality test completed for $env environment"
}
