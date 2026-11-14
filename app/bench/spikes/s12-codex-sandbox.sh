#!/usr/bin/env bash
# Spike S12 (bhishma MUST for the plain Codex Lead, codex 0.166): does the user's own Codex config, or an explicit
# sandbox flag, beat a per-launch permission profile? Each case runs once against the loopback stub (s2s5_stub.py) in a
# temp CODEX_HOME; the model asks for one shell call that reads a file in a denied "state" folder. Prints the session
# file's turn_context and what the read returned. Nothing leaves 127.0.0.1: the provider is set before any user table,
# and a case that reaches the real provider fails. No key: none is set, and the stub ignores auth.
#
#   bash app/bench/spikes/s12-codex-sandbox.sh            # every case, codex exec and the interactive Codex
#
# Result on codex 0.166.0 (2026-11-14), pinned in the profiles tests (`the_lead_read_back_*`):
# - profile only, user top-level sandbox_mode = "danger-full-access", a [profiles.p] table with it, user
#   sandbox_workspace_write.writable_roots naming the state folder, user default_permissions + [permissions.mine] with
#   "/" = "write": sandbox_policy workspace-write, the state folder "deny", the read "Operation not permitted";
# - a legacy top-level `profile = "p"`: codex refuses to start;
# - --sandbox danger-full-access, -s danger-full-access, --sandbox=danger-full-access: sandbox_policy
#   danger-full-access, file_system_sandbox_policy null, the file read;
# - the interactive Codex (the plain Lead's form) shows the same turn_context as codex exec for the profile, the user
#   sandbox_mode and -s. Its shell tool has another name, so the stub's read prints "unsupported call" there; the
#   turn_context is the evidence for that form.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
STUB="$HERE/s2s5_stub.py"; PTY="$HERE/s12_pty.py"
for f in "$STUB" "$PTY"; do [ -f "$f" ] || { echo "FAIL: $f is missing"; exit 2; }; done
command -v codex >/dev/null || { echo "SKIP: codex is not installed"; exit 0; }
R="$(mktemp -d "${TMPDIR:-/tmp}/vajra-s12-XXXXXX")"; R="$(cd "$R" && pwd -P)"
trap 'kill "${SP:-0}" 2>/dev/null; rm -rf "$R"' EXIT
STATE="$R/state"; WT="$R/wt"; mkdir -p "$STATE" "$WT"
echo "PLACEHOLDER-NOT-A-KEY" > "$STATE/key.txt"
( cd "$WT" && git init -q ) >/dev/null 2>&1
printf '[{"shell": "cat %s/key.txt; echo rc=$?"}, {"say": "done"}]' "$STATE" > "$R/plan.json"
/usr/bin/python3 -I "$STUB" "$R/plan.json" "$R/stub.log" "$R/port" > "$R/stub.out" 2>&1 &
SP=$!
for _ in $(seq 1 50); do [ -s "$R/port" ] && break; sleep 0.1; done
PORT="$(cat "$R/port" 2>/dev/null)"; [ -n "$PORT" ] || { echo "FAIL: the stub did not start"; exit 2; }
BIN="$(command -v codex)"
PROF=vajra-0123456789abcdef
PERM=(-c "permissions.$PROF.filesystem={\"/\"=\"read\",\"$STATE\"=\"none\",\"$WT\"=\"write\"}"
      -c "permissions.$PROF.network={enabled=false}" -c "default_permissions=\"$PROF\"" -c 'approval_policy="never"')
BAD=0

home_for() { # name extra-toml -> prints the CODEX_HOME made for the case
  local ch="$R/home-$1"; mkdir -p "$ch"
  {
    printf 'model_provider = "stub"\n%s\n' "$2"
    printf '\n[model_providers.stub]\nname = "Stub"\nbase_url = "http://127.0.0.1:%s/v1"\nwire_api = "responses"\nrequest_max_retries = 0\nstream_max_retries = 0\n' "$PORT"
    printf '\n[projects."%s"]\ntrust_level = "trusted"\n' "$WT"
  } > "$ch/config.toml"
  printf '%s' "$ch"
}

report() { # name codex-home out-file
  local f
  if grep -q 'api.openai.com' "$3" 2>/dev/null; then echo "   FAIL: reached the real provider"; BAD=1; fi
  f="$(find "$2/sessions" -name 'rollout-*.jsonl' 2>/dev/null | head -1)"
  if [ -n "$f" ]; then
    jq -c 'select(.type=="turn_context") | .payload | {sandbox_policy, file_system_sandbox_policy}' "$f" | head -1 | sed 's/^/   turn_context: /'
  else
    echo "   no session file: $(grep -v 'Reading additional' "$3" 2>/dev/null | head -c 200 | tr '\n' ' ')"
  fi
  printf '   the denied file: %s\n' "$(jq -r 'select(.k==1) | .results[0].text' "$R/stub.log" 2>/dev/null | grep -oE 'Operation not permitted|PLACEHOLDER-NOT-A-KEY|unsupported call' | head -1)"
}

exec_case() { # name extra-toml [flag...]
  local name="$1" extra="$2" ch; shift 2
  ch="$(home_for "exec-$name" "$extra")"; : > "$R/stub.log"
  ( cd "$WT" && CODEX_HOME="$ch" perl -e 'alarm 60; exec @ARGV' "$BIN" exec --json --skip-git-repo-check -m stub-model \
      ${@+"$@"} "${PERM[@]}" -- "go" < /dev/null > "$R/$name.out" 2>&1 )
  echo "== exec $name ${*:-} ${extra:+(user config: $(printf '%s' "$extra" | tr '\n' ' '))}"
  report "$name" "$ch" "$R/$name.out"
}

tui_case() { # name extra-toml [flag...]: the plain Lead's own form, the interactive Codex, in a pty
  local name="$1" extra="$2" ch; shift 2
  ch="$(home_for "tui-$name" "$extra")"; : > "$R/stub.log"
  ( cd "$WT" && CODEX_HOME="$ch" TERM=xterm-256color /usr/bin/python3 -I "$PTY" 40 "$R/stub.log" '"k": 1' -- \
      "$BIN" ${@+"$@"} "${PERM[@]}" -- "go" > "$R/tui-$name.out" 2>&1 )
  echo "== interactive $name ${*:-} ${extra:+(user config: $(printf '%s' "$extra" | tr '\n' ' '))}"
  report "tui-$name" "$ch" "$R/tui-$name.out"
}

exec_case profile ""
exec_case user_sandbox_mode 'sandbox_mode = "danger-full-access"'
exec_case user_profile_table '[profiles.p]
sandbox_mode = "danger-full-access"'
exec_case user_legacy_profile_key 'profile = "p"
[profiles.p]
sandbox_mode = "danger-full-access"'
exec_case user_writable_roots "sandbox_mode = \"workspace-write\"
[sandbox_workspace_write]
writable_roots = [\"$STATE\"]"
exec_case user_permissions "default_permissions = \"mine\"
[permissions.mine]
filesystem = { \"/\" = \"write\" }"
exec_case flag_long "" --sandbox danger-full-access
exec_case flag_short "" -s danger-full-access
exec_case flag_equals "" --sandbox=danger-full-access
tui_case profile ""
tui_case user_sandbox_mode 'sandbox_mode = "danger-full-access"'
tui_case flag_short "" -s danger-full-access
exit "$BAD"
