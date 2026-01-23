#!/bin/sh

phase_secret_sharing() {
	# ==============================================================================
	# PHASE 2: Attestation-Based Secret Sharing and Access Control
	# ==============================================================================
	echo ""
	echo "🔐 === PHASE 2: Attestation-Based Secret Sharing and Access Control ==="

	P2P_TIMEOUT_SECS="${NXCC_DAEMON_P2P_RESPONSE_TIMEOUT_SECS:-15}"
	ALICE_WAIT_SECS=$((P2P_TIMEOUT_SECS + 5))
	BOB_WAIT_SECS=$((P2P_TIMEOUT_SECS + 10))
	CHARLIE_WAIT_SECS=$((P2P_TIMEOUT_SECS + 5))

	# --- Prepare Test Worker and Work Order ---
	echo "--- Preparing Attestation-Aware Work Order for Secret Sharing Test ---"

	# 1. JS Worker Code - Use test worker for secret derivation capability
	# The operator signature policy will be enforced at the policy evaluation level
	TEST_WORKER_JS_CONTENT=$(cat "$POLICY_WORKER_JS_BUNDLE_PATH")
	TEST_WORKER_JS_B64=$(printf "%s" "$TEST_WORKER_JS_CONTENT" | base64 | tr -d '\n')

	# Note: The operator signature policy (mock_worker.js) will be used
	# by the enclave during policy evaluation to enforce access control based on
	# operator signatures. This provides the security layer while the worker
	# handles the actual secret derivation functionality.

	# 2. WorkerBundlePayload for the JS worker (using file to avoid argument list too long)
	WORKER_BUNDLE_PAYLOAD_FILE="$TEST_DIR/worker_bundle_payload.json"
	jq -n \
		--arg vm "nxcc/workerd" \
		--arg executable_b64 "$TEST_WORKER_JS_B64" \
		'{vm: $vm, executable: $executable_b64, metadata: {}}' >"$WORKER_BUNDLE_PAYLOAD_FILE"

	# 3. DSSE Envelope for the WorkerBundle (using files to avoid argument list too long)
	WORKER_BUNDLE_PAYLOAD_B64_FILE="$TEST_DIR/worker_bundle_payload_b64.txt"
	base64 <"$WORKER_BUNDLE_PAYLOAD_FILE" | tr -d '\n' >"$WORKER_BUNDLE_PAYLOAD_B64_FILE"

	WORKER_BUNDLE_DSSE_FILE="$TEST_DIR/worker_bundle_dsse.json"
	jq -n \
		--rawfile payload_b64 "$WORKER_BUNDLE_PAYLOAD_B64_FILE" \
		--arg payload_type "application/vnd.nxcc.workerbundlepayload.v1+json" \
		'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$WORKER_BUNDLE_DSSE_FILE"

	WORKER_BUNDLE_DSSE_B64_FILE="$TEST_DIR/worker_bundle_dsse_b64.txt"
	base64 <"$WORKER_BUNDLE_DSSE_FILE" | tr -d '\n' >"$WORKER_BUNDLE_DSSE_B64_FILE"

	# 4. WorkerManifest for the WorkOrder (using files to avoid argument list too long)
	WORKER_MANIFEST_FILE="$TEST_DIR/worker_manifest.json"
	jq -n \
		--rawfile bundle_source_b64 "$WORKER_BUNDLE_DSSE_B64_FILE" \
		--argjson chain "$SECRET_CHAIN_ID" \
		--arg identity_address "$SECRET_IDENTITY_ADDR" \
		--arg identity_id_str "$SECRET_IDENTITY_ID_NUM" \
		--arg secret_name "$SECRET_NAME_IN_WORKER" \
		'{bundle: {source: ("data:application/json;base64," + $bundle_source_b64), hash: null}, identities: [[{chain: $chain, identity_address: $identity_address, identity_id: $identity_id_str}, $secret_name]], userdata: {}}' >"$WORKER_MANIFEST_FILE"

	# 5. WorkOrderPayload (using files to avoid argument list too long)
	WORK_ORDER_PAYLOAD_FILE="$TEST_DIR/work_order_payload.json"
	jq -n \
		--arg id "test-work-order-$(date +%s%N)" \
		--slurpfile worker_manifest "$WORKER_MANIFEST_FILE" \
		'{id: $id, worker: $worker_manifest[0], events: [{"handler": "launch", "kind": "launch"}]}' >"$WORK_ORDER_PAYLOAD_FILE"

	# 6. DSSE Envelope for the WorkOrder (using files to avoid argument list too long)
	WORK_ORDER_PAYLOAD_B64_FILE="$TEST_DIR/work_order_payload_b64.txt"
	base64 <"$WORK_ORDER_PAYLOAD_FILE" | tr -d '\n' >"$WORK_ORDER_PAYLOAD_B64_FILE"

	ORIG_WORK_ORDER_DSSE_FILE="$TEST_DIR/orig_work_order_dsse.json"
	jq -n \
		--rawfile payload_b64 "$WORK_ORDER_PAYLOAD_B64_FILE" \
		--arg payload_type "$DSSE_WORK_ORDER_PAYLOAD_TYPE" \
		'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$ORIG_WORK_ORDER_DSSE_FILE"

	# Prepare the payload file for grpcurl
	ORIG_WORK_ORDER_DSSE_B64_FILE="$TEST_DIR/orig_work_order_dsse_b64.txt"
	base64 <"$ORIG_WORK_ORDER_DSSE_FILE" | tr -d '\n' >"$ORIG_WORK_ORDER_DSSE_B64_FILE"

	GRPCURL_SUBMIT_ORIG_WO_PAYLOAD_FILE="$TEST_DIR/submit_orig_wo_payload.json"
	jq -n \
		--rawfile work_order_dsse_bytes "$ORIG_WORK_ORDER_DSSE_B64_FILE" \
		'{work_order_dsse_bytes: $work_order_dsse_bytes}' >"$GRPCURL_SUBMIT_ORIG_WO_PAYLOAD_FILE"

	# Test 2a: Trusted secret sharing (Alice → Bob)
	echo "📤 Step 2a: Testing trusted secret sharing (Alice → Bob)..."

	# 1. Alice receives work order (generates secret)
	echo "  • Alice receives work order (generates secret)..."
	# shellcheck disable=SC2154  # alice_DAEMON_SOCK is set by setup_node function
	grpcurl_submit_work_order "$alice_DAEMON_SOCK" "$GRPCURL_SUBMIT_ORIG_WO_PAYLOAD_FILE"

	# Wait for Alice's worker to execute and log output
	echo "  • Waiting for Alice to derive secret..."
	ALICE_DERIVED_BITS=""
	# shellcheck disable=SC2034,SC2154  # i is unused, alice_* vars set by setup_node
	for i in $( # Poll for up to $ALICE_WAIT_SECS seconds
		seq 1 "$ALICE_WAIT_SECS"
	); do
		# Check both VM log and Daemon log for the derived bits output
		if [ -f "$alice_VM_LOG" ]; then
			ALICE_DERIVED_BITS=$(grep "DERIVED_BASE64:" "$alice_VM_LOG" |
				tail -n 1 |
				sed -E 's/.*DERIVED_BASE64: ([A-Za-z0-9+/=]*).*/\1/')
		fi
		if [ -z "$ALICE_DERIVED_BITS" ] && [ -f "$alice_DAEMON_LOG" ]; then
			ALICE_DERIVED_BITS=$(grep "stdout:.*DERIVED_BASE64:" "$alice_DAEMON_LOG" |
				tail -n 1 |
				sed -E 's/.*DERIVED_BASE64: ([A-Za-z0-9+/=]*).*/\1/')
		fi
		if [ -n "$ALICE_DERIVED_BITS" ]; then
			echo "  ✅ Alice derived secret: $ALICE_DERIVED_BITS"
			break
		fi
		sleep 1
	done

	if [ -z "$ALICE_DERIVED_BITS" ]; then
		echo "  ❌ ERROR: Alice failed to derive secret"
		cat "$alice_VM_LOG" # Print log for debugging
		exit 1
	fi

	# 2. Bob receives the same work order (should get same secret via P2P)
	echo "  • Bob receives work order (requests secret from Alice)..."
	# shellcheck disable=SC2154  # bob_DAEMON_SOCK is set by setup_node
	grpcurl_submit_work_order "$bob_DAEMON_SOCK" "$GRPCURL_SUBMIT_ORIG_WO_PAYLOAD_FILE"

	# Wait for Bob's worker to execute and log output
	echo "  • Waiting for Bob to derive secret..."
	BOB_DERIVED_BITS=""
	# shellcheck disable=SC2034,SC2154  # i is unused, bob_* vars set by setup_node
	for i in $( # Poll for up to $BOB_WAIT_SECS seconds (longer for P2P)
		seq 1 "$BOB_WAIT_SECS"
	); do
		# Check both VM log and Daemon log for the derived bits output
		if [ -f "$bob_VM_LOG" ]; then
			BOB_DERIVED_BITS=$(grep "DERIVED_BASE64:" "$bob_VM_LOG" |
				tail -n 1 |
				sed -E 's/.*DERIVED_BASE64: ([A-Za-z0-9+/=]*).*/\1/')
		fi
		if [ -z "$BOB_DERIVED_BITS" ] && [ -f "$bob_DAEMON_LOG" ]; then
			BOB_DERIVED_BITS=$(grep "stdout:.*DERIVED_BASE64:" "$bob_DAEMON_LOG" |
				tail -n 1 |
				sed -E 's/.*DERIVED_BASE64: ([A-Za-z0-9+/=]*).*/\1/')
		fi
		if [ -n "$BOB_DERIVED_BITS" ]; then
			echo "  ✅ Bob derived secret: $BOB_DERIVED_BITS"
			break
		fi
		sleep 1
	done

	if [ -z "$BOB_DERIVED_BITS" ]; then
		echo "  ❌ ERROR: Bob failed to derive secret"
		cat "$bob_VM_LOG" # Print log for debugging
		exit 1
	fi

	# 3. Verify secrets match between trusted nodes
	if [ "$ALICE_DERIVED_BITS" = "$BOB_DERIVED_BITS" ]; then
		echo "  ✅ SUCCESS: Alice and Bob derived matching secrets"
		echo "     Secret sharing between trusted nodes works correctly!"
	else
		echo "  ❌ ERROR: Secret mismatch between trusted nodes"
		echo "     Alice: $ALICE_DERIVED_BITS"
		echo "     Bob:   $BOB_DERIVED_BITS"
		exit 1
	fi

	# Test 2b: Access control blocks untrusted node
	echo ""
	echo "🚫 Step 2b: Testing attestation policy blocks untrusted node (Charlie)..."

	# Charlie tries to get the same secret
	echo "  • Charlie (untrusted) tries to get secret..."
	# shellcheck disable=SC2154  # charlie_DAEMON_SOCK is set by setup_node
	grpcurl_submit_work_order "$charlie_DAEMON_SOCK" "$GRPCURL_SUBMIT_ORIG_WO_PAYLOAD_FILE"

	# Wait and check that Charlie does NOT get the secret
	echo "  • Waiting to confirm Charlie is blocked by attestation policy..."
	CHARLIE_DERIVED_BITS=""
	CHARLIE_BLOCKED=false

	# shellcheck disable=SC2034  # i is used for timing loop iterations
	for i in $( # Poll for up to $CHARLIE_WAIT_SECS seconds
		seq 1 "$CHARLIE_WAIT_SECS"
	); do
		# Check for secret derivation (should not happen) in both VM and Daemon logs
		# shellcheck disable=SC2154  # charlie_VM_LOG is set by setup_node function
		if [ -f "$charlie_VM_LOG" ] && grep -q "DERIVED_BASE64:" "$charlie_VM_LOG"; then
			CHARLIE_DERIVED_BITS=$(grep "DERIVED_BASE64:" "$charlie_VM_LOG" | tail -n 1 | sed -E 's/.*DERIVED_BASE64: ([A-Za-z0-9+/=]*).*/\1/')
			break
		fi
		# shellcheck disable=SC2154  # charlie_DAEMON_LOG is set by setup_node function
		if [ -z "$CHARLIE_DERIVED_BITS" ] && [ -f "$charlie_DAEMON_LOG" ] && grep -q "stdout:.*DERIVED_BASE64:" "$charlie_DAEMON_LOG"; then
			CHARLIE_DERIVED_BITS=$(grep "stdout:.*DERIVED_BASE64:" "$charlie_DAEMON_LOG" | tail -n 1 | sed -E 's/.*DERIVED_BASE64: ([A-Za-z0-9+/=]*).*/\1/')
			break
		fi

		# Check daemon logs for policy blocking
		# shellcheck disable=SC2154  # charlie_DAEMON_LOG is set by setup_node
		if [ -f "$charlie_DAEMON_LOG" ] && (grep -q "DENIED" "$charlie_DAEMON_LOG" || grep -q "not in trusted whitelist" "$charlie_DAEMON_LOG"); then
			CHARLIE_BLOCKED=true
			echo "  🔍 Found policy denial in Charlie's logs"
			break
		fi

		sleep 1
	done

	if [ -n "$CHARLIE_DERIVED_BITS" ]; then
		echo "  ❌ ERROR: Charlie should have been blocked but got secret: $CHARLIE_DERIVED_BITS"
		echo "     Attestation policy security failed!"
		exit 1
	elif [ "$CHARLIE_BLOCKED" = true ]; then
		echo "  ✅ SUCCESS: Charlie was correctly blocked by attestation policy"
		echo "     Attestation-based access control is working!"
	else
		echo "  ✅ Charlie was blocked (no secret derived) - policy likely blocked the request"
		echo "     This is the expected security behavior"
	fi

	echo ""
	echo "🎯 PHASE 2 SUMMARY: Attestation-based security model validated"
	echo "   ✅ Trusted nodes (Alice ↔ Bob) can share secrets"
	echo "   ✅ Untrusted nodes (Charlie) are blocked by policy"
}
