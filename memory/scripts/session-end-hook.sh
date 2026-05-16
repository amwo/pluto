#!/usr/bin/env bash
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${MEMORY_ROOT:-${HERE}/..}"
STORE="${MEMORY_STORE:-team_sre}"

payload="$(cat 2>/dev/null || true)"
if command -v jq >/dev/null 2>&1 && [ -n "$payload" ]; then
  sid="$(printf '%s' "$payload" | jq -r '.session_id // empty')"
  meta="${ROOT}/sessions/${sid}/meta.json"
  if [ -n "${sid:-}" ] && [ -f "$meta" ]; then
    tmp="$(mktemp)"
    if jq --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" '.ended_at=$ts' "$meta" > "$tmp"; then
      mv "$tmp" "$meta"
    else
      rm -f "$tmp"
    fi
  fi
fi

if [ -n "${DREAMER:-}" ]; then
  :
elif [ -x "${HERE}/../target/release/dreamer" ]; then
  DREAMER="${HERE}/../target/release/dreamer"
elif [ -x "${HERE}/../target/debug/dreamer" ]; then
  DREAMER="${HERE}/../target/debug/dreamer"
elif command -v cargo >/dev/null 2>&1; then
  cargo build --release --quiet --manifest-path "${HERE}/../Cargo.toml" --bin dreamer >/dev/null 2>&1 || true
  DREAMER="${HERE}/../target/release/dreamer"
else
  DREAMER="dreamer"
fi

if [ ! -x "$DREAMER" ] && ! command -v "$DREAMER" >/dev/null 2>&1; then
  exit 0
fi

since="$(date -u -d '7 days ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || true)"
if [ -n "$since" ]; then
  "$DREAMER" --root "$ROOT" --store "$STORE" --since "$since" --apply || true
else
  "$DREAMER" --root "$ROOT" --store "$STORE" --apply || true
fi

jobs="${ROOT}/dreaming/jobs"
if [ -d "$jobs" ]; then
  ls -1dt "$jobs"/*/ 2>/dev/null | tail -n +21 | xargs -r rm -rf
fi
find "${ROOT}/sessions" -mindepth 1 -maxdepth 1 -type d -mtime +30 -exec rm -rf {} + 2>/dev/null || true
