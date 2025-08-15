#!/bin/bash
#
# Manages cloud and local Kubernetes resources for the nXCC confidential workload.
#
# This script is the main entrypoint. It sources its functionality from the
# scripts in the ./lib/ directory.
#
# IDEMPOTENCY GUARANTEE:
# All operations in this script are designed to be idempotent and safe to run multiple times:
#
# - `cluster create <env>`: Checks if cluster exists before creating, skips if already present
# - `k8s deploy <env>`: Helm upgrade --install ensures idempotent deployments  
# - `build <target>`: Docker builds are layer-cached and idempotent
# - `ci setup`: Creates resources only if they don't exist, updates policies if needed
#
# This allows the script to be used safely in automation, CI/CD pipelines, and development
# workflows without fear of duplicate resource creation or deployment conflicts.
#
# For e2e testing and development, you can run the same commands multiple times to ensure
# your environment reaches the desired state regardless of starting conditions.

set -e
set -o pipefail

# The root directory of the library scripts, relative to this script's location.
LIB_DIR="$(dirname "$0")/lib"

# Source all the functional components.
# common.sh must be first as others depend on it.
# shellcheck disable=SC1091  # Library files are sourced dynamically
source "${LIB_DIR}/common.sh"
source "${LIB_DIR}/build.sh"
source "${LIB_DIR}/ci.sh"
source "${LIB_DIR}/cluster.sh"
source "${LIB_DIR}/k8s.sh"
source "${LIB_DIR}/test.sh"


################################################################################
# Displays usage information.
################################################################################
usage() {
  echo "Usage: $0 [-y] <command> <subcommand> [args]"
  echo
  echo "Manages cloud (GCP) and local (KinD) resources for the nXCC application."
  echo
  echo "Options:"
  echo "  -y  Automatically answer 'yes' to all interactive confirmation prompts."
  echo
  echo "Commands:"
  echo "  build <local|gcp>"
  echo "    Builds Docker images for Intel TDX TEE (defaults to amd64)."
  echo "      local:    Builds for local KinD deployment (debug mode default)."
  echo "      gcp:      Builds and pushes to GCP Artifact Registry (release mode default)."
  echo "    Environment variables:"
  echo "      BUILD_MODE=debug|release              Build mode (debug faster, release optimized)"
  echo "      BUILD_PLATFORMS=linux/amd64          Target platforms (defaults to linux/amd64)"
  echo "                                            Use linux/amd64,linux/arm64 for multi-arch"
  echo
  echo "  ci <setup|teardown>"
  echo "    Manages GCP resources for CI/CD (Service Account, WIF, Artifact Registry)."
  echo "      setup:    Creates and configures all CI/CD resources."
  echo "      teardown: Deletes all CI/CD resources."
  echo
  echo "  cluster <create|destroy> <env>"
  echo "    Manages the Kubernetes cluster."
  echo "      <env>: gke | kind"
  echo "      create:   Creates the specified cluster."
  echo "      destroy:  Deletes the specified cluster."
  echo
  echo "  k8s <deploy|destroy|dump-debug> <env>"
  echo "    Manages the application deployment via Helm chart."
  echo "      <env>: debug | staging | prod"
  echo "      deploy:      Deploys or upgrades the application to the specified environment."
  echo "      destroy:     Uninstalls the application from the specified environment."
  echo "      dump-debug:  Dumps diagnostic information for a failed deployment."
  echo
  echo "  test <env>"
  echo "    Tests HTTP connectivity to the deployed NXCC node."
  echo "      <env>: debug | staging | prod"
  echo
  echo "Environment Notes:"
  echo "  - 'debug' environment is intended for the 'kind' cluster."
  echo "  - 'staging' and 'prod' environments are intended for the 'gke' cluster."
  echo
  echo "GCP Identity:"
  echo "  For 'ci' and 'gke' commands, the script will resolve your GCP identity automatically."
  echo "  You can override this by setting GCP_ACCOUNT and GCP_PROJECT_ID environment variables."
}

