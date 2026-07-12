# CPP Prompt Battery Report

Generated: 2026-07-06T17:24:53.054932558+00:00

Driven exactly as the desktop does (install → describe → discover → permission → execute) over real Docker + the live ClawHub index.

| Prompt | Category | Verdict | Detail |
|---|---|---|---|
| 1 marketplace-fetch | marketplace | PASS | live index has 30 entries |
| 2-4 install-lifecycle | marketplace | PASS | 30 skills present, 0 install failures |
| 5 uninstall | marketplace | PASS | oc_lorem_ipsum removed = Ok(true) |
| 7 discover-case | discovery | PASS | top: ["oc_html_to_text", "oc_case_converter", "oc_text_tool"] |
| 8 discover-hash | discovery | PASS | top: ["oc_sql_formatter", "oc_hash_tool", "oc_json_tool"] |
| 9 discover-json | discovery | PASS | top: ["oc_json_formatter", "oc_json_tool", "oc_csv_tool"] |
| 10 discover-jwt | discovery | PASS | top: ["oc_jwt_decoder", "oc_http_get", "oc_yaml_to_json"] |
| 11 discover-yaml | discovery | PASS | top: ["oc_yaml_to_json", "oc_csv_to_json", "oc_ip_info"] |
| 12 inspect-schema | inspect | PASS | unit_converter descriptor + input_schema present |
| 13 inspect-effects | inspect | PASS | dns_lookup effects=["network"] elevated=true |
| 14 tier-neverask | permission | PASS | word_counter → NeverAsk |
| 15 tier-network-prompt | permission | PASS | Prompt { tier: AskPerSession, prompt: PromptSpec { effects: ["network"], risk: "medium", reason: "capability requires approval for the requested effects" } } |
| 16 grant-reuse | permission | PASS | Allow { tier: AskPerSession, grant_id: Some("0c48ae81-2746-40c0-bd25-335eb2875cef") } |
| 18 revoke-reprompt | permission | PASS | Prompt { tier: AskPerSession, prompt: PromptSpec { effects: ["network"], risk: "medium", reason: "capability requires approval for the requested effects" } } |
| 17 tier-alwaysask | permission | PASS | code_sandbox → Prompt { tier: AlwaysAsk, prompt: PromptSpec { effects: ["subprocess"], risk: "high", reason: "system-modifying capability requires explicit approval on every use" } } |
| E baked-calculator | execute | PASS | {"expression":"173*49+12","result":8489} |
| E baked-hash | execute | PASS | {"algorithm":"sha256","hash":"9b8f387b7777fc5e0d0eff06370098409980b75695aa91e1886e34c4b190889a"} |
| 19 base64 | execute | NO_HANDLER | capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_base64_tool |
| 20 slug | execute | NO_HANDLER | capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_slug_generator |
| 21 uuid | execute | NO_HANDLER | capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_uuid_generator |
| 22 unit | execute | NO_HANDLER | capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_unit_converter |
| 23 math | execute | NO_HANDLER | capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_math_evaluator |
| 24 regex | execute | NO_HANDLER | capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_regex_extractor |
| 25 csv2json | execute | NO_HANDLER | capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_csv_to_json |
| 26 ts | execute | NO_HANDLER | capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_timestamp_converter |
| 27 pw | execute | NO_HANDLER | capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_password_generator |
| 28 color | execute | NO_HANDLER | capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_color_converter |
| 29 cron | execute | NO_HANDLER | capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_cron_describer |
| 30 recommend | platform | SKIP | 0 installable recommendations (empty catalog ⇒ SKIP) |
| 31 a9-generation | platform | SKIP | needs cloud LLM env (validated separately in kria-eval task 11.2) |
| 32 degraded | platform | SKIP | would stop the shared Docker daemon; not run in-harness |
| 33 timeline | platform | PASS | 26 capability events captured |

**PASS 18 · FAIL 0 · NO_HANDLER 11 · SKIP 3**

Notes: NO_HANDLER = installs + discovers + permission-gates correctly, but the OpenClaw substrate image has no execution handler for that skill yet (expected for the new pure-logic skills). SKIP = requires an external resource (cloud LLM) or a destructive action (stopping Docker) not run in-harness.
