# NXCC Infrastructure Module

This Terraform module deploys NXCC (Network eXecutable Cross-Chain) infrastructure on Google Cloud Platform, replacing the previous YAML+bash generator approach with clean, maintainable Terraform.

## Features

- **Worker Nodes**: Addressable nodes that handle HTTP traffic and main workload
- **Seed Nodes**: Internal-only nodes for P2P redundancy and secret replication
- **TDX Support**: Automatic TDX configuration for c3-standard instances
- **Multi-Region**: Deploy across multiple GCP regions with automatic zone distribution
- **Ephemeral Mode**: Cost-optimized preemptible instances for dev/testing
- **Per-Cloud Operator Keys**: Secure key distribution to all nodes
- **Comprehensive Validation**: Input validation and environment-specific checks

## Architecture

```
┌─ Worker Nodes (Addressable) ─┐    ┌─ Seed Nodes (Internal) ─┐
│  • External IP + HTTP API    │    │  • Internal IP only      │
│  • TDX enabled (c3-standard) │    │  • P2P communication     │
│  • Handle client requests    │    │  • Secret replication    │
└───────────────────────────────┘    └───────────────────────────┘
              │                                    │
              └──── P2P Network (port 9000) ──────┘
```

## Usage

### Development Environment

```hcl
module "nxcc" {
  source = "../../modules/nxcc"

  environment = "dev"
  namespace   = "alice"
  project_id  = "nxcc-example"

  workers = [{
    name         = "dev"
    region       = "us-central1"
    machine_type = "e2-standard-2"  # No TDX, cheaper
    ephemeral    = true
  }]

  seeds = {}  # No seeds needed for dev

  operator_keys = {
    gcp = ""  # Can use empty/test keys for dev
  }
}
```

### E2E Testing Environment

```hcl
module "nxcc" {
  source = "../../modules/nxcc"

  environment = "e2e"
  namespace   = var.test_id
  project_id  = var.project_id

  # Mix of TDX and non-TDX for compatibility testing
  workers = [
    {
      name = "worker1", region = "us-central1",
      machine_type = "c3-standard-4", ephemeral = true
    },
    {
      name = "worker2", region = "us-central1",
      machine_type = "c3-standard-4", ephemeral = true
    },
    {
      name = "worker3", region = "us-central1",
      machine_type = "e2-standard-2", ephemeral = true  # No TDX
    }
  ]

  seeds = {}  # E2E doesn't use seeds
}
```

### Production Environment

```hcl
module "nxcc" {
  source = "../../modules/nxcc"

  environment = "production"
  namespace   = "prod"
  project_id  = var.project_id

  # Multi-region workers
  workers = [
    { name = "eu-primary",   region = "europe-west4",   machine_type = "c3-standard-8" },
    { name = "us-primary",   region = "us-central1",    machine_type = "c3-standard-8" },
    { name = "asia-primary", region = "asia-southeast1", machine_type = "c3-standard-8" }
  ]

  # Distributed seeds for redundancy
  seeds = {
    eu_seeds = {
      regions = ["europe-west4", "europe-west1"], count = 3, machine_type = "c3-standard-4"
    }
    us_seeds = {
      regions = ["us-central1", "us-west1"], count = 3, machine_type = "c3-standard-4"
    }
    asia_seeds = {
      regions = ["asia-southeast1"], count = 2, machine_type = "c3-standard-4"
    }
  }

  operator_keys = {
    gcp = var.operator_key_from_secret_manager
  }
}
```

## Variables

### Required Variables

| Variable      | Type     | Description                                         |
| ------------- | -------- | --------------------------------------------------- |
| `environment` | `string` | Environment name (dev, staging, production, e2e-\*) |
| `project_id`  | `string` | GCP Project ID                                      |

### Optional Variables

