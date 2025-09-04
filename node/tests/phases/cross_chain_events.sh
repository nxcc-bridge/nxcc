#!/bin/sh

phase_cross_chain_events() {
	# ==============================================================================
	# PHASE 3: Cross-Chain Event Handling with Attestation Security
	# ==============================================================================
	echo ""
	echo "🌐 === PHASE 3: Cross-Chain Event Handling with Attestation Security ==="
	# Setup (nodes, anvil, contracts) is handled by the main script's dependency system.

	# Prepare Work Order for the event handling worker (using files to avoid argument list too long)
	EVENT_HANDLER_WORKER_JS_CONTENT=$(cat "$EVENT_HANDLER_WORKER_JS_BUNDLE_PATH")
	EVENT_HANDLER_WORKER_JS_B64=$(printf "%s" "$EVENT_HANDLER_WORKER_JS_CONTENT" | base64 | tr -d '\n')

	EVENT_WORKER_BUNDLE_PAYLOAD_FILE="$TEST_DIR/event_worker_bundle_payload.json"
	jq -n \
		--arg vm "nxcc/workerd" \
		--arg executable_b64 "$EVENT_HANDLER_WORKER_JS_B64" \
		'{vm: $vm, executable: $executable_b64, metadata: {}}' >"$EVENT_WORKER_BUNDLE_PAYLOAD_FILE"

	EVENT_WORKER_BUNDLE_PAYLOAD_B64_FILE="$TEST_DIR/event_worker_bundle_payload_b64.txt"
	base64 <"$EVENT_WORKER_BUNDLE_PAYLOAD_FILE" | tr -d '\n' >"$EVENT_WORKER_BUNDLE_PAYLOAD_B64_FILE"

	EVENT_WORKER_BUNDLE_DSSE_FILE="$TEST_DIR/event_worker_bundle_dsse.json"
	jq -n \
		--rawfile payload_b64 "$EVENT_WORKER_BUNDLE_PAYLOAD_B64_FILE" \
		--arg payload_type "application/vnd.nxcc.workerbundlepayload.v1+json" \
		'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$EVENT_WORKER_BUNDLE_DSSE_FILE"

	EVENT_WORKER_BUNDLE_DSSE_B64_FILE="$TEST_DIR/event_worker_bundle_dsse_b64.txt"
	base64 <"$EVENT_WORKER_BUNDLE_DSSE_FILE" | tr -d '\n' >"$EVENT_WORKER_BUNDLE_DSSE_B64_FILE"

	EVENT_WORKER_MANIFEST_FILE="$TEST_DIR/event_worker_manifest.json"
	jq -n \
		--rawfile bundle_source_b64 "$EVENT_WORKER_BUNDLE_DSSE_B64_FILE" \
		--arg rpc_url_1 "$ANVIL_RPC_URL_1" \
		--arg contract_addr_1 "$CONTRACT_ADDRESS_1" \
		--arg rpc_url_2 "$ANVIL_RPC_URL_2" \
		--arg contract_addr_2 "$CONTRACT_ADDRESS_2" \
		--arg pk "$WORKER_SENDER_PK" \
		--arg abi_string "$TEST_EVENTS_ABI_STRING" \
		"{
        bundle: {source: (\"data:application/json;base64,\" + \$bundle_source_b64), hash: null},
        identities: [],
        userdata: {
						chain1: { rpcUrl: \$rpc_url_1, contractAddress: \$contract_addr_1 },
						chain2: { rpcUrl: \$rpc_url_2, contractAddress: \$contract_addr_2 },
						contractAbi: \$abi_string,
						ethereumPrivateKey: \$pk
        }
    }" >"$EVENT_WORKER_MANIFEST_FILE"

	EVENT_VALUE_CHANGED_SIGNATURE=$(cast sig-event "ValueChanged(uint256,uint256,bytes)")
	OTHER_EVENT_SIGNATURE=$(cast sig-event "OtherEvent(uint256)")

	EVENT_WORK_ORDER_PAYLOAD_FILE="$TEST_DIR/event_work_order_payload.json"
	jq -n \
		--arg id "cross-chain-work-order-$(date +%s%N)" \
		--slurpfile worker_manifest "$EVENT_WORKER_MANIFEST_FILE" \
		--argjson chain_1 "$ANVIL_CHAIN_ID_1" \
		--arg contract_address_1 "$CONTRACT_ADDRESS_1" \
		--arg value_changed_sig "$EVENT_VALUE_CHANGED_SIGNATURE" \
		--argjson chain_2 "$ANVIL_CHAIN_ID_2" \
		--arg contract_address_2 "$CONTRACT_ADDRESS_2" \
		--arg other_event_sig "$OTHER_EVENT_SIGNATURE" \
		--arg anvil_ws_url_1 "$ANVIL_WS_URL_1" \
		--arg anvil_ws_url_2 "$ANVIL_WS_URL_2" \
		'{
 id: $id,
 worker: $worker_manifest[0],
 events: [
            {"handler": "launch", "kind": "launch"},
            {
                "handler": "valueChanged",
                "kind": "web3_event",
                "chain": $chain_1,
                "address": [$contract_address_1],
                "topics": [[$value_changed_sig]],
                "gateways": [$anvil_ws_url_1]
            },
            {
                "handler": "otherEvent",
                "kind": "web3_event",
                "chain": $chain_2,
                "address": [$contract_address_2],
                "topics": [[$other_event_sig]],
                "gateways": [$anvil_ws_url_2]
            }
        ]
    }' >"$EVENT_WORK_ORDER_PAYLOAD_FILE"

	EVENT_WORK_ORDER_PAYLOAD_B64_FILE="$TEST_DIR/event_work_order_payload_b64.txt"
	base64 <"$EVENT_WORK_ORDER_PAYLOAD_FILE" | tr -d '\n' >"$EVENT_WORK_ORDER_PAYLOAD_B64_FILE"

	EVENT_WORK_ORDER_DSSE_FILE="$TEST_DIR/event_work_order_dsse.json"
	jq -n \
		--rawfile payload_b64 "$EVENT_WORK_ORDER_PAYLOAD_B64_FILE" \
		--arg payload_type "$DSSE_WORK_ORDER_PAYLOAD_TYPE" \
		'{payload: $payload_b64, payloadType: $payload_type, signatures: [{keyid: "mock", sig: "'"$(printf "%s" "mocksig" | base64 | tr -d '\n')"'"}]}' >"$EVENT_WORK_ORDER_DSSE_FILE"

	EVENT_WORK_ORDER_DSSE_B64_FILE="$TEST_DIR/event_work_order_dsse_b64.txt"
	base64 <"$EVENT_WORK_ORDER_DSSE_FILE" | tr -d '\n' >"$EVENT_WORK_ORDER_DSSE_B64_FILE"

	GRPCURL_SUBMIT_EVENT_WO_PAYLOAD_FILE="$TEST_DIR/submit_event_wo_payload.json"
	jq -n \
		--rawfile work_order_dsse_bytes "$EVENT_WORK_ORDER_DSSE_B64_FILE" \
		'{work_order_dsse_bytes: $work_order_dsse_bytes}' >"$GRPCURL_SUBMIT_EVENT_WO_PAYLOAD_FILE"

	echo "Submitting event-listening work order to Alice and Bob..."
	# shellcheck disable=SC2154  # alice_DAEMON_SOCK and bob_DAEMON_SOCK are set by setup_node
	grpcurl_submit_work_order "$alice_DAEMON_SOCK" "$GRPCURL_SUBMIT_EVENT_WO_PAYLOAD_FILE"
	grpcurl_submit_work_order "$bob_DAEMON_SOCK" "$GRPCURL_SUBMIT_EVENT_WO_PAYLOAD_FILE"

	# 4c. Trigger ValueChanged on chain 1, check update on chain 2
	echo "Triggering ValueChanged on chain 1..."
	NEW_VALUE_1=42
	DATA_1="0xdeadbeef"
	cast send --rpc-url "$ANVIL_RPC_URL_1" --private-key "$DEPLOYER_PK" "$CONTRACT_ADDRESS_1" \
		"triggerEvent(uint256,bytes)" "$NEW_VALUE_1" "$DATA_1" >/dev/null

	echo "Polling contract on chain 2 for update..."
	CONTRACT_UPDATED_SUCCESSFULLY=false
	for i in $( # Poll for updates
		seq 1 5
	); do
		CURRENT_VALUE=$(cast call --rpc-url "$ANVIL_RPC_URL_2" "$CONTRACT_ADDRESS_2" "value()(uint256)")
		CURRENT_DATA=$(cast call --rpc-url "$ANVIL_RPC_URL_2" "$CONTRACT_ADDRESS_2" "eventDataPayload()(bytes)")

		if [ "$CURRENT_VALUE" -eq "$NEW_VALUE_1" ] && [ "$CURRENT_DATA" = "$DATA_1" ]; then
			echo "SUCCESS (Cross-Chain Test Part 1): Contract on chain 2 updated as expected."
			CONTRACT_UPDATED_SUCCESSFULLY=true
			break
		fi
		printf "."
		sleep 1
	done

	if [ "$CONTRACT_UPDATED_SUCCESSFULLY" != "true" ]; then
		echo "ERROR (Cross-Chain Test Part 1): Contract on chain 2 not updated as expected after timeout."
		echo "Expected value: $NEW_VALUE_1, Got: $CURRENT_VALUE"
		echo "Expected data: $DATA_1, Got: $CURRENT_DATA"
		echo "Alice daemon log:"
		# shellcheck disable=SC2154  # alice_* and bob_* vars are set by setup_node
		cat "$alice_DAEMON_LOG" || true
		echo "Alice VM log:"
		cat "$alice_VM_LOG" || true
		echo "Bob daemon log:"
		# shellcheck disable=SC2154  # bob_DAEMON_LOG is set by setup_node function
		cat "$bob_DAEMON_LOG" || true
		echo "Bob VM log:"
		cat "$bob_VM_LOG" || true
		exit 1
	fi

	# 4d. Trigger OtherEvent on chain 2, check update on chain 1
	echo "Triggering OtherEvent on chain 2..."
	NEW_VALUE_2=99
	DATA_2="0x" # The worker should submit empty bytes for OtherEvent
	cast send --rpc-url "$ANVIL_RPC_URL_2" --private-key "$DEPLOYER_PK" "$CONTRACT_ADDRESS_2" \
		"triggerOtherEvent(uint256)" "$NEW_VALUE_2" >/dev/null

	echo "Polling contract on chain 1 for update..."
	CONTRACT_UPDATED_SUCCESSFULLY=false
	# shellcheck disable=SC2034  # i is intentionally unused in polling loop
	for i in $( # Poll for updates
		seq 1 10
	); do
		CURRENT_VALUE=$(cast call --rpc-url "$ANVIL_RPC_URL_1" "$CONTRACT_ADDRESS_1" "value()(uint256)")
		CURRENT_DATA=$(cast call --rpc-url "$ANVIL_RPC_URL_1" "$CONTRACT_ADDRESS_1" "eventDataPayload()(bytes)")

		if [ "$CURRENT_VALUE" -eq "$NEW_VALUE_2" ] && [ "$CURRENT_DATA" = "$DATA_2" ]; then
			echo "SUCCESS (Cross-Chain Test Part 2): Contract on chain 1 updated as expected."
			CONTRACT_UPDATED_SUCCESSFULLY=true
			break
		fi
		printf "."
		sleep 1
	done

	if [ "$CONTRACT_UPDATED_SUCCESSFULLY" != "true" ]; then
		echo "ERROR (Cross-Chain Test Part 2): Contract on chain 1 not updated as expected after timeout."
		echo "Expected value: $NEW_VALUE_2, Got: $CURRENT_VALUE"
		echo "Expected data: $DATA_2, Got: $CURRENT_DATA"
		echo "Alice daemon log:"
		# shellcheck disable=SC2154  # alice_* and bob_* vars are set by setup_node
		cat "$alice_DAEMON_LOG" || true
		echo "Alice VM log:"
		cat "$alice_VM_LOG" || true
		echo "Bob daemon log:"
		# shellcheck disable=SC2154  # bob_DAEMON_LOG is set by setup_node function
		cat "$bob_DAEMON_LOG" || true
		echo "Bob VM log:"
		cat "$bob_VM_LOG" || true
		exit 1
	fi
}
