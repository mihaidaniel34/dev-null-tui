#!/bin/sh
#
# Installs the latest devnull release:
#
#   curl -fsSL https://raw.githubusercontent.com/mihaidaniel34/dev-null-tui/main/install.sh | sh
#
# DEVNULL_INSTALL_DIR picks where it goes, ~/.local/bin by default. The download is checked against the release's
# SHA-256 before anything is installed.

set -eu

main() {
  repo="mihaidaniel34/dev-null-tui"
  dir="${DEVNULL_INSTALL_DIR:-$HOME/.local/bin}"

  case "$(uname -s)" in
    Linux) os="unknown-linux-musl" ;;
    Darwin) os="apple-darwin" ;;
    *) fail "No prebuilt binary for $(uname -s). Try: cargo install --git https://github.com/$repo" ;;
  esac

  case "$(uname -m)" in
    x86_64 | amd64) arch="x86_64" ;;
    arm64 | aarch64) arch="aarch64" ;;
    *) fail "No prebuilt binary for $(uname -m). Try: cargo install --git https://github.com/$repo" ;;
  esac

  for command in curl tar uname mktemp; do
    command -v "$command" > /dev/null || fail "$command is required."
  done

  asset="devnull-$arch-$os.tar.gz"
  url="https://github.com/$repo/releases/latest/download/$asset"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  echo "Downloading $asset…"
  curl -fsSL "$url" -o "$tmp/$asset" || fail "Could not download $url"
  curl -fsSL "$url.sha256" -o "$tmp/$asset.sha256" || fail "Could not download the checksum."

  expected="$(cut -d ' ' -f 1 "$tmp/$asset.sha256")"
  if command -v sha256sum > /dev/null; then
    actual="$(sha256sum "$tmp/$asset" | cut -d ' ' -f 1)"
  else
    actual="$(shasum -a 256 "$tmp/$asset" | cut -d ' ' -f 1)"
  fi
  [ "$expected" = "$actual" ] || fail "The checksum doesn't match. Nothing was installed."

  tar -xzf "$tmp/$asset" -C "$tmp"
  mkdir -p "$dir"
  cp "$tmp/devnull" "$dir/devnull"
  chmod 755 "$dir/devnull"

  echo "Installed $("$dir/devnull" --version) to $dir/devnull"
  case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "Add $dir to your PATH to run it as devnull." ;;
  esac
}

fail() {
  echo "devnull: $*" >&2
  exit 1
}

main "$@"
