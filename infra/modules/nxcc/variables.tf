# Variables for NXCC Infrastructure Module

variable "environment" {
  description = "Environment name (staging, production, dev, e2e-test-id)"
  type        = string
  validation {
    condition     = length(var.environment) > 0 && length(var.environment) <= 20
    error_message = "Environment name must be 1-20 characters."
  }
}

variable "namespace" {
  description = "Namespace for resource isolation (default, dev-username, e2e-test-id)"
  type        = string
  default     = "default"
  validation {
    condition     = length(var.namespace) > 0 && length(var.namespace) <= 20
    error_message = "Namespace must be 1-20 characters."
  }
}

variable "project_id" {
  description = "GCP Project ID"
  type        = string
  validation {
    condition     = length(var.project_id) > 0
    error_message = "Project ID is required."
  }
}

variable "docker_image" {
  description = "Docker image for NXCC nodes"
  type        = string
  default     = "ghcr.io/nxcc-bridge/node:latest"
}

variable "node_image" {
  description = "Base VM image for nodes"
  type        = string
  default     = "ubuntu-os-cloud/ubuntu-2404-lts-amd64"
}

variable "workers" {
  description = "List of worker node configurations"
  type = list(object({
    name         = string
    region       = string
    machine_type = string
    disk_size    = optional(number, 10)
    zone         = optional(string, null) # If null, uses zone 'a'
    ephemeral    = optional(bool, false)  # Use preemptible instances for cost savings
  }))
  default = []

  validation {
    condition = alltrue([
      for w in var.workers : contains([
        "europe-west4", "europe-west1",
        "us-central1", "us-west1", "us-east1",
        "asia-southeast1"
      ], w.region)
    ])
    error_message = "All worker regions must support TDX instances."
  }

  validation {
    condition = alltrue([
      for w in var.workers : length(w.name) > 0 && length(w.name) <= 15
    ])
    error_message = "Worker names must be 1-15 characters."
  }
}

variable "seeds" {
  description = "Map of seed node group configurations"
  type = map(object({
    regions      = list(string)
    count        = number
    machine_type = string
    ephemeral    = optional(bool, false)
  }))
  default = {}

  validation {
    condition = alltrue([
      for seed_key, seed in var.seeds : alltrue([
        for region in seed.regions : contains([
          "europe-west4", "europe-west1",
          "us-central1", "us-west1", "us-east1",
          "asia-southeast1"
        ], region)
      ])
    ])
    error_message = "All seed regions must support TDX instances."
  }

  validation {
    condition = alltrue([
      for seed_key, seed in var.seeds : seed.count > 0 && seed.count <= 10
    ])
    error_message = "Seed count must be between 1 and 10 per group."
  }

  validation {
    condition = alltrue([
      for seed_key, seed in var.seeds : startswith(seed.machine_type, "c3-standard")
    ])
    error_message = "Seeds must use c3-standard machine types for TDX support."
  }
}

variable "operator_keys" {
  description = "Per-cloud operator keys"
  type = object({
    gcp = optional(string, "") # GCP Secret Manager secret name or key content
  })
  default = {
    gcp = ""
  }
  sensitive = true
}

variable "allowed_ssh_cidrs" {
  description = "CIDR blocks allowed for SSH access"
  type        = list(string)
  default     = ["0.0.0.0/0"]

  validation {
    condition = alltrue([
      for cidr in var.allowed_ssh_cidrs : can(cidrhost(cidr, 0))
    ])
    error_message = "All SSH CIDR blocks must be valid."
  }
}

variable "ssh_keys" {
  description = "SSH public keys for node access (format: 'user:ssh-rsa AAAAB3...')"
  type        = string
  default     = ""
  sensitive   = true
}

# Local validation and computed values
locals {
  # TDX-supported regions (as of 2024)
  tdx_regions = toset([
    "europe-west4", "europe-west1",
    "us-central1", "us-west1", "us-east1",
    "asia-southeast1"
  ])

  # Supported machine types
  supported_machine_types = toset([
    "c3-standard-2", "c3-standard-4", "c3-standard-8", "c3-standard-22", "c3-standard-44",
    "e2-standard-2", "e2-standard-4" # e2 for e2e tests (no TDX)
  ])

  # Check if this looks like an e2e test environment
  is_e2e_environment = startswith(var.environment, "e2e") || startswith(var.namespace, "e2e")
}

# Additional validation checks
check "machine_type_support" {
  assert {
    condition = alltrue([
      for w in var.workers : contains(local.supported_machine_types, w.machine_type)
    ])
    error_message = "Unsupported machine type. Use c3-standard-* for TDX or e2-standard-* for e2e tests."
  }
}

check "e2e_test_configuration" {
  assert {
    condition = !local.is_e2e_environment || (
      length(var.seeds) == 0 &&                            # E2E tests don't use seeds
      length(var.workers) >= 2 && length(var.workers) <= 5 # E2E uses 2-5 small workers
    )
    error_message = "E2E environments should have 2-5 workers and no seeds."
  }
}

check "production_has_seeds" {
  assert {
    condition     = var.environment != "production" || length(var.seeds) > 0
    error_message = "Production environment should have seed nodes for redundancy."
  }
}

check "tee_mix_validation" {
  assert {
    condition = !local.is_e2e_environment || (
      # E2E can mix TDX (c3-standard) and non-TDX (e2-standard) for testing
      length([for w in var.workers : w if startswith(w.machine_type, "c3-standard")]) >= 1 &&
      length([for w in var.workers : w if startswith(w.machine_type, "e2-standard")]) >= 0
      ) || (
      # Non-e2e should be consistent (all TDX)
      alltrue([for w in var.workers : startswith(w.machine_type, "c3-standard")])
    )
    error_message = "E2E environments can mix machine types. Production should use c3-standard (TDX) only."
  }
}