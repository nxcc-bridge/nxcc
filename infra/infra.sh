#!/bin/bash
#
# Manages cloud and local resources for the nXCC confidential workload.
#
# This script is the main entrypoint. It sources its functionality from the
# scripts in the ./lib/ directory.
#
# IDEMPOTENCY GUARANTEE:
# All operations in this script are designed to be idempotent and safe to run multiple times.

set -e
set -o pipefail

# The root directory of the library scripts, relative to this script's location.
LIB_DIR="$(dirname "$0")/lib"

# Source all the functional components.
# common.sh must be first as others depend on it.
# shellcheck disable=SC1091  # Library files are sourced dynamically
source "${LIB_DIR}/common.sh"
source "${LIB_DIR}/ci.sh"
source "${LIB_DIR}/cluster.sh"
source "${LIB_DIR}/image.sh"
source "${LIB_DIR}/dev.sh" # Changed to the new single entrypoint
source "${LIB_DIR}/state_management.sh"

################################################################################
# Displays usage information.
################################################################################
usage() {
	echo "Usage: $0 [-y] [--bucket=NAME] <command> <subcommand> [args]"
	echo
	echo "Manages cloud (GCP) and local resources for the nXCC application."
	echo
	echo "Options:"
	echo "  -y             Automatically answer 'yes' to all interactive confirmation prompts."
	echo "  --bucket=NAME  Override GCS bucket for Terraform state (default: PROJECT_ID-terraform-state)"
	echo
	echo "Commands:"
	echo "  image <build|push|list>"
	echo "    Manages Docker images with multi-registry support."
	echo "      build:    Build source images locally (--debug or --release required)"
	echo "      push:     Push local images to targets (kind, gcp, aws, azure)"
	echo "      list:     List images in targets (local, gcp, aws, azure)"
	echo
	echo "  ci <setup|teardown>"
	echo "    Manages GCP resources for CI/CD (Service Account, WIF, Artifact Registry)."
	echo "      setup:    Creates and configures all CI/CD resources."
	echo "      teardown: Deletes all CI/CD resources."
	echo
	echo "  cluster <create|destroy> <env>"
	echo "    Manages the GKE cluster."
	echo "      <env>: gke"
	echo "      create:   Creates the specified cluster."
	echo "      destroy:  Deletes the specified cluster."
	echo
	echo "  test <env>"
	echo "    Tests HTTP connectivity to the deployed NXCC node."
	echo "      <env>: debug | staging | prod"
	echo
	echo "  keys <generate> <args>"
	echo "    Manages operator signing keys for attestation policies."
	echo "      generate <output-file>       Generates a new Ed25519 operator signing key"
	echo
	echo "  deploy <create|destroy|status|plan> <env>"
	echo "    Manages NXCC infrastructure using Terraform modules (replaces topology)."
	echo "      create:   Creates infrastructure, auto-generating operator keys"
	echo "      destroy:  Destroys infrastructure and cleans up resources" 
	echo "      status:   Shows current deployment status and connection info"
	echo "      plan:     Shows what changes would be made without applying"
	echo "      <env>:    Environment name (staging, production, dev-username, e2e-testid)"
	echo
	echo "  dev <create|ssh|push|destroy|status|cleanup|container|local>"
	echo "    Manages TDX development VM for real hardware testing and local development containers."
	echo "      create:     Creates complete TDX-enabled VM with dependencies, verification, and container."
	echo "                  Uses preemptible instances by default (add --dedicated for guaranteed availability)."
	echo "      ssh:        SSH into the development VM (add '-- command' to run specific command)."
	echo "      push:       Sync local code to the development VM (git-tracked files only)."
	echo "      container:  Start/restart development container on VM (add --detached for background)."
	echo "      status:     Shows VM status, IP address, and connection info."
	echo "      destroy:    Destroys the TDX development VM."
	echo "      cleanup:    Alternative destroy command (same as destroy)."
	echo "      local:      Runs a local development container with all tools pre-installed."
	echo "                  Options: --platform <linux/amd64|linux/arm64> --build"
	echo
	echo "Image Commands (Detailed):"
	echo "  image build [options]       Build source images locally"
	echo "    --debug                   Debug build (fast, larger) → nxcc-node:debug"
	echo "    --release                 Release build (optimized) → nxcc-node:latest"
	echo "    --tag=TAG                 Custom local tag → nxcc-node:TAG"
	echo
	echo "  image push <target> [options]  Push local image to target"
	echo "    Targets: gcp, aws, azure"
	echo "    --source=TAG              Local source tag (default: latest)"
	echo "    --tag=TAG                 Target tag (default: target-specific)"
	echo
	echo "  image list [target]         List images"
	echo "    Targets: local, gcp, aws, azure (default: gcp)"
	echo
	echo "  Examples:"
	echo "    image build --debug                # Build debug image locally"
	echo "    image push gcp --source=debug      # Push debug build to GCP"
	echo "    image push gcp --tag=staging-test  # Push with custom tag"
	echo
	echo "GCP Identity:"
	echo "  For 'ci' and 'gke' commands, the script will resolve your GCP identity automatically."
	echo "  You can override this by setting GCP_ACCOUNT and GCP_PROJECT_ID environment variables."
	echo
	echo "Common Workflow Examples:"
	echo "  # Deploy environments (automatically generates operator keys)"
	echo "  $0 deploy create staging              # Deploy staging environment"
	echo "  $0 deploy create dev-alice            # Deploy dev environment for 'alice'"
	echo "  $0 deploy create e2e-pr-123          # Deploy E2E test environment"
	echo "  $0 deploy status staging             # Show deployment info & endpoints"
	echo "  $0 deploy destroy dev-alice          # Clean up dev environment"
	echo
	echo "  # Generate operator keys manually (if needed)"
	echo "  $0 keys generate                     # Generate key, output base64"
	echo "  $0 keys generate operator.key        # Generate key, save to file"
	echo
	echo "  # Using custom GCS bucket"
	echo "  $0 --bucket=my-company-terraform-state deploy create staging"
	echo "  GCS_STATE_BUCKET=my-bucket $0 deploy create staging  # Via environment variable"
}

