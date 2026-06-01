# KRIA Upgrade Rough Idea

Status: rough product direction
Language: Hinglish

## 1. Core Problem

Current n8n integration kaam kar raha hai, but user-friendly nahi hai.

Abhi agar new workflow integrate karna ho to user ko kaafi cheeze manually
samajhni padti hain:

- workflow config
- input schema
- output schema
- trigger endpoint
- callback setup
- HITL policy
- tags, aliases, prompts
- risk/data scope

Ye product ke liye ideal nahi hai. User ka goal simple hai:

```text
Workflow choose karo
Prompt do
KRIA kaam karwa de
Output chat me dikha de
```

## 2. Best Direction

Recommended approach:

```text
KRIA n8n Universal Execution Adapter
```

Main idea:

```text
User Prompt
  -> KRIA workflow select kare
  -> KRIA saved Workflow Runtime Profile dekhe
  -> KRIA prompt ko structured JSON me convert kare
  -> KRIA n8n workflow ko correct method se run kare
  -> KRIA n8n execution poll kare
  -> KRIA output auto-detect kare
  -> KRIA result chat/UI me show kare
```

Is approach me main n8n workflow me extra Code/Callback/HTTP nodes compulsory
nahi honge.

## 3. High-Level Diagram

```text
┌─────────────┐
│ User Prompt │
└──────┬──────┘
       │
       v
┌─────────────────────┐
│ KRIA Workflow Match │
└──────┬──────────────┘
       │
       v
┌──────────────────────────┐
│ Workflow Runtime Profile │
│ trigger/input/output/HITL│
└──────┬───────────────────┘
       │
       v
┌──────────────────────┐
│ Prompt -> JSON Input │
└──────┬───────────────┘
       │
       v
┌──────────────────────┐
│ Run n8n Workflow     │
│ API/Webhook/Schedule │
└──────┬───────────────┘
       │
       v
┌──────────────────────┐
│ Poll Execution Result│
└──────┬───────────────┘
       │
       v
┌──────────────────────┐
│ Auto Extract Output  │
└──────┬───────────────┘
       │
       v
┌──────────────────────┐
│ Show Result in KRIA  │
└──────────────────────┘
```

## 4. Workflow Runtime Profile

Jab user KRIA me workflow configure kare, KRIA workflow ko analyze/test karke
ek internal profile save kare.

User ko TOML manually fill nahi karna chahiye.

Profile me KRIA internally ye save kare:

```text
workflow_id
display_name
description
trigger_strategy
input_schema
payload_mapping
output_extraction_strategy
credential_requirements
risk_level
data_scope
hitl_policy
timeout/retry policy
example_prompts
last_successful_output_shape
```

Future runs me KRIA same profile use karegi.

```text
First setup:
Analyze -> Generate profile -> Test -> User approve -> Save

Every run:
Prompt -> Use saved profile -> JSON input -> Run -> Poll -> Show output
```

## 5. Plug-And-Play Setup Flow

Ideal user flow:

```text
1. KRIA Settings kholo
2. n8n tab me "Sync Workflows" click karo
3. KRIA workflows discover kare
4. User workflow select kare
5. KRIA auto-configure kare
6. Test run
7. Approve
8. Chat se use karo
```

User-facing settings simple honi chahiye:

```text
Name
Description
Status
Test button
Approve button
Example prompts
```

Developer details hidden rahen:

```text
schemas
output extractor
trigger strategy
risk metadata
raw n8n JSON
```

## 6. Run Strategy

KRIA ko user se trigger type choose karwane ki zaroorat nahi honi chahiye.
KRIA workflow JSON inspect karke khud decide kare.

```text
┌───────────────────────┐
│ n8n Workflow JSON     │
└──────────┬────────────┘
           │
           v
┌──────────────────────────────┐
│ KRIA detects trigger pattern │
└──────────┬───────────────────┘
           │
           ├─ Webhook Trigger  -> call webhook
           ├─ Manual Trigger   -> execute via n8n API
           ├─ Schedule Trigger -> monitor latest runs / run-now if safe
           ├─ Form Trigger     -> build form payload
           ├─ Chat Trigger     -> send chat-style payload
           └─ Event Trigger    -> monitor/listener mode
```

Default should be:

```text
No workflow modification
No extra callback node required
KRIA executes or monitors workflow
KRIA polls n8n execution result
KRIA shows output in chat
```

Current callback/HMAC mode should stay, but as advanced realtime mode.

## 7. Prompt To Structured JSON

LLM ka best use yahi hai:

```text
User prompt
+ workflow input schema
+ workflow examples
-> structured JSON payload
```

Example:

```text
User:
run get_movies workflow and get action movies

KRIA sends to n8n:
{
  "genre": "action",
  "limit": 10
}
```

n8n ko full prompt parse nahi karna chahiye except fallback/debug ke liye.

## 8. Automatic Output Handling

User ko output node manually select nahi karna chahiye by default.

KRIA execution data se useful output auto-detect kare:

```text
Priority:
1. Final successful node output
2. Last node with non-empty JSON
3. Node name/type containing output/result/response
4. Webhook response body
5. Execution summary fallback
```

If KRIA unsure ho:

```text
I found multiple possible outputs. Which one should I show in chat?
```

User ek baar choose kare, KRIA profile me save kar le.

## 9. Human In The Loop

HITL ko sirf n8n pe nahi chhodna chahiye.

Best ownership:

```text
n8n = workflow pause/wait/execute engine
KRIA = user-facing approval UI + policy + audit
```

Flow:

```text
Workflow reaches approval/wait state
  -> KRIA detects waiting execution
  -> KRIA chat/UI me approval card show kare
  -> User Approve / Reject
  -> KRIA decision audit kare
  -> KRIA n8n ko resume/cancel signal bheje
  -> Workflow continues or stops
```

HITL profile fields:

```text
hitl_policy: none | before_run | during_run | before_side_effect | always
approval_source: kria | n8n_wait_node | external
resume_strategy: n8n_api | webhook_resume | manual_link
approval_payload_schema
risk_summary
audit_required
```

User ko n8n dashboard me jaane ki zaroorat nahi honi chahiye for normal approval.

## 10. Recommended Implementation Order

```text
Phase A: n8n workflow discovery from settings
Phase B: Workflow Runtime Profile model
Phase C: Auto metadata/input/output profile generation
Phase D: API/Webhook execution + polling
Phase E: Auto output extraction
Phase F: Schema-based prompt -> JSON extraction
Phase G: HITL detection + KRIA approval UI
Phase H: Optional callback/adapter mode for advanced realtime workflows
```

## 11. Final Product Goal

Final experience should feel like:

```text
Connect n8n
Sync workflows
Test workflow
Approve workflow
Use from chat
```

Not like:

```text
Edit TOML
Add callback node
Write code node
Configure HMAC manually
Select output node manually
Debug JSON manually
```

KRIA ka role:

```text
Make n8n workflows easy, safe, structured, and chat-controllable.
```

