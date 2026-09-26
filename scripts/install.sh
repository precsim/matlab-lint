#!/usr/bin/env sh
set -eu

usage() {
  cat <<'EOF'
Usage: install.sh [--version TAG|latest] [--install-dir DIR] [--dry-run]

Downloads and verifies a released mstyle binary.

Environment overrides:
  MSTYLE_VERSION      Release tag such as v0.1.0, or latest
  MSTYLE_INSTALL_DIR  Installation directory
EOF
}

version="${MSTYLE_VERSION:-latest}"
install_dir="${MSTYLE_INSTALL_DIR:-${HOME:?HOME is not set}/.local/bin}"
dry_run=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      [ "$#" -ge 2 ] || { echo "install.sh: --version requires a value" >&2; exit 2; }
      version="$2"
      shift 2
      ;;
    --install-dir)
      [ "$#" -ge 2 ] || { echo "install.sh: --install-dir requires a value" >&2; exit 2; }
      install_dir="$2"
      shift 2
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "install.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "$(uname -s)" in
  Linux)
    platform=linux
    ;;
  Darwin)
    platform=macos
    ;;
  *)
    echo "install.sh: unsupported operating system: $(uname -s)" >&2
    exit 1
    ;;
esac

case "$(uname -m)" in
  x86_64|amd64)
    arch=x86_64
    ;;
  arm64|aarch64)
    arch=arm64
    ;;
  *)
    echo "install.sh: unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

if [ "$platform" = "linux" ] && [ "$arch" != "x86_64" ]; then
  echo "install.sh: Linux $arch release binaries are not currently published" >&2
  exit 1
fi

asset="mstyle-$platform-$arch"

case "$version" in
  latest)
    base_url="https://github.com/precsim/matlab-lint/releases/latest/download"
    ;;
  v*)
    base_url="https://github.com/precsim/matlab-lint/releases/download/$version"
    ;;
  *)
    echo "install.sh: --version must be 'latest' or a release tag such as v0.1.0" >&2
    exit 2
    ;;
esac

url="$base_url/$asset"
checksum_url="$url.sha256"
target="$install_dir/mstyle"

if [ "$dry_run" -eq 1 ]; then
  printf 'asset=%s\nurl=%s\nchecksum_url=%s\ninstall_path=%s\n' \
    "$asset" "$url" "$checksum_url" "$target"
  exit 0
fi

command -v curl >/dev/null 2>&1 || {
  echo "install.sh: curl is required" >&2
  exit 1
}

tmpdir="$(mktemp -d 2>/dev/null || mktemp -d -t mstyle-install)"
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

curl -fL --retry 3 --output "$tmpdir/$asset" "$url"
curl -fL --retry 3 --output "$tmpdir/$asset.sha256" "$checksum_url"

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$tmpdir" && sha256sum -c "$asset.sha256")
elif command -v shasum >/dev/null 2>&1; then
  (cd "$tmpdir" && shasum -a 256 -c "$asset.sha256")
else
  echo "install.sh: sha256sum or shasum is required for checksum verification" >&2
  exit 1
fi

mkdir -p "$install_dir"
cp "$tmpdir/$asset" "$target"
chmod 0755 "$target"

echo "installed mstyle to $target"
case ":${PATH:-}:" in
  *":$install_dir:"*) ;;
  *) echo "add $install_dir to PATH to run mstyle by name" ;;
esac
