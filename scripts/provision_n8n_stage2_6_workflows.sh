#!/usr/bin/env bash
# Provision destructive-safe Stage 2.6 n8n callback harness workflows.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONTAINER="${N8N_DOCKER_CONTAINER:-n8n}"
HOST_FILE="/tmp/kria_stage2_6_workflows.json"
CONTAINER_FILE="/tmp/kria_stage2_6_workflows.json"

if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required to provision the local n8n workflows." >&2
    exit 1
fi

if ! docker ps --format '{{.Names}}' | grep -Fxq "$CONTAINER"; then
    echo "n8n container '$CONTAINER' is not running." >&2
    exit 1
fi

if ! docker exec "$CONTAINER" sh -lc '[ -n "$KRIA_N8N_SIGNING_SECRET" ]' >/dev/null 2>&1; then
    echo "n8n container '$CONTAINER' is missing KRIA_N8N_SIGNING_SECRET." >&2
    exit 1
fi

python3 - "$HOST_FILE" <<'PY'
import json
import sys

output = sys.argv[1]

workflows = [
    {
        "id": "KriaGmailInboxD1",
        "name": "KRIA Gmail Inbox Digest Harness",
        "workflow_id": "gmail_inbox_digest",
        "path": "kria-gmail-inbox-digest",
        "result": "Harness inbox digest prepared. No Gmail provider action was performed.",
    },
    {
        "id": "KriaGmailSearch1",
        "name": "KRIA Gmail Message Search Harness",
        "workflow_id": "gmail_search_messages",
        "path": "kria-gmail-search-messages",
        "result": "Harness Gmail search completed. No Gmail provider action was performed.",
    },
    {
        "id": "KriaGmailDraft01",
        "name": "KRIA Gmail Draft Creator Harness",
        "workflow_id": "gmail_send_draft",
        "path": "kria-gmail-send-draft",
        "result": "Harness Gmail draft prepared for human review. No real Gmail draft was created.",
    },
    {
        "id": "KriaCalendarMt1",
        "name": "KRIA Calendar Meeting Creator Harness",
        "workflow_id": "calendar_create_meeting",
        "path": "kria-calendar-create-meeting",
        "result": "Harness calendar meeting draft prepared. No real calendar invite was created.",
    },
    {
        "id": "KriaSlackPost001",
        "name": "KRIA Slack Update Poster Harness",
        "workflow_id": "slack_post_update",
        "path": "kria-slack-post-update",
        "result": "Harness Slack update prepared for approval. No real Slack message was posted.",
    },
]

