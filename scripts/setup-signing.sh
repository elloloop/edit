#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# Apple Code Signing Setup
#
# Opens URLs in your default browser (the one you're already logged into).
# No Playwright, no puppeteer, no fresh browser.
# =============================================================================

cd "$(dirname "$0")/.."

REPO="elloloop/edit"
WORK_DIR=".signing"
KEY_FILE="$WORK_DIR/dev-id.key"
CSR_FILE="$WORK_DIR/dev-id.csr"
CER_FILE="$WORK_DIR/dev-id.cer"
PEM_FILE="$WORK_DIR/dev-id.pem"
P12_FILE="$WORK_DIR/dev-id.p12"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'
info()  { echo -e "${CYAN}${BOLD}info${NC}  $1"; }
ok()    { echo -e "${GREEN}${BOLD}  ok${NC}  $1"; }
warn()  { echo -e "${YELLOW}${BOLD}warn${NC}  $1"; }
fail()  { echo -e "${RED}${BOLD}fail${NC}  $1"; exit 1; }
ask()   { read -rp "  $1" "$2"; }

echo ""
echo -e "  ${BOLD}> edit${NC}  Apple code signing setup"
echo -e "  Uses your default browser — no separate login needed."
echo ""

mkdir -p "$WORK_DIR"

# ─────────────────────────────────────────────────────────────────────────────
# Step 1: Generate CSR + private key
# ─────────────────────────────────────────────────────────────────────────────

if [ -f "$CSR_FILE" ]; then
  info "CSR already exists, reusing"
else
  info "Generating private key + CSR..."
  openssl genrsa -out "$KEY_FILE" 2048 2>/dev/null
  openssl req -new -key "$KEY_FILE" -out "$CSR_FILE" \
    -subj "/CN=edit Developer ID/O=Elloloop/C=US" 2>/dev/null
  ok "Generated $KEY_FILE and $CSR_FILE"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Step 2: Create certificate on Apple Developer portal
# ─────────────────────────────────────────────────────────────────────────────

if [ -f "$CER_FILE" ]; then
  info "Certificate already downloaded, skipping"
else
  echo ""
  info "Opening Apple Developer portal in your browser..."
  open "https://developer.apple.com/account/resources/certificates/add"

  echo ""
  echo -e "  ${BOLD}In the browser:${NC}"
  echo "    1. Select ${BOLD}Developer ID Application${NC}"
  echo "    2. Click Continue"
  echo "    3. Choose ${BOLD}G2 Sub-CA${NC} (default is fine)"
  echo "    4. Click Continue"
  echo "    5. Upload this CSR file: ${BOLD}$(pwd)/$CSR_FILE${NC}"
  echo "    6. Click Continue"
  echo "    7. Click ${BOLD}Download${NC}"
  echo ""
  echo -e "  ${YELLOW}Tip:${NC} drag the CSR file from Finder, or click 'Choose File' and navigate to:"
  echo -e "  ${BOLD}$(pwd)/$CSR_FILE${NC}"
  echo ""

  # Copy CSR path to clipboard for easy pasting
  echo -n "$(pwd)/$CSR_FILE" | pbcopy 2>/dev/null && \
    ok "CSR path copied to clipboard"

  echo ""
  read -rp "  Where did the .cer download to? (drag file here or press Enter for ~/Downloads): " CER_PATH
  CER_PATH="${CER_PATH:-$HOME/Downloads/developerID_application.cer}"
  # Strip quotes that drag-and-drop adds
  CER_PATH="${CER_PATH//\'/}"
  CER_PATH="${CER_PATH//\"/}"
  CER_PATH="${CER_PATH## }"

  # Try common names if the exact file doesn't exist
  if [ ! -f "$CER_PATH" ]; then
    for f in "$HOME/Downloads/developerID_application.cer" \
             "$HOME/Downloads/DeveloperIDApplication.cer" \
             "$HOME/Downloads"/developer_id_application*.cer; do
      if [ -f "$f" ]; then
        CER_PATH="$f"
        break
      fi
    done
  fi

  if [ ! -f "$CER_PATH" ]; then
    fail "Certificate not found at: $CER_PATH"
  fi

  cp "$CER_PATH" "$CER_FILE"
  ok "Certificate saved to $CER_FILE"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Step 3: Convert .cer → .p12
# ─────────────────────────────────────────────────────────────────────────────

echo ""
ask "Choose a password for the .p12 file: " P12_PASS

info "Converting certificate to .p12..."
openssl x509 -inform DER -in "$CER_FILE" -out "$PEM_FILE" 2>/dev/null
openssl pkcs12 -export -out "$P12_FILE" \
  -inkey "$KEY_FILE" -in "$PEM_FILE" \
  -password "pass:$P12_PASS" 2>/dev/null
ok "Created $P12_FILE"

# ─────────────────────────────────────────────────────────────────────────────
# Step 4: App-specific password
# ─────────────────────────────────────────────────────────────────────────────

echo ""
info "Opening Apple ID for app-specific password..."
open "https://account.apple.com/account/manage/security/app-specific-passwords"

echo ""
echo -e "  ${BOLD}In the browser:${NC}"
echo "    1. Click the ${BOLD}+${NC} button"
echo "    2. Name it: ${BOLD}edit-signing${NC}"
echo "    3. Copy the generated password"
echo ""

ask "Paste the app-specific password here: " APP_PASS

if [ -z "$APP_PASS" ]; then
  fail "No password provided"
fi
ok "Got app-specific password"

# ─────────────────────────────────────────────────────────────────────────────
# Step 5: Collect remaining info
# ─────────────────────────────────────────────────────────────────────────────

echo ""
info "Opening Apple Developer account page for Team ID..."
open "https://developer.apple.com/account#MembershipDetailsCard"
echo ""

ask "Enter your 10-character Team ID (shown on that page): " TEAM_ID
ask "Enter your Apple ID email: " APPLE_ID

# ─────────────────────────────────────────────────────────────────────────────
# Step 6: Set GitHub secrets
# ─────────────────────────────────────────────────────────────────────────────

echo ""
info "Setting GitHub secrets on $REPO..."

P12_B64=$(base64 -i "$P12_FILE")

set_secret() {
  echo -n "$2" | gh secret set "$1" -R "$REPO" && ok "Set $1" || warn "Failed to set $1"
}

set_secret "APPLE_CERTIFICATE_BASE64" "$P12_B64"
set_secret "APPLE_CERTIFICATE_PASSWORD" "$P12_PASS"
set_secret "APPLE_TEAM_ID" "$TEAM_ID"
set_secret "APPLE_ID" "$APPLE_ID"
set_secret "APPLE_ID_PASSWORD" "$APP_PASS"

# ─────────────────────────────────────────────────────────────────────────────
# Done
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo -e "  ${GREEN}${BOLD}Done!${NC}"
echo ""
echo "  Next: tag a release to build signed + notarized binaries:"
echo "    git tag v0.3.0 && git push origin v0.3.0"
echo ""
warn "Sensitive files are in .signing/ — delete when done:"
warn "  rm -rf .signing/"
echo ""
