#!/bin/bash
#
# Blockchain setup functions for E2E tests
#

# Source common.sh
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

# Deploy Identity contract to Anvil blockchain
deploy_identity_contract() {
	local anvil_rpc_url="$1"
	local deployer_private_key="$2"

	log "Deploying Identity contract to $anvil_rpc_url..." >&2

	# Get project root dynamically
	local script_dir
	script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
	local project_root
	project_root="$(cd "$script_dir/../.." && pwd)"

	cd "$project_root/contracts/evm" || error "Failed to change to contracts directory"

	# Install dependencies if needed
	if [[ ! -d "dependencies" ]]; then
		log "Installing Solidity dependencies..." >&2
		forge soldeer install
	fi

	# Build contracts
	log "Building contracts..." >&2
	forge build >&2

	# Deploy Identity contract using the tested deploy script
	local deploy_output
	deploy_output=$(PRIVATE_KEY="$deployer_private_key" forge script script/DeployIdentity.s.sol \
		--rpc-url "$anvil_rpc_url" \
		--broadcast 2>&1)

	local contract_address

	# Check if deployment succeeded or if contract already exists
	if echo "$deploy_output" | grep -q "CreateCollision"; then
		# Contract already deployed, compute the address
		contract_address="0xb1c985140805a55bf6d5Ea42232B73023dc51eE0" # Default salt CREATE2 address
		log "Identity contract already deployed at deterministic address: $contract_address" >&2
	else
		# Extract the contract address from successful deployment
		contract_address=$(echo "$deploy_output" | grep "Identity deployed to:" | awk '{print $4}')

		if [[ -z "$contract_address" ]]; then
			error "Failed to deploy Identity contract. Output: $deploy_output"
		fi
	fi

	log "Identity contract deployed at: $contract_address" >&2
	printf "%s" "$contract_address"
}

# Create identity NFT with policy using NXCC CLI
create_identity_with_policy() {
	local anvil_rpc_url="$1"
	local contract_address="$2"
	local policy_bundle_path="$3"
	local policy_manifest_path="$4"
	local signer_private_key="$5"

	log "Creating identity with policy using cast commands..." >&2

	# First upload the policy bundle to get its URL
	local bundle_url
	bundle_url=$(upload_policy_bundle "$policy_bundle_path")

	# Then upload the policy manifest pointing to the bundle URL
	local policy_url
	policy_url=$(upload_policy_manifest "$policy_manifest_path" "$bundle_url")

	# Create identity by minting NFT with policy URL
	log "Minting new identity NFT with policy URL..." >&2
	local mint_output
	if ! mint_output=$(cast send "$contract_address" "mint(string)" "$policy_url" \
		--rpc-url "$anvil_rpc_url" \
		--private-key "$signer_private_key" 2>&1); then
		error "Failed to mint identity NFT: $mint_output"
	fi

	# Get the transaction hash from output (exclude logs JSON)
	local tx_hash
	tx_hash=$(echo "$mint_output" | grep "^transactionHash" | awk '{print $2}')

	if [[ -z "$tx_hash" ]]; then
		error "Failed to extract transaction hash from mint output: $mint_output"
	fi

	log "Identity minted in transaction: $tx_hash" >&2

	# Get the minted token ID using a simpler approach
	local token_id
	# Use transaction receipt to get the token ID from Transfer event
	if [[ -n "$tx_hash" ]]; then
		# Wait a moment for transaction to be included
		sleep 1
		local receipt_logs
		receipt_logs=$(cast receipt "$tx_hash" --rpc-url "$anvil_rpc_url" 2>/dev/null | grep -A 1 "Transfer")
		# Extract the token ID from the Transfer event (last topic)
		token_id=$(echo "$receipt_logs" | grep "topics" -A 10 | grep "0x" | tail -1 | grep -o '0x[0-9a-fA-F]*' | tail -1)
		if [[ -n "$token_id" ]]; then
			token_id=$(printf "%d" "$token_id")
		fi
	fi

	# Fallback: find the highest token ID owned by the signer
	if [[ -z "$token_id" ]]; then
		local signer_address
		signer_address=$(cast wallet address --private-key "$signer_private_key")
		# Try token IDs 1-20 to find the latest one owned by signer
		for test_id in {20..1}; do
			local owner
			owner=$(cast call "$contract_address" "ownerOf(uint256)" "$test_id" --rpc-url "$anvil_rpc_url" 2>/dev/null)
			# Remove 0x prefix and padding, compare addresses
			local owner_clean="${owner#0x}"
			owner_clean="${owner_clean#000000000000000000000000}"
			local signer_clean="${signer_address#0x}"
			if [[ "$(echo "$owner_clean" | tr '[:upper:]' '[:lower:]')" == "$(echo "$signer_clean" | tr '[:upper:]' '[:lower:]')" ]]; then
				token_id="$test_id"
				break
			fi
		done
		log "Using latest owned token ID: $token_id" >&2
	fi

	log "Identity created with token ID: $token_id and policy URL: $policy_url" >&2
	echo "$token_id"
}

