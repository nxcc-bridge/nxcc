#!/bin/sh

phase_http_worker() {
	# ==============================================================================
	# PHASE 4: HTTP Worker Testing with Attestation Policies
	# ==============================================================================
	echo ""
	echo "📡 === PHASE 4: HTTP Worker Testing with Attestation Policies ==="
	
	# 5a. Deploy the HTTP echo worker
	# shellcheck disable=SC2154 # alice_DAEMON_SOCK is set by setup
	HTTP_ECHO_WORK_ORDER_ID=$(deploy_http_echo_worker "$alice_DAEMON_SOCK" "main-test")
	echo "HTTP Echo Work Order ID (mount segment): $HTTP_ECHO_WORK_ORDER_ID"

	# Give some time for the worker to be mounted and ready
	sleep 2

	# 5b. Send HTTP request to the worker via Alice's daemon
	echo "Sending HTTP POST request to the echo worker..."
	HTTP_REQUEST_BODY="Hello From Test Script"
	HTTP_RESPONSE_FILE="$TEST_DIR/http_echo_worker_response.json"
	HTTP_STATUS_CODE=$(curl -s -w "%{http_code}" -X POST \
		-H "Content-Type: text/plain" \
		-H "X-Custom-Test-Header: custom-value" \
		-d "$HTTP_REQUEST_BODY" \
		"http://127.0.0.1:${NODE1_HTTP_PORT}/w/${HTTP_ECHO_WORK_ORDER_ID}/test/path?queryArg=testVal" \
		-o "$HTTP_RESPONSE_FILE")

	echo "HTTP Echo Worker Response Status Code: $HTTP_STATUS_CODE"
	echo "HTTP Echo Worker Response Body:"
	cat "$HTTP_RESPONSE_FILE"

	if [ "$HTTP_STATUS_CODE" -ne 200 ]; then
		echo "ERROR (HTTP Worker Test): Worker returned status $HTTP_STATUS_CODE, expected 200."
		echo "Alice daemon log:"
		# shellcheck disable=SC2154  # alice_* and bob_* vars are set by setup_node
		cat "$alice_DAEMON_LOG" || true
		echo "Alice VM log:"
		cat "$alice_VM_LOG" || true
		exit 1
	fi

	# 5c. Verify the response
	jq -e '.message == "HTTP Echo Worker Response"' "$HTTP_RESPONSE_FILE" >/dev/null || {
		echo "ERROR (HTTP Worker Test): Incorrect message"
		exit 1
	}
	jq -e '.method == "POST"' "$HTTP_RESPONSE_FILE" >/dev/null || {
		echo "ERROR (HTTP Worker Test): Incorrect method"
		exit 1
	}
	jq -e '.pathname == "/test/path"' "$HTTP_RESPONSE_FILE" >/dev/null || {
		echo "ERROR (HTTP Worker Test): Incorrect pathname"
		exit 1
	}
	jq -e '.searchParams.queryArg == "testVal"' "$HTTP_RESPONSE_FILE" >/dev/null || {
		echo "ERROR (HTTP Worker Test): Incorrect queryArg"
		exit 1
	}
	jq -e '.headers["content-type"] == "text/plain"' "$HTTP_RESPONSE_FILE" >/dev/null || {
		echo "ERROR (HTTP Worker Test): Incorrect content-type header"
		exit 1
	}
	jq -e '.headers["x-custom-test-header"] == "custom-value"' "$HTTP_RESPONSE_FILE" >/dev/null || {
		echo "ERROR (HTTP Worker Test): Incorrect x-custom-test-header"
		exit 1
	}
	jq -e ".body == \"$HTTP_REQUEST_BODY\"" "$HTTP_RESPONSE_FILE" >/dev/null || {
		echo "ERROR (HTTP Worker Test): Incorrect body echo"
		exit 1
	}

	echo "SUCCESS (HTTP Worker Test): HTTP echo worker responded correctly."
}
