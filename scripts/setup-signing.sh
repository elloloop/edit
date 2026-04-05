#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# Apple Code Signing Setup — fully automated via API
#
# Only one browser step: creating the API key (if you don't have one).
# Everything else is API calls.
# =============================================================================

cd "$(dirname "$0")/.."

REPO="elloloop/edit"
WORK_DIR=".signing"
KEY_FILE="$WORK_DIR/dev-id.key"
CSR_FILE="$WORK_DIR/dev-id.csr"
CER_FILE="$WORK_DIR/dev-id.cer"
PEM_FILE="$WORK_DIR/dev-id.pem"
P12_FILE="$WORK_DIR/dev-id.p12"
JWT_FILE="$WORK_DIR/jwt.txt"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'
info()  { echo -e "${CYAN}${BOLD}info${NC}  $1"; }
ok()    { echo -e "${GREEN}${BOLD}  ok${NC}  $1"; }
warn()  { echo -e "${YELLOW}${BOLD}warn${NC}  $1"; }
fail()  { echo -e "${RED}${BOLD}fail${NC}  $1"; exit 1; }

echo ""
echo -e "  ${BOLD}> edit${NC}  Apple code signing setup"
echo -e "  Uses Apple's API — no clicking through the portal."
echo ""

mkdir -p "$WORK_DIR"

# ─────────────────────────────────────────────────────────────────────────────
# Step 1: Get API key details
# ─────────────────────────────────────────────────────────────────────────────

echo -e "  ${BOLD}You need an App Store Connect API key.${NC}"
echo "  If you don't have one, create one now:"
echo ""
echo "    1. Go to: https://appstoreconnect.apple.com/access/integrations/api"
echo "    2. Click + to create a key"
echo "    3. Name: edit-signing, Access: Admin"
echo "    4. Download the .p8 file"
echo ""

read -rp "  Open that page now? [Y/n] " OPEN_PAGE
if [[ "${OPEN_PAGE:-y}" =~ ^[Yy]?$ ]]; then
  open "https://appstoreconnect.apple.com/access/integrations/api" 2>/dev/null || true
fi

echo ""
read -rp "  Path to your .p8 API key file (drag here): " API_KEY_PATH
API_KEY_PATH="${API_KEY_PATH//\'/}"
API_KEY_PATH="${API_KEY_PATH//\"/}"
API_KEY_PATH="${API_KEY_PATH## }"
API_KEY_PATH="${API_KEY_PATH%% }"

[ -f "$API_KEY_PATH" ] || fail "File not found: $API_KEY_PATH"
ok "Found API key file"

read -rp "  Key ID (shown in App Store Connect): " KEY_ID
read -rp "  Issuer ID (shown at the top of the API keys page): " ISSUER_ID

# ─────────────────────────────────────────────────────────────────────────────
# Step 2: Generate JWT token
# ─────────────────────────────────────────────────────────────────────────────

info "Generating JWT..."

node -e "
const fs = require('fs');
const crypto = require('crypto');

const keyId = '$KEY_ID';
const issuerId = '$ISSUER_ID';
const privateKey = fs.readFileSync('$API_KEY_PATH', 'utf8');

const now = Math.floor(Date.now() / 1000);
const header = { alg: 'ES256', kid: keyId, typ: 'JWT' };
const payload = {
  iss: issuerId,
  iat: now,
  exp: now + 1200,  // 20 min
  aud: 'appstoreconnect-v1'
};

