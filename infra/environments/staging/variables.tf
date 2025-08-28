# Variables for Staging Environment

variable "project_id" {
  description = "GCP Project ID"
  type        = string
  default     = "nxcc-462803"
}

variable "docker_image" {
  description = "Docker image for NXCC nodes (typically release candidates)"
  type        = string
  default     = "ghcr.io/nxcc-bridge/node:latest"
}

variable "operator_key_gcp" {
  description = "GCP operator key for staging environment (auto-generated if not provided)"
  type        = string
  sensitive   = true
  default     = "" # Auto-generate if empty
}

variable "allowed_ssh_cidrs" {
  description = "CIDR blocks allowed for SSH access to staging"
  type        = list(string)
  default = [
    "10.0.0.0/8",    # Internal networks
    "172.16.0.0/12", # Private networks
    "192.168.0.0/16" # Local networks
  ]
  validation {
    condition = alltrue([
      for cidr in var.allowed_ssh_cidrs : can(cidrhost(cidr, 0))
    ])
    error_message = "All SSH CIDR blocks must be valid."
  }
}

# Optional overrides for staging testing
variable "enable_debug_logging" {
  description = "Enable debug logging for staging troubleshooting"
  type        = bool
  default     = false
}

variable "custom_machine_type" {
  description = "Override default machine type for staging testing"
  type        = string
  default     = "c3-standard-4"
  validation {
    condition = contains([
      "c3-standard-4", "c3-standard-8", "c3-standard-22"
    ], var.custom_machine_type)
    error_message = "Machine type must be a supported c3-standard instance."
  }
}