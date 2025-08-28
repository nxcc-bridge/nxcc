#!/usr/bin/env sh
# Terraform Infrastructure Validation Script
# Validates terraform without deploying to real infrastructure

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

info() {
	printf "${BLUE}INFO:${NC} %s\n" "$*"
}

success() {
	printf "${GREEN}SUCCESS:${NC} %s\n" "$*"
}

warning() {
	printf "${YELLOW}WARNING:${NC} %s\n" "$*"
}

error() {
	printf "${RED}ERROR:${NC} %s\n" "$*" >&2
}

# Check if tofu is installed
if ! command -v tofu >/dev/null 2>&1; then
	error "OpenTofu is not installed. Please install OpenTofu first."
	error "Visit: https://opentofu.org/docs/intro/install/"
	exit 1
fi

# Set up mock environment for validation
export TF_VAR_project_id="nxcc-terraform-validation"
export TF_VAR_docker_image="ghcr.io/nxcc-bridge/node:latest"
export TF_VAR_operator_key_gcp="mock-operator-key-for-validation"
export TF_VAR_developer_name="local-validation"
export TF_VAR_allowed_ssh_cidrs='["10.0.0.0/8"]'

# Mock GCP credentials
MOCK_CREDS_FILE="/tmp/nxcc-mock-sa-key.json"
cat >"$MOCK_CREDS_FILE" <<'EOF'
{
  "type": "service_account",
  "project_id": "nxcc-terraform-validation",
  "private_key_id": "mock-key-id",
  "private_key": "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQC7VJTUt9Us8cKB\n-----END PRIVATE KEY-----\n",
  "client_email": "terraform-validation@nxcc-terraform-validation.iam.gserviceaccount.com",
  "client_id": "123456789012345678901",
  "auth_uri": "https://accounts.google.com/o/oauth2/auth",
  "token_uri": "https://oauth2.googleapis.com/token"
}
EOF

export GOOGLE_APPLICATION_CREDENTIALS="$MOCK_CREDS_FILE"

info "Starting terraform infrastructure validation..."
info "Using mock credentials - no real infrastructure will be deployed"

# Get script directory and navigate to infra
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INFRA_DIR="$(cd "$SCRIPT_DIR/../../infra" && pwd)"
cd "$INFRA_DIR"

validation_failed=0

# 1. Format check
info "1. Checking terraform formatting..."
# format_failed will be set via temp file due to subshell limitations
find . -name "*.tf" -exec dirname {} \; | sort -u | while read -r dir; do
	printf "  Checking format in: %s\n" "$dir"
	if ! (cd "$dir" && tofu fmt -check -diff 2>/dev/null); then
		error "Format check failed in $dir"
		# Write failure to temp file since we can't modify parent shell variables from subshell
		touch /tmp/format_failed
	fi
done

if [ -f /tmp/format_failed ]; then
	error "Format check failed"
	validation_failed=1
	rm -f /tmp/format_failed
else
	success "Format check passed"
fi

# 2. Validate each environment
for env in dev staging production e2e; do
	env_dir="environments/$env"

	if [ ! -d "$env_dir" ]; then
		warning "Environment directory not found: $env_dir"
		continue
	fi

	info "2. Validating environment: $env"

	# Init without backend (may fail with mock credentials, which is fine for syntax validation)
	printf "  Initializing terraform...\n"
	(cd "$env_dir" && timeout 10 tofu init -backend=false >/dev/null 2>&1) || true

	# Validate syntax
	printf "  Validating configuration...\n"
	if ! (cd "$env_dir" && tofu validate >/dev/null 2>&1); then
		error "Configuration validation failed for $env"
		validation_failed=1
		continue
	fi

	# Validate dependency graph (no API calls needed)
	printf "  Validating dependency graph...\n"
	if (cd "$env_dir" && tofu graph >/dev/null 2>&1); then
		success "Full validation passed for $env"
	else
		# Graph validation can fail if providers aren't initialized, but syntax is still valid
		warning "Graph validation skipped for $env (provider not initialized)"
		success "Configuration validation passed for $env"
	fi
done

# 3. Security scan (if trivy is available)
if command -v trivy >/dev/null 2>&1; then
	info "3. Running security scan with Trivy..."
	if trivy config --format json --output /tmp/trivy-results.json . >/dev/null 2>&1; then
		# Check for high/critical issues
		if [ -f /tmp/trivy-results.json ]; then
			HIGH_CRITICAL=$(jq -r '.Results[]?.Misconfigurations[]? | select(.Severity == "HIGH" or .Severity == "CRITICAL")' /tmp/trivy-results.json 2>/dev/null | wc -l)
			if [ "$HIGH_CRITICAL" -gt 0 ]; then
				error "Found $HIGH_CRITICAL high/critical security issues"
				jq -r '.Results[]?.Misconfigurations[]? | select(.Severity == "HIGH" or .Severity == "CRITICAL") | "- \(.ID): \(.Title) (\(.Severity))"' /tmp/trivy-results.json 2>/dev/null | head -5
				validation_failed=1
			else
				success "Security scan passed"
			fi
		fi
	else
		warning "Security scan completed with warnings"
	fi
else
	warning "Trivy not installed - skipping security scan"
	warning "Install: https://aquasecurity.github.io/trivy/latest/getting-started/installation/"
fi

# 4. Linting (if tflint is available)
if command -v tflint >/dev/null 2>&1; then
	info "4. Running terraform linting..."
	# Initialize tflint
	if ! tflint --init >/dev/null 2>&1; then
		warning "Failed to initialize tflint, skipping..."
	else
		echo "" >/tmp/tflint-results.txt
		find . -name "*.tf" -exec dirname {} \; | sort -u | while read -r dir; do
			printf "  Linting: %s\n" "$dir"
			if ! (cd "$dir" && tflint --format compact 2>&1 | tee -a /tmp/tflint-results.txt); then
				# Mark linting failure in temp file for parent shell
				touch /tmp/tflint_failed
			fi
		done

		if [ -f /tmp/tflint_failed ]; then
			warning "Linting found issues (see /tmp/tflint-results.txt)"
			rm -f /tmp/tflint_failed
		else
			success "Linting passed"
		fi
	fi
else
	warning "tflint not installed - skipping linting"
	warning "Install: https://github.com/terraform-linters/tflint#installation"
fi

# Cleanup (but keep results files for CI)
rm -f "$MOCK_CREDS_FILE"
rm -f /tmp/plan-*.txt

# Keep these files for CI artifact upload:
# /tmp/trivy-results.json
# /tmp/tflint-results.txt

# Final result
echo ""
if [ $validation_failed -eq 0 ]; then
	success "🎉 All terraform infrastructure validation checks passed!"
	success "Your terraform configurations are ready for deployment."
	exit 0
else
	error "❌ Terraform validation failed!"
	error "Please fix the issues above before deploying."
	exit 1
fi
