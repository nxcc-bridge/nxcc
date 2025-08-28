#!/bin/bash
#
# Functions for managing Terraform state buckets and locking.
# This script is intended to be sourced, not executed directly.

################################################################################
# Sets up GCS state bucket and enables state locking for an environment
# Arguments:
#   $1: Environment name (staging, prod, dev-username, etc.)
################################################################################
setup_state_backend() {
    local env="$1"
    local bucket_name="${RESOLVED_PROJECT_ID}-nxcc-terraform-state"
    local state_prefix="environments/${env}"
    
    # Handle dev environments separately
    if [[ "$env" =~ ^dev- ]]; then
        local username="${env#dev-}"
        state_prefix="dev/${username}"
    fi
    
    info "Setting up state backend for environment: $env"
    info "Bucket: gs://${bucket_name}"
    info "Prefix: ${state_prefix}"
    
    # Create state bucket if it doesn't exist
    if ! gsutil ls "gs://${bucket_name}" &>/dev/null; then
        info "Creating state bucket: $bucket_name"
        
        # Create bucket with versioning and lifecycle management
        if gsutil mb -p "$RESOLVED_PROJECT_ID" -c STANDARD -l "${GCP_AR_LOCATION:-europe-west4}" "gs://${bucket_name}"; then
            success "State bucket created: $bucket_name"
        else
            error "Failed to create state bucket: $bucket_name"
        fi
        
        # Enable versioning for state file recovery
        info "Enabling versioning on state bucket..."
        gsutil versioning set on "gs://${bucket_name}"
        
        # Set lifecycle policy to manage old versions
        create_state_lifecycle_policy "gs://${bucket_name}"
        
        success "State bucket configured with versioning and lifecycle management"
    else
        info "State bucket already exists: $bucket_name"
    fi
    
    # Verify bucket access
    if ! gsutil ls "gs://${bucket_name}" &>/dev/null; then
        error "Cannot access state bucket: $bucket_name. Check permissions for account: $RESOLVED_GCP_ACCOUNT"
    fi
    
    # Create local state tracking directory
    local state_tracking_dir="$HOME/.nxcc/topologies/$env"
    mkdir -p "$state_tracking_dir"
    
    # Save state backend configuration for reference
    cat > "$state_tracking_dir/backend.json" <<EOF
{
  "bucket": "$bucket_name",
  "prefix": "$state_prefix",
  "project": "$RESOLVED_PROJECT_ID",
  "created": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "created_by": "$RESOLVED_GCP_ACCOUNT"
}
EOF
    
    success "State backend ready: gs://${bucket_name}/${state_prefix}"
}

################################################################################
# Creates lifecycle policy for state bucket to manage old versions
# Arguments:
#   $1: Bucket URI (e.g., gs://bucket-name)
################################################################################
create_state_lifecycle_policy() {
    local bucket_uri="$1"
    local lifecycle_file="/tmp/nxcc-state-lifecycle-$$.json"
    
    # Create lifecycle policy to delete old versions after 90 days
    # and non-current versions after 30 days
    cat > "$lifecycle_file" <<'EOF'
{
  "rule": [
    {
      "action": {
        "type": "Delete"
      },
      "condition": {
        "age": 90,
        "isLive": false
      }
    },
    {
      "action": {
        "type": "Delete"
      },
      "condition": {
        "numNewerVersions": 10
      }
    }
  ]
}
EOF
    
    info "Setting lifecycle policy on state bucket..."
    if gsutil lifecycle set "$lifecycle_file" "$bucket_uri"; then
        success "Lifecycle policy applied to state bucket"
    else
        warn "Failed to set lifecycle policy on state bucket"
    fi
    
    rm -f "$lifecycle_file"
}

################################################################################
# Verifies state backend access and configuration
# Arguments:
#   $1: Environment name
################################################################################
verify_state_backend() {
    local env="$1"
    local bucket_name="${RESOLVED_PROJECT_ID}-nxcc-terraform-state"
    local state_prefix="environments/${env}"
    
    # Handle dev environments
    if [[ "$env" =~ ^dev- ]]; then
        local username="${env#dev-}"
        state_prefix="dev/${username}"
    fi
    
    info "Verifying state backend for environment: $env"
    
    # Check bucket exists and is accessible
    if ! gsutil ls "gs://${bucket_name}" &>/dev/null; then
        error "State bucket not accessible: gs://${bucket_name}"
    fi
    
    # Check versioning is enabled
    local versioning_status=$(gsutil versioning get "gs://${bucket_name}" | grep "Enabled" || echo "")
    if [[ -z "$versioning_status" ]]; then
        warn "Versioning is not enabled on state bucket"
    else
        success "State bucket versioning is enabled"
    fi
    
    # Check if state file exists
    local state_path="gs://${bucket_name}/${state_prefix}/default.tfstate"
    if gsutil ls "$state_path" &>/dev/null; then
        info "State file exists: $state_path"
        
        # Show last modified time
        local last_modified=$(gsutil stat "$state_path" | grep "Update time:" | cut -d: -f2- | xargs)
        info "Last modified: $last_modified"
    else
        info "No existing state file found (new environment)"
    fi
    
    success "State backend verification completed"
}

