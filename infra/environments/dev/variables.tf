# Variables for Development Environment

variable "developer_name" {
  description = "Developer name for namespace isolation (e.g., 'alice', 'bob')"
  type        = string
  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{0,14}$", var.developer_name))
    error_message = "Developer name must be lowercase, start with a letter, and be 1-15 characters."
  }
}

variable "project_id" {
  description = "GCP Project ID"
  type        = string
  default     = "nxcc-462803" # Can be overridden
}

variable "docker_image" {
  description = "Docker image for NXCC nodes"
  type        = string
  default     = "ghcr.io/nxcc-bridge/node:dev-latest"
}

variable "operator_key_gcp" {
  description = "GCP operator key for development (can use test keys)"
  type        = string
  default     = "" # Empty for dev - will use default test key
  sensitive   = true
}