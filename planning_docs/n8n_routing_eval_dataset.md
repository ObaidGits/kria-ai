# KRIA n8n Routing Eval Dataset

Date: 2026-05-29
Purpose: Stage 3 preparation only. This dataset does not enable semantic
routing, embeddings, or auto-run behavior.

## Dataset Rules

Expected Stage 3 first-slice behavior:

- Easy prompts should resolve to one workflow.
- Medium prompts may resolve to one workflow only when metadata clearly supports
  the phrase.
- Hard prompts should usually ask for confirmation or clarification.
- No ambiguous prompt should auto-run.
- Disabled, draft, or unapproved workflows must not run.

## Target Workflow Set For Evaluation

Only `test_workflow` exists in the current KRIA registry. The remaining workflow
IDs below are proposed catalog targets for evaluation and planning.

| Workflow ID | Status Today | Purpose |
| --- | --- | --- |
| `test_workflow` | Existing approved diagnostic | Verify n8n callback path |
| `gmail_inbox_digest` | Proposed | Summarize inbox |
| `gmail_search_messages` | Proposed | Search mail |
| `gmail_send_draft` | Proposed | Create email draft |
| `calendar_create_meeting` | Proposed | Schedule meeting |
| `slack_post_update` | Proposed | Post Slack update |
| `jira_create_ticket` | Proposed | Create Jira ticket |
| `github_issue_triage` | Proposed | Triage GitHub issues |
| `invoice_extract_to_sheet` | Proposed | Extract invoice data |
| `daily_business_brief` | Proposed | Create daily business brief |

## Prompt Dataset