def make_workflow(spec, index):
    workflow_id = spec["workflow_id"]
    path = spec["path"]
    result = spec["result"]
    code = f"""const nodeCrypto = require('crypto');
const incoming = $input.first().json;
const body = incoming && typeof incoming.body === 'object' && incoming.body !== null ? incoming.body : incoming;
const workflowId = body.workflow_id || '{workflow_id}';
const workflowVersion = body.workflow_version || 'v1';
const correlationId = body.correlation_id || `manual-${{Date.now()}}`;
const causationId = body.causation_id || correlationId;
const inputPayload = body.input_payload || {{}};
const nowMs = Date.now();
const executedAt = new Date(nowMs).toISOString();
const n8nRunId = typeof $execution !== 'undefined' && $execution.id ? String($execution.id) : `exec-${{nowMs}}`;
const eventId = `evt-${{correlationId}}-${{nowMs}}`;
const result = '{result}';
const confirmedByUser = inputPayload.confirmed_by_user === true;
let outputEvidence = {{}};
if (workflowId === 'gmail_inbox_digest') {{
  outputEvidence = {{
    message_count: 3,
    message_refs: ['harness-gmail-msg-1', 'harness-gmail-msg-2', 'harness-gmail-msg-3']
  }};
}} else if (workflowId === 'gmail_search_messages') {{
  outputEvidence = {{
    matches: [
      {{ message_ref: 'harness-gmail-search-1', summary: 'Harness search match for the requested query' }}
    ]
  }};
}} else if (workflowId === 'gmail_send_draft') {{
  outputEvidence = {{
    draft_ref: `harness-draft-${{correlationId}}`,
    preview: inputPayload.body || 'Harness draft preview',
    confirmed_by_user: confirmedByUser
  }};
}} else if (workflowId === 'calendar_create_meeting') {{
  outputEvidence = {{
    event_ref: `harness-calendar-${{correlationId}}`,
    meeting_time: inputPayload.start_time || 'unspecified',
    confirmed_by_user: confirmedByUser
  }};
}} else if (workflowId === 'slack_post_update') {{
  outputEvidence = {{
    message_ref: `harness-slack-${{correlationId}}`,
    permalink: `harness://slack/${{correlationId}}`,
    confirmed_by_user: confirmedByUser
  }};
}}
const callbackEnvelope = {{
  schema_version: 'kria.n8n.callback.v1',
  correlation_id: correlationId,
  causation_id: causationId,
  event_id: eventId,
  sequence_number: 1,
  workflow_id: workflowId,
  workflow_version: workflowVersion,
  n8n_run_id: n8nRunId,
  status: 'completed',
  evidence: {{
    result,
    ...outputEvidence,
    workflow_purpose: '{workflow_id}',
    received_input_payload: inputPayload,
    executed_at: executedAt,
    occurred_at_ms: nowMs
  }},
  side_effects: [],
  occurred_at_ms: nowMs
}};
const callbackBody = JSON.stringify(callbackEnvelope);
const signingSecret = $env.KRIA_N8N_SIGNING_SECRET || '';
const callbackSignature = signingSecret
  ? 'sha256=' + nodeCrypto.createHmac('sha256', signingSecret).update(callbackBody).digest('hex')
  : '';
return [{{
  json: {{
    ...callbackEnvelope,
    result,
    received_input_payload: inputPayload,
    executed_at: executedAt,
    callback_body: callbackBody,
    callback_signature: callbackSignature
  }}
}}];"""
    y = 220 + index * 180
    return {
        "id": spec["id"],
        "name": spec["name"],
        "active": True,
        "nodes": [
            {
                "parameters": {
                    "httpMethod": "POST",
                    "path": path,
                    "responseMode": "responseNode",
                    "options": {},
                },
                "id": f"{workflow_id}-webhook",
                "name": "Webhook",
                "type": "n8n-nodes-base.webhook",
                "typeVersion": 2,
                "position": [240, y],
                "webhookId": path,
            },
            {
                "parameters": {"jsCode": code},
                "id": f"{workflow_id}-process",
                "name": "Process",
                "type": "n8n-nodes-base.code",
                "typeVersion": 2,
                "position": [460, y],
            },
            {
                "parameters": {
                    "method": "POST",
                    "url": "http://host.docker.internal:3001/api/n8n/callback",
                    "authentication": "none",
                    "sendHeaders": True,
                    "headerParameters": {
                        "parameters": [
                            {"name": "Content-Type", "value": "application/json"},
                            {"name": "x-kria-signature", "value": "={{ $json.callback_signature }}"},
                            {"name": "x-kria-correlation-id", "value": "={{ $json.correlation_id }}"},
                        ]
                    },
                    "sendBody": True,
                    "bodyParameters": {"parameters": []},
                    "specifyBody": "json",
                    "jsonBody": "={{ $json.callback_body }}",
                    "options": {},
                },
                "id": f"{workflow_id}-callback",
                "name": "Send Callback to KRIA",
                "type": "n8n-nodes-base.httpRequest",
                "typeVersion": 4.2,
                "position": [680, y],
            },
            {
                "parameters": {
                    "respondWith": "json",
                    "responseBody": "={{ JSON.stringify({ received: true, correlation_id: $('Process').first().json.correlation_id, n8n_run_id: $('Process').first().json.n8n_run_id }) }}",
                    "options": {},
                },
                "id": f"{workflow_id}-respond",
                "name": "Respond to KRIA",
                "type": "n8n-nodes-base.respondToWebhook",
                "typeVersion": 1.1,
                "position": [680, y + 120],
            },
        ],
        "connections": {
            "Webhook": {"main": [[{"node": "Process", "type": "main", "index": 0}]]},
            "Process": {
                "main": [[
                    {"node": "Send Callback to KRIA", "type": "main", "index": 0},
                    {"node": "Respond to KRIA", "type": "main", "index": 0},
                ]]
            },
        },
        "settings": {"executionOrder": "v1"},
        "tags": [],
    }

with open(output, "w", encoding="utf-8") as handle:
    json.dump([make_workflow(spec, index) for index, spec in enumerate(workflows)], handle, indent=2)
PY

