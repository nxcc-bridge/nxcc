# Staging Environment
# Single worker with backup seeds for pre-production testing

terraform {
  required_version = ">= 1.5"

  backend "gcs" {}
}

provider "google" {
  project = var.project_id
  region  = "europe-west4"
}

# Staging deployment - production-like but smaller scale
module "nxcc" {
  source = "../../modules/nxcc"

  environment = "staging"
  namespace   = "default"
  project_id  = var.project_id

  docker_image = var.docker_image

  # Single primary worker in Europe
  workers = [{
    name         = "worker"
    region       = "europe-west4"
    machine_type = "c3-standard-4" # Full TDX capability
    disk_size    = 10
    ephemeral    = false # Persistent for staging stability
  }]

  # Backup seeds for redundancy testing
  seeds = {
    eu_backup = {
      regions      = ["europe-west4"]
      count        = 1
      machine_type = "c3-standard-4" # c3-standard-2 not available
      ephemeral    = false
    }
  }

  # Allow access for staging testing
  allowed_ssh_cidrs = ["0.0.0.0/0"] # Open for testing

  operator_keys = {
    gcp = var.operator_key_gcp
  }
}

# Outputs for staging operations
output "worker_endpoint" {
  description = "Primary worker HTTP endpoint"
  value       = module.nxcc.worker_endpoints["worker"].http_url
}

output "deployment_summary" {
  description = "Staging deployment overview"
  value       = module.nxcc.deployment_summary
}

output "p2p_bootstrap_nodes" {
  description = "P2P bootstrap configuration for staging network"
  value       = module.nxcc.p2p_bootstrap_nodes
}

output "monitoring_targets" {
  description = "Endpoints for monitoring systems"
  value = {
    worker_instances = module.nxcc.worker_instances
    seed_instances   = module.nxcc.seed_instances
    worker_endpoints = module.nxcc.worker_endpoints
  }
}

output "ssh_commands" {
  description = "SSH commands for staging access"
  value       = module.nxcc.ssh_commands
}

# Operator key outputs for user distribution
output "operator_key_info" {
  description = "Operator key information and extraction instructions"
  value       = module.nxcc.operator_key_instructions
}

# Output for integration testing
output "staging_config" {
  description = "Configuration for staging integration tests"
  value = jsonencode({
    environment     = "staging"
    worker_endpoint = module.nxcc.worker_endpoints["worker"].http_url
    worker_ip       = module.nxcc.worker_instances["worker"].external_ip
    total_nodes     = module.nxcc.deployment_summary.total_workers + module.nxcc.deployment_summary.total_seeds
    regions         = module.nxcc.deployment_summary.regions_used
    vpc_name        = module.nxcc.vpc_name
  })
}
