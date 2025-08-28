# Variables for Production Environment

variable "project_id" {
  description = "GCP Project ID for production"
  type        = string
  validation {
    condition     = length(var.project_id) > 0
    error_message = "Production project ID is required."
  }
}

variable "docker_image" {
  description = "Docker image for production NXCC nodes (should be a tagged release)"
  type        = string
  validation {
    condition = (
      can(regex("^ghcr\\.io/nxcc-bridge/node:v[0-9]+\\.[0-9]+\\.[0-9]+(-[a-zA-Z0-9]+)?$", var.docker_image)) ||
      can(regex("^ghcr\\.io/nxcc-bridge/node:latest$", var.docker_image))
    )
    error_message = "Production should use tagged releases (vX.Y.Z) or 'latest' tag."
  }
}

variable "operator_key_gcp" {
  description = "Production GCP operator key (from Secret Manager)"
  type        = string
  sensitive   = true
  validation {
    condition     = length(var.operator_key_gcp) > 0
    error_message = "Production operator key is required."
  }
}

variable "worker_machine_type" {
  description = "Machine type for production workers"
  type        = string
  default     = "c3-standard-8"
  validation {
    condition = contains([
      "c3-standard-4", "c3-standard-8", "c3-standard-22", "c3-standard-44"
    ], var.worker_machine_type)
    error_message = "Worker machine type must be a supported c3-standard instance."
  }
}

variable "seed_machine_type" {
  description = "Machine type for production seeds"
  type        = string
  default     = "c3-standard-4"
  validation {
    condition = contains([
      "c3-standard-4", "c3-standard-8", "c3-standard-22"
    ], var.seed_machine_type)
    error_message = "Seed machine type must be a supported c3-standard instance."
  }
}

variable "seed_count_per_region" {
  description = "Number of seed nodes per region group"
  type        = number
  default     = 3
  validation {
    condition     = var.seed_count_per_region >= 1 && var.seed_count_per_region <= 5
    error_message = "Seed count per region must be between 1 and 5."
  }
}

variable "allowed_ssh_cidrs" {
  description = "Highly restricted CIDR blocks for production SSH access"
  type        = list(string)
  default = [
    # Examples - replace with actual production networks
    "10.0.0.0/8",    # Internal corporate network
    "203.0.113.0/24" # Specific admin subnet (RFC5737 example)
  ]
  validation {
    condition = alltrue([
      for cidr in var.allowed_ssh_cidrs : can(cidrhost(cidr, 0))
    ])
    error_message = "All SSH CIDR blocks must be valid."
  }
  validation {
    condition     = !contains(var.allowed_ssh_cidrs, "0.0.0.0/0")
    error_message = "Production SSH access must not include 0.0.0.0/0 (open to internet)."
  }
}

# Production feature flags
variable "enable_enhanced_monitoring" {
  description = "Enable enhanced monitoring for production"
  type        = bool
  default     = true
}

variable "enable_backup_automation" {
  description = "Enable automated backup procedures"
  type        = bool
  default     = true
}

variable "maintenance_window" {
  description = "Maintenance window in UTC (HH:MM format)"
  type        = string
  default     = "02:00" # 2 AM UTC
  validation {
    condition     = can(regex("^[0-2][0-9]:[0-5][0-9]$", var.maintenance_window))
    error_message = "Maintenance window must be in HH:MM format (24-hour)."
  }
}

# Security validation
variable "require_encrypted_disks" {
  description = "Require encrypted boot disks (always true for production)"
  type        = bool
  default     = true
  validation {
    condition     = var.require_encrypted_disks == true
    error_message = "Production must use encrypted disks."
  }
}

variable "environment_validation" {
  description = "Validation that this is production configuration"
  type        = string
  default     = "production"
  validation {
    condition     = var.environment_validation == "production"
    error_message = "This configuration is only for production environment."
  }
}