# Upload policy manifest to publicly accessible GCS bucket
upload_policy_manifest() {
	local manifest_path="$1"
	local bundle_url="$2"

	log "Uploading policy manifest to public storage..." >&2

	# Generate unique filename based on manifest content hash
	local manifest_hash
	manifest_hash=$(sha256sum "$manifest_path" | cut -d' ' -f1 | head -c 16)
	local remote_filename="policy-manifest-${manifest_hash}.json"

	# Upload to GCS bucket with public read access
	local bucket_name="nxcc-462803-e2e-policies"

	# Create bucket if it doesn't exist
	if ! gsutil ls "gs://$bucket_name" >/dev/null 2>&1; then
		log "Creating public GCS bucket for policy storage..." >&2
		gsutil mb "gs://$bucket_name"
		# Make bucket publicly readable
		gsutil iam ch allUsers:objectViewer "gs://$bucket_name"
	fi

	# Modify the manifest to point to the bundle URL
	local temp_manifest
	temp_manifest=$(mktemp)
	jq --arg bundle_url "$bundle_url" '.bundle.source = $bundle_url' "$manifest_path" >"$temp_manifest"

	# Upload the modified policy manifest
	gsutil cp "$temp_manifest" "gs://$bucket_name/$remote_filename"

	# Make the object publicly readable
	gsutil acl ch -u AllUsers:R "gs://$bucket_name/$remote_filename"

	# Clean up temp file
	rm "$temp_manifest"

	# Return the public HTTP URL
	local policy_url="https://storage.googleapis.com/$bucket_name/$remote_filename"
	log "Policy manifest uploaded to: $policy_url" >&2
	echo "$policy_url"
}

# Upload policy bundle to publicly accessible GCS bucket
upload_policy_bundle() {
	local bundle_path="$1"

	log "Uploading policy bundle to public storage..." >&2

	# Generate unique filename based on bundle content hash
	local bundle_hash
	bundle_hash=$(sha256sum "$bundle_path" | cut -d' ' -f1 | head -c 16)
	local remote_filename="policy-bundle-${bundle_hash}.json"

	# Upload to GCS bucket with public read access
	local bucket_name="nxcc-462803-e2e-policies"

	# Create bucket if it doesn't exist
	if ! gsutil ls "gs://$bucket_name" >/dev/null 2>&1; then
		log "Creating public GCS bucket for policy storage..." >&2
		gsutil mb "gs://$bucket_name"
		# Make bucket publicly readable
		gsutil iam ch allUsers:objectViewer "gs://$bucket_name"
	fi

	# Upload the policy bundle
	gsutil cp "$bundle_path" "gs://$bucket_name/$remote_filename"

	# Make the object publicly readable
	gsutil acl ch -u AllUsers:R "gs://$bucket_name/$remote_filename"

	# Return the public HTTP URL
	local policy_url="https://storage.googleapis.com/$bucket_name/$remote_filename"
	log "Policy bundle uploaded to: $policy_url" >&2
	echo "$policy_url"
}

# Bundle policy using NXCC CLI
bundle_policy() {
	local manifest_path="$1"
	local output_path="$2"

	log "Bundling policy from manifest: $manifest_path" >&2

	# Get project root dynamically
	local script_dir
	script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
	local project_root
	project_root="$(cd "$script_dir/../.." && pwd)"

	cd "$project_root" || error "Failed to change to project root"

	# Build CLI if needed
	if [[ ! -f "sdk/cli/dist/index.js" ]]; then
		log "Building NXCC CLI..." >&2
		if ! (cd sdk/cli && pnpm install && pnpm build); then
			error "Failed to build NXCC CLI"
		fi
	fi

	# Use the CLI to bundle the policy
	if ! npx --prefix sdk/cli nxcc bundle "$manifest_path" --out "$output_path"; then
		error "Failed to bundle policy with CLI"
	fi

	log "Policy bundled successfully at: $output_path" >&2
}

