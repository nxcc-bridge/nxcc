#!/bin/bash

# Shellcheck script for NXCC project
# Checks all shell scripts in the project for issues
# Usage: ./shellcheck.sh [--fix]
#   --fix: Automatically fix formatting issues where possible

set -euo pipefail

# Parse command line arguments
FIX_MODE=false
while [[ $# -gt 0 ]]; do
	case $1 in
	--fix)
		FIX_MODE=true
		shift
		;;
	*)
		echo "Unknown option: $1"
		echo "Usage: $0 [--fix]"
		exit 1
		;;
	esac
done

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Find the project root directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

if [[ "${FIX_MODE}" == "true" ]]; then
	echo -e "${YELLOW}Running shellcheck with auto-fix on all shell scripts in the project...${NC}"
else
	echo -e "${YELLOW}Running shellcheck on all shell scripts in the project...${NC}"
fi
echo "Project root: ${PROJECT_ROOT}"

# Auto-detect shell scripts using git ls-files

# Create temporary file to store script list
TEMP_SCRIPT_LIST=$(mktemp)
trap 'rm -f "${TEMP_SCRIPT_LIST}"' EXIT

# Find shell scripts by extension and shebang
{
	git ls-files '*.sh' '*.bash'
	git ls-files | while IFS= read -r file; do
		if [[ -f "${file}" ]] && head -n1 "${file}" 2>/dev/null | grep -q '^#!/.*sh'; then
			echo "${file}"
		fi
	done
} | sort -u >"${TEMP_SCRIPT_LIST}"

# Read into array
SHELL_SCRIPTS=()
while IFS= read -r script; do
	SHELL_SCRIPTS+=("${script}")
done <"${TEMP_SCRIPT_LIST}"

# Check if required tools are available
if ! command -v shellcheck &>/dev/null; then
	echo -e "${RED}Error: shellcheck is not installed. Please install it first.${NC}"
	echo "On Ubuntu/Debian: sudo apt-get install shellcheck"
	echo "On macOS: brew install shellcheck"
	exit 1
fi

if ! command -v shfmt &>/dev/null; then
	echo -e "${RED}Error: shfmt is not installed. Please install it first.${NC}"
	echo "On Ubuntu/Debian: sudo apt-get install shfmt"
	echo "On macOS: brew install shfmt"
	echo "Or install via Go: go install mvdan.cc/sh/v3/cmd/shfmt@latest"
	exit 1
fi

# Change to project root
cd "${PROJECT_ROOT}"

# Track results
TOTAL_SCRIPTS=0
PASSED_SCRIPTS=0
FAILED_SCRIPTS=0
FORMAT_FAILED_SCRIPTS=0
FORMAT_FIXED_SCRIPTS=0

echo ""
echo "Found ${#SHELL_SCRIPTS[@]} shell scripts to check"

# Exit early if no scripts found
if [[ ${#SHELL_SCRIPTS[@]} -eq 0 ]]; then
	echo -e "${YELLOW}No shell scripts found in the repository${NC}"
	exit 0
fi

echo ""
echo "Checking scripts..."

# Check each script
for script in "${SHELL_SCRIPTS[@]}"; do
	if [[ -f "${script}" ]]; then
		echo -n "Checking ${script}... "
		TOTAL_SCRIPTS=$((TOTAL_SCRIPTS + 1))

		# Check formatting first
		if ! shfmt -d "${script}" >/dev/null 2>&1; then
			if [[ "${FIX_MODE}" == "true" ]]; then
				echo -n "fixing format... "
				if shfmt -w "${script}"; then
					echo -e "${GREEN}FORMAT FIXED${NC}"
					FORMAT_FIXED_SCRIPTS=$((FORMAT_FIXED_SCRIPTS + 1))
				else
					echo -e "${RED}FORMAT FAIL${NC}"
					echo "  Could not automatically fix formatting issues in ${script}"
					FAILED_SCRIPTS=$((FAILED_SCRIPTS + 1))
					FORMAT_FAILED_SCRIPTS=$((FORMAT_FAILED_SCRIPTS + 1))
					continue
				fi
			else
				echo -e "${RED}FORMAT FAIL${NC}"
				echo "  Formatting issues found. Run: shfmt -w ${script}"
				FAILED_SCRIPTS=$((FAILED_SCRIPTS + 1))
				FORMAT_FAILED_SCRIPTS=$((FORMAT_FAILED_SCRIPTS + 1))
				continue
			fi
		fi

		# Then check with shellcheck
		if shellcheck --exclude=SC1091,SC2317 "${script}"; then
			echo -e "${GREEN}PASS${NC}"
			PASSED_SCRIPTS=$((PASSED_SCRIPTS + 1))
		else
			echo -e "${RED}FAIL${NC}"
			FAILED_SCRIPTS=$((FAILED_SCRIPTS + 1))
		fi
	else
		echo -e "${YELLOW}Warning: ${script} not found${NC}"
	fi
done

echo ""
echo "=================================="
echo "Shell Scripts Check Results Summary:"
echo "Total scripts checked: ${TOTAL_SCRIPTS}"
echo -e "Passed: ${GREEN}${PASSED_SCRIPTS}${NC}"
echo -e "Failed: ${RED}${FAILED_SCRIPTS}${NC}"
if [[ ${FORMAT_FAILED_SCRIPTS} -gt 0 ]]; then
	echo -e "Format failures: ${RED}${FORMAT_FAILED_SCRIPTS}${NC}"
fi
if [[ ${FORMAT_FIXED_SCRIPTS} -gt 0 ]]; then
	echo -e "Format fixes applied: ${GREEN}${FORMAT_FIXED_SCRIPTS}${NC}"
fi
echo "=================================="

if [[ ${FAILED_SCRIPTS} -gt 0 ]]; then
	if [[ ${FORMAT_FAILED_SCRIPTS} -gt 0 ]]; then
		echo -e "${RED}Some scripts failed formatting checks. Run '$0 --fix' to automatically fix formatting.${NC}"
	fi
	echo -e "${RED}Some scripts failed checks. Please fix the issues above.${NC}"
	exit 1
else
	if [[ ${FORMAT_FIXED_SCRIPTS} -gt 0 ]]; then
		echo -e "${GREEN}All scripts passed linting checks! ${FORMAT_FIXED_SCRIPTS} formatting issues were automatically fixed.${NC}"
	else
		echo -e "${GREEN}All scripts passed linting and formatting checks!${NC}"
	fi
	exit 0
fi
