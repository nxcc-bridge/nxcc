# E2E Test Environment
# Ephemeral multi-worker setup for end-to-end testing

terraform {
  required_version = ">= 1.5"

  backend "gcs" {}
}

provider "google" {
  project = var.project_id
  region  = "us-central1"
}

# E2E deployment - multiple workers in same region, mix of TDX and non-TDX
module "nxcc" {
  source = "../../modules/nxcc"

  environment = "e2e"
  namespace   = var.test_id
  project_id  = var.project_id

  docker_image = var.docker_image

  # E2E tests use 3 workers: 2 with TDX, 1 without for testing compatibility
  workers = [
    {
      name         = "worker1"
      region       = "us-central1"
      machine_type = "c3-standard-4" # TDX enabled
      disk_size    = 10
      ephemeral    = true # E2E tests use ephemeral instances
    },
    {
      name         = "worker2"
      region       = "us-central1"
      machine_type = "c3-standard-4" # TDX enabled
      disk_size    = 10
      ephemeral    = true
    },
    {
      name         = "worker3"
      region       = "us-central1"
      machine_type = "e2-standard-2" # No TDX for compatibility testing
      disk_size    = 10
      ephemeral    = true
    }
  ]

  # E2E tests don't use seeds - just workers
  seeds = {}

  # Open SSH for CI/test runners
  allowed_ssh_cidrs = ["0.0.0.0/0"]

  # Bootstrap peers for E2E test network
  bootstrap_peers = []

  operator_keys = {
    gcp = var.operator_key_gcp
  }
}

# Outputs for test automation
output "worker_endpoints" {
  description = "All worker HTTP endpoints for testing"
  value = {
    for name, endpoint in module.nxcc.worker_endpoints :
    name => endpoint.http_url
  }
}

output "p2p_bootstrap_nodes" {
  description = "P2P bootstrap nodes for network testing"
  value       = module.nxcc.p2p_bootstrap_nodes
}

output "test_configuration" {
  description = "E2E test environment configuration"
  value = {
    test_id         = var.test_id
    total_workers   = length(module.nxcc.worker_instances)
    tdx_workers     = length([for name, instance in module.nxcc.worker_instances : name if instance.tee_enabled])
    non_tdx_workers = length([for name, instance in module.nxcc.worker_instances : name if !instance.tee_enabled])
    all_ephemeral   = module.nxcc.deployment_summary.is_ephemeral
  }
}

output "ssh_commands" {
  description = "SSH commands for debugging test failures"
  value       = module.nxcc.ssh_commands
}

# Output for CI/CD integration
output "ci_outputs" {
  description = "Structured outputs for CI/CD pipeline integration"
  value = jsonencode({
    environment = "e2e"
    test_id     = var.test_id
    workers = [
      for name, instance in module.nxcc.worker_instances : {
        name         = name
        ip_address   = instance.external_ip
        http_url     = module.nxcc.worker_endpoints[name].http_url
        tee_enabled  = instance.tee_enabled
        machine_type = instance.machine_type
      }
    ]
    bootstrap_nodes = module.nxcc.p2p_bootstrap_nodes
    vpc_name        = module.nxcc.vpc_name
  })
}
