#!/usr/bin/env node

// =============================================================================
// Apple Developer Certificate Setup + GitHub Secrets
//
// This script:
//   1. Generates a CSR + private key
//   2. Opens Apple Developer portal → creates "Developer ID Application" cert
//   3. Downloads the .cer, converts to .p12
//   4. Opens appleid.apple.com → creates an app-specific password
//   5. Sets all 5 GitHub secrets on elloloop/edit
//
// Usage: node scripts/setup-signing.mjs
// =============================================================================

import { chromium } from "playwright";
import { execSync } from "child_process";
import { existsSync, readFileSync, mkdirSync } from "fs";
import { join } from "path";
import { createInterface } from "readline";

const REPO = "elloloop/edit";
const WORK_DIR = join(import.meta.dirname, "..", ".signing");
const KEY_FILE = join(WORK_DIR, "dev-id.key");
const CSR_FILE = join(WORK_DIR, "dev-id.csr");
const CER_FILE = join(WORK_DIR, "dev-id.cer");
const PEM_FILE = join(WORK_DIR, "dev-id.pem");
const P12_FILE = join(WORK_DIR, "dev-id.p12");

function ask(question) {
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  return new Promise((resolve) => {
    rl.question(question, (answer) => {
      rl.close();
      resolve(answer.trim());
    });
  });
}

function run(cmd) {
  return execSync(cmd, { encoding: "utf-8", stdio: "pipe" }).trim();
}

function log(msg) {
  console.log(`\x1b[32m  ok\x1b[0m  ${msg}`);
}
function info(msg) {
  console.log(`\x1b[36minfo\x1b[0m  ${msg}`);
}
function warn(msg) {
  console.log(`\x1b[33mwarn\x1b[0m  ${msg}`);
}

// =============================================================================
// Step 1: Generate CSR + Private Key
// =============================================================================

async function generateCSR() {
  if (!existsSync(WORK_DIR)) mkdirSync(WORK_DIR, { recursive: true });

  if (existsSync(KEY_FILE) && existsSync(CSR_FILE)) {
    info("CSR already exists, reusing");
    return;
  }

  info("Generating private key + CSR...");
  run(
    `openssl genrsa -out "${KEY_FILE}" 2048`
  );
  run(
    `openssl req -new -key "${KEY_FILE}" -out "${CSR_FILE}" -subj "/CN=edit Developer ID/O=Elloloop/C=US"`
  );
  log("Generated private key + CSR");
}

// =============================================================================
// Step 2: Create certificate on Apple Developer portal
// =============================================================================

async function createCertificate(browser) {
  info("Opening Apple Developer portal...");
  info("You will need to log in and complete 2FA.");
  console.log("");

  const context = await browser.newContext({
    acceptDownloads: true,
  });
  const page = await context.newPage();

  // Go to certificates page
  await page.goto(
    "https://developer.apple.com/account/resources/certificates/add"
  );

  // Wait for user to log in — the certificates page will eventually load
  info("Waiting for you to log in (2FA)...");
  info('Once logged in, the "Create a New Certificate" page should appear.');

  // Wait for the certificate creation form to appear (user has logged in)
  // Apple's form has radio buttons for certificate types
  await page.waitForSelector('input[type="radio"]', {
    timeout: 300_000, // 5 min for login
  });
  log("Logged in to Apple Developer portal");

  // Select "Developer ID Application"
  // Apple labels this with text, let's find it
  const devIdRadio = page.locator(
    'text="Developer ID Application" >> xpath=ancestor::label//input[type="radio"]'
  );

  const devIdAlt = page.locator(
    'label:has-text("Developer ID Application") input[type="radio"]'
  );

  try {
    if (await devIdRadio.isVisible({ timeout: 3000 })) {
      await devIdRadio.click();
    } else {
      await devIdAlt.click();
    }
    log('Selected "Developer ID Application"');
  } catch {
    warn(
      'Could not auto-select "Developer ID Application".'
    );
    warn("Please select it manually, then press Enter here.");
    await ask("Press Enter when ready...");
  }

  // Click Continue
  try {
    const continueBtn = page.locator(
      'button:has-text("Continue"), a:has-text("Continue"), input[value="Continue"]'
    );
    await continueBtn.first().click();
    log("Clicked Continue");
  } catch {
    warn("Please click Continue manually, then press Enter.");
    await ask("Press Enter when ready...");
  }

  // Wait for the CSR upload page
  await page.waitForTimeout(2000);

  // Upload CSR file
  try {
    const fileInput = page.locator('input[type="file"]');
    await fileInput.waitFor({ timeout: 15_000 });
    await fileInput.setInputFiles(CSR_FILE);
    log("Uploaded CSR file");
  } catch {
    warn(`Please upload the CSR file manually: ${CSR_FILE}`);
    await ask("Press Enter when ready...");
  }

  // Click Continue again
  await page.waitForTimeout(1000);
  try {
    const continueBtn = page.locator(
      'button:has-text("Continue"), a:has-text("Continue"), input[value="Continue"]'
    );
    await continueBtn.first().click();
    log("Clicked Continue");
  } catch {
    warn("Please click Continue manually.");
    await ask("Press Enter when ready...");
  }

  // Wait for download page and download the certificate
  info("Waiting for certificate to be ready...");
  await page.waitForTimeout(3000);

  try {
    const downloadBtn = page.locator(
      'button:has-text("Download"), a:has-text("Download")'
    );
    const [download] = await Promise.all([
      page.waitForEvent("download", { timeout: 30_000 }),
      downloadBtn.first().click(),
    ]);
    await download.saveAs(CER_FILE);
    log(`Downloaded certificate to ${CER_FILE}`);
  } catch {
    warn("Please download the certificate manually.");
    warn(`Save it as: ${CER_FILE}`);
    await ask("Press Enter when the file is saved...");
  }

  await context.close();
}