################################################################################
# Main execution block.
################################################################################
main() {
  local auto_yes=false
  
  # Parse -y flag
  if [[ "$1" == "-y" ]]; then
    auto_yes=true
    shift
  fi
  
  local command="${1-}"
  local subcommand="${2-}"
  local env="${3-}"

  if [[ -z "$command" ]]; then
    usage
    exit 1
  fi

  case "$command" in
    build)
      case "$subcommand" in
        local)
          build_local_image
          ;;
        gcp)
          build_gcp_image
          ;;
        *)
          error "Invalid subcommand for 'build'. Use 'local' or 'gcp'."
          ;;
      esac
      ;;

    ci)
      check_deps gcloud
      resolve_gcp_identity # CI commands always need GCP identity
      case "$subcommand" in
        setup)
          if [[ "$auto_yes" == true ]]; then
            AUTO_YES=true cicd_setup
          else
            cicd_setup
          fi
          ;;
        teardown)
          if [[ "$auto_yes" == true ]]; then
            cicd_teardown
          else
            read -p "Are you sure you want to delete all CI/CD resources in project ${RESOLVED_PROJECT_ID}? [y/N] " -n 1 -r; echo
            if [[ $REPLY =~ ^[Yy]$ ]]; then cicd_teardown; else info "Teardown cancelled."; fi
          fi
          ;;
        *)
          error "Invalid subcommand for 'ci'. Use 'setup' or 'teardown'."
          ;;
      esac
      ;;

    cluster)
      case "$subcommand" in
        create)
          case "$env" in
            gke) cluster_create_gke ;;
            kind) cluster_create_kind ;;
            "") error "Missing environment for 'cluster create'. Use 'gke' or 'kind'." ;;
            *) error "Invalid environment for 'cluster create'. Use 'gke' or 'kind'." ;;
          esac
          ;;
        destroy)
          case "$env" in
            gke)
              if [[ "$auto_yes" == true ]]; then
                cluster_destroy_gke
              else
                read -p "Are you sure you want to delete the GKE cluster '${GKE_CLUSTER_NAME}'? [y/N] " -n 1 -r; echo
                if [[ $REPLY =~ ^[Yy]$ ]]; then cluster_destroy_gke; else info "Cluster deletion cancelled."; fi
              fi
              ;;
            kind)
              if [[ "$auto_yes" == true ]]; then
                cluster_destroy_kind
              else
                read -p "Are you sure you want to delete the KinD cluster '${KIND_CLUSTER_NAME}'? [y/N] " -n 1 -r; echo
                if [[ $REPLY =~ ^[Yy]$ ]]; then cluster_destroy_kind; else info "Cluster deletion cancelled."; fi
              fi
              ;;
            "") error "Missing environment for 'cluster destroy'. Use 'gke' or 'kind'." ;;
            *) error "Invalid environment for 'cluster destroy'. Use 'gke' or 'kind'." ;;
          esac
          ;;
        *)
          error "Invalid subcommand for 'cluster'. Use 'create' or 'destroy'."
          ;;
      esac
      ;;

    k8s)
      case "$subcommand" in
        deploy)
          if [[ -z "$env" ]]; then error "Missing environment for 'k8s deploy'. Use 'debug', 'staging', or 'prod'."; fi
          k8s_deploy "$env"
          ;;
        destroy)
          if [[ -z "$env" ]]; then error "Missing environment for 'k8s destroy'. Use 'debug', 'staging', or 'prod'."; fi
          local release_to_destroy="nxcc-node-${env}"
          if [[ "$auto_yes" == true ]]; then
            k8s_destroy "$env"
          else
            read -p "Are you sure you want to uninstall the application '${release_to_destroy}' from the '${env}' environment? [y/N] " -n 1 -r; echo
            if [[ $REPLY =~ ^[Yy]$ ]]; then k8s_destroy "$env"; else info "Application uninstall cancelled."; fi
          fi
          ;;
        dump-debug)
          if [[ -z "$env" ]]; then error "Missing environment for 'k8s dump-debug'. Use 'debug', 'staging', or 'prod'."; fi
          k8s_dump_debug_info "$env"
          ;;
        *)
          error "Invalid subcommand for 'k8s'. Use 'deploy', 'destroy', or 'dump-debug'."
          ;;
      esac
      ;;

    test)
      if [[ -z "$subcommand" ]]; then error "Missing environment for 'test'. Use 'debug', 'staging', or 'prod'."; fi
      test_connectivity "$subcommand"
      ;;

    *)
      usage
      exit 1
      ;;
  esac
}

# Execute the main function, passing all script arguments.
main "$@"
