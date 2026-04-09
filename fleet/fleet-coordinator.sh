#!/usr/bin/env bash
# fleet-coordinator.sh — Vajra Fleet Mode: parallel Claude Code agent coordinator
# Usage: fleet-coordinator.sh "task1" "task2" "task3"
set -euo pipefail

# --- Constants ---
readonly VAJRA_DIR="${HOME}/.claude/vajra"
readonly WORKTREE_DIR="${VAJRA_DIR}/worktrees"
readonly RELAY_DIR="${VAJRA_DIR}/relay"
readonly CONFIG_FILE="${VAJRA_DIR}/config.json"
readonly HMAC_KEY_FILE="${VAJRA_DIR}/.hmac-key"
readonly LOG_FILE="${VAJRA_DIR}/fleet-$(date +%Y%m%d-%H%M%S).log"
readonly DEFAULT_MAX_AGENTS=6

# --- State arrays ---
declare -a AGENT_PIDS=()
declare -a AGENT_WORKTREES=()
declare -a AGENT_BRANCHES=()
declare -a AGENT_IDS=()
declare -a AGENT_TASKS=()
declare -a AGENT_EXIT_CODES=()
CLEANUP_DONE=false

# --- Logging ---
log() {
    local level="$1"; shift
    local ts
    ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf '[%s] [%s] %s\n' "$ts" "$level" "$*" | tee -a "$LOG_FILE"
}

log_info()  { log "INFO"  "$@"; }
log_warn()  { log "WARN"  "$@"; }
log_error() { log "ERROR" "$@"; }

# --- Helpers ---
generate_uuid() {
    if command -v uuidgen &>/dev/null; then
        uuidgen | tr '[:upper:]' '[:lower:]'
    elif [[ -r /proc/sys/kernel/random/uuid ]]; then
        cat /proc/sys/kernel/random/uuid
    else
        # Fallback: pseudo-random hex
        head -c 16 /dev/urandom | xxd -p | sed 's/\(.\{8\}\)\(.\{4\}\)\(.\{4\}\)\(.\{4\}\)\(.\{12\}\)/\1-\2-\3-\4-\5/'
    fi
}

read_config_value() {
    local key="$1" default="$2"
    if [[ -f "$CONFIG_FILE" ]] && command -v jq &>/dev/null; then
        local val
        val="$(jq -r "$key // empty" "$CONFIG_FILE" 2>/dev/null || true)"
        if [[ -n "$val" ]]; then
            printf '%s' "$val"
            return
        fi
    fi
    printf '%s' "$default"
}

ensure_hmac_key() {
    if [[ ! -f "$HMAC_KEY_FILE" ]]; then
        log_info "Generating HMAC key at ${HMAC_KEY_FILE}"
        mkdir -p "$(dirname "$HMAC_KEY_FILE")"
        head -c 32 /dev/urandom | xxd -p | tr -d '\n' > "$HMAC_KEY_FILE"
        chmod 600 "$HMAC_KEY_FILE"
    fi
}

compute_hmac() {
    local payload="$1"
    local key
    key="$(cat "$HMAC_KEY_FILE")"
    printf '%s' "$payload" | openssl dgst -sha256 -hmac "$key" -hex 2>/dev/null | awk '{print $NF}'
}

verify_hmac() {
    local payload="$1" expected_hmac="$2"
    local actual_hmac
    actual_hmac="$(compute_hmac "$payload")"
    [[ "$actual_hmac" == "$expected_hmac" ]]
}

