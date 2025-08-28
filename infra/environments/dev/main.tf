# Development Environment
# Simple single-worker setup for individual developers

terraform {
  required_version = ">= 1.5"

  backend "gcs" {}
}

provider "google" {
  project = var.project_id
  region  = "europe-west4" # TDX supported region
}

# Development deployment - single worker, no seeds
module "nxcc" {
  source = "../../modules/nxcc"

  environment = "dev"
  namespace   = var.developer_name
  project_id  = var.project_id

  docker_image = var.docker_image

  workers = [{
    name         = "dev"
    region       = "europe-west4"  # TDX supported region
    machine_type = "c3-standard-4" # Smallest available c3-standard for TDX support
    disk_size    = 10
    ephemeral    = true # Allow preemption for cost savings
  }]

  # No seeds needed for dev
  seeds = {}

  # Dev-friendly SSH access
  allowed_ssh_cidrs = ["0.0.0.0/0"]

  operator_keys = {
    gcp = var.operator_key_gcp # Dev can use test keys
  }
}

# Outputs for easy access
output "dev_worker_ip" {
  description = "External IP of the dev worker"
  value       = module.nxcc.worker_endpoints["dev"].ip_address
}

output "dev_http_endpoint" {
  description = "HTTP endpoint for the dev worker"
  value       = module.nxcc.worker_endpoints["dev"].http_url
}

output "ssh_command" {
  description = "SSH command to connect to the dev worker"
  value       = module.nxcc.ssh_commands["worker-dev"]
}

output "deployment_info" {
  description = "Development environment information"
  value = {
    developer   = var.developer_name
    worker_ip   = module.nxcc.worker_endpoints["dev"].ip_address
    environment = "dev"
    cost_mode   = "optimized (ephemeral instances)"
  }
}
