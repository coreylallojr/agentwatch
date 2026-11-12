#!/bin/sh
# Install agentwatch on macOS or Linux:
#   curl -fsSL https://raw.githubusercontent.com/coreylallojr/agentwatch/main/install.sh | sh
# Downloads the latest release, checks its SHA-256, installs to ~/.local/bin (override with AGENTWATCH_INSTALL_DIR),
# then runs `agentwatch setup` (skills + MCP server for Claude Code). AGENTWATCH_NO_SETUP=1 skips setup.
set -eu
repo=coreylallojr/agentwatch
base="https://github.com/$repo/releases/latest/download"
dir="${AGENTWATCH_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  *) echo "No prebuilt agentwatch for $(uname -s) $(uname -m). Build it: cargo install --git https://github.com/$repo agentwatch" >&2; exit 1 ;;
esac
asset="agentwatch-$target"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $asset.tar.gz ..."
curl -fsSL "$base/$asset.tar.gz" -o "$tmp/$asset.tar.gz"
curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS"
expected=$(grep " $asset.tar.gz\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
[ -n "$expected" ] || { echo "SHA256SUMS has no entry for $asset.tar.gz" >&2; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then actual=$(sha256sum "$tmp/$asset.tar.gz" | cut -d' ' -f1)
else actual=$(shasum -a 256 "$tmp/$asset.tar.gz" | cut -d' ' -f1); fi
[ "$expected" = "$actual" ] || { echo "Checksum mismatch for $asset.tar.gz" >&2; exit 1; }

tar xzf "$tmp/$asset.tar.gz" -C "$tmp"
mkdir -p "$dir"
[ -x "$dir/agentwatch" ] && "$dir/agentwatch" stop >/dev/null 2>&1 || true
install -m 755 "$tmp/$asset/agentwatch" "$dir/agentwatch"
echo "Installed $("$dir/agentwatch" --version) to $dir/agentwatch"
case ":$PATH:" in *":$dir:"*) ;; *) echo "Note: $dir is not on your PATH. Add it to your shell profile: export PATH=\"$dir:\$PATH\"" ;; esac

if [ -z "${AGENTWATCH_NO_SETUP:-}" ]; then "$dir/agentwatch" setup
else echo "Skipped setup. Run 'agentwatch setup' later to add the skills and MCP server."; fi