# --- Cleanup ---
cleanup() {
    if [[ "$CLEANUP_DONE" == "true" ]]; then
        return
    fi
    CLEANUP_DONE=true
    log_info "Cleanup initiated"

    # Terminate any still-running agents
    for i in "${!AGENT_PIDS[@]}"; do
        local pid="${AGENT_PIDS[$i]}"
        if kill -0 "$pid" 2>/dev/null; then
            log_warn "Sending SIGTERM to agent ${AGENT_IDS[$i]} (PID ${pid})"
            kill -TERM "$pid" 2>/dev/null || true
        fi
    done

    # Grace period for agents to exit
    sleep 2

    # Force kill stragglers
    for i in "${!AGENT_PIDS[@]}"; do
        local pid="${AGENT_PIDS[$i]}"
        if kill -0 "$pid" 2>/dev/null; then
            log_warn "Force killing agent ${AGENT_IDS[$i]} (PID ${pid})"
            kill -9 "$pid" 2>/dev/null || true
        fi
    done

    # Remove worktrees
    for i in "${!AGENT_WORKTREES[@]}"; do
        local wt="${AGENT_WORKTREES[$i]}"
        local branch="${AGENT_BRANCHES[$i]}"
        if [[ -d "$wt" ]]; then
            log_info "Removing worktree: ${wt}"
            git worktree remove --force "$wt" 2>/dev/null || {
                log_warn "Failed to remove worktree ${wt}, attempting manual cleanup"
                rm -rf "$wt" 2>/dev/null || true
                git worktree prune 2>/dev/null || true
            }
        fi
        # Delete temporary branch
        if git rev-parse --verify "$branch" &>/dev/null; then
            git branch -D "$branch" 2>/dev/null || true
        fi
    done

    # Clean relay files
    if [[ -d "$RELAY_DIR" ]]; then
        log_info "Cleaning relay directory"
        rm -f "${RELAY_DIR}"/*.json 2>/dev/null || true
    fi

    log_info "Cleanup complete"
}

trap cleanup SIGINT SIGTERM EXIT

# --- Validation ---
validate_environment() {
    if ! command -v git &>/dev/null; then
        log_error "git is not installed"
        exit 1
    fi

    if ! git rev-parse --is-inside-work-tree &>/dev/null; then
        log_error "Not inside a git repository"
        exit 1
    fi

    if ! command -v claude &>/dev/null; then
        log_error "claude CLI is not installed or not in PATH"
        exit 1
    fi

    if ! command -v openssl &>/dev/null; then
        log_error "openssl is not installed (required for HMAC signing)"
        exit 1
    fi

    if ! command -v jq &>/dev/null; then
        log_warn "jq is not installed; config file and relay parsing will use fallbacks"
    fi
}

# --- Agent spawning ---
spawn_agent() {
    local task="$1"
    local agent_id="$2"
    local worktree_path="$3"

    log_info "Spawning agent ${agent_id} for task: ${task}"

    # Write task prompt to a temp file for the agent
    local prompt_file="${worktree_path}/.vajra-task.md"
    cat > "$prompt_file" <<PROMPT
# Fleet Task

You are a Vajra fleet agent (id: ${agent_id}).

## Task
${task}

## Discovery Relay
When you discover something important (a finding, blocker, or insight), write it to:
  ${RELAY_DIR}/${agent_id}.json

Use this exact JSON format:
{
  "agentId": "${agent_id}",
  "timestamp": "<ISO 8601>",
  "type": "<finding|blocker|insight>",
  "content": "<your discovery>",
  "relevantFiles": ["<file paths>"]
}

Then sign the file by appending an "hmac" field. Compute HMAC-SHA256 of the JSON content
(without the hmac field) using the key in ${HMAC_KEY_FILE}.

## Constraints
- Stay within the worktree: ${worktree_path}
- Complete the task, then exit
PROMPT

    # Launch Claude Code targeting the worktree
    (
        cd "$worktree_path"
        claude --print --prompt "$(cat "$prompt_file")" > "${worktree_path}/.vajra-output.log" 2>&1
    ) &

    echo $!
}

# --- Result collection ---
collect_discoveries() {
    local discoveries=()
    local verified=0
    local rejected=0

    if [[ ! -d "$RELAY_DIR" ]]; then
        log_info "No relay directory found; no discoveries to collect"
        return
    fi

    for relay_file in "${RELAY_DIR}"/*.json; do
        [[ -f "$relay_file" ]] || continue

        if ! command -v jq &>/dev/null; then
            log_warn "Cannot parse relay file without jq: ${relay_file}"
            continue
        fi

        local agent_id content hmac_value payload
        agent_id="$(jq -r '.agentId // "unknown"' "$relay_file" 2>/dev/null)"
        hmac_value="$(jq -r '.hmac // empty' "$relay_file" 2>/dev/null)"

        if [[ -z "$hmac_value" ]]; then
            log_warn "Rejecting unsigned discovery from ${agent_id}"
            ((rejected++)) || true
            continue
        fi

        # Extract payload (JSON without the hmac field) for verification
        payload="$(jq -c 'del(.hmac)' "$relay_file" 2>/dev/null)"

        if verify_hmac "$payload" "$hmac_value"; then
            content="$(jq -r '.content // "no content"' "$relay_file" 2>/dev/null)"
            local dtype
            dtype="$(jq -r '.type // "unknown"' "$relay_file" 2>/dev/null)"
            discoveries+=("  [${dtype}] (${agent_id}): ${content}")
            ((verified++)) || true
        else
            log_warn "HMAC verification failed for discovery from ${agent_id} -- REJECTING"
            ((rejected++)) || true
        fi
    done

    log_info "Discoveries: ${verified} verified, ${rejected} rejected"
    for d in "${discoveries[@]+"${discoveries[@]}"}"; do
        log_info "$d"
    done
}

# --- Main ---
main() {
    if [[ $# -eq 0 ]]; then
        echo "Usage: fleet-coordinator.sh \"task1\" \"task2\" \"task3\" ..."
        echo "  Runs parallel Claude Code agents, one per task, in isolated git worktrees."
        exit 1
    fi

    local max_agents
    max_agents="$(read_config_value '.fleet.maxAgents' "$DEFAULT_MAX_AGENTS")"

    if [[ $# -gt $max_agents ]]; then
        log_error "Too many tasks ($#). Maximum allowed: ${max_agents}"
        exit 1
    fi

    log_info "Fleet coordinator starting with $# task(s), max agents: ${max_agents}"

    validate_environment

    # Ensure directories exist
    mkdir -p "$WORKTREE_DIR" "$RELAY_DIR"
    ensure_hmac_key

    local total=$#
    local succeeded=0
    local failed=0

    # Spawn agents
    for task in "$@"; do
        local uuid
        uuid="$(generate_uuid)"
        local agent_id="fleet-${uuid}"
        local branch="vajra-fleet-${uuid}"
        local worktree_path="${WORKTREE_DIR}/${agent_id}"

        log_info "Creating worktree: ${worktree_path} on branch ${branch}"
        if ! git worktree add "$worktree_path" -b "$branch" 2>/dev/null; then
            log_error "Failed to create worktree for task: ${task}"
            ((failed++)) || true
            continue
        fi

        local pid
        pid="$(spawn_agent "$task" "$agent_id" "$worktree_path")"

        AGENT_PIDS+=("$pid")
        AGENT_WORKTREES+=("$worktree_path")
        AGENT_BRANCHES+=("$branch")
        AGENT_IDS+=("$agent_id")
        AGENT_TASKS+=("$task")
        AGENT_EXIT_CODES+=("")

        log_info "Agent ${agent_id} started (PID: ${pid})"
    done

    if [[ ${#AGENT_PIDS[@]} -eq 0 ]]; then
        log_error "No agents were spawned. Exiting."
        exit 1
    fi

    log_info "All agents spawned. Monitoring ${#AGENT_PIDS[@]} agent(s)..."

    # Monitor loop: wait for each agent, track exit codes
    local remaining=${#AGENT_PIDS[@]}
    declare -A completed=()

    while [[ $remaining -gt 0 ]]; do
        for i in "${!AGENT_PIDS[@]}"; do
            local pid="${AGENT_PIDS[$i]}"
            [[ -n "${completed[$i]+_}" ]] && continue

            if ! kill -0 "$pid" 2>/dev/null; then
                # Process finished — retrieve exit code
                wait "$pid" 2>/dev/null && local ec=0 || local ec=$?
                AGENT_EXIT_CODES[$i]="$ec"
                completed[$i]=1
                ((remaining--)) || true

                if [[ $ec -eq 0 ]]; then
                    log_info "Agent ${AGENT_IDS[$i]} completed successfully (exit: 0)"
                    ((succeeded++)) || true
                else
                    log_warn "Agent ${AGENT_IDS[$i]} failed (exit: ${ec})"
                    ((failed++)) || true
                fi
            fi
        done

        if [[ $remaining -gt 0 ]]; then
            sleep 3
        fi
    done

    # Collect discoveries
    log_info "--- Discovery Relay Collection ---"
    collect_discoveries

    # Summary
    log_info "========================================="
    log_info "Fleet complete: ${succeeded}/${total} agents succeeded, ${failed} failed"
    log_info "========================================="

    for i in "${!AGENT_IDS[@]}"; do
        local status="SUCCESS"
        if [[ "${AGENT_EXIT_CODES[$i]}" != "0" ]]; then
            status="FAILED (exit: ${AGENT_EXIT_CODES[$i]})"
        fi
        log_info "  ${AGENT_IDS[$i]}: ${status} -- ${AGENT_TASKS[$i]}"
    done

    log_info "Log written to: ${LOG_FILE}"

    # Exit non-zero if any agent failed
    if [[ $failed -gt 0 ]]; then
        exit 1
    fi
}

main "$@"
