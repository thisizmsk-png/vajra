#!/usr/bin/env bash
# v0.6 guard cases for hooks/pre-tool-use.sh:
#   1. Text inside heredoc bodies and quoted arguments of plain readers is not
#      judged as commands or paths (the heredoc false positive), while code
#      handed to shells and interpreters still is.
#   2. ~/.claude/vajra/kb/ and campaigns/*/tasks.json* change only through
#      scripts/vajra_kb.py and scripts/vajra_tasks.py; reading stays allowed.
# Temp HOME only. Run: bash tests/hooks/run-guard-text.sh   (exit 0 = all pass)

set -uo pipefail
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-gtx-XXXXXX")"
trap 'chmod -R u+w "$TEST_HOME" 2>/dev/null; rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"
H="$HOME"

SKILL="$H/.claude/skills/vajra"
VS="$H/.claude/vajra"
KB="$VS/kb/0123456789abcdef"
mkdir -p "$SKILL/hooks" "$SKILL/scripts" "$VS/campaigns/demo" "$KB/pending" "$H/.claude/agents"
cp "$REPO/hooks/pre-tool-use.sh" "$SKILL/hooks/"
HOOK="$SKILL/hooks/pre-tool-use.sh"
for s in vajra_kb.py vajra_tasks.py vajra-campaign.sh vajra-verify-work.sh; do echo "#" > "$SKILL/scripts/$s"; done
echo k > "$VS/.hmac-key"
echo '{}' > "$VS/campaigns/demo/tasks.json"
: > "$KB/events.jsonl"
PROJ="$H/proj"; mkdir -p "$PROJ"
SQ="'"
KBPY="python3 ~/.claude/skills/vajra/scripts/vajra_kb.py"
TASKS="python3 ~/.claude/skills/vajra/scripts/vajra_tasks.py"

PASS=0; FAIL=0
judge() { # expect desc rc
  local got=OTHER
  [ "$3" -eq 0 ] && got=ALLOW
  [ "$3" -eq 2 ] && got=BLOCK
  if [ "$got" = "$1" ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); printf 'FAIL  exp=%-5s got=%-5s(rc=%s) :: %s\n' "$1" "$got" "$3" "$2"; fi
}
bash_case() { # expect desc command [cwd]
  local json rc
  json="$(jq -nc --arg c "$3" --arg d "${4:-$PROJ}" '{session_id:"t",cwd:$d,hook_event_name:"PreToolUse",tool_name:"Bash",tool_input:{command:$c}}')"
  printf '%s' "$json" | bash "$HOOK" >/dev/null 2>&1; rc=$?
  judge "$1" "$2" "$rc"
}
file_case() { # expect desc tool path
  local json rc
  json="$(jq -nc --arg t "$3" --arg p "$4" --arg d "$PROJ" '{session_id:"t",cwd:$d,hook_event_name:"PreToolUse",tool_name:$t,tool_input:{file_path:$p,path:$p,content:"x",old_string:"a",new_string:"b",pattern:"x"}}')"
  printf '%s' "$json" | bash "$HOOK" >/dev/null 2>&1; rc=$?
  judge "$1" "$2" "$rc"
}

# --- 1. Heredoc bodies are data for reader sinks ---------------------------
# The reported false positive, verbatim shape.
bash_case ALLOW "cat > file <<'EOF' whose body names the skill dir" "cat > ~/proj/x.md <<'EOF'
# Notes
The installed copy at ~/.claude/skills/vajra is protected; never edit it.
Run: rm -rf ~/.claude/skills/vajra && git push --force   (text only)
EOF"
bash_case ALLOW "unquoted delimiter, body without \$( or backticks" "cat > notes.md <<EOF
see \$HOME/.claude/skills/vajra/hooks; then | tee ~/.claude/agents/x > ~/.claude/settings.json
EOF"
bash_case ALLOW "<<- with tab-indented body and terminator" "cat > notes.md <<-'EOF'
	~/.claude/vajra/kb/ is protected
	EOF
