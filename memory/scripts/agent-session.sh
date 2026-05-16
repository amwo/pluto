#!/usr/bin/env bash
set -euo pipefail

ROOT="${MEMORY_ROOT:-memory}"
AGENT="${1:?usage: agent-session.sh <agent_id> [store_refs csv]}"
SESSION="${AGENT}-$(date -u +%Y%m%dT%H%M%SZ)"
STORE_REFS="${2:-team_sre}"

SESSION_DIR="${ROOT}/sessions/${SESSION}"
mkdir -p "${SESSION_DIR}"

REFS_JSON=$(printf '"%s",' ${STORE_REFS//,/ })
REFS_JSON="[${REFS_JSON%,}]"

cat > "${SESSION_DIR}/meta.json" <<EOF
{"session_id":"${SESSION}","agent_id":"${AGENT}","started_at":"$(date -u +%Y-%m-%dT%H:%M:%SZ)","ended_at":null,"store_refs":${REFS_JSON}}
EOF
: > "${SESSION_DIR}/transcript.jsonl"

echo "export MEMORY_ROOT=${ROOT} MEMORY_AGENT_ID=${AGENT} MEMORY_SESSION_ID=${SESSION}"
