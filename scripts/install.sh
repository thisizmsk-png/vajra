#!/usr/bin/env bash
set -euo pipefail

# Vajra Agent Harness — Install Script
# Creates directory structure, generates HMAC key, initializes SQLite, registers hooks

VAJRA_HOME="${HOME}/.claude/vajra"
VAJRA_SKILL="${HOME}/.claude/skills/vajra"
SETTINGS_FILE="${HOME}/.claude/settings.json"

echo "=== Vajra Agent Harness — Install ==="

# 1. Check dependencies
echo "[1/6] Checking dependencies..."
command -v git >/dev/null 2>&1 || { echo "ERROR: git not found"; exit 1; }
command -v node >/dev/null 2>&1 || { echo "ERROR: node not found"; exit 1; }

NODE_VERSION=$(node -v | sed 's/v//' | cut -d. -f1)
if [ "$NODE_VERSION" -lt 18 ]; then
  echo "ERROR: Node.js 18+ required (found v${NODE_VERSION})"
  exit 1
fi

echo "  git: $(git --version | head -1)"
echo "  node: $(node -v)"

# 2. Create directory structure
echo "[2/6] Creating directory structure..."
mkdir -p "${VAJRA_HOME}/campaigns"
mkdir -p "${VAJRA_HOME}/memory/topics"
mkdir -p "${VAJRA_HOME}/memory/archive"
mkdir -p "${VAJRA_HOME}/relay"

# Set permissions
chmod 700 "${VAJRA_HOME}"
chmod 700 "${VAJRA_HOME}/campaigns"

# 3. Generate HMAC key if not present
echo "[3/6] Setting up HMAC key..."
HMAC_KEY_FILE="${VAJRA_HOME}/.hmac-key"
if [ ! -f "${HMAC_KEY_FILE}" ]; then
  node -e "require('crypto').randomBytes(32).toString('hex')" > "${HMAC_KEY_FILE}"
  chmod 600 "${HMAC_KEY_FILE}"
  echo "  Generated new HMAC key"
else
  echo "  HMAC key exists, skipping"
fi

# 4. Initialize memory index if not present
echo "[4/6] Initializing memory..."
INDEX_FILE="${VAJRA_HOME}/memory/index.md"
if [ ! -f "${INDEX_FILE}" ]; then
  echo "# Vajra Memory Index" > "${INDEX_FILE}"
  echo "" >> "${INDEX_FILE}"
  echo "<!-- Entries below. Max 200 lines. -->" >> "${INDEX_FILE}"
  chmod 600 "${INDEX_FILE}"
  echo "  Created memory index"
else
  echo "  Memory index exists, skipping"
fi

# 5. Install npm dependencies (if package.json exists)
echo "[5/6] Checking npm dependencies..."
if [ -f "${VAJRA_SKILL}/package.json" ]; then
  if cd "${VAJRA_SKILL}" && npm install --production 2>&1 | tail -3; then
    echo "  Dependencies installed"
  else
    echo "  WARNING: npm install failed — some features may not work" >&2
  fi
else
  echo "  No package.json yet — skipping npm install"
fi

# 6. Verify manifest integrity
echo "[6/6] Verifying installation..."
if [ -f "${VAJRA_SKILL}/manifest.json" ]; then
  echo "  Manifest found — run /vajra verify to check integrity"
else
  echo "  No manifest yet — will be generated after all files are created"
fi

echo ""
echo "=== Vajra installed ==="
echo "  Data dir: ${VAJRA_HOME}"
echo "  Skill dir: ${VAJRA_SKILL}"
echo ""
echo "Run '/vajra help' to get started."
