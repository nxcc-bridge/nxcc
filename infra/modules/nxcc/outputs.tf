# Outputs for NXCC Infrastructure Module

output "vpc_id" {
  description = "ID of the created VPC network"
  value       = google_compute_network.nxcc_vpc.id
}

output "vpc_name" {
  description = "Name of the created VPC network"
  value       = google_compute_network.nxcc_vpc.name
}

output "subnets" {
  description = "Information about created subnets"
  value = {
    for region, subnet in google_compute_subnetwork.regional_subnets : region => {
      name       = subnet.name
      cidr_range = subnet.ip_cidr_range
      region     = subnet.region
    }
  }
}

output "worker_instances" {
  description = "Information about worker instances"
  value = merge(
    # Bootstrap worker (if exists)
    length(google_compute_instance.bootstrap_worker) > 0 ? {
      (var.workers[0].name) = {
        name         = google_compute_instance.bootstrap_worker[0].name
        internal_ip  = google_compute_instance.bootstrap_worker[0].network_interface[0].network_ip
        external_ip  = google_compute_instance.bootstrap_worker[0].network_interface[0].access_config[0].nat_ip
        zone         = google_compute_instance.bootstrap_worker[0].zone
        region       = var.workers[0].region
        machine_type = google_compute_instance.bootstrap_worker[0].machine_type
        tee_enabled  = startswith(google_compute_instance.bootstrap_worker[0].machine_type, "c3-standard")
        addressable  = true
        is_bootstrap = true
      }
    } : {},
    # Regular workers (after bootstrap)
    {
      for name, instance in google_compute_instance.workers : name => {
        name         = instance.name
        internal_ip  = instance.network_interface[0].network_ip
        external_ip  = instance.network_interface[0].access_config[0].nat_ip
        zone         = instance.zone
        region       = var.workers[index(var.workers.*.name, name)].region
        machine_type = instance.machine_type
        tee_enabled  = startswith(instance.machine_type, "c3-standard")
        addressable  = true
        is_bootstrap = false
      }
    }
  )
}

output "seed_instances" {
  description = "Information about seed instances"
  value = {
    for name, instance in google_compute_instance.seeds : name => {
      name         = instance.name
      internal_ip  = instance.network_interface[0].network_ip
      zone         = instance.zone
      region       = local.seed_instances[name].region
      machine_type = instance.machine_type
      group        = local.seed_instances[name].group_key
      tee_enabled  = true # All seeds use TDX
    }
  }
}

output "p2p_bootstrap_nodes" {
  description = "List of P2P bootstrap node addresses for network initialization"
  value = concat(
    # Bootstrap worker node (internal IP + peer ID for VPC P2P discovery)
    local.bootstrap_peer_multiaddrs,
    # External bootstrap peers (provided by operator)
    var.bootstrap_peers,
    # Note: Additional worker and seed multiaddrs would require their peer IDs too
    # For now, they can discover each other through the bootstrap worker
  )
}

output "worker_endpoints" {
  description = "HTTP API endpoints for worker nodes"
  value = merge(
    # Bootstrap worker endpoint (if exists)
    length(google_compute_instance.bootstrap_worker) > 0 ? {
      (var.workers[0].name) = {
        http_url     = "http://${google_compute_instance.bootstrap_worker[0].network_interface[0].access_config[0].nat_ip}:6922"
        internal_url = "http://${google_compute_instance.bootstrap_worker[0].network_interface[0].network_ip}:6922"
        ip_address   = google_compute_instance.bootstrap_worker[0].network_interface[0].access_config[0].nat_ip
      }
    } : {},
    # Regular worker endpoints
    {
      for name, instance in google_compute_instance.workers : name => {
        http_url     = "http://${instance.network_interface[0].access_config[0].nat_ip}:6922"
        internal_url = "http://${instance.network_interface[0].network_ip}:6922"
        ip_address   = instance.network_interface[0].access_config[0].nat_ip
      }
    }
  )
}

output "service_account" {
  description = "Service account used by NXCC nodes"
  value = {
    email        = google_service_account.nxcc_nodes.email
    display_name = google_service_account.nxcc_nodes.display_name
    id           = google_service_account.nxcc_nodes.id
  }
}

output "ssh_commands" {
  description = "SSH commands to connect to instances"
  value = merge(
    # Bootstrap worker SSH command (if exists)
    length(google_compute_instance.bootstrap_worker) > 0 ? {
      "bootstrap-worker-${var.workers[0].name}" = "gcloud compute ssh ${google_compute_instance.bootstrap_worker[0].name} --zone=${google_compute_instance.bootstrap_worker[0].zone}"
    } : {},
    # Regular worker SSH commands
    {
      for name, instance in google_compute_instance.workers :
      "worker-${name}" => "gcloud compute ssh ${instance.name} --zone=${instance.zone}"
    },
    # Seed SSH commands
    {
      for name, instance in google_compute_instance.seeds :
      "seed-${name}" => "gcloud compute ssh ${instance.name} --zone=${instance.zone}"
    }
  )
}

