# Variables for E2E Test Environment

variable "test_id" {
  description = "Unique test ID for namespace isolation (e.g., 'ci-1234', 'pr-567')"
  type        = string
  validation {
    condition     = can(regex("^[a-z0-9][a-z0-9-]{0,18}[a-z0-9]$", var.test_id))
    error_message = "Test ID must be lowercase alphanumeric with hyphens, 2-20 characters."
  }
}

variable "project_id" {
  description = "GCP Project ID"
  type        = string
  default     = "nxcc-462803"
}

variable "docker_image" {
  description = "Docker image for NXCC nodes (typically a PR build or test image)"
  type        = string
  # Default to latest for e2e, but typically overridden by CI
  default = "ghcr.io/nxcc-bridge/node:latest"
}

variable "operator_key_gcp" {
  description = "GCP operator key for E2E testing"
  type        = string
  default     = "" # E2E tests can use test keys
  sensitive   = true
}

variable "test_timeout" {
  description = "Test timeout in minutes (for cleanup automation)"
  type        = number
  default     = 60
  validation {
    condition     = var.test_timeout > 0 && var.test_timeout <= 120
    error_message = "Test timeout must be between 1 and 120 minutes."
  }
}