################################################################################
# Lists all environments with state files
################################################################################
list_state_environments() {
    local bucket_name="${RESOLVED_PROJECT_ID}-nxcc-terraform-state"
    
    info "Listing environments with Terraform state..."
    
    if ! gsutil ls "gs://${bucket_name}" &>/dev/null; then
        warn "No state bucket found: gs://${bucket_name}"
        info "Run './infra.sh deploy create <env>' to initialize"
        return 0
    fi
    
    echo
    info "Production Environments:"
    gsutil ls "gs://${bucket_name}/environments/" 2>/dev/null | \
        sed 's|.*/environments/||; s|/$||' | \
        grep -v '^$' | \
        sed 's/^/  /' || echo "  None found"
    
    echo
    info "Dev Environments:"
    gsutil ls "gs://${bucket_name}/dev/" 2>/dev/null | \
        sed 's|.*/dev/||; s|/$||' | \
        grep -v '^$' | \
        sed 's/^/  dev-/' || echo "  None found"
    
    echo
}

################################################################################
# Backs up state file to local storage
# Arguments:
#   $1: Environment name
#   $2: Backup directory (optional, defaults to ~/.nxcc/backups)
################################################################################
backup_state() {
    local env="$1"
    local backup_dir="${2:-$HOME/.nxcc/backups}"
    local bucket_name="${RESOLVED_PROJECT_ID}-nxcc-terraform-state"
    local state_prefix="environments/${env}"
    
    # Handle dev environments
    if [[ "$env" =~ ^dev- ]]; then
        local username="${env#dev-}"
        state_prefix="dev/${username}"
    fi
    
    local timestamp=$(date +%Y%m%d-%H%M%S)
    local backup_file="${backup_dir}/${env}-${timestamp}.tfstate"
    
    info "Backing up state for environment: $env"
    
    mkdir -p "$backup_dir"
    
    local state_path="gs://${bucket_name}/${state_prefix}/default.tfstate"
    
    if gsutil cp "$state_path" "$backup_file" 2>/dev/null; then
        success "State backed up to: $backup_file"
        
        # Keep only last 10 backups per environment
        cleanup_old_backups "$env" "$backup_dir"
    else
        error "Failed to backup state file: $state_path"
    fi
}

################################################################################
# Cleans up old backup files, keeping only the most recent ones
# Arguments:
#   $1: Environment name
#   $2: Backup directory
################################################################################
cleanup_old_backups() {
    local env="$1"
    local backup_dir="$2"
    local keep_count=10
    
    # Find and remove old backups, keeping the most recent ones
    find "$backup_dir" -name "${env}-*.tfstate" -type f | \
        sort -r | \
        tail -n +$((keep_count + 1)) | \
        while read -r old_backup; do
            info "Removing old backup: $(basename "$old_backup")"
            rm -f "$old_backup"
        done
}

################################################################################
# Restores state file from backup
# Arguments:
#   $1: Environment name
#   $2: Backup file path
################################################################################
restore_state() {
    local env="$1"
    local backup_file="$2"
    local bucket_name="${RESOLVED_PROJECT_ID}-nxcc-terraform-state"
    local state_prefix="environments/${env}"
    
    # Handle dev environments
    if [[ "$env" =~ ^dev- ]]; then
        local username="${env#dev-}"
        state_prefix="dev/${username}"
    fi
    
    if [[ ! -f "$backup_file" ]]; then
        error "Backup file not found: $backup_file"
    fi
    
    warn "This will overwrite the current state for environment: $env"
    read -p "Are you sure you want to restore from backup? [y/N] " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        info "Restore cancelled."
        return 0
    fi
    
    info "Restoring state from backup: $backup_file"
    
    local state_path="gs://${bucket_name}/${state_prefix}/default.tfstate"
    
    if gsutil cp "$backup_file" "$state_path"; then
        success "State restored from backup"
    else
        error "Failed to restore state from backup"
    fi
}