function base64url(obj) {
  return Buffer.from(JSON.stringify(obj))
    .toString('base64')
    .replace(/=/g, '').replace(/\+/g, '-').replace(/\//g, '_');
}

const headerB64 = base64url(header);
const payloadB64 = base64url(payload);
const signingInput = headerB64 + '.' + payloadB64;

const sign = crypto.createSign('SHA256');
sign.update(signingInput);
const sig = sign.sign(privateKey);

// ES256 signature: convert DER to raw r||s (64 bytes)
// Node crypto outputs DER format, we need raw for JWT
const derToRaw = (der) => {
  let offset = 3;
  let rLen = der[offset]; offset++;
  let r = der.subarray(offset, offset + rLen); offset += rLen;
  offset++; // 0x02
  let sLen = der[offset]; offset++;
  let s = der.subarray(offset, offset + sLen);
  // Pad/trim to 32 bytes each
  if (r.length > 32) r = r.subarray(r.length - 32);
  if (s.length > 32) s = s.subarray(s.length - 32);
  const raw = Buffer.alloc(64);
  r.copy(raw, 32 - r.length);
  s.copy(raw, 64 - s.length);
  return raw;
};

const rawSig = derToRaw(sig);
const sigB64 = rawSig.toString('base64')
  .replace(/=/g, '').replace(/\+/g, '-').replace(/\//g, '_');

const jwt = signingInput + '.' + sigB64;
fs.writeFileSync('$JWT_FILE', jwt);
" || fail "JWT generation failed"

JWT=$(cat "$JWT_FILE")
ok "JWT generated"

# ─────────────────────────────────────────────────────────────────────────────
# Step 3: Generate CSR
# ─────────────────────────────────────────────────────────────────────────────

if [ -f "$CSR_FILE" ]; then
  info "CSR already exists, reusing"
else
  info "Generating private key + CSR..."
  openssl genrsa -out "$KEY_FILE" 2048 2>/dev/null
  openssl req -new -key "$KEY_FILE" -out "$CSR_FILE" \
    -subj "/CN=edit Developer ID/O=Elloloop/C=US" 2>/dev/null
  ok "Generated CSR"
fi

CSR_CONTENT=$(cat "$CSR_FILE")

# ─────────────────────────────────────────────────────────────────────────────
# Step 4: Create certificate via Apple API
# ─────────────────────────────────────────────────────────────────────────────

info "Creating Developer ID Application certificate via API..."

API_RESPONSE=$(curl -s -X POST "https://api.appstoreconnect.apple.com/v1/certificates" \
  -H "Authorization: Bearer $JWT" \
  -H "Content-Type: application/json" \
  -d "$(node -e "
    const csr = require('fs').readFileSync('$CSR_FILE', 'utf8');
    console.log(JSON.stringify({
      data: {
        type: 'certificates',
        attributes: {
          csrContent: csr,
          certificateType: 'DEVELOPER_ID_APPLICATION'
        }
      }
    }));
  ")")

# Check for errors
ERROR=$(echo "$API_RESPONSE" | node -e "
  const d = JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
  if (d.errors) { console.log(d.errors[0].detail || d.errors[0].title); }
" 2>/dev/null || echo "")

if [ -n "$ERROR" ]; then
  warn "API error: $ERROR"
  echo ""
  warn "This can happen if you already have 2 Developer ID certs (Apple's limit)."
  warn "Check existing certs at: https://developer.apple.com/account/resources/certificates/list"
  echo ""
  read -rp "  Try listing existing certificates instead? [Y/n] " TRY_LIST
  if [[ "${TRY_LIST:-y}" =~ ^[Yy]?$ ]]; then
    info "Fetching existing Developer ID Application certificates..."
    LIST_RESPONSE=$(curl -s "https://api.appstoreconnect.apple.com/v1/certificates?filter[certificateType]=DEVELOPER_ID_APPLICATION" \
      -H "Authorization: Bearer $JWT")

    CERT_DATA=$(echo "$LIST_RESPONSE" | node -e "
      const d = JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
      if (d.data && d.data.length > 0) {
        // Get the first valid cert
        const cert = d.data[0];
        console.log(cert.attributes.certificateContent);
      }
    " 2>/dev/null || echo "")

    if [ -n "$CERT_DATA" ]; then
      echo "$CERT_DATA" | base64 --decode > "$CER_FILE" 2>/dev/null
      ok "Downloaded existing certificate"
    else
      fail "No existing Developer ID Application certificates found"
    fi
  else
    fail "Certificate creation failed"
  fi
else
  # Extract the certificate content from the response
  echo "$API_RESPONSE" | node -e "
    const d = JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
    const cert = d.data.attributes.certificateContent;
    require('fs').writeFileSync('$CER_FILE', Buffer.from(cert, 'base64'));
  "
  ok "Certificate created and downloaded"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Step 5: Convert to .p12
# ─────────────────────────────────────────────────────────────────────────────

echo ""
read -rp "  Choose a password for the .p12 file: " P12_PASS

info "Converting to .p12..."
openssl x509 -inform DER -in "$CER_FILE" -out "$PEM_FILE" 2>/dev/null
openssl pkcs12 -export -out "$P12_FILE" \
  -inkey "$KEY_FILE" -in "$PEM_FILE" \
  -password "pass:$P12_PASS" 2>/dev/null
ok "Created $P12_FILE"

# ─────────────────────────────────────────────────────────────────────────────
# Step 6: App-specific password (for notarization)
# ─────────────────────────────────────────────────────────────────────────────

echo ""
info "You need an app-specific password for notarization."
echo "  Create one at: https://account.apple.com/account/manage/security/app-specific-passwords"
echo ""

read -rp "  Open that page? [Y/n] " OPEN_ASP
if [[ "${OPEN_ASP:-y}" =~ ^[Yy]?$ ]]; then
  open "https://account.apple.com/account/manage/security/app-specific-passwords" 2>/dev/null || true
fi

echo ""
echo "  Click +, name it 'edit-signing', copy the password."
echo ""
read -rp "  Paste the app-specific password: " APP_PASS
[ -n "$APP_PASS" ] || fail "No password provided"
ok "Got app-specific password"

# ─────────────────────────────────────────────────────────────────────────────
# Step 7: Collect Team ID + Apple ID
# ─────────────────────────────────────────────────────────────────────────────

echo ""
read -rp "  Your Apple Team ID (10-char, from developer.apple.com/account): " TEAM_ID
read -rp "  Your Apple ID email: " APPLE_ID

# ─────────────────────────────────────────────────────────────────────────────
# Step 8: Set GitHub secrets
# ─────────────────────────────────────────────────────────────────────────────

echo ""
info "Setting GitHub secrets on $REPO..."

P12_B64=$(base64 -i "$P12_FILE")

set_secret() {
  echo -n "$2" | gh secret set "$1" -R "$REPO" 2>/dev/null && ok "Set $1" || warn "Failed to set $1"
}

set_secret "APPLE_CERTIFICATE_BASE64" "$P12_B64"
set_secret "APPLE_CERTIFICATE_PASSWORD" "$P12_PASS"
set_secret "APPLE_TEAM_ID" "$TEAM_ID"
set_secret "APPLE_ID" "$APPLE_ID"
set_secret "APPLE_ID_PASSWORD" "$APP_PASS"

echo ""
echo -e "  ${GREEN}${BOLD}Done!${NC} All secrets configured."
echo ""
echo "  Tag a release to build signed + notarized binaries:"
echo "    git tag v0.3.0 && git push origin v0.3.0"
echo ""
warn "Delete sensitive files when done: rm -rf .signing/"
echo ""
