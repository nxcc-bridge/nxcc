#!/bin/bash
#
# Manages cloud and local Kubernetes resources for the nXCC confidential workload.
#
# This script is the main entrypoint. It sources its functionality from the
# scripts in the ./lib/ directory.

set -e
set -o pipefail

# The root directory of the library scripts, relative to this script's location.
LIB_DIR="$(dirname "$0")/lib"

# Source all the functional components.
# common.sh must be first as others depend on it.
source "${LIB_DIR}/common.sh"
source "${LIB_DIR}/ci.sh"
source "${LIB_DIR}/cluster.sh"
source "${LIB_DIR}/k8s.sh"


################################################################################
# Displays usage information.
################################################################################
usage() {
  echo "Usage: $0 <command> <subcommand> [args]"
  echo
  echo "Manages cloud (GCP) and local (KinD) resources for the nXCC application."
  echo
  echo "Commands:"
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
  echo "  k8s <deploy|destroy> <env>"
  echo "    Manages the application deployment via Helm chart."
  echo "      <env>: debug | staging | prod"
  echo "      deploy:   Deploys or upgrades the application to the specified environment."
  echo "      destroy:  Uninstalls the application from the specified environment."
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
  local command="${1-}"
  local subcommand="${2-}"
  local env="${3-}"

  if [[ -z "$command" ]]; then
    usage
    exit 1
  fi

  case "$command" in
    ci)
      check_deps gcloud
      resolve_gcp_identity # CI commands always need GCP identity
      case "$subcommand" in
        setup)
          cicd_setup
          ;;
        teardown)
          read -p "Are you sure you want to delete all CI/CD resources in project ${PROJECT_ID}? [y/N] " -n 1 -r; echo
          if [[ $REPLY =~ ^[Yy]$ ]]; then cicd_teardown; else info "Teardown cancelled."; fi
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
              read -p "Are you sure you want to delete the GKE cluster '${GKE_CLUSTER_NAME}'? [y/N] " -n 1 -r; echo
              if [[ $REPLY =~ ^[Yy]$ ]]; then cluster_destroy_gke; else info "Cluster deletion cancelled."; fi
              ;;
            kind)
              read -p "Are you sure you want to delete the KinD cluster '${KIND_CLUSTER_NAME}'? [y/N] " -n 1 -r; echo
              if [[ $REPLY =~ ^[Yy]$ ]]; then cluster_destroy_kind; else info "Cluster deletion cancelled."; fi
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
          read -p "Are you sure you want to uninstall the application '${release_to_destroy}' from the '${env}' environment? [y/N] " -n 1 -r; echo
          if [[ $REPLY =~ ^[Yy]$ ]]; then k8s_destroy "$env"; else info "Application uninstall cancelled."; fi
          ;;
        *)
          error "Invalid subcommand for 'k8s'. Use 'deploy' or 'destroy'."
          ;;
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