| ID | Workflow | Level | Prompt | Expected Routing Outcome |
| --- | --- | --- | --- | --- |
| R001 | `test_workflow` | Easy | Run test_workflow | Select `test_workflow` |
| R002 | `test_workflow` | Easy | Run Test Workflow | Select `test_workflow` |
| R003 | `test_workflow` | Easy | Trigger test n8n workflow | Select `test_workflow` |
| R004 | `test_workflow` | Easy | Execute kria test workflow now | Select `test_workflow` |
| R005 | `test_workflow` | Medium | Check that n8n callbacks are working | Select `test_workflow` only if alias/example exists |
| R006 | `test_workflow` | Medium | Run the diagnostic automation | Select `test_workflow` only if diagnostic category is unique |
| R007 | `test_workflow` | Medium | Verify the KRIA n8n round trip | Select `test_workflow` only if roundtrip examples exist |
| R008 | `test_workflow` | Medium | Smoke test the workflow bridge | Select `test_workflow` only if smoke-test alias exists |
| R009 | `test_workflow` | Hard | Check if automation is healthy | Clarify: diagnostic workflow vs runtime health check |
| R010 | `test_workflow` | Hard | Test everything | Clarify: n8n diagnostic vs broader KRIA tests |
| R011 | `gmail_inbox_digest` | Easy | Run gmail_inbox_digest | Select `gmail_inbox_digest` |
| R012 | `gmail_inbox_digest` | Easy | Run Inbox Digest | Select `gmail_inbox_digest` |
| R013 | `gmail_inbox_digest` | Easy | Summarize my inbox | Select `gmail_inbox_digest` |
| R014 | `gmail_inbox_digest` | Easy | Show me important emails from today | Select `gmail_inbox_digest` |
| R015 | `gmail_inbox_digest` | Medium | What did I miss in email this morning | Select `gmail_inbox_digest` |
| R016 | `gmail_inbox_digest` | Medium | Give me a quick email brief | Select `gmail_inbox_digest` |
| R017 | `gmail_inbox_digest` | Medium | Pull together unread priority messages | Select `gmail_inbox_digest` |
| R018 | `gmail_inbox_digest` | Medium | Catch me up on my inbox | Select `gmail_inbox_digest` |
| R019 | `gmail_inbox_digest` | Hard | Handle my email | Clarify: digest, search, or draft/send |
| R020 | `gmail_inbox_digest` | Hard | Find out what everyone sent | Clarify: inbox digest vs search messages |
| R021 | `gmail_search_messages` | Easy | Run gmail_search_messages | Select `gmail_search_messages` |
| R022 | `gmail_search_messages` | Easy | Search Gmail messages | Select `gmail_search_messages` |
| R023 | `gmail_search_messages` | Easy | Find emails from accounting | Select `gmail_search_messages` |
| R024 | `gmail_search_messages` | Easy | Search my mail for the invoice thread | Select `gmail_search_messages` |
| R025 | `gmail_search_messages` | Medium | Pull up emails about the client renewal | Select `gmail_search_messages` |
| R026 | `gmail_search_messages` | Medium | Find the last message from Sara | Select `gmail_search_messages` |
| R027 | `gmail_search_messages` | Medium | Look for messages with the budget attachment | Select `gmail_search_messages` |
| R028 | `gmail_search_messages` | Medium | Show emails from last week about onboarding | Select `gmail_search_messages` |
| R029 | `gmail_search_messages` | Hard | Check the email about the meeting | Clarify: search mail vs calendar meeting |
| R030 | `gmail_search_messages` | Hard | Get the report from mail | Clarify: search mail vs invoice/report extraction |
| R031 | `gmail_send_draft` | Easy | Run gmail_send_draft | Select `gmail_send_draft` |
| R032 | `gmail_send_draft` | Easy | Create an email draft | Select `gmail_send_draft` |
| R033 | `gmail_send_draft` | Easy | Draft an email to the client | Select `gmail_send_draft` |
| R034 | `gmail_send_draft` | Easy | Prepare a Gmail reply | Select `gmail_send_draft` |
| R035 | `gmail_send_draft` | Medium | Write a response but do not send it | Select `gmail_send_draft` |
| R036 | `gmail_send_draft` | Medium | Compose a follow-up for the invoice thread | Select `gmail_send_draft` |
| R037 | `gmail_send_draft` | Medium | Prepare a polite reply asking for the files | Select `gmail_send_draft` |
| R038 | `gmail_send_draft` | Medium | Make a draft telling them I will review it tomorrow | Select `gmail_send_draft` |
| R039 | `gmail_send_draft` | Hard | Send the report to the team | Clarify: email draft vs Slack post |
| R040 | `gmail_send_draft` | Hard | Reply to everyone | Clarify recipient, thread, and send/draft policy |
| R041 | `calendar_create_meeting` | Easy | Run calendar_create_meeting | Select `calendar_create_meeting` |
| R042 | `calendar_create_meeting` | Easy | Schedule a meeting | Select `calendar_create_meeting` |
| R043 | `calendar_create_meeting` | Easy | Create a calendar event tomorrow | Select `calendar_create_meeting` |
| R044 | `calendar_create_meeting` | Easy | Book a 30 minute call with Ali | Select `calendar_create_meeting` |
| R045 | `calendar_create_meeting` | Medium | Put a review session on my calendar | Select `calendar_create_meeting` |
| R046 | `calendar_create_meeting` | Medium | Find a time and invite the team | Select `calendar_create_meeting` after missing-input check |
| R047 | `calendar_create_meeting` | Medium | Set up a planning call for next week | Select `calendar_create_meeting` |
| R048 | `calendar_create_meeting` | Medium | Add the demo to my schedule | Select `calendar_create_meeting` |
| R049 | `calendar_create_meeting` | Hard | Discuss the report with everyone | Clarify: meeting vs Slack/email share |
| R050 | `calendar_create_meeting` | Hard | Handle the client follow-up | Clarify: email draft, meeting, or CRM note |
| R051 | `slack_post_update` | Easy | Run slack_post_update | Select `slack_post_update` |
| R052 | `slack_post_update` | Easy | Post an update to Slack | Select `slack_post_update` |
| R053 | `slack_post_update` | Easy | Send this to the team channel | Select `slack_post_update` |
| R054 | `slack_post_update` | Easy | Share the release note in Slack | Select `slack_post_update` |
| R055 | `slack_post_update` | Medium | Let the team know the build passed | Select `slack_post_update` |
| R056 | `slack_post_update` | Medium | Publish today's status in the project channel | Select `slack_post_update` |
| R057 | `slack_post_update` | Medium | Announce the meeting moved to 3 PM | Select `slack_post_update` if destination is Slack |
| R058 | `slack_post_update` | Medium | Put this update where the team will see it | Select `slack_post_update` only if channel context exists |
| R059 | `slack_post_update` | Hard | Share the report with everyone | Clarify: Slack, email, or meeting |
| R060 | `slack_post_update` | Hard | Publish the update | Clarify destination and audience |
| R061 | `jira_create_ticket` | Easy | Run jira_create_ticket | Select `jira_create_ticket` |
| R062 | `jira_create_ticket` | Easy | Create a Jira ticket | Select `jira_create_ticket` |
| R063 | `jira_create_ticket` | Easy | Open a bug in Jira | Select `jira_create_ticket` |
| R064 | `jira_create_ticket` | Easy | File a support task in Jira | Select `jira_create_ticket` |
| R065 | `jira_create_ticket` | Medium | Turn this error into a ticket | Select `jira_create_ticket` |
| R066 | `jira_create_ticket` | Medium | Track this customer issue for engineering | Select `jira_create_ticket` |
| R067 | `jira_create_ticket` | Medium | Make a bug task for the login failure | Select `jira_create_ticket` |
| R068 | `jira_create_ticket` | Medium | Create a follow-up item for the API timeout | Select `jira_create_ticket` |
| R069 | `jira_create_ticket` | Hard | Create an issue for this | Clarify: Jira ticket vs GitHub issue |
| R070 | `jira_create_ticket` | Hard | Track this bug | Clarify tracker and project |
| R071 | `github_issue_triage` | Easy | Run github_issue_triage | Select `github_issue_triage` |
| R072 | `github_issue_triage` | Easy | Triage GitHub issues | Select `github_issue_triage` |
| R073 | `github_issue_triage` | Easy | Label open repo bugs | Select `github_issue_triage` |
| R074 | `github_issue_triage` | Easy | Review new GitHub issues | Select `github_issue_triage` |
| R075 | `github_issue_triage` | Medium | Sort recent bug reports by priority | Select `github_issue_triage` if repo context exists |
| R076 | `github_issue_triage` | Medium | Find duplicate issues in the repo | Select `github_issue_triage` |
| R077 | `github_issue_triage` | Medium | Triage stale GitHub tickets | Select `github_issue_triage` |
| R078 | `github_issue_triage` | Medium | Summarize urgent repo issues | Select `github_issue_triage` |
| R079 | `github_issue_triage` | Hard | Organize bug reports | Clarify GitHub vs Jira |
| R080 | `github_issue_triage` | Hard | Clean up the issue backlog | Clarify tracker and repository/project |
| R081 | `invoice_extract_to_sheet` | Easy | Run invoice_extract_to_sheet | Select `invoice_extract_to_sheet` |
| R082 | `invoice_extract_to_sheet` | Easy | Extract this invoice into a sheet | Select `invoice_extract_to_sheet` |
| R083 | `invoice_extract_to_sheet` | Easy | Add invoice details to spreadsheet | Select `invoice_extract_to_sheet` |
| R084 | `invoice_extract_to_sheet` | Easy | Parse the vendor invoice | Select `invoice_extract_to_sheet` |
| R085 | `invoice_extract_to_sheet` | Medium | Put the billing document into our tracker | Select `invoice_extract_to_sheet` if finance context exists |
| R086 | `invoice_extract_to_sheet` | Medium | Capture amount, vendor, and due date | Select `invoice_extract_to_sheet` |
| R087 | `invoice_extract_to_sheet` | Medium | Convert this receipt into a row | Select `invoice_extract_to_sheet` |
| R088 | `invoice_extract_to_sheet` | Medium | Log this bill for review | Select `invoice_extract_to_sheet` |
| R089 | `invoice_extract_to_sheet` | Hard | Process this document | Clarify invoice, email, or ticket |
| R090 | `invoice_extract_to_sheet` | Hard | Handle this payment thing | Clarify action and destination |
| R091 | `daily_business_brief` | Easy | Run daily_business_brief | Select `daily_business_brief` |
| R092 | `daily_business_brief` | Easy | Create my daily business brief | Select `daily_business_brief` |
| R093 | `daily_business_brief` | Easy | Give me today's operations summary | Select `daily_business_brief` |
| R094 | `daily_business_brief` | Easy | Summarize business updates for today | Select `daily_business_brief` |
| R095 | `daily_business_brief` | Medium | What should I know before standup | Select `daily_business_brief` |
| R096 | `daily_business_brief` | Medium | Prepare my morning work brief | Select `daily_business_brief` |
| R097 | `daily_business_brief` | Medium | Pull together today's key updates | Select `daily_business_brief` |
| R098 | `daily_business_brief` | Medium | Make a short leadership summary | Select `daily_business_brief` |
| R099 | `daily_business_brief` | Hard | Brief me | Clarify personal brief, business brief, or inbox digest |
| R100 | `daily_business_brief` | Hard | Summarize everything | Clarify sources and scope |

## Acceptance Targets

| Level | Expected Behavior | Target |
| --- | --- | ---: |
| Easy | Correct unique workflow or safe no-run if unapproved | 100% |
| Medium | Correct workflow when metadata supports it, otherwise clarification | >= 90% |
| Hard | Clarification instead of auto-run | >= 95% |
| Disabled/draft workflow | Never auto-run | 100% |

## Notes For Future Eval Runner

This dataset should be used after the catalog contains at least three approved
workflows. The first eval runner should report:

- prompt ID,
- expected workflow or expected clarification,
- actual selected workflow,
- whether the workflow auto-ran,
- whether the prompt required confirmation,
- pass/fail.

Stage 3 must not auto-run hard prompts. The safe behavior is to present top
candidates and ask the user to choose.
