#!/usr/bin/env sh
# Install uksfta from the latest GitHub release.
# Usage: curl -fsSL https://github.com/UKSFTA/UKSFTA-Tools/releases/latest/download/install.sh | sh
# The script downloads uksfta, verifies its SHA256 checksum, and installs it
# to ~/.local/bin (Linux) or /usr/local/bin (macOS root) with a PATH note.

set -eu

REPO="UKSFTA/UKSFTA-Tools"
ASSET="uksfta"

detect_os() {
	case "$(uname -s)" in
	Linux) echo "linux" ;;
	Darwin) echo "macos" ;;
	*) echo "unsupported" ;;
	esac
}

get_latest_release() {
	curl -fsSL "https://api.github.com/repos/$REPO/releases/latest"
}

get_checksum() {
	tag=$1
	asset=$2
	curl -fsSL "https://github.com/$REPO/releases/download/$tag/SHA256SUMS" |
		awk -v asset="$asset" '$2 == asset { print $1; exit }'
}

os=$(detect_os)
if [ "$os" = "unsupported" ]; then
	echo "Unsupported OS: $(uname -s). Linux and macOS are supported." >&2
	exit 1
fi

echo "Finding latest uksfta release..."
release=$(get_latest_release)
tag=$(printf '%s' "$release" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p')
url=$(printf '%s' "$release" | sed -n 's/.*"browser_download_url": *"\([^"]*uksfta\)".*/\1/p' | head -1)

if [ -z "$tag" ] || [ -z "$url" ]; then
	echo "Could not determine release details." >&2
	exit 1
fi
echo "Latest release: $tag"

echo "Fetching checksum..."
expected=$(get_checksum "$tag" "$ASSET")
if [ -z "$expected" ]; then
	echo "Checksum for $ASSET not found in SHA256SUMS." >&2
	exit 1
fi

tmp="${TMPDIR:-/tmp}/uksfta.$$"
trap 'rm -f "$tmp"' EXIT HUP INT TERM

echo "Downloading $ASSET..."
curl -fsSL -o "$tmp" "$url"

actual=$(sha256sum "$tmp" | awk '{ print $1 }')
if [ "$actual" != "$expected" ]; then
	echo "Checksum mismatch. Expected $expected, got $actual. Aborting." >&2
	exit 1
fi
echo "Checksum verified."

install_dir="$HOME/.local/bin"
if [ "$os" = "macos" ] && [ "$(id -u)" -eq 0 ]; then
	install_dir="/usr/local/bin"
fi

mkdir -p "$install_dir"
install -m 0755 "$tmp" "$install_dir/$ASSET"
rm -f "$tmp"

case ":$PATH:" in
*":$install_dir:"*) ;;
*)
	echo "Note: $install_dir is not on your PATH."
	echo "Add it with: export PATH=\"$install_dir:\$PATH\""
	;;
esac

echo "Installed uksfta $tag to $install_dir/$ASSET"
echo "Run 'uksfta --help' to verify."
