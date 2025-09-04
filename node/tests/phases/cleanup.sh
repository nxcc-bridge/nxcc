#!/bin/sh

# --- Cleanup Function ---
cleanup() {
	set +x # Don't debug log cleanup commands
	echo "Cleaning up..."

	# Kill any other potential background processes
	# This is a bit aggressive, ensure it doesn't kill unrelated processes if tests run in parallel
	pkill -P $$ >/dev/null 2>&1 || true

	# Kill node processes
	cleanup_node "$NODE1_NAME"
	cleanup_node "$NODE2_NAME"
	cleanup_node "$NODE3_NAME"

	stop_anvils

	# Remove the test directory
	if [ -d "$TEST_DIR" ]; then
		echo "Removing test directory: $TEST_DIR"
		rm -rf "$TEST_DIR"
	fi

	killall nxcc-daemon 2>&1 || true

	echo "Cleanup finished."

	# Force exit to ensure no lingering processes
	# shellcheck disable=SC3048  # SIGINT/SIGTERM prefixes are widely supported
	trap - EXIT SIGINT SIGTERM
}
