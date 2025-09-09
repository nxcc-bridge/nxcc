# NXCC Infrastructure Module
# Deploys worker and seed nodes for NXCC network across regions

terraform {
  required_version = ">= 1.5"
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 5.0"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.5"
    }
  }
}

# Data sources
data "google_compute_zones" "available" {
  for_each = toset(local.all_regions)
  region   = each.value
}

# Local computations
locals {
  # Extract all regions from workers and seeds
  worker_regions = toset([for w in var.workers : w.region])
  seed_regions   = toset(flatten([for s in var.seeds : s.regions]))
  all_regions    = setunion(local.worker_regions, local.seed_regions)

  # Generate CIDR blocks for each region
  region_cidrs = {
    for idx, region in tolist(local.all_regions) :
    region => "10.${idx}.0.0/16"
  }

  # Zone distribution for instances
  zone_distribution = {
    for region in local.all_regions :
    region => data.google_compute_zones.available[region].names
  }

  # Consistent naming
  name_prefix = var.namespace != "default" ? "${var.namespace}-${var.environment}" : var.environment

  # Flatten seed instances for creation
  seed_instances = merge([
    for seed_key, seed in var.seeds : merge([
      for region_idx, region in seed.regions : {
        for instance_idx in range(seed.count) :
        "${seed_key}-${region}-${instance_idx + 1}" => {
          region       = region
          machine_type = seed.machine_type
          zone         = local.zone_distribution[region][instance_idx % length(local.zone_distribution[region])]
          ephemeral    = seed.ephemeral
          group_key    = seed_key
          region_idx   = region_idx
          instance_idx = instance_idx
        }
      }
    ]...)
  ]...)
}

# Service Account for NXCC nodes
resource "google_service_account" "nxcc_nodes" {
  account_id   = "nxcc-${local.name_prefix}-nodes"
  display_name = "NXCC ${local.name_prefix} Nodes Service Account"
  description  = "Service account for NXCC worker and seed nodes in ${var.environment}"
}

resource "google_project_iam_member" "nxcc_node_roles" {
  for_each = toset([
    "roles/compute.viewer",
    "roles/secretmanager.secretAccessor",
    "roles/artifactregistry.reader"
  ])

  project = var.project_id
  role    = each.value
  member  = "serviceAccount:${google_service_account.nxcc_nodes.email}"
}

# Grant access to the specific operator key secret
resource "google_secret_manager_secret_iam_member" "operator_key_access" {
  secret_id = google_secret_manager_secret.operator_key.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.nxcc_nodes.email}"
}

# Random suffix for unique resource naming
resource "random_id" "suffix" {
  byte_length = 4
}

# Operator Key Management
# Generate Ed25519 operator key if not provided
resource "random_bytes" "operator_private_key" {
  count  = var.operator_keys.gcp == "" ? 1 : 0
  length = 32 # Ed25519 private key size
}

# Store operator key in GCP Secret Manager
resource "google_secret_manager_secret" "operator_key" {
  secret_id = "nxcc-${local.name_prefix}-operator-key"

  replication {
    auto {}
  }

  labels = {
    environment = var.environment
    namespace   = var.namespace
    key_type    = "operator_signing"
    managed_by  = "terraform"
  }
}

resource "google_secret_manager_secret_version" "operator_key" {
  secret      = google_secret_manager_secret.operator_key.id
  secret_data = var.operator_keys.gcp != "" ? var.operator_keys.gcp : random_bytes.operator_private_key[0].base64
}

# VPC Network
resource "google_compute_network" "nxcc_vpc" {
  name                    = "nxcc-${local.name_prefix}-vpc"
  auto_create_subnetworks = false
  description             = "NXCC ${local.name_prefix} environment VPC"
}

# Regional Subnets
resource "google_compute_subnetwork" "regional_subnets" {
  for_each = local.region_cidrs

  name          = "nxcc-${local.name_prefix}-${each.key}"
  ip_cidr_range = each.value
  network       = google_compute_network.nxcc_vpc.name
  region        = each.key
  description   = "NXCC ${local.name_prefix} subnet for ${each.key}"
}

# Firewall Rules
resource "google_compute_firewall" "nxcc_p2p" {
  name    = "nxcc-${local.name_prefix}-p2p"
  network = google_compute_network.nxcc_vpc.name

  allow {
    protocol = "tcp"
    ports    = ["9000"]
  }

  source_ranges = ["10.0.0.0/8"] # Internal VPC only for P2P
  target_tags   = ["nxcc-${local.name_prefix}", "nxcc-p2p"]

  description = "Allow P2P communication between NXCC nodes"
}

resource "google_compute_firewall" "nxcc_http" {
  name    = "nxcc-${local.name_prefix}-http"
  network = google_compute_network.nxcc_vpc.name

  allow {
    protocol = "tcp"
    ports    = ["6922"]
  }

  source_ranges = ["0.0.0.0/0"] # Public access for HTTP API
  target_tags   = ["nxcc-${local.name_prefix}", "nxcc-addressable"]

  description = "Allow HTTP API access for addressable worker nodes"
}

resource "google_compute_firewall" "nxcc_ssh" {
  name    = "nxcc-${local.name_prefix}-ssh"
  network = google_compute_network.nxcc_vpc.name

  allow {
    protocol = "tcp"
    ports    = ["22"]
  }

  source_ranges = var.allowed_ssh_cidrs
  target_tags   = ["nxcc-${local.name_prefix}"]

  description = "SSH access for NXCC node administration"
}

