# CPP E2E Validation Report (dispatcher / chat path)

Generated: 2026-07-09T18:20:46.005419326+00:00

Driven through the REAL `CapabilityDispatchHandler` (the `openclaw` chat tool) → CapabilityPlatform → OpenClawProvider → Docker.

| Test | Category | Verdict | ms | Detail |
|---|---|---|---|---|
| 01 arithmetic | arithmetic | PASS | 386 | {"expression":"((45*12)+87)/3","result":209} |
| 02 arithmetic-nl | arithmetic | PASS | 67 | {"expression":"2^10","result":1024} |
| 03 arithmetic-nl2 | arithmetic | PASS | 68 | {"expression":"100 - 7 * 3","result":79} |
| 04 hash-sha256 | hashing | PASS | 76 | {"algorithm":"sha256","hash":"9b8f387b7777fc5e0d0eff06370098409980b75695aa91e1886e34c4b190889a"} |
| 05 hash-md5 | hashing | PASS | 71 | {"algorithm":"md5","hash":"5d41402abc4b2a76b9719d911017c592"} |
| 06 hash-nl | hashing | PASS | 67 | {"algorithm":"sha256","hash":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"} |
| 07 json-minify | json | PASS | 73 | {"valid":true,"output":"{\"b\":2,\"a\":1}"} |
| 08 json-pretty | json | PASS | 76 | {"valid":true,"output":"{\n  \"x\": 1\n}"} |
| 09 json-validate | json | PASS | 68 | {"valid":true,"output":"{\n  \"ok\": true\n}"} |
| 10 regex | regex | PASS | 78 | {"matches":["1","2","3"],"count":3} |
| 11 csv-parse | csv | PASS | 76 | {"rows":[{"a":"1","b":"2"}]} |
| 12 markdown | markdown | PASS | 76 | {"html":"<h1>Title</h1>"} |
| 13 text-upper | string | PASS | 62 | {"output":"HELLO"} |
| 14 text-lower | string | PASS | 70 | {"output":"hello"} |
| 15 gzip | compression | PASS | 72 | {"base64":"H4sIAAAAAAAAA0vOzy0oSi0uVshNBQAQ5dGyCwAAAA==","original_bytes":11,"compressed_bytes":31} |
| 16 mcp-reverse | mcp | PASS | 10 | "airk" |
| 17 mcp-wordcount | mcp | PASS | 10 | {"words":3} |
| 18 unknown-cap | negative | PASS | 7 | "No installed capability matches: 'physically water my office plants right now'. Try the Marketplace to install one." |
| 19 unknown-cap2 | negative | PASS | 7 | "No installed capability matches: 'book me a flight to Tokyo next tuesday'. Try the Marketplace to install one." |
| 20 malformed-expr | negative | PASS | 83 | capability execution failed: capability execution failed: Error: mismatched parentheses |
| 21 malformed-json | negative | PASS | 73 | {"valid":false,"error":"Expected property name or '}' in JSON at position 1 (line 1 column 2)"} |
| 22 empty-query | negative | PASS | 0 | no query provided to the capability dispatcher |
| 17 perm-gate | permission | PASS | 12 | first network call gated: Some("'Web Fetch' requires approval (effects: network). Approve it once in the Capabilities → Approval Center, then retry.") |
| 18 grant-reuse | permission | PASS | 73 | post-approval no re-prompt (success=false, err=Some("capability execution failed: capability execution failed: OpenClaw tool execution failed: tool call failed: [-32602] Unknown tool: oc_web_fetch")) |

**PASS 24 · FAIL 0 · total 24 · avg 69ms**