docker cp "$HOST_FILE" "$CONTAINER:$CONTAINER_FILE"
if ! docker exec "$CONTAINER" n8n import:workflow --input="$CONTAINER_FILE"; then
    echo "Failed to import Stage 2.6 workflows into n8n." >&2
    exit 1
fi

for workflow_id in KriaGmailInboxD1 KriaGmailSearch1 KriaGmailDraft01 KriaCalendarMt1 KriaSlackPost001; do
    if ! docker exec "$CONTAINER" n8n publish:workflow --id="$workflow_id" >/dev/null 2>&1; then
        docker exec "$CONTAINER" n8n update:workflow --id="$workflow_id" --active=true >/dev/null
    fi
done

# The n8n CLI updates the database, but the running process does not always
# register newly active production webhooks until restart.
docker restart "$CONTAINER" >/dev/null
for _ in $(seq 1 45); do
    if curl -sS -m 2 "http://127.0.0.1:5678/healthz" >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

REGISTRY_PATH="${N8N_WORKFLOW_REGISTRY:-$HOME/.kria/n8n/workflow_registry.json}"
python3 - "$REGISTRY_PATH" <<'PY'
import json
import pathlib
import sys
import time

registry_path = pathlib.Path(sys.argv[1]).expanduser()
registry_path.parent.mkdir(parents=True, exist_ok=True)
now = int(time.time() * 1000)

if registry_path.exists():
    try:
        store = json.loads(registry_path.read_text(encoding="utf-8"))
    except Exception:
        store = {}
else:
    store = {}

records = []
for record in store.get("workflows", []):
    if isinstance(record, dict):
        records.append(record)

def workflow_record(
    workflow_id,
    display_name,
    endpoint_path,
    risk_tier,
    hitl_policy,
    category,
    description,
    examples,
    tags,
    aliases,
    allowed_actions,
    data_scope,
    expected_evidence,
):
    existing = next((item for item in records if item.get("workflow_id") == workflow_id), {})
    created_at = existing.get("created_at_ms") or now
    return {
        "workflow_id": workflow_id,
        "workflow_version": "v1",
        "display_name": display_name,
        "endpoint_path": endpoint_path,
        "status": "approved",
        "environment": "dev",
        "risk_tier": risk_tier,
        "irreversibility_class": "read_only",
        "timeout_class": "interactive",
        "owner": "kria-harness",
        "requires_callback": True,
        "input_schema_ref": f"schemas/n8n/{workflow_id}.input.json",
        "output_schema_ref": f"schemas/n8n/{workflow_id}.output.json",
        "credential_requirements": ["none"],
        "hitl_policy": hitl_policy,
        "category": category,
        "description": description,
        "example_prompts": examples,
        "tags": tags,
        "aliases": aliases,
        "allowed_actions": allowed_actions,
        "data_scope": data_scope,
        "expected_evidence": expected_evidence,
        "source": "stage2_6_harness_provision",
        "created_at_ms": created_at,
        "updated_at_ms": now,
    }

