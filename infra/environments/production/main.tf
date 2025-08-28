# Production Environment
# Multi-region deployment with full redundancy

terraform {
  required_version = ">= 1.5"

  backend "gcs" {}
}

provider "google" {
  project = var.project_id
  region  = "europe-west4" # Primary region
}

# Production deployment - full scale with geographic distribution
module "nxcc" {
  source = "../../modules/nxcc"

  environment = "production"
  namespace   = "prod"
  project_id  = var.project_id

  docker_image = var.docker_image

  # Production workers across key regions
  workers = [
    {
      name         = "eu-primary"
      region       = "europe-west4"
      machine_type = var.worker_machine_type
      disk_size    = 10
      ephemeral    = false # Production uses persistent instances
    },
    {
      name         = "us-primary"
      region       = "us-central1"
      machine_type = var.worker_machine_type
      disk_size    = 10
      ephemeral    = false
    },
    {
      name         = "asia-primary"
      region       = "asia-southeast1"
      machine_type = var.worker_machine_type
      disk_size    = 10
      ephemeral    = false
    }
  ]

  # Distributed seed nodes for maximum redundancy
  seeds = {
    eu_seeds = {
      regions      = ["europe-west4", "europe-west1"]
      count        = var.seed_count_per_region
      machine_type = var.seed_machine_type
      ephemeral    = false
    }
    us_seeds = {
      regions      = ["us-central1", "us-west1"]
      count        = var.seed_count_per_region
      machine_type = var.seed_machine_type
      ephemeral    = false
    }
    asia_seeds = {
      regions      = ["asia-southeast1"]
      count        = var.seed_count_per_region
      machine_type = var.seed_machine_type
      ephemeral    = false
    }
  }

  # Production security - restricted SSH access
  allowed_ssh_cidrs = var.allowed_ssh_cidrs

  operator_keys = {
    gcp = var.operator_key_gcp # Production operator key from Secret Manager
  }
}

# Production monitoring outputs
output "production_endpoints" {
  description = "All production HTTP endpoints"
  value = {
    for name, endpoint in module.nxcc.worker_endpoints :
    name => {
      url        = endpoint.http_url
      ip_address = endpoint.ip_address
      region     = module.nxcc.worker_instances[name].region
    }
  }
}

output "monitoring_targets" {
  description = "Complete monitoring configuration"
  value = {
    workers = {
      for name, instance in module.nxcc.worker_instances : name => {
        name         = instance.name
        internal_ip  = instance.internal_ip
        external_ip  = instance.external_ip
        region       = instance.region
        machine_type = instance.machine_type
        http_url     = module.nxcc.worker_endpoints[name].http_url
      }
    }
    seeds = {
      for name, instance in module.nxcc.seed_instances : name => {
        name        = instance.name
        internal_ip = instance.internal_ip
        region      = instance.region
        group       = instance.group
      }
    }
    p2p_bootstrap = module.nxcc.p2p_bootstrap_nodes
    deployment    = module.nxcc.deployment_summary
  }
}

output "operational_info" {
  description = "Information for production operations"
  value = {
    environment        = "production"
    total_workers      = module.nxcc.deployment_summary.total_workers
    total_seeds        = module.nxcc.deployment_summary.total_seeds
    regions_deployed   = module.nxcc.deployment_summary.regions_used
    vpc_name           = module.nxcc.vpc_name
    all_instance_names = module.nxcc.all_instance_names
  }
}

output "disaster_recovery" {
  description = "Information for disaster recovery procedures"
  value = {
    primary_regions = ["europe-west4", "us-central1", "asia-southeast1"]
    backup_regions  = ["europe-west1", "us-west1"]
    seed_distribution = {
      eu_seeds   = ["europe-west4", "europe-west1"]
      us_seeds   = ["us-central1", "us-west1"]
      asia_seeds = ["asia-southeast1"]
    }
    worker_external_ips = module.nxcc.worker_external_ips
    seed_internal_ips   = module.nxcc.seed_internal_ips
  }
}

# Sensitive outputs (marked as sensitive)
output "ssh_commands" {
  description = "SSH commands for production access"
  value       = module.nxcc.ssh_commands
  sensitive   = true
}

output "service_account" {
  description = "Production service account information"
  value       = module.nxcc.service_account
}

# Operator key outputs for user distribution
output "operator_key_info" {
  description = "Operator key information and extraction instructions"
  value       = module.nxcc.operator_key_instructions
}

output "operator_key_secret_info" {
  description = "Secret Manager information for operator key management"
  value       = module.nxcc.operator_key_secret
  sensitive   = false
}