| Variable            | Type           | Default                             | Description                      |
| ------------------- | -------------- | ----------------------------------- | -------------------------------- |
| `namespace`         | `string`       | `"default"`                         | Namespace for resource isolation |
| `docker_image`      | `string`       | `"ghcr.io/nxcc-bridge/node:latest"` | Docker image for NXCC nodes      |
| `workers`           | `list(object)` | `[]`                                | Worker node configurations       |
| `seeds`             | `map(object)`  | `{}`                                | Seed node group configurations   |
| `operator_keys.gcp` | `string`       | `""`                                | GCP operator key (sensitive)     |
| `allowed_ssh_cidrs` | `list(string)` | `["0.0.0.0/0"]`                     | SSH access CIDR blocks           |
| `ssh_keys`          | `string`       | `""`                                | SSH public keys                  |

### Worker Configuration

```hcl
workers = [
  {
    name         = "worker-name"      # Required: unique name
    region       = "us-central1"      # Required: GCP region
    machine_type = "c3-standard-4"    # Required: instance type
    disk_size    = 50                 # Optional: disk size in GB
    zone         = "a"                # Optional: specific zone (a, b, c)
    ephemeral    = false              # Optional: use preemptible instances
  }
]
```

### Seed Configuration

```hcl
seeds = {
  group_name = {
    regions      = ["region1", "region2"]  # Required: deployment regions
    count        = 2                       # Required: instances per region
    machine_type = "c3-standard-2"         # Required: instance type
    ephemeral    = false                   # Optional: use preemptible instances
  }
}
```

## Outputs

### Instance Information

- `worker_instances` - Complete worker instance details
- `seed_instances` - Complete seed instance details
- `worker_endpoints` - HTTP API endpoints for workers
- `ssh_commands` - SSH connection commands

### Network Information

- `vpc_id`, `vpc_name` - Created VPC information
- `subnets` - Regional subnet details
- `p2p_bootstrap_nodes` - P2P network bootstrap addresses

### Monitoring & Operations

- `deployment_summary` - High-level deployment overview
- `service_account` - Node service account details
- `all_instance_names` - All instance names for bulk operations

## Validation Rules

The module includes comprehensive validation:

### Machine Types

- **Workers**: c3-standard-_ (TDX) or e2-standard-_ (E2E only)
- **Seeds**: Must use c3-standard-\* (TDX required)

### Regions

- Must use TDX-supported regions: europe-west4, us-central1, asia-southeast1, etc.

### Environment-Specific Rules

- **E2E**: 2-5 workers, no seeds, ephemeral instances allowed
- **Production**: Must have seeds, no 0.0.0.0/0 SSH access
- **Dev**: Flexible configuration for development needs

## TDX Support

- **Automatic**: c3-standard-\* instances automatically get TDX configuration
- **Manual Override**: e2-standard-\* instances skip TDX (for E2E compatibility testing)
- **Validation**: Ensures TDX requirements are met per environment

## Security Features

- **Service Accounts**: Minimal permissions (compute.viewer, secretmanager.secretAccessor)
- **Networking**: VPC isolation, internal P2P communication
- **SSH Access**: Configurable CIDR restrictions
- **Operator Keys**: Secure per-cloud key distribution
- **Firewall**: Granular port access (22, 6922, 9000)

## Cost Optimization

- **Ephemeral Instances**: Use preemptible instances for dev/testing (60-90% cheaper)
- **Right-sized Disks**: Separate disk sizes for workers vs seeds
- **Regional Distribution**: Deploy only where needed

## Migration from YAML Generator

This module replaces the previous bash-based YAML generator:

**Before (bash generator):**

- 286+ lines of generated Terraform per environment
- Complex bash scripts with heredocs
- Hard to debug and maintain
- Resource names 50+ characters long

**After (module):**

- 15-30 lines per environment configuration
- Clean, readable HCL
- Standard Terraform debugging
- Clean resource naming

## Example Environment Configs

See the `environments/` directory for complete examples:

- `environments/dev/` - Development setup
- `environments/e2e/` - E2E testing configuration
- `environments/staging/` - Pre-production environment
- `environments/production/` - Full production deployment

## Requirements

- Terraform >= 1.5
- Google Cloud Provider ~> 5.0
- GCP project with Compute Engine API enabled
- Appropriate IAM permissions for Terraform execution
