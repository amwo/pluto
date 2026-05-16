#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${MEMORY_ROOT:-${HERE}/..}"
EVENT="${1:?usage: transcript-hook.sh <event>}"

command -v jq >/dev/null 2>&1 || exit 0

payload="$(cat)"
sid="$(printf '%s' "$payload" | jq -r '.session_id // empty')"
[ -n "$sid" ] || exit 0

sdir="${ROOT}/sessions/${sid}"
transcript="${sdir}/transcript.jsonl"
now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

append() {
  mkdir -p "$sdir"
  ( flock 9; printf '%s\n' "$1" >&9 ) 9>>"$transcript"
}

case "$EVENT" in
  SessionStart)
    mkdir -p "$sdir"
    if [ ! -f "${sdir}/meta.json" ]; then
      jq -n --arg id "$sid" --arg ts "$now" \
        '{session_id:$id,agent_id:"claude-code",started_at:$ts,ended_at:null,store_refs:["team_sre"]}' \
        > "${sdir}/meta.json"
    fi
    : >> "$transcript"
    ;;
  UserPromptSubmit)
    text="$(printf '%s' "$payload" | jq -r '.prompt // .user_prompt // ""')"
    append "$(jq -cn --arg t "$text" '{kind:"message",role:"user",text:$t}')"
    ;;
  PostToolUse)
    append "$(printf '%s' "$payload" | jq -c \
      '{kind:"tool_call",tool:(.tool_name // "unknown"),input:((.tool_input // {})|tostring),status:"ok",output:""}')"
    ;;
  PostToolUseFailure)
    append "$(printf '%s' "$payload" | jq -c \
      '{kind:"tool_call",tool:(.tool_name // "unknown"),input:((.tool_input // {})|tostring),status:"error",output:((.tool_response // {})|tostring)}')"
    ;;
esac
