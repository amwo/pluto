#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${MEMORY_ROOT:-${HERE}/..}"
STORE="${1:-team_sre}"

if [ -n "${DREAMER:-}" ]; then
  :
elif [ -x "${HERE}/../target/release/dreamer" ]; then
  DREAMER="${HERE}/../target/release/dreamer"
elif [ -x "${HERE}/../target/debug/dreamer" ]; then
  DREAMER="${HERE}/../target/debug/dreamer"
else
  DREAMER="dreamer"
fi

if [ -n "${MEMORY_SESSION_ID:-}" ]; then
  META="${ROOT}/sessions/${MEMORY_SESSION_ID}/meta.json"
  if [ -f "${META}" ]; then
    tmp=$(mktemp)
    sed "s/\"ended_at\":null/\"ended_at\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"/" "${META}" > "${tmp}"
    mv "${tmp}" "${META}"
  fi
fi

exec "${DREAMER}" --root "${ROOT}" --store "${STORE}"