echo done"
bash_case ALLOW "tee with heredoc" "tee notes.md <<'EOF' >/dev/null
~/.claude/vajra/campaigns/demo/tasks.json changes only through vajra_tasks.py
EOF"
bash_case ALLOW "two heredocs in one command" "cat > a.md <<'A' && cat > b.md <<'B'
~/.claude/skills/vajra/SKILL.md
A
~/.claude/agents/arjuna.md
B"
bash_case ALLOW "heredoc body mentioning the HMAC key file name" "cat > SEC.md <<'EOF'
The key lives in ~/.claude/vajra/.hmac-key and is never read by agents.
EOF"
bash_case ALLOW "heredoc then a normal command" "cat > a.md <<'EOF'
~/.claude/skills/vajra
EOF
git status"

# Real targets are still judged.
bash_case BLOCK "heredoc into the skill dir"          "cat > ~/.claude/skills/vajra/x.md <<'EOF'
hello
EOF"
bash_case BLOCK "heredoc appended to the kb log"      "cat >> ~/.claude/vajra/kb/0123456789abcdef/events.jsonl <<'EOF'
{}
EOF"
bash_case BLOCK "tee heredoc into tasks.json"         "tee ~/.claude/vajra/campaigns/demo/tasks.json <<'EOF'
{}
EOF"
bash_case BLOCK "command after the heredoc still checked" "cat > a.md <<'EOF'
text
EOF
rm ~/.claude/skills/vajra/SKILL.md"