harness_records = [
    workflow_record(
        "test_workflow",
        "Test Workflow",
        "/webhook/c68f6f2c-4175-4c96-913b-1b5162f356e5",
        "Green",
        "none",
        "diagnostic",
        "Safe diagnostic callback workflow used to verify KRIA and n8n connectivity.",
        [
            "Run test_workflow",
            "Run Test Workflow",
            "Trigger test n8n workflow",
            "Execute kria test workflow now",
            "Check that n8n callbacks are working",
            "Run the diagnostic automation",
            "Verify the KRIA n8n round trip",
            "Smoke test the workflow bridge",
        ],
        ["diagnostic", "test", "n8n", "callback", "read_only"],
        ["test workflow", "diagnostic workflow", "test n8n workflow"],
        [],
        ["diagnostic"],
        ["result"],
    ),
    workflow_record(
        "gmail_inbox_digest",
        "Inbox Digest",
        "/webhook/kria-gmail-inbox-digest",
        "Green",
        "none",
        "email",
        "Harness workflow that returns a safe inbox digest result for KRIA routing and callback testing.",
        [
            "Run gmail_inbox_digest",
            "Run Inbox Digest",
            "Summarize my inbox",
            "Show me important emails from today",
            "What did I miss in email this morning",
            "Give me a quick email brief",
            "Pull together unread priority messages",
            "Catch me up on my inbox",
        ],
        ["email", "gmail", "inbox", "digest", "read_only", "harness"],
        ["inbox digest", "summarize my inbox", "email brief"],
        ["gmail.messages.read"],
        ["email_metadata", "user_requested"],
        ["result", "message_count"],
    ),
    workflow_record(
        "gmail_search_messages",
        "Gmail Message Search",
        "/webhook/kria-gmail-search-messages",
        "Green",
        "none",
        "email",
        "Harness workflow that returns deterministic Gmail search results without real provider writes.",
        [
            "Run gmail_search_messages",
            "Search Gmail messages",
            "Find emails from accounting",
            "Search my mail for the invoice thread",
            "Pull up emails about the client renewal",
            "Find the last message from Sara",
            "Look for messages with the budget attachment",
            "Show emails from last week about onboarding",
        ],
        ["email", "gmail", "search", "read_only", "harness"],
        ["gmail search", "search gmail messages", "find emails"],
        ["gmail.messages.read"],
        ["email_metadata", "user_requested"],
        ["result", "matches"],
    ),
    workflow_record(
        "gmail_send_draft",
        "Gmail Draft Creator",
        "/webhook/kria-gmail-send-draft",
        "Yellow",
        "required_review",
        "email",
        "Harness workflow that prepares a Gmail draft artifact for human review without sending email.",
        [
            "Run gmail_send_draft",
            "Create an email draft",
            "Draft an email to the client",
            "Prepare a Gmail reply",
            "Write a response but do not send it",
            "Compose a follow-up for the invoice thread",
            "Prepare a polite reply asking for the files",
            "Make a draft telling them I will review it tomorrow",
        ],
        ["email", "gmail", "draft", "write_review", "harness"],
        ["gmail draft", "create an email draft", "draft email"],
        ["gmail.drafts.create"],
        ["email_metadata", "email_body", "user_requested"],
        ["result", "draft_ref", "confirmed_by_user"],
    ),
    workflow_record(
        "calendar_create_meeting",
        "Calendar Meeting Creator",
        "/webhook/kria-calendar-create-meeting",
        "Yellow",
        "required_review",
        "calendar",
        "Harness workflow that prepares a calendar meeting artifact for review without sending invites.",
        [
            "Run calendar_create_meeting",
            "Schedule a meeting",
            "Create a calendar event tomorrow",
            "Book a 30 minute call with Ali",
            "Put a review session on my calendar",
            "Find a time and invite the team",
            "Set up a planning call for next week",
            "Add the demo to my schedule",
        ],
        ["calendar", "meeting", "schedule", "write_review", "harness"],
        ["calendar meeting", "schedule a meeting", "create meeting"],
        ["calendar.events.create"],
        ["calendar_metadata", "attendees", "user_requested"],
        ["result", "event_ref", "confirmed_by_user"],
    ),
    workflow_record(
        "slack_post_update",
        "Slack Update Poster",
        "/webhook/kria-slack-post-update",
        "Yellow",
        "required_review",
        "messaging",
        "Harness workflow that prepares a Slack update artifact for review without posting to Slack.",
        [
            "Run slack_post_update",
            "Post an update to Slack",
            "Send this to the team channel",
            "Share the release note in Slack",
            "Let the team know the build passed",
            "Publish today's status in the project channel",
            "Announce the meeting moved to 3 PM",
            "Put this update where the team will see it",
        ],
        ["slack", "message", "team", "write_review", "harness"],
        ["slack update", "post to slack", "team update"],
        ["slack.chat.write"],
        ["message_content", "channel", "user_requested"],
        ["result", "message_ref", "confirmed_by_user"],
    ),
]

ids = {record["workflow_id"] for record in harness_records}
records = [record for record in records if record.get("workflow_id") not in ids]
records.extend(harness_records)
records.sort(key=lambda record: record.get("workflow_id", ""))

store = {
    "schema_version": "kria.n8n.workflow_registry.v1",
    "updated_at_ms": now,
    "workflows": records,
}
registry_path.write_text(json.dumps(store, indent=2) + "\n", encoding="utf-8")
try:
    registry_path.chmod(0o600)
except Exception:
    pass
print(f"Seeded KRIA workflow registry: {registry_path}")
PY

echo "Provisioned Stage 2.6 n8n workflows:"
docker exec "$CONTAINER" n8n list:workflow | grep -E 'KRIA (Gmail|Calendar|Slack)' || true