// =============================================================================
// Step 3: Convert .cer → .p12
// =============================================================================

async function convertToP12() {
  if (!existsSync(CER_FILE)) {
    throw new Error(`Certificate file not found: ${CER_FILE}`);
  }

  const password = await ask(
    "Choose a password for the .p12 file (remember this): "
  );

  info("Converting .cer → .pem → .p12...");

  // Apple .cer is DER format
  run(`openssl x509 -inform DER -in "${CER_FILE}" -out "${PEM_FILE}"`);

  run(
    `openssl pkcs12 -export -out "${P12_FILE}" -inkey "${KEY_FILE}" -in "${PEM_FILE}" -password pass:${password}`
  );

  log("Created .p12 file");
  return password;
}

// =============================================================================
// Step 4: Create app-specific password on appleid.apple.com
// =============================================================================

async function createAppSpecificPassword(browser) {
  info("Opening appleid.apple.com to create an app-specific password...");
  info("You may need to log in again.");
  console.log("");

  const context = await browser.newContext();
  const page = await context.newPage();

  await page.goto("https://account.apple.com/sign-in");

  info("Waiting for you to log in...");

  // Wait for the account page to load after login
  // After login, navigate to the app-specific passwords page
  await page.waitForURL("**/account/**", { timeout: 300_000 });
  log("Logged in to Apple ID");

  await page.waitForTimeout(2000);
  await page.goto("https://account.apple.com/account/manage/security/app-specific-passwords");

  info('On the app-specific passwords page.');
  info('Click "+" or "Generate" to create a new password.');
  info('Name it "edit-signing" or similar.');
  console.log("");

  const appPassword = await ask(
    "Paste the app-specific password Apple shows you: "
  );

  await context.close();
  return appPassword;
}

// =============================================================================
// Step 5: Set GitHub secrets
// =============================================================================

async function setGitHubSecrets(p12Password, appPassword) {
  info("Setting GitHub secrets on " + REPO + "...");

  // Base64 encode the .p12
  const p12Base64 = run(`base64 -i "${P12_FILE}"`);

  // Get Team ID from the user
  const teamId = await ask("Enter your Apple Team ID (10-char, from developer.apple.com/account): ");
  const appleId = await ask("Enter your Apple ID email: ");

  // Set secrets using gh CLI
  const secrets = {
    APPLE_CERTIFICATE_BASE64: p12Base64,
    APPLE_CERTIFICATE_PASSWORD: p12Password,
    APPLE_TEAM_ID: teamId,
    APPLE_ID: appleId,
    APPLE_ID_PASSWORD: appPassword,
  };

  for (const [name, value] of Object.entries(secrets)) {
    try {
      execSync(`gh secret set ${name} -R ${REPO}`, {
        input: value,
        encoding: "utf-8",
        stdio: ["pipe", "pipe", "pipe"],
      });
      log(`Set secret: ${name}`);
    } catch (e) {
      warn(`Failed to set ${name}: ${e.message}`);
      warn(`Set it manually at https://github.com/${REPO}/settings/secrets/actions`);
    }
  }

  log("All GitHub secrets configured");
}

// =============================================================================
// Main
// =============================================================================

async function main() {
  console.log("");
  console.log("  \x1b[1m> edit\x1b[0m  Apple code signing setup");
  console.log("  This will create a Developer ID cert and set GitHub secrets.");
  console.log("");

  // Step 1
  await generateCSR();

  // Launch browser (persistent so login state carries over)
  const browser = await chromium.launch({
    headless: false,
    slowMo: 300,
  });

  try {
    // Step 2
    if (!existsSync(CER_FILE)) {
      await createCertificate(browser);
    } else {
      info("Certificate already downloaded, skipping portal step");
    }

    // Step 3
    const p12Password = await convertToP12();

    // Step 4
    const appPassword = await createAppSpecificPassword(browser);

    // Step 5
    await setGitHubSecrets(p12Password, appPassword);
  } finally {
    await browser.close();
  }

  console.log("");
  console.log("  \x1b[32m\x1b[1m  Done!\x1b[0m");
  console.log("");
  console.log("  Next: tag a release to build signed binaries:");
  console.log("    git tag v0.3.0 && git push origin v0.3.0");
  console.log("");

  // Clean up sensitive files
  warn(`Signing files are in ${WORK_DIR} — delete when done:`);
  warn(`  rm -rf ${WORK_DIR}`);
}

main().catch((e) => {
  console.error(`\x1b[31mError:\x1b[0m ${e.message}`);
  process.exit(1);
});
