#!/usr/bin/env bash
set -euo pipefail

ROOT="${MEMORY_ROOT:-memory}"
STORE="${1:-team_sre}"
DREAMER="${DREAMER:-dreamer}"

if [ -n "${MEMORY_SESSION_ID:-}" ]; then
  META="${ROOT}/sessions/${MEMORY_SESSION_ID}/meta.json"
  if [ -f "${META}" ]; then
    tmp=$(mktemp)
    sed "s/\"ended_at\":null/\"ended_at\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"/" "${META}" > "${tmp}"
    mv "${tmp}" "${META}"
  fi
fi

exec "${DREAMER}" --root "${ROOT}" --store "${STORE}"
