#!/bin/sh
set -eu

DEFAULT_ZENROOM_VERSION="v5.29.0"
DEFAULT_ZENROOM_BASE_URL="https://github.com/dyne/Zenroom/releases/download"
DEFAULT_ZENROOM_SHA256_ARM_64="84722e4284e17cbe861cd09cdd39c8e93e4ca84a983c30326393d13fe41b4938"
DEFAULT_ZENROOM_SHA256_X86_64="8d7a60ec8d381a1945f33188c077270d92ef4c02acfed8fe223cc50322918652"

cmd_name="${0##*/}"

zenroom_version="${ZENROOM_VERSION:-$DEFAULT_ZENROOM_VERSION}"
zenroom_base_url="${ZENROOM_BASE_URL:-$DEFAULT_ZENROOM_BASE_URL}"
zenroom_cache_dir="${ZENROOM_CACHE_DIR:-/var/cache/nxcc/zenroom}"

arch_hint="${ZENROOM_ARCH:-}"
if [ -z "$arch_hint" ]; then
	arch_hint="${ZENROOM_TARGETARCH:-${TARGETARCH:-}}"
fi
if [ -z "$arch_hint" ]; then
	arch_hint="$(uname -m)"
fi

case "$arch_hint" in
	arm64|aarch64|arm_64)
		zenroom_arch="arm_64"
		;;
	amd64|x86_64)
		zenroom_arch="x86_64"
		;;
	*)
		echo "Unsupported architecture '$arch_hint' for Zenroom download." >&2
		exit 1
		;;
esac

if [ -n "${ZENROOM_SHA256:-}" ]; then
	zenroom_expected_sha="$ZENROOM_SHA256"
else
	case "$zenroom_arch" in
		arm_64)
			zenroom_expected_sha="${ZENROOM_SHA256_ARM_64:-$DEFAULT_ZENROOM_SHA256_ARM_64}"
			;;
		x86_64)
			zenroom_expected_sha="${ZENROOM_SHA256_X86_64:-$DEFAULT_ZENROOM_SHA256_X86_64}"
			;;
		*)
			echo "Missing Zenroom SHA256 for architecture '$zenroom_arch'." >&2
			exit 1
			;;
	esac
fi

if [ -z "$zenroom_expected_sha" ]; then
	echo "Missing Zenroom SHA256; refusing to download." >&2
	exit 1
fi

zenroom_url="${ZENROOM_URL:-${zenroom_base_url}/${zenroom_version}/zenroom-${zenroom_arch}-linux.zip}"
install_dir="${zenroom_cache_dir}/${zenroom_version}/${zenroom_arch}"
bin_path="${install_dir}/${cmd_name}"

if [ ! -x "$bin_path" ]; then
	if ! command -v curl >/dev/null 2>&1; then
		echo "curl is required to download Zenroom." >&2
		exit 1
	fi
	if ! command -v unzip >/dev/null 2>&1; then
		echo "unzip is required to extract Zenroom." >&2
		exit 1
	fi
	if ! command -v sha256sum >/dev/null 2>&1; then
		echo "sha256sum is required to verify Zenroom." >&2
		exit 1
	fi

	umask 022
	mkdir -p "$zenroom_cache_dir"
	tmp_dir="$(mktemp -d "${zenroom_cache_dir}/tmp.XXXXXX")"
	cleanup() {
		rm -rf "$tmp_dir"
	}
	trap cleanup EXIT

	zip_path="${tmp_dir}/zenroom.zip"
	curl -fSL "$zenroom_url" -o "$zip_path"
	if ! printf '%s  %s\n' "$zenroom_expected_sha" "$zip_path" | sha256sum -c - >/dev/null 2>&1; then
		echo "Zenroom download failed checksum verification." >&2
		exit 1
	fi

	unzip -q "$zip_path" -d "${tmp_dir}/unpacked"

	for name in zenroom zencode-exec lua-exec; do
		found_path="$(find "${tmp_dir}/unpacked" -type f -name "$name" -print -quit)"
		if [ -n "$found_path" ]; then
			install -m 0755 "$found_path" "${tmp_dir}/$name"
		fi
	done

	if [ ! -f "${tmp_dir}/${cmd_name}" ]; then
		echo "Zenroom archive did not include expected binary '${cmd_name}'." >&2
		exit 1
	fi

	mkdir -p "$install_dir"
	for name in zenroom zencode-exec lua-exec; do
		if [ -f "${tmp_dir}/$name" ]; then
			install -m 0755 "${tmp_dir}/$name" "${install_dir}/$name"
		fi
	done

	trap - EXIT
	rm -rf "$tmp_dir"
fi

exec "$bin_path" "$@"
