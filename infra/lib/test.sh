#!/bin/bash
#
# Functions for testing NXCC deployments.
# This script is intended to be sourced, not executed directly.

################################################################################
# Tests HTTP connectivity to the deployed NXCC node.
# Arguments:
#   $1: The environment to test ('debug', 'staging', 'prod').
################################################################################
test_connectivity() {
  local env="$1"
  local namespace="${env}"
  local helm_release_name="nxcc-node-${env}"

  info "Testing HTTP connectivity to NXCC node in '${env}' environment..."
  check_deps kubectl curl

  # Check if the deployment exists
  if ! helm status "${helm_release_name}" --namespace "${namespace}" &>/dev/null; then
    error "Helm release '${helm_release_name}' not found in namespace '${namespace}'. Deploy it first using: $0 k8s deploy ${env}"
  fi

  # Check if pods are running
  local worker_pod
  worker_pod=$(kubectl get pods -n "${namespace}" -l app.kubernetes.io/component=worker -o jsonpath='{.items[0].metadata.name}' 2>/dev/null)
  
  if [[ -z "$worker_pod" ]]; then
    error "No worker pods found in namespace '${namespace}'. Check deployment status with: kubectl get pods -n ${namespace}"
  fi

  local pod_status
  pod_status=$(kubectl get pod "${worker_pod}" -n "${namespace}" -o jsonpath='{.status.phase}' 2>/dev/null)
  
  if [[ "$pod_status" != "Running" ]]; then
    error "Worker pod '${worker_pod}' is not running (status: ${pod_status}). Check logs with: kubectl logs ${worker_pod} -n ${namespace}"
  fi

  success "Worker pod '${worker_pod}' is running."

  # Test connectivity via port-forward
  local test_port="8080"
  local daemon_port="6922"
  
  info "Setting up port-forward to test connectivity..."
  info "Port-forwarding ${worker_pod}:${daemon_port} -> localhost:${test_port}"
  
  # Start port-forward in background
  kubectl port-forward -n "${namespace}" pod/"${worker_pod}" "${test_port}:${daemon_port}" >/dev/null 2>&1 &
  local pf_pid=$!
  
  # Function to cleanup port-forward
  cleanup_port_forward() {
    if kill -0 "$pf_pid" 2>/dev/null; then
      info "Stopping port-forward (PID: ${pf_pid})..."
      kill "$pf_pid" 2>/dev/null
      wait "$pf_pid" 2>/dev/null
    fi
  }
  
  # Set trap to cleanup on exit
  trap cleanup_port_forward EXIT
  
  # Wait for port-forward to be ready
  sleep 3
  
  # Test HTTP endpoints
  local base_url="http://localhost:${test_port}"
  local endpoints=("/api/" "/w/" "/api/health" "/")
  local success_count=0
  
  info "Testing HTTP endpoints..."
  
  for endpoint in "${endpoints[@]}"; do
    local full_url="${base_url}${endpoint}"
    info "Testing endpoint: ${endpoint}"
    
    if curl -s -f --max-time 10 "${full_url}" >/dev/null 2>&1; then
      success "✓ ${endpoint} - responded successfully (2xx)"
      ((success_count++))
    else
      local http_code
      http_code=$(curl -s -w "%{http_code}" --max-time 10 "${full_url}" -o /dev/null 2>/dev/null || echo "000")
      if [[ "$http_code" == "404" ]]; then
        info "  ${endpoint} - returned 404 (endpoint exists but not found)"
      elif [[ "$http_code" == "000" ]]; then
        warn "  ${endpoint} - connection failed"
      else
        info "  ${endpoint} - returned HTTP ${http_code}"
      fi
    fi
  done
  
  # Test basic connectivity to the daemon
  info "Testing basic TCP connectivity to NXCC daemon..."
  if curl -s --max-time 5 "${base_url}/" >/dev/null 2>&1; then
    success "✓ Basic HTTP connectivity to NXCC daemon is working"
  else
    local http_code
    http_code=$(curl -s -w "%{http_code}" --max-time 5 "${base_url}/" -o /dev/null 2>/dev/null || echo "000")
    if [[ "$http_code" == "404" ]]; then
      success "✓ NXCC daemon is responding (HTTP 404 indicates the service is running)"
    else
      warn "Basic connectivity test failed (HTTP code: ${http_code})"
    fi
  fi
  
  # Show deployment info
  info "Deployment status in namespace '${namespace}':"
  kubectl get pods,services,ingress -n "${namespace}" -o wide 2>/dev/null || warn "Could not retrieve deployment status"
  
  # Show recent logs
  info "Recent logs from worker pod:"
  kubectl logs "${worker_pod}" -n "${namespace}" --tail=10 2>/dev/null || warn "Could not retrieve logs"
  
  cleanup_port_forward
  trap - EXIT  # Remove the trap
  
  if [[ $success_count -gt 0 ]]; then
    success "HTTP connectivity test completed. NXCC node in '${env}' environment is reachable."
    info "Access the service using: kubectl port-forward -n ${namespace} pod/${worker_pod} ${test_port}:${daemon_port}"
  else
    warn "HTTP connectivity test completed with warnings. NXCC daemon is running but specific endpoints may not be implemented."
    info "Check application logs for more details: kubectl logs ${worker_pod} -n ${namespace}"
  fi
}