################################################################################
# Main execution block.
################################################################################
main() {
	local auto_yes=false
	local bucket_override=""

	# Parse flags
	while [[ $# -gt 0 ]]; do
		case "$1" in
		-y)
			auto_yes=true
			shift
			;;
		--bucket)
			if [[ -z "$2" ]]; then
				error "--bucket requires a value"
			fi
			bucket_override="$2"
			shift 2
			;;
		--bucket=*)
			bucket_override="${1#--bucket=}"
			shift
			;;
		*)
			break
			;;
		esac
	done
	
	# Export bucket override for use by common.sh functions
	if [[ -n "$bucket_override" ]]; then
		export OVERRIDE_GCS_BUCKET="$bucket_override"
	fi

	local command="${1-}"
	local subcommand="${2-}"
	local env="${3-}"

	if [[ -z "$command" ]]; then
		usage
		exit 1
	fi

	case "$command" in
	image)
		case "$subcommand" in
		build)
			shift 2 # Remove command and subcommand
			image_build "$@"
			;;
		push)
			shift 2 # Remove command and subcommand
			image_push "$@"
			;;
		list)
			shift 2 # Remove command and subcommand
			image_list "$@"
			;;
		*) error "Invalid subcommand for 'image'. Use 'build', 'push', or 'list'." ;;
		esac
		;;

	ci)
		check_deps gcloud
		resolve_gcp_identity
		case "$subcommand" in
		setup)
			if [[ "$auto_yes" == true ]]; then AUTO_YES=true cicd_setup; else cicd_setup; fi
			;;
		teardown)
			if [[ "$auto_yes" == true ]]; then
				cicd_teardown
			else
				read -p "Are you sure you want to delete all CI/CD resources in project ${RESOLVED_PROJECT_ID}? [y/N] " -n 1 -r
				echo
				if [[ $REPLY =~ ^[Yy]$ ]]; then cicd_teardown; else info "Teardown cancelled."; fi
			fi
			;;
		*) error "Invalid subcommand for 'ci'. Use 'setup' or 'teardown'." ;;
		esac
		;;

	cluster)
		case "$subcommand" in
		create)
			case "$env" in
			gke) cluster_create_gke ;;
			*) error "Invalid or missing environment for 'cluster create'. Use 'gke'." ;;
			esac
			;;
		destroy)
			case "$env" in
			gke)
				if [[ "$auto_yes" == true ]]; then
					cluster_destroy_gke
				else
					read -p "Are you sure you want to delete the GKE cluster '${GKE_CLUSTER_NAME}'? [y/N] " -n 1 -r
					echo
					if [[ $REPLY =~ ^[Yy]$ ]]; then cluster_destroy_gke; else info "Cluster deletion cancelled."; fi
				fi
				;;
			*) error "Invalid or missing environment for 'cluster destroy'. Use 'gke'." ;;
			esac
			;;
		*) error "Invalid subcommand for 'cluster'. Use 'create' or 'destroy'." ;;
		esac
		;;

	keys)
		case "$subcommand" in
		generate)
			if [[ -z "$env" ]]; then error "Missing output file for 'keys generate'. Provide a file path."; fi
			generate_operator_key "$env"
			;;
		*) error "Invalid subcommand for 'keys'. Use 'generate'." ;;
		esac
		;;

	deploy)
		check_deps tofu
		resolve_gcp_identity
		case "$subcommand" in
		create)
			if [[ -z "$env" ]]; then error "Missing environment name for 'deploy create'. Provide an environment (staging, production, dev-username)."; fi
			deploy_create "$env"
			;;
		destroy)
			if [[ -z "$env" ]]; then error "Missing environment name for 'deploy destroy'. Provide an environment."; fi
			if [[ "$auto_yes" == true ]]; then
				deploy_destroy "$env" --auto-approve
			else
				deploy_destroy "$env"
			fi
			;;
		status)
			if [[ -z "$env" ]]; then error "Missing environment name for 'deploy status'. Provide an environment."; fi
			deploy_status "$env"
			;;
		plan)
			if [[ -z "$env" ]]; then error "Missing environment name for 'deploy plan'. Provide an environment."; fi
			deploy_plan "$env"
			;;
		*) error "Invalid subcommand for 'deploy'. Use 'create', 'destroy', 'status', or 'plan'." ;;
		esac
		;;

	dev)
		case "$subcommand" in
		--help | help | -h)
			cat <<'EOF'
