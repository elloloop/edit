#!/usr/bin/env bash
set -euo pipefail

# edit — TUI code editor installer
# Downloads prebuilt binary from GitHub Releases. No build tools needed.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/elloloop/edit/main/install.sh | sh
#
# Options (env vars):
#   INSTALL_DIR  — where to install (default: /usr/local/bin)

REPO="elloloop/edit"
BINARY="edit"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

# --- Colors ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info() { echo -e "${CYAN}${BOLD}info${NC}  $1"; }
ok()   { echo -e "${GREEN}${BOLD}  ok${NC}  $1"; }
warn() { echo -e "${YELLOW}${BOLD}warn${NC}  $1"; }
fail() { echo -e "${RED}${BOLD}fail${NC}  $1"; exit 1; }

echo ""
echo -e "${BOLD}  > edit${NC} installer"
echo -e "  A TUI code editor for the age of AI agents"
echo ""

# --- Detect OS and architecture ---
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin)  PLATFORM="apple-darwin" ;;
  Linux)   PLATFORM="unknown-linux-gnu" ;;
  MINGW*|MSYS*|CYGWIN*)
    fail "On Windows, download the .exe directly from https://github.com/${REPO}/releases/latest"
    ;;
  *)       fail "Unsupported OS: $OS. Download binaries at https://github.com/${REPO}/releases/latest" ;;
esac

case "$ARCH" in
  x86_64)        TARGET_ARCH="x86_64" ;;
  aarch64|arm64) TARGET_ARCH="aarch64" ;;
  *)             fail "Unsupported architecture: $ARCH" ;;
esac

TARGET="${TARGET_ARCH}-${PLATFORM}"
ASSET="edit-${TARGET}"

info "Detected platform: ${TARGET}"

# --- Find latest release ---
info "Fetching latest release..."

RELEASE_JSON=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null) || \
  fail "Could not reach GitHub API. Check your internet connection."

TAG=$(echo "$RELEASE_JSON" | grep '"tag_name"' | head -1 | cut -d'"' -f4)

if [ -z "$TAG" ]; then
  fail "No releases found. Visit https://github.com/${REPO}/releases"
fi

ok "Latest release: ${TAG}"

# --- Download binary ---
URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET}"

info "Downloading ${ASSET}..."

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

HTTP_CODE=$(curl -fsSL -w '%{http_code}' -o "${TMPDIR}/${BINARY}" "$URL" 2>/dev/null) || true

if [ ! -f "${TMPDIR}/${BINARY}" ] || [ "$HTTP_CODE" = "404" ]; then
  fail "Binary not found for ${TARGET}. Check https://github.com/${REPO}/releases/tag/${TAG}"
fi

ok "Downloaded $(du -h "${TMPDIR}/${BINARY}" | cut -f1 | xargs) from release ${TAG}"

chmod +x "${TMPDIR}/${BINARY}"

# --- Install ---
if [ -w "$INSTALL_DIR" ]; then
  mkdir -p "$INSTALL_DIR"
  mv "${TMPDIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
  ok "Installed to ${INSTALL_DIR}/${BINARY}"
elif [ -w "$(dirname "$INSTALL_DIR")" ]; then
  mkdir -p "$INSTALL_DIR"
  mv "${TMPDIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
  ok "Installed to ${INSTALL_DIR}/${BINARY}"
else
  info "Need sudo to install to ${INSTALL_DIR}"
  sudo mkdir -p "$INSTALL_DIR"
  sudo mv "${TMPDIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
  sudo chmod +x "${INSTALL_DIR}/${BINARY}"
  ok "Installed to ${INSTALL_DIR}/${BINARY}"
fi

# --- Verify ---
if command -v "$BINARY" &>/dev/null; then
  ok "Verified: $(which $BINARY)"
else
  warn "${INSTALL_DIR} may not be in your PATH"
  warn "Add this to your shell profile:  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi

# --- Set as default EDITOR ---
SHELL_RC=""
if [ -f "$HOME/.zshrc" ]; then
  SHELL_RC="$HOME/.zshrc"
elif [ -f "$HOME/.bashrc" ]; then
  SHELL_RC="$HOME/.bashrc"
fi

if [ -n "$SHELL_RC" ]; then
  if ! grep -q 'EDITOR=edit' "$SHELL_RC" 2>/dev/null; then
    echo "" >> "$SHELL_RC"
    echo "# Set edit as default editor (for git, codex, etc.)" >> "$SHELL_RC"
    echo "export EDITOR=edit" >> "$SHELL_RC"
    echo "export VISUAL=edit" >> "$SHELL_RC"
    ok "Set EDITOR=edit in ${SHELL_RC}"
  fi
fi

# --- Install .desktop file on Linux ---
if [ "$OS" = "Linux" ]; then
  DESKTOP_DIR="$HOME/.local/share/applications"
  DESKTOP_URL="https://raw.githubusercontent.com/${REPO}/main/edit.desktop"
  mkdir -p "$DESKTOP_DIR"
  curl -fsSL "$DESKTOP_URL" -o "${DESKTOP_DIR}/edit.desktop" 2>/dev/null && \
    ok "Installed edit.desktop for file associations" || true
fi

echo ""
echo -e "${GREEN}${BOLD}  Installation complete!${NC}"
echo ""
echo -e "  Run ${BOLD}edit${NC} to get started."
echo -e "  Run ${BOLD}edit <file>${NC} to open a file."
echo -e "  Run ${BOLD}edit <dir>${NC} to browse a directory."
echo ""
echo -e "  Type ${BOLD}help${NC} in the command bar for all commands."
echo -e "  Set as default editor: tools like git, codex, claude will use it."
echo ""