resource "google_compute_firewall" "nxcc_internal" {
  name    = "nxcc-${local.name_prefix}-internal"
  network = google_compute_network.nxcc_vpc.name

  allow {
    protocol = "tcp"
    ports    = ["0-65535"]
  }

  allow {
    protocol = "udp"
    ports    = ["0-65535"]
  }

  allow {
    protocol = "icmp"
  }

  source_ranges = [for cidr in local.region_cidrs : cidr]
  target_tags   = ["nxcc-${local.name_prefix}"]

  description = "Allow all internal communication within NXCC VPC"
}

# Worker Instances
resource "google_compute_instance" "workers" {
  for_each = {
    for idx, worker in var.workers :
    worker.name => worker
  }

  name         = "nxcc-${local.name_prefix}-${each.key}"
  machine_type = each.value.machine_type
  zone         = "${each.value.region}-${each.value.zone != null ? each.value.zone : "a"}"

  # TDX configuration - enable for c3-standard instances, disable for others (e2e tests)
  dynamic "confidential_instance_config" {
    for_each = startswith(each.value.machine_type, "c3-standard") ? [1] : []
    content {
      enable_confidential_compute = true
      confidential_instance_type  = "TDX"
    }
  }

  scheduling {
    on_host_maintenance = each.value.ephemeral || startswith(each.value.machine_type, "c3-standard") ? "TERMINATE" : "MIGRATE"
    automatic_restart   = !each.value.ephemeral
    preemptible         = each.value.ephemeral
  }

  boot_disk {
    initialize_params {
      image = var.node_image
      size  = each.value.disk_size
      type  = startswith(each.value.machine_type, "c3-") ? "pd-ssd" : (each.value.ephemeral ? "pd-standard" : "pd-ssd")
    }
  }

  network_interface {
    network    = google_compute_network.nxcc_vpc.name
    subnetwork = google_compute_subnetwork.regional_subnets[each.value.region].name

    # Workers are addressable - they get external IPs
    access_config {
      # Ephemeral external IP
    }
  }

  metadata_startup_script = templatefile("${path.module}/templates/startup-worker.sh", {
    docker_image              = var.docker_image
    node_type                 = "worker"
    environment               = var.environment
    namespace                 = var.namespace
    operator_key              = google_secret_manager_secret.operator_key.secret_id # Secret name for retrieval
    bootstrap_peers  = join(",", var.bootstrap_peers)
  })

  metadata = {
    ssh-keys = var.ssh_keys != "" ? var.ssh_keys : "ubuntu:${file("~/.ssh/id_rsa.pub")}"
  }

  service_account {
    email  = google_service_account.nxcc_nodes.email
    scopes = ["cloud-platform"]
  }

  tags = ["nxcc-${local.name_prefix}", "nxcc-worker", "nxcc-p2p", "nxcc-addressable"]

  labels = {
    environment = var.environment
    namespace   = var.namespace
    node_type   = "worker"
    region      = each.value.region
    addressable = "true"
    ephemeral   = tostring(each.value.ephemeral)
    tee_enabled = startswith(each.value.machine_type, "c3-standard") ? "tdx" : "none"
  }
}

# Seed Instances (internal only, P2P communication with workers)
resource "google_compute_instance" "seeds" {
  for_each = local.seed_instances

  name         = "nxcc-${local.name_prefix}-seed-${replace(each.key, "_", "-")}"
  machine_type = each.value.machine_type
  zone         = each.value.zone

  # Seeds use TDX (always c3-standard instances)
  confidential_instance_config {
    enable_confidential_compute = true
    confidential_instance_type  = "TDX"
  }

  scheduling {
    on_host_maintenance = "TERMINATE"
    automatic_restart   = !each.value.ephemeral
    preemptible         = each.value.ephemeral
  }

  boot_disk {
    initialize_params {
      image = var.node_image
      size  = 10
      type  = startswith(each.value.machine_type, "c3-") ? "pd-ssd" : (each.value.ephemeral ? "pd-standard" : "pd-ssd")
    }
  }

  network_interface {
    network    = google_compute_network.nxcc_vpc.name
    subnetwork = google_compute_subnetwork.regional_subnets[each.value.region].name
    # No access_config = internal VPC only
  }

  metadata_startup_script = templatefile("${path.module}/templates/startup-seed.sh", {
    docker_image              = var.docker_image
    node_type                 = "seed"
    environment               = var.environment
    namespace                 = var.namespace
    operator_key              = google_secret_manager_secret.operator_key.secret_id # Secret name for retrieval
    bootstrap_peers  = join(",", var.bootstrap_peers)
  })

  metadata = {
    ssh-keys = var.ssh_keys != "" ? var.ssh_keys : "ubuntu:${file("~/.ssh/id_rsa.pub")}"
  }

  service_account {
    email  = google_service_account.nxcc_nodes.email
    scopes = ["cloud-platform"]
  }

  tags = ["nxcc-${local.name_prefix}", "nxcc-seed", "nxcc-p2p"]

  labels = {
    environment = var.environment
    namespace   = var.namespace
    node_type   = "seed"
    region      = each.value.region
    group       = each.value.group_key
    tee_enabled = "tdx"
  }
}