# Get blockchain endpoint from terraform
get_blockchain_endpoint() {
	local test_id="$1"

	# Get project root dynamically
	local script_dir
	script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
	local project_root
	project_root="$(cd "$script_dir/../.." && pwd)"

	cd "$project_root/infra/environments/e2e" || error "Failed to change to e2e environment"

	local blockchain_endpoint
	blockchain_endpoint=$(tofu output -json blockchain_endpoint 2>/dev/null)

	if [[ "$blockchain_endpoint" == "null" || -z "$blockchain_endpoint" ]]; then
		error "No blockchain endpoint found in deployment. Is the blockchain node deployed?"
	fi

	echo "$blockchain_endpoint" | jq -r '.rpc_url'
}

# Setup complete blockchain environment for E2E tests
setup_blockchain_environment() {
	local test_id="$1"

	log "Setting up blockchain environment for test: $test_id"

	# Get blockchain RPC URL
	local anvil_rpc_url
	anvil_rpc_url=$(get_blockchain_endpoint "$test_id")
	log "Using blockchain RPC: $anvil_rpc_url"

	# Wait for blockchain to be ready
	log "Waiting for blockchain to be ready..."
	local max_attempts=30
	local attempt=1
	while [[ $attempt -le $max_attempts ]]; do
		if curl -s -X POST \
			-H "Content-Type: application/json" \
			-d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
			"$anvil_rpc_url" | grep -q "0x7a69"; then
			log "Blockchain is ready"
			break
		fi
		log "Waiting for blockchain... (attempt $attempt/$max_attempts)"
		sleep 2
		((attempt++))
	done

	if [[ $attempt -gt $max_attempts ]]; then
		error "Blockchain failed to become ready within timeout"
	fi

	# Deploy Identity contract
	local deployer_pk="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80" # Anvil default[0]
	local identity_contract_address
	identity_contract_address=$(deploy_identity_contract "$anvil_rpc_url" "$deployer_pk")

	# Build and bundle TDX validation policy
	log "Building policies..."
	# Get project root dynamically
	local script_dir
	script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
	local project_root
	project_root="$(cd "$script_dir/../.." && pwd)"

	if ! (cd "$project_root/e2e/policies" && pnpm install && pnpm build); then
		error "Failed to build policies"
	fi

	local tdx_policy_bundle="$project_root/e2e/policies/tdx-policy-bundle.json"
	bundle_policy "$project_root/e2e/policies/tdx-validation-manifest.json" "$tdx_policy_bundle"

	# Create TDX validation identity with policy
	local tdx_identity_id
	tdx_identity_id=$(create_identity_with_policy "$anvil_rpc_url" "$identity_contract_address" "$tdx_policy_bundle" "$project_root/e2e/policies/tdx-validation-manifest.json" "$deployer_pk")

	# Bundle permissive policy (same code, different userdata)
	local permissive_policy_bundle="$project_root/e2e/policies/permissive-policy-bundle.json"
	bundle_policy "$project_root/e2e/policies/permissive-manifest.json" "$permissive_policy_bundle"

	# Create permissive identity with policy
	local permissive_identity_id
	permissive_identity_id=$(create_identity_with_policy "$anvil_rpc_url" "$identity_contract_address" "$permissive_policy_bundle" "$project_root/e2e/policies/permissive-manifest.json" "$deployer_pk")

	# Export environment variables for e2e tests
	export E2E_ANVIL_RPC_URL="$anvil_rpc_url"
	export E2E_IDENTITY_CONTRACT_ADDRESS="$identity_contract_address"
	export E2E_TDX_IDENTITY_ID="$tdx_identity_id"
	export E2E_PERMISSIVE_IDENTITY_ID="$permissive_identity_id"

	success "Blockchain environment setup completed"
	log "  Anvil RPC: $anvil_rpc_url"
	log "  Identity Contract: $identity_contract_address"
	log "  TDX Identity ID: $tdx_identity_id"
	log "  Permissive Identity ID: $permissive_identity_id"
}

# Generate test worker manifest with current blockchain IP
generate_test_manifest() {
	local output_path="$1"
	local identity_id="$2"
	local test_id="${3:-e2e-default}"

	# Get current blockchain endpoint
	local anvil_rpc_url
	anvil_rpc_url=$(get_blockchain_endpoint "$test_id")

	# Get contract address (should be deterministic)
	local contract_address="0xb1c985140805a55bf6d5Ea42232B73023dc51eE0"

	cat >"$output_path" <<EOF
{
  "bundle": {
    "source": "dist/worker.js",
    "hash": null
  },
  "identities": [
    [
      {
        "Gateway": {
          "gateway_url": "$anvil_rpc_url",
          "contract_address": "$contract_address",
          "token_id": $identity_id
        }
      },
      "test_secret"
    ]
  ],
  "events": [
    {
      "handler": "launch",
      "kind": "launch"
    }
  ],
  "userdata": {}
}
EOF

	log "Generated manifest with gateway URL: $anvil_rpc_url" >&2
}