TDX Development Environment Commands
===================================

Usage: ./infra.sh dev <command> [options]

Commands:
  create [--dedicated]     Create a new TDX-enabled development VM
                          --dedicated  Use dedicated (non-preemptible) instance
  
  ssh [-- <command>]       Connect to the development VM via SSH
                          -- <command>  Execute a specific command on the VM
  
  push [<directory>]       Sync local code to the development VM
                          <directory>   Source directory (defaults to current)
  
  destroy                  Delete the development VM (with confirmation)
  cleanup                  Delete VM managed by infra.sh (with confirmation)  
  status                   Show development VM status and details
  
  container [start|restart|status] [-d]
                          Manage the development container
                          -d, --detached  Start without connecting
  
  local [options]          Run development container locally

Environment Variables:
  TDX_VM_NAME              VM name (default: nxcc-tdx-dev)
  TDX_VM_ZONE              GCP zone (default: europe-west4-a)
  TDX_VM_MACHINE_TYPE      Machine type (default: c3-standard-4)
  TDX_VM_PREEMPTIBLE       Use preemptible instance (default: true)
  NXCC_DEV_IMAGE           Container image (default: ghcr.io/nxcc-bridge/dev:latest)

Examples:
  ./infra.sh dev create            # Create preemptible development VM
  ./infra.sh dev create --dedicated   # Create dedicated development VM
  ./infra.sh dev ssh               # Connect to VM
  ./infra.sh dev ssh -- 'docker ps'   # Execute command on VM
  ./infra.sh dev push              # Sync current directory
  ./infra.sh dev container start   # Start development container
  ./infra.sh dev status            # Check VM status
  
Note: The development VM includes TDX (Trust Domain Extensions) support for
confidential computing and automatically sets up the NXCC development container.
EOF
			;;
		create)
			check_deps tofu yq gcloud ssh
			resolve_gcp_identity
			shift 2
			dev_create_vm "$@"
			;;
		ssh)
			check_deps tofu yq gcloud ssh
			resolve_gcp_identity
			shift 2
			dev_ssh "$@"
			;;
		push)
			check_deps tofu yq gcloud ssh
			resolve_gcp_identity
			shift 2
			dev_sync "$@"
			;;
		destroy)
			check_deps tofu yq gcloud
			resolve_gcp_identity
			if [[ "$auto_yes" == true ]]; then
				dev_destroy_vm
			else
				dev_destroy_vm  # topology_destroy has its own confirmation
			fi
			;;
		cleanup)
			# Alias for destroy (backwards compatibility)
			check_deps tofu yq gcloud
			resolve_gcp_identity
			dev_destroy_vm
			;;
		status)
			check_deps tofu yq gcloud
			resolve_gcp_identity
			dev_status
			;;
		container)
			check_deps docker gcloud ssh
			shift 2
			dev_manage_container start "$@"
			;;
		local)
			check_deps docker gcloud ssh
			shift 2
			dev_run_container "$@"
			;;
		*) error "Invalid subcommand for 'dev'. Use 'create', 'ssh', 'push', 'destroy', 'status', 'cleanup', 'container', 'local', or '--help'." ;;
		esac
		;;

	*)
		usage
		exit 1
		;;
	esac
}

# Execute the main function, passing all script arguments.
main "$@"
