#!/usr/bin/env bash
set -euo pipefail

# Apple code signing setup — installs deps and runs the Playwright script
# Usage: ./scripts/setup-signing.sh

cd "$(dirname "$0")"

echo ""
echo "  Installing Playwright..."
npm install --silent 2>/dev/null
npx playwright install chromium 2>/dev/null

echo ""
node setup-signing.mjs
