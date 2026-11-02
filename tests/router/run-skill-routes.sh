#!/usr/bin/env bash
# Asks a person would really type, and the skill or tool each must reach. Temp HOME only.
# Run: bash tests/router/run-skill-routes.sh
set -uo pipefail
command -v python3 >/dev/null || { echo "SKIP: python3 required"; exit 0; }
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vajra-skill-routes-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
export HOME="$WORK/home"; mkdir -p "$HOME"

PASS=0; FAIL=0
ctx() {
  printf '/vajra %s' "$1" | jq -cRs '{session_id:"s1",cwd:"/tmp",hook_event_name:"UserPromptSubmit",prompt:.}' \
    | python3 -I "$REPO/scripts/vajra_route.py" | jq -r '.hookSpecificOutput.additionalContext // empty'
}
want() { if ctx "$1" | grep -q -- "$2"; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: '$1' should reach $2"; fi; }
none() { if ctx "$1" | grep -q 'No route matched'; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: '$1' should match no route: $(ctx "$1" | cut -c1-200)"; fi; }

# ctx() adds the /vajra prefix: plain asks reach skills through skill-catalog instead.
want "build me a pricing page for the app" frontend-designer
want "make a dashboard for my metrics" frontend-designer
want "add a settings screen in react" frontend-designer
want "build a landing page" frontend-designer
want "please fix the login bug" systematic-debugging
want "add a three.js globe to the homepage" threejs-fundamentals
want "write a glsl fragment shader for the hero" threejs-shaders
want "the bottom sheet needs better drag physics" apple-design
want "make the swipe gesture feel right" apple-design
want "animate the modal open and close" design-motion-principles
want "add micro-interactions to the form" design-motion-principles
want "build a scroll-driven story page" gsap-scrolltrigger
want "use gsap for the intro" gsap-core
want "suggest a color palette and font pairing" ui-ux-pro-max
want "critique this design, it looks generic" impeccable
want "pick a design direction for the app" impeccable
want "lint our DESIGN.md" design-md-lint
want "start a new website for my bakery" hallmark
want "add a shadcn table" shadcn-mcp
want "run lighthouse on the homepage" chrome-devtools-mcp
want "make a manim worked-math scene" manim-video
want "make an animated chart for a short" video-shotcraft
want "render this in remotion" remotion-video
want "improve my youtube shorts metadata" claude-youtube
want "find viral hooks in my niche" yt-viral
want "write an instagram reel hook" ig-reel
want "add sound effects to the video" earshot
want "try a local tts voice-over" kokoro
want "what are people saying about bun this month" last30days
want "trim this clip with ffmpeg" ffmpeg-skill

# A missing catalog skill gets the pinned install line and its conditions.
c="$(ctx "make the swipe gesture feel right")"
echo "$c" | grep -q 'vajra_skills.py add <name>'; [ $? -eq 0 ] && PASS=$((PASS+1)) || { FAIL=$((FAIL+1)); echo "FAIL :: no install line"; }
echo "$c" | grep -q "Never imitate Apple's look"; [ $? -eq 0 ] && PASS=$((PASS+1)) || { FAIL=$((FAIL+1)); echo "FAIL :: conditions dropped"; }
mkdir -p "$HOME/.claude/skills/apple-design"
c="$(ctx "make the swipe gesture feel right")"
if echo "$c" | grep -q 'vajra_skills.py add'; then FAIL=$((FAIL+1)); echo "FAIL :: installed skill still told to install"; else PASS=$((PASS+1)); fi

# Personal skills never appear in a public route.
if grep -q -E '"(video-tutorial|trip-planner|humanizer)"' "$REPO/config/default.json" "$REPO/config/skills.json"; then FAIL=$((FAIL+1)); echo "FAIL :: personal skill in public routes"; else PASS=$((PASS+1)); fi

# Everyday talk and near misses stay quiet.
none "thanks, that looks right"
none "what time is it in Seattle"
none "summarize this email"
none "commit that and stop"
none "the animal shelter opens at nine"
none "my gesture of thanks"
none "read my instagram"

# Every catalog skill and tool is findable from the skill-catalog table.
for n in $(jq -r '(.skills+.tools)|keys[]' "$REPO/config/skills.json"); do
  if [ "$n" = sentry-skill-scanner ] || grep -q -- "$n" "$REPO/bundled-skills/skill-catalog/SKILL.md"; then PASS=$((PASS+1));
  else FAIL=$((FAIL+1)); echo "FAIL :: skill-catalog does not name $n"; fi
done

# Every routed skill or tool exists somewhere: bundled or in the catalog.
for s in $(jq -r '.routing.patterns[]|.skill//empty' "$REPO/config/default.json" | sort -u); do
  if [ -d "$REPO/bundled-skills/$s" ] || jq -e --arg s "$s" '.skills[$s]' "$REPO/config/skills.json" >/dev/null; then PASS=$((PASS+1));
  else FAIL=$((FAIL+1)); echo "FAIL :: route names unknown skill $s"; fi
done
for t in $(jq -r '.routing.patterns[]|.tool//empty' "$REPO/config/default.json" | sort -u); do
  if jq -e --arg t "$t" '.tools[$t]' "$REPO/config/skills.json" >/dev/null; then PASS=$((PASS+1));
  else FAIL=$((FAIL+1)); echo "FAIL :: route names unknown tool $t"; fi
done

echo "skill routes: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