output "deployment_summary" {
  description = "Summary of the deployed infrastructure"
  value = {
    environment          = var.environment
    namespace            = var.namespace
    project_id           = var.project_id
    total_workers        = length(var.workers)
    total_seeds          = length(var.seeds) > 0 ? sum([for s in var.seeds : s.count]) : 0
    regions_used         = length(local.all_regions)
    vpc_name             = google_compute_network.nxcc_vpc.name
    name_prefix          = local.name_prefix
    is_ephemeral         = anytrue([for w in var.workers : w.ephemeral])
    tee_enabled          = alltrue([for w in var.workers : startswith(w.machine_type, "c3-standard")])
    operator_key_enabled = true
  }
}

# Outputs for monitoring and automation
output "worker_internal_ips" {
  description = "Internal IP addresses of worker nodes"
  value = concat(
    # Bootstrap worker internal IP (if exists)
    length(google_compute_instance.bootstrap_worker) > 0 ? [
      google_compute_instance.bootstrap_worker[0].network_interface[0].network_ip
    ] : [],
    # Regular worker internal IPs
    [for instance in google_compute_instance.workers : instance.network_interface[0].network_ip]
  )
}

output "worker_external_ips" {
  description = "External IP addresses of worker nodes"
  value = concat(
    # Bootstrap worker external IP (if exists)
    length(google_compute_instance.bootstrap_worker) > 0 ? [
      google_compute_instance.bootstrap_worker[0].network_interface[0].access_config[0].nat_ip
    ] : [],
    # Regular worker external IPs
    [for instance in google_compute_instance.workers : instance.network_interface[0].access_config[0].nat_ip]
  )
}

output "seed_internal_ips" {
  description = "Internal IP addresses of seed nodes"
  value       = [for instance in google_compute_instance.seeds : instance.network_interface[0].network_ip]
}

output "all_instance_names" {
  description = "All instance names for bulk operations"
  value = concat(
    # Bootstrap worker name (if exists)
    length(google_compute_instance.bootstrap_worker) > 0 ? [
      google_compute_instance.bootstrap_worker[0].name
    ] : [],
    # Regular worker names
    [for instance in google_compute_instance.workers : instance.name],
    # Seed names
    [for instance in google_compute_instance.seeds : instance.name]
  )
}

# Output for load balancer setup (future use)
output "addressable_instance_groups" {
  description = "Instance information for load balancer configuration"
  value = merge(
    # Bootstrap worker (if exists)
    length(google_compute_instance.bootstrap_worker) > 0 ? {
      (var.workers[0].name) = {
        instance_name = google_compute_instance.bootstrap_worker[0].name
        zone          = google_compute_instance.bootstrap_worker[0].zone
        region        = var.workers[0].region
        internal_ip   = google_compute_instance.bootstrap_worker[0].network_interface[0].network_ip
        external_ip   = google_compute_instance.bootstrap_worker[0].network_interface[0].access_config[0].nat_ip
      }
    } : {},
    # Regular workers
    {
      for name, instance in google_compute_instance.workers : name => {
        instance_name = instance.name
        zone          = instance.zone
        region        = var.workers[index(var.workers.*.name, name)].region
        internal_ip   = instance.network_interface[0].network_ip
        external_ip   = instance.network_interface[0].access_config[0].nat_ip
      }
    }
  )
}

# Operator Key Management Outputs
output "operator_key_secret" {
  description = "GCP Secret Manager secret information for operator key"
  value = {
    secret_id   = google_secret_manager_secret.operator_key.secret_id
    secret_name = google_secret_manager_secret.operator_key.name
    project_id  = var.project_id
  }
  sensitive = false # Secret metadata is not sensitive
}

output "operator_key_instructions" {
  description = "Instructions for extracting the public key for user distribution"
  value = {
    algorithm         = "Ed25519"
    purpose           = "operator_attestation_signing"
    environment       = var.environment
    namespace         = var.namespace
    secret_name       = google_secret_manager_secret.operator_key.secret_id
    extract_command   = "Use NXCC daemon to extract public key from the operator private key in Secret Manager"
    distribution_note = "The public key must be extracted from the private key using the NXCC daemon and distributed to users for signature verification"
  }
}

# Sensitive outputs (private key information)
output "operator_key_management" {
  description = "Private operator key management information (sensitive)"
  value = {
    secret_id     = google_secret_manager_secret.operator_key.secret_id
    secret_name   = google_secret_manager_secret.operator_key.name
    project_id    = var.project_id
    key_generated = var.operator_keys.gcp == "" ? "auto_generated" : "user_provided"
    algorithm     = "Ed25519"
    key_size      = "32_bytes"
  }
  sensitive = true
}

# Bootstrap peer information for dynamic P2P network setup
output "bootstrap_peer_multiaddr" {
  description = "Libp2p multiaddr of the bootstrap worker for P2P network discovery"
  value       = length(local.bootstrap_peer_multiaddrs) > 0 ? local.bootstrap_peer_multiaddrs[0] : ""
}

output "bootstrap_worker_info" {
  description = "Information about the bootstrap worker node"
  value = length(google_compute_instance.bootstrap_worker) > 0 ? {
    name          = google_compute_instance.bootstrap_worker[0].name
    internal_ip   = google_compute_instance.bootstrap_worker[0].network_interface[0].network_ip
    external_ip   = google_compute_instance.bootstrap_worker[0].network_interface[0].access_config[0].nat_ip
    zone          = google_compute_instance.bootstrap_worker[0].zone
    peer_id       = local.bootstrap_peer_id
    p2p_multiaddr = length(local.bootstrap_peer_multiaddrs) > 0 ? local.bootstrap_peer_multiaddrs[0] : ""
  } : {}
}