# Bodies that are code, or flow into code, are still analyzed.
bash_case BLOCK "heredoc piped into sh"               "cat <<'EOF' | sh
rm -rf ~/.claude/skills/vajra
EOF"
bash_case BLOCK "heredoc piped with |& into bash"     "cat <<'EOF' |& bash
rm ~/.claude/skills/vajra/SKILL.md
EOF"
bash_case BLOCK "heredoc names piped into xargs rm"   "cat <<'EOF' | xargs rm
$H/.claude/skills/vajra/SKILL.md
EOF"
bash_case BLOCK "heredoc fed to bash"                 "bash <<'EOF'
rm ~/.claude/skills/vajra/SKILL.md
EOF"
bash_case BLOCK "heredoc fed to python3 -"            "python3 - <<'EOF'
open('$H/.claude/skills/vajra/SKILL.md', 'w').write('x')
EOF"
bash_case BLOCK "unquoted heredoc with \$( in body"   "cat > x.md <<EOF
\$(rm ~/.claude/skills/vajra/SKILL.md)
EOF"
bash_case BLOCK "unquoted heredoc with backticks"     "cat > x.md <<EOF
\`rm ~/.claude/skills/vajra/SKILL.md\`
EOF"
bash_case BLOCK "heredoc into process substitution"   "cat <<'EOF' > >(sh)
rm ~/.claude/skills/vajra/SKILL.md
EOF"
bash_case BLOCK "redirect before command (no sink)"   "<<'EOF' bash
rm ~/.claude/skills/vajra/SKILL.md
EOF"
bash_case BLOCK "\$'..' quoting falls back to strict" "echo \$'a\\${SQ} <<EOF${SQ}
rm ~/.claude/skills/vajra/SKILL.md
EOF"
bash_case BLOCK "apostrophe inside a comment"         "cat x # it${SQ}s
rm ~/.claude/skills/vajra/SKILL.md # ${SQ}"
bash_case BLOCK "odd delimiter falls back to strict"  "cat <<E\"O\"F
rm ~/.claude/skills/vajra/SKILL.md
EOF"

# --- 2. Quoted arguments of plain readers ----------------------------------
bash_case ALLOW "echo quoted text with ; | > into a file" 'echo "see ~/.claude/skills/vajra; then | tee x > y" > notes.md'
bash_case ALLOW "printf single-quoted redirect text"      "printf '%s\n' 'a > ~/.claude/skills/vajra/x' > notes.md"
bash_case ALLOW "grep for a protected path pattern"       'grep -n "~/.claude/vajra/kb; x" README.md'
bash_case BLOCK "echo text piped into sh"                 "echo 'rm ~/.claude/skills/vajra/SKILL.md' | sh"
bash_case BLOCK "bash -c string is code"                  "bash -c 'echo x; rm ~/.claude/skills/vajra/SKILL.md'"
bash_case BLOCK "sh -c double-quoted string is code"      'sh -c "cat a | tee ~/.claude/skills/vajra/x"'
bash_case BLOCK "eval string is code"                     "eval 'rm ~/.claude/skills/vajra/SKILL.md'"
bash_case BLOCK "command substitution inside quotes"      'echo "$(rm ~/.claude/skills/vajra/SKILL.md)"'
bash_case BLOCK "redirect target is not a command word"   ">echo bash -c 'rm ~/.claude/skills/vajra/SKILL.md'"
bash_case BLOCK ">& does not start a new command"         "bash -c >&echo 'rm ~/.claude/skills/vajra/SKILL.md'"
bash_case BLOCK "quoted redirect target still checked"    'echo hi > "$HOME/.claude/skills/vajra/x"'

# The key's file name as text vs. as a target.
bash_case ALLOW "echo naming the key file into a temp file" "echo \"never read .hmac-key directly\" > $TEST_HOME/tmpnote.txt"
bash_case ALLOW "printf naming the key file"               "printf '%s\n' 'key: ~/.claude/vajra/.hmac-key' > notes.md"
bash_case BLOCK "cat the key"                              'cat ~/.claude/vajra/.hmac-key'
bash_case BLOCK "cat the key (quoted path)"                "cat '$H/.claude/vajra/.hmac-key'"
bash_case BLOCK "key name echoed into xargs"               "echo '.hmac-key' | xargs -I{} cat ~/.claude/vajra/{}"
bash_case BLOCK "key name echoed to a file then xargs"     "echo '.hmac-key' > f; xargs -I{} cat ~/.claude/vajra/{} < f"
bash_case BLOCK "grep the key"                             "grep -c . '$H/.claude/vajra/.hmac-key'"

# --- 3. Commit messages ------------------------------------------------------
bash_case ALLOW "git commit -m naming a protected path"   'git commit -m "docs: ~/.claude/skills/vajra is protected; see SECURITY.md"'
bash_case ALLOW "git commit heredoc message"              "git add -A && git commit -m \"\$(cat <<'EOF'
guard: judge real write targets

Text like ~/.claude/skills/vajra; rm -rf / | sh inside the message is data.
EOF
)\""
bash_case BLOCK "commit message with \$(rm ...)"          'git commit -m "$(rm ~/.claude/skills/vajra/SKILL.md)"'
bash_case BLOCK "command after a commit heredoc"          "git commit -m \"\$(cat <<'EOF'
msg
EOF
)\" && rm ~/.claude/skills/vajra/SKILL.md"
bash_case BLOCK "git commit in the skill dir"             'git -C ~/.claude/skills/vajra commit -m "x"'

# --- 4. Knowledge ledger and task graphs -------------------------------------
bash_case ALLOW "kb script add with text naming paths"   "$KBPY add --type fact --title \"guard\" --body \"protects ~/.claude/vajra/kb and ~/.claude/skills/vajra/hooks\" --source proposed"
bash_case ALLOW "kb script add --source verified"         "$KBPY add --type delivery --title x --body y --source verified --evidence data/walkthroughs/x/evidence.json"
bash_case ALLOW "kb script pending/show"                  "$KBPY pending --json && $KBPY show kb-abc123"
bash_case ALLOW "kb script search"                        "$KBPY search \"~/.claude/vajra/kb layout\" --format ids --k 3"
bash_case ALLOW "kb script from inside the kb dir"        "$KBPY doctor" "$KB"
bash_case ALLOW "tasks script"                            "$TASKS ready"
bash_case ALLOW "tasks script status"                     "$TASKS status t1"
bash_case ALLOW "cat kb events"                           'cat ~/.claude/vajra/kb/0123456789abcdef/events.jsonl'
bash_case ALLOW "grep -r over kb"                         'grep -r decision ~/.claude/vajra/kb'
bash_case ALLOW "cat tasks.json"                          'cat ~/.claude/vajra/campaigns/demo/tasks.json | jq .'
file_case ALLOW "Read kb file"                            Read "$KB/events.jsonl"
file_case ALLOW "Read tasks.json"                         Read "$VS/campaigns/demo/tasks.json"
file_case ALLOW "Grep kb dir"                             Grep "$VS/kb"

file_case BLOCK "Write kb events"                         Write "$KB/events.jsonl"
file_case BLOCK "Edit kb INDEX.md"                        Edit "$KB/INDEX.md"
file_case BLOCK "Write kb pending record"                 Write "$KB/pending/kb-abc123.json"
file_case BLOCK "Write tasks.json"                        Write "$VS/campaigns/demo/tasks.json"
file_case BLOCK "Edit tasks.json"                         Edit "$VS/campaigns/demo/tasks.json"
bash_case BLOCK "append to kb events"                     'echo "{}" >> ~/.claude/vajra/kb/0123456789abcdef/events.jsonl'
bash_case BLOCK "rm kb.sqlite"                            'rm ~/.claude/vajra/kb/0123456789abcdef/kb.sqlite'
bash_case BLOCK "rm -r kb"                                'rm -r ~/.claude/vajra/kb'
bash_case BLOCK "sqlite3 on kb.sqlite"                    "sqlite3 ~/.claude/vajra/kb/0123456789abcdef/kb.sqlite 'delete from records'"
bash_case BLOCK "mv a pending record"                     'mv ~/.claude/vajra/kb/0123456789abcdef/pending/kb-1.json /tmp/'
bash_case BLOCK "cp over tasks.json"                      'cp /tmp/t.json ~/.claude/vajra/campaigns/demo/tasks.json'
bash_case BLOCK "sed -i tasks.json"                       "sed -i '' 's/pending/done/' ~/.claude/vajra/campaigns/demo/tasks.json"
bash_case BLOCK "jq rewrite of tasks.json via tee"        'jq ".x=1" ~/.claude/vajra/campaigns/demo/tasks.json | tee ~/.claude/vajra/campaigns/demo/tasks.json'
bash_case BLOCK "take the tasks lock by hand"             'mkdir ~/.claude/vajra/campaigns/demo/tasks.json.lock'
bash_case BLOCK "python writes kb directly"               "python3 -c \"open('$KB/events.jsonl','a').write('x')\""
bash_case BLOCK "kb script output flag into skill dir"    "$KBPY export --out ~/.claude/skills/vajra/x.json"
bash_case BLOCK "kb script output flag (=) into kb"       "$KBPY export --output=$KB/events.jsonl"
bash_case BLOCK "kb script stdout redirected into kb"     "$KBPY export > ~/.claude/vajra/kb/0123456789abcdef/events.jsonl"
bash_case BLOCK "kb approve is the user's (TTY)"         "$KBPY approve kb-abc123"
bash_case BLOCK "kb reject is the user's"                 "$KBPY --json reject kb-abc123"
bash_case BLOCK "kb forget is the user's"                 "$KBPY forget kb-abc123"
bash_case BLOCK "kb approve from a dev clone too"         "python3 ~/proj/scripts/vajra_kb.py approve kb-abc123"
bash_case BLOCK "kb add --source user from a tool call"   "$KBPY add --type fact --title x --body y --source user"
bash_case BLOCK "kb add --source=user"                    "$KBPY add --type fact --title x --body y --source=user"
bash_case BLOCK "kb add --source \"user\" (quoted)"       "$KBPY add --title x --source \"user\" --body y"
bash_case BLOCK "kb test override env var"                "VAJRA_ATMAN_ASSUME_HUMAN=1 $KBPY approve kb-abc123"
bash_case BLOCK "test override set inside a heredoc body" "cat > run.sh <<'EOF'
export VAJRA_ATMAN_ASSUME_HUMAN=1
EOF
bash run.sh"
bash_case BLOCK "kb script never takes the key"           "$KBPY add --evidence ~/.claude/vajra/.hmac-key"
bash_case BLOCK "verify-work args are still checked"      "bash ~/.claude/skills/vajra/scripts/vajra-verify-work.sh x \"rm ~/.claude/skills/vajra/SKILL.md\""
bash_case BLOCK "non-reader in kb dir"                    'touch x' "$KB"

# --- 4b. Commands the deliver workflow's agents run (stream T) ---------------
bash_case ALLOW "env-prefixed verify-work"                "VAJRA_WALKTHROUGH_DIR=data/walkthroughs bash ~/.claude/skills/vajra/scripts/vajra-verify-work.sh deliver-1 \"npm test\" \"npm run build\""
bash_case ALLOW "two env prefixes before verify-work"     "CI=1 VAJRA_WALKTHROUGH_DIR=data/walkthroughs bash ~/.claude/skills/vajra/scripts/vajra-verify-work.sh x \"npm test\""
bash_case BLOCK "env prefix pointing into the skill dir"  "VAJRA_WALKTHROUGH_DIR=~/.claude/skills/vajra/x bash ~/.claude/skills/vajra/scripts/vajra-verify-work.sh x \"npm test\""
bash_case ALLOW "tasks add --campaign"                    "$TASKS add --campaign deliver-20261011-0930 --id t1 --title \"Add CSV export\" --owner bhima --files \"src/export/**\" --acceptance \"exports ~/.claude-free CSV\""
bash_case ALLOW "tasks validate/waves/graph --campaign"   "$TASKS validate --campaign deliver-1 && $TASKS waves --campaign deliver-1 && $TASKS graph --campaign deliver-1"
bash_case ALLOW "tasks claim/report/close --campaign"     "$TASKS claim t1 w1 --campaign deliver-1 && $TASKS report t1 done --summary \"ok; tests pass\" --campaign deliver-1 && $TASKS close t1 --evidence data/walkthroughs/x/evidence.json --campaign deliver-1"
bash_case ALLOW "tasks status in the campaign dir"        "$TASKS status t1 --campaign demo" "$VS/campaigns/demo"
bash_case BLOCK "direct write to tasks.json stays blocked" "echo '{}' > ~/.claude/vajra/campaigns/deliver-1/tasks.json"
bash_case ALLOW "kb search with the ask (deliver understand)" "$KBPY search \"add a CSV export; it must stream | not buffer\" --format ids --k 5"

# --- 4c. Review fixes (fixer G) ----------------------------------------------
# G1: the key-name masking in echo/printf text never applies to a redirect
# target or to anything after > >> < or |.
bash_case BLOCK "echo hex > \"\$HOME/.../.hmac-key\""         'echo "deadbeef" > "$HOME/.claude/vajra/.hmac-key"'
bash_case BLOCK "printf hex > \"\$HOME/.../.hmac-key\""       'printf "deadbeef" > "$HOME/.claude/vajra/.hmac-key"'
bash_case BLOCK "echo hex >> key (append)"                   'echo "deadbeef" >> "$HOME/.claude/vajra/.hmac-key"'
bash_case BLOCK "printf >> key (append)"                     "printf '%s' deadbeef >> \"\$HOME/.claude/vajra/.hmac-key\""
bash_case BLOCK "echo > key, single-quoted target"           "echo 'deadbeef' > '$H/.claude/vajra/.hmac-key'"
bash_case BLOCK "printf > key, single-quoted target"         "printf 'deadbeef' > '$H/.claude/vajra/.hmac-key'"
bash_case BLOCK "echo > .HMAC-KEY (case)"                    'echo "deadbeef" > "$HOME/.claude/vajra/.HMAC-KEY"'
bash_case BLOCK "printf > .Hmac-Key (case, single quotes)"   "printf x > '$H/.claude/vajra/.Hmac-Key'"
bash_case BLOCK "echo > .hmac-key.tmp"                       'echo "deadbeef" > "$HOME/.claude/vajra/.hmac-key.tmp"'
bash_case BLOCK "printf > .hmac-key.tmp then mv"             "printf 'deadbeef' > '$H/.claude/vajra/.hmac-key.tmp' && mv '$H/.claude/vajra/.hmac-key.tmp' '$H/.claude/vajra/.hmac-key'"
bash_case BLOCK "echo >\"dir/\".hmac-key (quote inside the word)" 'echo deadbeef >"$HOME/.claude/vajra/".hmac-key'
bash_case BLOCK "echo >'...key' with no space"               "echo deadbeef >'$H/.claude/vajra/.hmac-key'"
bash_case BLOCK "echo 1>> key"                               'echo deadbeef 1>>"$HOME/.claude/vajra/.hmac-key"'
bash_case BLOCK "echo &> key"                                'echo deadbeef &> "$HOME/.claude/vajra/.hmac-key"'
bash_case BLOCK "echo >| key (noclobber override)"           'echo deadbeef >| "$HOME/.claude/vajra/.hmac-key"'
bash_case BLOCK "word after > (echo > f \"key\")"            'echo > out.txt "$HOME/.claude/vajra/.hmac-key"'
bash_case BLOCK "printf < key"                               'printf "%s" < "$HOME/.claude/vajra/.hmac-key"'
bash_case BLOCK "echo | tee key"                             'echo deadbeef | tee "$HOME/.claude/vajra/.hmac-key"'
bash_case BLOCK "echo | tee key (relative, in key dir)"      "echo deadbeef | tee '.hmac-key'" "$VS"
bash_case BLOCK "echo > relative key in key dir"             "echo deadbeef > '.hmac-key'" "$VS"
bash_case ALLOW "echo naming the key, redirected elsewhere"  'echo "never read .hmac-key directly" > notes.md'
bash_case ALLOW "printf naming the key, appended elsewhere"  "printf '%s\n' 'the .HMAC-KEY stays private' >> notes.md"

# G2: any argparse abbreviation of --source (vajra_kb.py also checks a token).
for opt in --s --so --sou --sour --sourc --source --SOURCE; do
  bash_case BLOCK "kb add $opt user"                         "$KBPY add --type fact --title x --body y $opt user"
  bash_case BLOCK "kb add $opt=user"                         "$KBPY add --type fact --title x --body y $opt=user"
done
bash_case BLOCK "kb add --so \"user\" (quoted value)"        "$KBPY add --title x --so \"user\" --body y"
bash_case BLOCK "kb add --sou='User'"                        "$KBPY add --title x --sou='User' --body y"
bash_case ALLOW "kb add --so proposed"                       "$KBPY add --type fact --title x --body y --so proposed --session abcdef12"
bash_case ALLOW "kb add --session user-like id"              "$KBPY add --type fact --title x --body y --source proposed --session user"
bash_case ALLOW "kb search for the word user"                "$KBPY search \"--source user\" --format ids"

# G3: false positives on Vajra's own generated commands.
bash_case ALLOW "kb add whose quoted text has ; | > ( )"     "$KBPY add --type lesson --title \"guard: rm -rf ~/.claude/skills/vajra is blocked; good\" --body \"a | b > ~/.claude/agents/x (and) & more\" --source proposed --session abcdef12"
bash_case ALLOW "kb add with single-quoted body"             "$KBPY add --type fact --title 'x' --body 'see ~/.claude/vajra/kb; then > ~/.claude/settings.json' --source proposed --session abcdef12"
bash_case ALLOW "tasks report with ; | > in the summary"     "$TASKS report t1 done --summary \"edited ~/.claude/skills/vajra/x; ok | then > y (done)\" --campaign demo"
bash_case ALLOW "bash vajra-campaign.sh progress text"      "bash ~/.claude/skills/vajra/scripts/vajra-campaign.sh progress \"wave 2: touched ~/.claude/agents; (ok) > next\" --campaign demo"
bash_case ALLOW "vajra-campaign.sh pass --evidence text"    "bash ~/.claude/skills/vajra/scripts/vajra-campaign.sh pass f1 --evidence \"npm test | tail; ~/.claude/vajra/kb ok\" --campaign demo"
bash_case ALLOW "git commit -qm naming protected paths"     'git commit -qm "guard: ~/.claude/skills/vajra; rm -rf / | sh is text"'
bash_case ALLOW "git commit -am naming protected paths"     'git commit -am "edit ~/.claude/agents/x > y; done"'
bash_case BLOCK "kb add stdout redirected into kb"           "$KBPY add --title \"x; y\" --body z --source proposed > ~/.claude/vajra/kb/0123456789abcdef/events.jsonl"
bash_case BLOCK "kb add then a real rm"                      "$KBPY add --title \"x\" --body y --source proposed; rm -rf ~/.claude/skills/vajra"
bash_case BLOCK "kb add --out into the skill dir (quoted)"   "$KBPY export --out \"~/.claude/skills/vajra/x.json\""
bash_case BLOCK "python -c before the kb script name"        "python3 -c \"import os; os.remove('$H/.claude/skills/vajra/SKILL.md')\" vajra_kb.py"
bash_case BLOCK "git commit -qm then a real rm"              'git commit -qm "x" && rm ~/.claude/skills/vajra/SKILL.md'

# G5: vajra_tasks.py's lock directory.
file_case BLOCK "Write campaigns/*/.tasks.lock"              Write "$VS/campaigns/demo/.tasks.lock"
file_case BLOCK "Edit campaigns/*/.tasks.lock/owner"         Edit "$VS/campaigns/demo/.tasks.lock/owner"
file_case BLOCK "Write .tasks.lock in a new campaign"        Write "$VS/campaigns/deliver-9/.tasks.lock"
bash_case BLOCK "mkdir the tasks lock by hand"               'mkdir ~/.claude/vajra/campaigns/demo/.tasks.lock'
bash_case BLOCK "mkdir -p the tasks lock"                    'mkdir -p "$HOME/.claude/vajra/campaigns/deliver-9/.tasks.lock"'
bash_case BLOCK "rmdir the tasks lock"                       'rmdir ~/.claude/vajra/campaigns/demo/.tasks.lock'
bash_case BLOCK "mkdir the lock from the campaign dir"       'mkdir .tasks.lock' "$VS/campaigns/demo"
bash_case ALLOW "ls the tasks lock"                          'ls -la ~/.claude/vajra/campaigns/demo/.tasks.lock'

# G4: fast glob path (no / or ~, cwd outside ~/.claude) still sees symlinks.
LNK="$H/lnkproj"; mkdir -p "$LNK"; ln -s "$SKILL" "$LNK/skilllink"
bash_case BLOCK "glob matching a symlink to the skill dir"   'rm -r skill*' "$LNK"
DOTL="$H/dotproj"; mkdir -p "$DOTL"; ln -s "$H/.claude/agents" "$DOTL/.hidden"
bash_case BLOCK "dot-glob matching a dot symlink"            'rm -r .hid*' "$DOTL"
touch "$PROJ/a1" "$PROJ/a2"
bash_case ALLOW "glob in a cwd with no symlinks"             'touch a* [ab]1 ?2' "$PROJ"

# --- 5. Latency ------------------------------------------------------------
now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }
BIG="cat > notes.md <<'EOF'
$(for i in $(seq 200); do echo "line $i names ~/.claude/skills/vajra; and | other > text"; done)
EOF"
lat() { # label command runs -> prints ms
  local json start end
  json="$(jq -nc --arg c "$2" --arg d "$PROJ" '{cwd:$d,tool_name:"Bash",tool_input:{command:$c}}')"
  start="$(now_ms)"
  for _ in $(seq "$3"); do printf '%s' "$json" | bash "$HOOK" >/dev/null 2>&1; done
  end="$(now_ms)"
  echo $(( (end - start) / $3 ))
}
MS_PLAIN="$(lat plain 'git status && npm test 2>&1 | tail -n 20' 20)"
MS_QUOTE="$(lat quoted 'git commit -m "wip: x; y" && npm test' 20)"
MS_BIG="$(lat heredoc "$BIG" 10)"
echo "guard latency: plain ${MS_PLAIN} ms, quoted ${MS_QUOTE} ms, 200-line heredoc ${MS_BIG} ms"
bash_case ALLOW "200-line heredoc naming protected paths" "$BIG"
LIMIT="${VAJRA_LATENCY_LIMIT_MS:-150}"
[ "$MS_QUOTE" -lt "$LIMIT" ]; judge ALLOW "quoted command under ${LIMIT} ms" $?
[ "$MS_BIG" -lt $((LIMIT * 2)) ]; judge ALLOW "200-line heredoc under $((LIMIT * 2)) ms" $?

# Bounded cost: the hook has a 10 s timeout and onFailure "block", so a slow
# guard blocks the call. Big inputs must be decided in well under that.
big_case() { # expect desc command max_ms
  local start end ms rc
  # Through files: a 1 MB command does not fit in an argument list.
  printf '%s' "$3" > "$TEST_HOME/big.cmd"
  jq -nc --rawfile c "$TEST_HOME/big.cmd" --arg d "$PROJ" '{cwd:$d,tool_name:"Bash",tool_input:{command:$c}}' > "$TEST_HOME/big.json" \
    || { judge "$1" "$2 (payload could not be built)" 99; return; }
  start="$(now_ms)"; bash "$HOOK" < "$TEST_HOME/big.json" >/dev/null 2>"$TEST_HOME/big.err"; rc=$?; end="$(now_ms)"
  # A block must be the guard's decision, not a payload or internal error.
  if [ "$rc" = 2 ] && grep -q "internal error\|unparseable" "$TEST_HOME/big.err"; then rc=98; fi
  ms=$((end - start))
  echo "  $2: ${ms} ms"
  judge "$1" "$2" "$rc"
  [ "$ms" -lt "$4" ]; judge ALLOW "$2 under $4 ms" $?
}
body200() { python3 -c 'import sys
print("\n".join(sys.argv[1] % i for i in range(int(sys.argv[2]))))' "$1" "$2"; }
B1="$(body200 'line %d names ~/.claude/skills/vajra; and | x > y and some more words' 3200)"
big_case ALLOW "~200 KB cat heredoc (inert, skipped)" "cat > notes.md <<'EOF'
$B1
EOF" 3000
B2="$(body200 'x_%d = compute(a, b)  # words here and more words to pad it out' 3200)"
big_case BLOCK "~200 KB python heredoc (too large, fast block)" "python3 - <<'EOF'
$B2
EOF" 3000
B3="$(body200 'x_%d = 1  # w' 1500)"
big_case BLOCK "~20 KB python heredoc (over the token cap, fast block)" "python3 - <<'EOF'
$B3
EOF" 3000
B4="$(body200 'rm ~/.claude/skills/vajra/f%d' 1000)"
big_case BLOCK "~30 KB bash heredoc with protected rm" "bash <<'EOF'
$B4
EOF" 3000

# Review-fix bounds (G4): every case is decided in under 3 s.
rep() { python3 -c 'import sys
s, n = sys.argv[1], int(sys.argv[2])
print((s * (n // len(s) + 1))[:n], end="")' "$1" "$2"; }
P4K="$(rep '(a),[b] ' 4080)"
big_case ALLOW "4,093-char python3 -c \"(a),[b] ...\"" "python3 -c \"$P4K\"" 3000
P31K="$(rep '(a),[b] ' 31000)"
big_case BLOCK "31 KB python3 -c \"(a),[b] ...\" (token cap)" "python3 -c \"$P31K\"" 3000
PYH="$(body200 'x_%d = f(a, [b])' 250 | head -c 4000)"
big_case ALLOW "4 KB python heredoc" "python3 - <<'EOF'
$PYH
EOF" 3000
NODE4K="$(rep 'console.log([1,2],(3)); ' 4000)"
big_case ALLOW "node -e at 4 KB" "node -e \"$NODE4K\"" 3000
CAT512="$(rep 'line names ~/.claude/skills/vajra; | x > y
' $((512 * 1024 - 40)))"
big_case ALLOW "512 KB cat heredoc (inert, skipped)" "cat > notes.md <<'EOF'
$CAT512
EOF" 3000
big_case BLOCK "over 512 KB raw (fast block)" "cat > notes.md <<'EOF'
${CAT512}${CAT512}
EOF" 3000

echo "----------------------------------------"
echo "guard text/kb: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
