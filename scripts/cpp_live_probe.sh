#!/usr/bin/env bash
# Drive the REAL desktop chat pipeline over the local API (same n8n pre-fallback
# + agent loop + CPP path the desktop send_message uses). Not an internal test.
set -u
TOKEN="$(cat ~/.kria/api_token)"
API="http://127.0.0.1:3001/api/chat"
SID="probe-$(date +%s)"

ask() {
  local msg="$1"
  echo "════════════════════════════════════════════════════════════"
  echo "USER: $msg"
  curl -s -m 120 -X POST "$API" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d "$(jq -nc --arg m "$msg" --arg s "$SID" '{message:$m, session_id:$s, source:"desktop_chat", from_user:"Desktop"}')" \
  | jq -r '"STATUS: \(.status // "?")\nREPLY: \(.reply // .message // .)\n"' 2>/dev/null \
  || echo "(raw curl failed)"
}

for p in "$@"; do ask "$p"; done
