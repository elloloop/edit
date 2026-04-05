#!/usr/bin/env bash
set -euo pipefail

# edit — TUI code editor installer
# Usage: curl -fsSL https://raw.githubusercontent.com/elloloop/edit/main/install.sh | sh

REPO="elloloop/edit"
BINARY="edit"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${CYAN}${BOLD}info${NC}  $1"; }
ok()    { echo -e "${GREEN}${BOLD}  ok${NC}  $1"; }
warn()  { echo -e "${YELLOW}${BOLD}warn${NC}  $1"; }
fail()  { echo -e "${RED}${BOLD}fail${NC}  $1"; exit 1; }

echo ""
echo -e "${BOLD}  > edit${NC} installer"
echo -e "  A TUI code editor for the age of AI agents"
echo ""

# --- Detect OS and architecture ---
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin) PLATFORM="apple-darwin" ;;
  Linux)  PLATFORM="unknown-linux-gnu" ;;
  *)      fail "Unsupported OS: $OS (only macOS and Linux are supported)" ;;
esac

case "$ARCH" in
  x86_64)  TARGET_ARCH="x86_64" ;;
  aarch64) TARGET_ARCH="aarch64" ;;
  arm64)   TARGET_ARCH="aarch64" ;;
  *)       fail "Unsupported architecture: $ARCH" ;;
esac

TARGET="${TARGET_ARCH}-${PLATFORM}"
info "Detected platform: ${TARGET}"

# --- Try downloading prebuilt binary from GitHub releases ---
LATEST_URL="https://api.github.com/repos/${REPO}/releases/latest"

download_release() {
  info "Checking for prebuilt binary..."

  if command -v curl &>/dev/null; then
    RELEASE_JSON=$(curl -fsSL "$LATEST_URL" 2>/dev/null || echo "")
  elif command -v wget &>/dev/null; then
    RELEASE_JSON=$(wget -qO- "$LATEST_URL" 2>/dev/null || echo "")
  else
    return 1
  fi

  if [ -z "$RELEASE_JSON" ]; then
    return 1
  fi

  # Extract download URL for our target
  ASSET_URL=$(echo "$RELEASE_JSON" | grep -o "\"browser_download_url\": *\"[^\"]*${TARGET}[^\"]*\"" | head -1 | cut -d'"' -f4)

  if [ -z "$ASSET_URL" ]; then
    return 1
  fi

  info "Downloading from release: ${ASSET_URL}"
  TMPDIR=$(mktemp -d)
  TMPFILE="${TMPDIR}/${BINARY}"

  if curl -fsSL "$ASSET_URL" -o "$TMPFILE"; then
    chmod +x "$TMPFILE"
    return 0
  fi

  return 1
}

# --- Build from source with Cargo ---
build_from_source() {
  info "Building from source with Cargo..."

  if ! command -v cargo &>/dev/null; then
    warn "Cargo not found. Installing Rust toolchain via rustup..."
    if command -v curl &>/dev/null; then
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    else
      fail "Neither cargo nor curl found. Please install Rust: https://rustup.rs"
    fi
    source "$HOME/.cargo/env" 2>/dev/null || export PATH="$HOME/.cargo/bin:$PATH"
  fi

  ok "Cargo found: $(cargo --version)"

  # Clone and build
  TMPDIR=$(mktemp -d)
  info "Cloning ${REPO}..."
  git clone --depth 1 "https://github.com/${REPO}.git" "${TMPDIR}/edit-src" 2>/dev/null || \
    fail "Failed to clone repository. Check your internet connection."

  info "Building (this may take a minute)..."
  cd "${TMPDIR}/edit-src"
  cargo build --release --quiet 2>/dev/null || cargo build --release
  TMPFILE="${TMPDIR}/edit-src/target/release/${BINARY}"

  if [ ! -f "$TMPFILE" ]; then
    fail "Build failed — binary not found at ${TMPFILE}"
  fi

  ok "Build complete"
}

# --- Install ---
install_binary() {
  local src="$1"

  # Try the install directory, fall back to ~/.local/bin
  if [ -w "$INSTALL_DIR" ] || [ -w "$(dirname "$INSTALL_DIR")" ]; then
    mkdir -p "$INSTALL_DIR"
    cp "$src" "${INSTALL_DIR}/${BINARY}"
    chmod +x "${INSTALL_DIR}/${BINARY}"
    ok "Installed to ${INSTALL_DIR}/${BINARY}"
  else
    info "Need sudo to install to ${INSTALL_DIR}"
    sudo mkdir -p "$INSTALL_DIR"
    sudo cp "$src" "${INSTALL_DIR}/${BINARY}"
    sudo chmod +x "${INSTALL_DIR}/${BINARY}"
    ok "Installed to ${INSTALL_DIR}/${BINARY}"
  fi
}

# --- Main ---

TMPFILE=""

# Try prebuilt binary first
if download_release 2>/dev/null; then
  ok "Downloaded prebuilt binary"
else
  info "No prebuilt binary found for ${TARGET}, building from source..."
  build_from_source
fi

if [ -n "$TMPFILE" ] && [ -f "$TMPFILE" ]; then
  install_binary "$TMPFILE"
else
  fail "Installation failed — no binary to install"
fi

# Cleanup
if [ -n "${TMPDIR:-}" ] && [ -d "$TMPDIR" ]; then
  rm -rf "$TMPDIR"
fi

echo ""
echo -e "${GREEN}${BOLD}  Installation complete!${NC}"
echo ""
echo -e "  Run ${BOLD}edit${NC} to get started."
echo -e "  Run ${BOLD}edit <file>${NC} to open a file."
echo -e "  Run ${BOLD}edit <dir>${NC} to browse a directory."
echo ""
echo -e "  Type ${BOLD}help${NC} in the command bar for all commands."
echo ""
