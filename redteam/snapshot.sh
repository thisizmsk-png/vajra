#!/usr/bin/env bash
# snapshot.sh — Create a read-only snapshot of ~/.claude/vajra/ for red team self-testing
#
# Usage:
#   source snapshot.sh   # Sets VAJRA_SNAPSHOT_DIR, registers cleanup trap
#   snapshot.sh           # Prints snapshot path to stdout
#
# The snapshot is a full copy with all files set read-only.
# Cleanup is automatic on exit, SIGINT, or SIGTERM.

set -euo pipefail

VAJRA_SOURCE="${HOME}/.claude/vajra"
SNAPSHOT_DIR=""

cleanup() {
    if [[ -n "${SNAPSHOT_DIR}" && -d "${SNAPSHOT_DIR}" ]]; then
        # Restore write permissions so rm can delete
        chmod -R u+w "${SNAPSHOT_DIR}" 2>/dev/null || true
        rm -rf "${SNAPSHOT_DIR}"
    fi
}

# Register cleanup on exit and common signals
trap cleanup EXIT
trap cleanup SIGINT
trap cleanup SIGTERM

create_snapshot() {
    # Verify source exists
    if [[ ! -d "${VAJRA_SOURCE}" ]]; then
        echo "ERROR: Source directory ${VAJRA_SOURCE} does not exist" >&2
        exit 1
    fi

    # Create temp directory for snapshot
    SNAPSHOT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/vajra-snapshot-XXXXXXXX")"
    chmod 700 "$SNAPSHOT_DIR"

    # Copy all state to snapshot
    cp -a "${VAJRA_SOURCE}/." "${SNAPSHOT_DIR}/"

    # Make everything read-only
    chmod -R a-w "${SNAPSHOT_DIR}"

    # Export for sourced usage
    export VAJRA_SNAPSHOT_DIR="${SNAPSHOT_DIR}"

    # Print path to stdout for script usage
    echo "${SNAPSHOT_DIR}"
}

# Run
create_snapshot
