# Memory Graph Production Redesign — Complete Planned Traceability

**Status:** Planned/Unverified. This ledger maps obligations; it does not prove implementation. No row may advance without a commit-specific Evidence Artifact manifest. Checked tasks, code presence, screenshots, or this mapping are not proof.

## 1. Registers, Codes, and Coverage Policy

Normative requirements are MGR-001–MGR-048 (**48**). Binding decisions are MGD-001–MGD-046 (**46**): MGD-001–022 are preserved; MGD-023–046 are binding Planned target decisions, not shipped claims. Audit findings are MG-C01–C07 (**7**), MG-H01–H17 (**17**), MG-M01–M28 (**28**), MG-L01–L13 (**13**), totaling **65**. Opportunities are MG-O01–O31 (**31**), not additional findings.

Workstream codes: `WE` evidence, `WA` authority, `WS` security/policy, `WL` lifecycle/recovery, `WM` semantic model, `WR` retrieval, `WC` cognition/scheduler, `WP` canonical API, `WH` human Digital Twin/list/actions, `W2` Canvas2D, `WX` release/evolution/supply chain, `W3` optional 3D. Full definitions are in `implementation-roadmap.md` §2.

Artifact classes: `A-MAN` manifest/coverage; `A-DB` schema/SQL/invariant hashes; `A-PROP` property/counterexample; `A-SEC` paired-world/auth/redaction; `A-API` DTO/host golden; `A-RET` judgments/RRF/traces; `A-REC` fault/recovery/rebuild; `A-IO` interchange/migration; `A-UI` reducer/E2E/action transcript; `A-VIS` screenshot+semantic review; `A-A11Y` axe/keyboard/Orca; `A-PERF` latency/frame/CPU/heap/query plan; `A-SBOM` lock/license/SBOM/vulnerability; `A-REV` signed human review.

Risk and validation IDs refer to `risk-analysis.md` and `validation.md`. Design sections are the explicit `design.md` §20 mapping for MGR rows; MGD rows cite the sections implementing the decision. Gates are earliest-to-final evidence span, not completion state.

## 2. Requirement Ledger — 48/48 Planned/Unverified

| Requirement | Design | Work | Validation | Principal risks | Gates | Evidence | Status |
|---|---|---|---|---|---|---|---|
| MGR-001 Epistemic truth | §§2,9,11,19.2,19.9 | WE,WM,WH | V-TRUTH-01,V-DT-01,V-VIS-01 | R-TRUTH-LAUNDER,R-DT-FALSE | F0–F5 | A-MAN,A-UI,A-VIS,A-REV | Planned/Unverified |
| MGR-002 Mixed graph projection | §§4.2,8,10,19.3,19.7 | WM,WP,WH | V-SEM-01,V-GRAPH-01,V-UI-UNIT-01 | R-AUTH-SPLIT,R-TRUTH-LAUNDER | F2–F4 | A-PROP,A-API,A-UI | Planned/Unverified |
| MGR-003 Server boundary | §§8.3,13,19.8 | WS,WP | V-POLICY-02,V-XPORT-01,V-FAULT-01 | R-POLICY-LEAK,R-TIMING-LEAK | F1–F5 | A-SEC,A-API,A-REC | Planned/Unverified |
| MGR-004 Scope/sensitivity isolation | §§4.1,13,19.2 | WS | V-POLICY-01..02,V-XPORT-01 | R-POLICY-LEAK,R-TIMING-LEAK | F1–F5 | A-PROP,A-SEC,A-API | Planned/Unverified |
| MGR-005 Governed relation writes | §§4.2,5,19.3–19.4 | WA,WM | V-AUTH-01..03,V-SEM-01 | R-AUTH-ATOMIC,R-AUTH-SPLIT | F1–F2 | A-DB,A-PROP | Planned/Unverified |
| MGR-006 Full-corpus ranked search | §§6.1–6.4,8.1,9 | WR,WP,WH | V-RET-01..03,V-PERF-01,V-DT-01 | R-RETRIEVAL-REGRESSION,R-UI-PERF-CHEAT | F3–F5 | A-RET,A-PERF,A-UI | Planned/Unverified |
| MGR-007 Bounded graph API | §§6.5,8.1,10.2 | WM,WR,WP | V-GRAPH-01,V-XPORT-01,V-PERF-01 | R-POLICY-LEAK,R-SCHED-STARVE | F2–F5 | A-PROP,A-API,A-PERF | Planned/Unverified |
| MGR-008 Revision/patch consistency | §§5.2,9.2,19.7,19.9 | WA,WP,WH | V-AUTH-01,V-UI-UNIT-01,V-E2E-01,V-FAULT-01 | R-PATCH-WINDOW,R-AUTH-ATOMIC | F1–F5 | A-DB,A-UI,A-REC | Planned/Unverified |
| MGR-009 Bounded execution | §§6.2,13,19.1,19.11 | WC,WR | V-FAULT-01,V-PERF-01,V-RESOURCE-01 | R-SCHED-STARVE | F1–F5 | A-REC,A-PERF | Planned/Unverified |
| MGR-010 Temporal correctness | §§4.2,6.5,19.2 | WM,WR,WH | V-TRUTH-01,V-RET-01,V-DT-01 | R-TRUTH-LAUNDER | F2–F4 | A-PROP,A-RET,A-UI | Planned/Unverified |
| MGR-011 Honest analytics | §§10–11,19.10 | WE,WM,WH | V-TRUTH-01,V-UI-UNIT-01,V-VIS-01 | R-TRUTH-LAUNDER,R-DT-FALSE | F0–F5 | A-MAN,A-UI,A-VIS | Planned/Unverified |
| MGR-012 Renderer-neutral scene/actions | §§10,19.1,19.7,19.10 | WH,W2 | V-UI-UNIT-01,V-DT-01,V-A11Y-01 | R-A11Y-DRIFT,R-DT-FALSE | F2–F5 | A-UI,A-A11Y | Planned/Unverified |
| MGR-013 Focus concurrency | §§9.2,19.9 | WH | V-UI-UNIT-01,V-E2E-01,V-A11Y-01 | R-PATCH-WINDOW,R-A11Y-DRIFT | F4–F5 | A-UI,A-A11Y | Planned/Unverified |
| MGR-014 Accessible composite | §§11.1–11.2,19.9–19.10 | WH,W2 | V-A11Y-01,V-DT-01,V-VIS-01 | R-A11Y-DRIFT,R-CANVAS-WEBKIT | F4–F5 | A-A11Y,A-UI,A-VIS,A-REV | Planned/Unverified |
| MGR-015 Adaptive authoritative 2D | §§10–12,19.10–19.11 | W2,WH | V-UI-UNIT-01,V-VIS-01,V-RESOURCE-01 | R-CANVAS-WEBKIT,R-UI-PERF-CHEAT | F4–F5 | A-UI,A-VIS,A-PERF | Planned/Unverified |
| MGR-016 Responsive input | §§11.1,19.9 | W2,WH | V-A11Y-01,V-VIS-01,V-E2E-01 | R-CANVAS-WEBKIT,R-A11Y-DRIFT | F4–F5 | A-A11Y,A-VIS,A-UI | Planned/Unverified |
| MGR-017 Fault/recovery | §§5.3,9.2,19.5,19.7 | WL,WP | V-REC-01,V-REBUILD-01,V-FAULT-01,V-E2E-01 | R-CORRUPT-AUTH,R-CORRUPT-DERIVED | F1–F5 | A-REC,A-UI,A-REV | Planned/Unverified |
| MGR-018 Relation ontology/evidence | §§4.2,7.2,19.3 | WM | V-SEM-01,V-TRUTH-01 | R-AUTH-SPLIT,R-TRUTH-LAUNDER | F2–F4 | A-PROP,A-DB | Planned/Unverified |
| MGR-019 Entity provenance/resolution | §§7.1,19.2–19.3 | WM,WH | V-ENTITY-01,V-E2E-01 | R-WRONG-MERGE,R-POLICY-LEAK | F2–F4 | A-PROP,A-UI,A-REV | Planned/Unverified |
| MGR-020 Transport parity | §§8,19.1,19.7 | WP,WS | V-XPORT-01,V-FAULT-01 | R-POLICY-LEAK,R-AUTH-SPLIT | F3–F5 | A-API,A-SEC,A-REC | Planned/Unverified |
| MGR-021 Multi-window ownership | §§9.2,19.9 | WH,WX | V-UI-UNIT-01,V-E2E-01,V-LIFE-01 | R-PATCH-WINDOW,R-DELETE-RESIDUE | F5 | A-UI,A-REC | Planned/Unverified |
| MGR-022 Idle/interaction budgets | §§10.2,12–13,19.11 | WC,W2,WX | V-RESOURCE-01,V-PERF-01 | R-SCHED-STARVE,R-CANVAS-WEBKIT | F4–F5 | A-PERF,A-REV | Planned/Unverified |
| MGR-023 Scale-aware navigation | §§6.5,8.1,10.2,19.10 | WR,WP,W2 | V-GRAPH-01,V-PERF-01,V-RESOURCE-01 | R-VECTOR-SCALE,R-UI-PERF-CHEAT | F3–F5 | A-PROP,A-PERF,A-UI | Planned/Unverified |
| MGR-024 Explain/correct | §§9.1,19.7,19.9 | WM,WH | V-ENTITY-01,V-TRUTH-01,V-DT-01,V-E2E-01 | R-WRONG-MERGE,R-DT-FALSE | F2–F5 | A-PROP,A-UI,A-REV | Planned/Unverified |
| MGR-025 Retrieval-use traces | §§4.3,6.4,19.4,19.7 | WR,WH | V-RET-02,V-DT-01,V-E2E-01 | R-RETRIEVAL-REGRESSION,R-TRUTH-LAUNDER | F3–F4 | A-RET,A-UI | Planned/Unverified |
| MGR-026 Visual authority encoding | §§11.3,19.10 | WH,W2 | V-VIS-01,V-A11Y-01 | R-TRUTH-LAUNDER,R-A11Y-DRIFT | F4–F5 | A-VIS,A-A11Y,A-REV | Planned/Unverified |
| MGR-027 Testing/evidence gates | §§16–18,19.11 | WE,WX | all V-* plus orphan linter | R-UI-PERF-CHEAT,R-SUPPLY | F0–F6 | all artifact classes | Planned/Unverified |
| MGR-028 Privacy-safe observability | §§13,19.11 | WS,WX | V-POLICY-02,V-RESOURCE-01,V-SBOM-01 | R-OBSERVABILITY,R-TIMING-LEAK | F1–F5 | A-SEC,A-PERF,A-SBOM | Planned/Unverified |
| MGR-029 Documentation/audit continuity | §§1,16,20 | WE | coverage/orphan linter,V-REG-01 | R-TRUTH-LAUNDER | F0–F5 | A-MAN,A-REV | Planned/Unverified |
| MGR-030 Optional true 3D | §§15,19.10 | W3 | V-3D-01,V-A11Y-01,V-SBOM-01 | R-3D-OPTIONAL,R-SUPPLY | F6 | A-UI,A-A11Y,A-PERF,A-SBOM,A-REV | Planned/Unverified |
| MGR-031 Control Center integrity | §§9,19.9 | WH | V-DT-01,V-E2E-01,V-VIS-01 | R-DT-FALSE,R-A11Y-DRIFT | F4–F5 | A-UI,A-VIS,A-REV | Planned/Unverified |
| MGR-032 Evolution seam | §§3–4,8,14,19.1 | WA,WP,WX | V-SCHEMA-01,V-IO-01,V-XPORT-01 | R-MIGRATION-LOSS,R-INTERCHANGE-LOSS | F1–F5 | A-DB,A-IO,A-API | Planned/Unverified |
| MGR-033 SQLite authority/events | §§4–5,19.4 | WA | V-AUTH-01..03,V-SCHEMA-01 | R-AUTH-SPLIT,R-AUTH-ATOMIC,R-EVENT-PLAINTEXT | F1 | A-DB,A-PROP,A-REC | Planned/Unverified |
| MGR-034 Typed records/provenance | §§4,8,19.2,19.7 | WA,WM,WP | V-SCHEMA-01,V-SEM-01,V-IO-01 | R-MIGRATION-LOSS,R-TRUTH-LAUNDER | F1–F2 | A-DB,A-PROP,A-IO | Planned/Unverified |
| MGR-035 Write policy/memory modes | §§3,5,19.1–19.4 | WA,WS | V-POLICY-01..02,V-AUTH-01 | R-AUTH-SPLIT,R-POLICY-LEAK | F1 | A-PROP,A-SEC,A-DB | Planned/Unverified |
| MGR-036 Five-strategy retrieval | §§6,19.4,19.11 | WR | V-VECTOR-01,V-RET-01..03,V-PERF-01 | R-MODEL-PIN,R-VECTOR-SCALE,R-RETRIEVAL-REGRESSION | F3 | A-RET,A-PERF,A-SBOM | Planned/Unverified |
| MGR-037 Truth/supersession | §§7.2,19.2–19.3 | WM | V-TRUTH-01,V-SEM-01 | R-TRUTH-LAUNDER,R-CONS-ESCALATE | F2–F3 | A-PROP,A-DB | Planned/Unverified |
| MGR-038 Active goals/recall | §§4.3,6.5,9,19.2 | WM,WR,WH | V-RET-01..03,V-DT-01 | R-RETRIEVAL-REGRESSION,R-DT-FALSE | F3–F4 | A-RET,A-UI | Planned/Unverified |
| MGR-039 Consolidation/lineage | §§4.3,7.3,19.3 | WC,WM | V-CONS-01,V-TRUTH-01 | R-CONS-ESCALATE,R-POLICY-LEAK | F3–F5 | A-PROP,A-REC,A-REV | Planned/Unverified |
| MGR-040 Lifecycle/erasure | §§5.4,19.2,19.6 | WL,WH | V-LIFE-01,V-E2E-01,V-FAULT-01 | R-DELETE-RESIDUE,R-EVENT-PLAINTEXT | F1–F5 | A-REC,A-UI,A-SEC | Planned/Unverified |
| MGR-041 Crypto truth | §§5.4,19.6 | WL,WS | V-CRYPTO-01,V-VIS-01 | R-EVENT-PLAINTEXT,R-DELETE-RESIDUE | F1–F5 | A-SEC,A-REC,A-VIS,A-REV | Planned/Unverified |
| MGR-042 Index convergence/model migration | §§4.4,5.3,19.5 | WL,WR | V-REBUILD-01,V-VECTOR-01,V-FAULT-01 | R-CORRUPT-DERIVED,R-MODEL-PIN | F1–F3 | A-REC,A-RET,A-DB | Planned/Unverified |
| MGR-043 Source/tool isolation | §§7.4,13,19.1–19.2 | WS,WC | V-POLICY-01..02,V-TOOL-01,V-XPORT-01 | R-POLICY-LEAK,R-TOOL-ESCALATE | F1–F3 | A-SEC,A-PROP,A-API | Planned/Unverified |
| MGR-044 Tool learning | §§4.3,7.4,19.4 | WC | V-TOOL-01,V-CONS-01 | R-TOOL-ESCALATE,R-CONS-ESCALATE | F3–F5 | A-PROP,A-REC | Planned/Unverified |
| MGR-045 Offline/resource cognition | §§6.4,13,19.5 | WC,WL,WR | V-RET-01,V-FAULT-01,V-RESOURCE-01 | R-SCHED-STARVE,R-CORRUPT-DERIVED | F1–F5 | A-REC,A-PERF,A-RET | Planned/Unverified |
| MGR-046 Consent/source lifecycle | §§14,19.2,19.6,19.9 | WM,WS,WH | V-SEM-01,V-IO-01,V-E2E-01 | R-POLICY-LEAK,R-INTERCHANGE-LOSS | F2–F4 | A-PROP,A-IO,A-UI | Planned/Unverified |
| MGR-047 FOSS/SBOM | §§10.1,15,19.10 | WX,W3 | V-SBOM-01,V-3D-01 | R-SUPPLY,R-PROJECT-LICENSE,R-MODEL-PIN | F1–F6 | A-SBOM,A-REV | Planned/Unverified |
| MGR-048 Backend-first order | §§1–3,18–20 | all | predecessor/orphan gate linter,all V-* | R-DT-FALSE,R-3D-OPTIONAL | F0–F6 | A-MAN,A-REV | Planned/Unverified |

## 3. Decision Ledger — 46/46 Planned/Unverified

“Preserved” describes decision provenance, not implementation status. Every decision remains implementation-evidence Unverified.

| Decision | Design implementation | Work | Validation | Principal risks | Gate | Evidence | Status |
|---|---|---|---|---|---|---|---|
| MGD-001 Trust/correction/explanation | §§2,9,11 | WM,WH | V-TRUTH-01,V-DT-01,V-E2E-01 | R-TRUTH-LAUNDER,R-DT-FALSE | F0–F5 | A-UI,A-VIS,A-REV | Planned/Unverified |
| MGD-002 Authoritative 2D; 3D nonblocking | §§10–12,15 | WH,W2,W3 | V-DT-01,V-A11Y-01,V-3D-01 | R-CANVAS-WEBKIT,R-3D-OPTIONAL | F4–F6 | A-UI,A-A11Y,A-PERF | Planned/Unverified |
| MGD-003 Entity-primary typed projection | §§4.2,8,10 | WM,WP | V-SEM-01,V-GRAPH-01 | R-AUTH-SPLIT,R-TRUTH-LAUNDER | F2–F4 | A-PROP,A-API | Planned/Unverified |
| MGD-004 Generated navigation not topology | §§2,10–11 | WM,WH | V-TRUTH-01,V-VIS-01 | R-TRUTH-LAUNDER | F2–F5 | A-PROP,A-VIS | Planned/Unverified |
| MGD-005 Honest component/community | §§10–11 | WM,WH | V-TRUTH-01,V-VIS-01 | R-TRUTH-LAUNDER | F2–F5 | A-PROP,A-VIS | Planned/Unverified |
| MGD-006 Loopback/secure remote | §§8.3,13 | WS,WP | V-POLICY-02,V-XPORT-01 | R-POLICY-LEAK,R-TIMING-LEAK | F1–F5 | A-SEC,A-API | Planned/Unverified |
| MGD-007 Restrictive policy propagation | §§4.1,13 | WS | V-POLICY-01,V-POLICY-02 | R-POLICY-LEAK | F1–F5 | A-PROP,A-SEC | Planned/Unverified |
| MGD-008 Registry relation identity | §§4.2,7.2 | WM | V-SEM-01 | R-AUTH-SPLIT | F2 | A-PROP,A-DB | Planned/Unverified |
| MGD-009 Multi-evidence relations | §§4.2,7.2 | WM | V-SEM-01,V-TRUTH-01 | R-TRUTH-LAUNDER | F2–F3 | A-PROP | Planned/Unverified |
| MGD-010 Governed atomic writes | §§3–5 | WA,WS | V-AUTH-01,V-AUTH-02,V-AUTH-03,V-POLICY-01 | R-AUTH-ATOMIC,R-AUTH-SPLIT | F1 | A-DB,A-PROP | Planned/Unverified |
| MGD-011 One semantic revision/commit | §§5.1–5.2 | WA,WP | V-AUTH-01,V-UI-UNIT-01 | R-AUTH-ATOMIC,R-PATCH-WINDOW | F1–F5 | A-DB,A-UI | Planned/Unverified |
| MGD-012 Transport-neutral parity | §§3,8 | WP | V-XPORT-01 | R-AUTH-SPLIT,R-POLICY-LEAK | F3–F5 | A-API,A-SEC | Planned/Unverified |
| MGD-013 Touch-scoped 2D | §§11–12 | W2,WH | V-A11Y-01,V-VIS-01,V-E2E-01 | R-CANVAS-WEBKIT,R-A11Y-DRIFT | F4–F5 | A-A11Y,A-VIS | Planned/Unverified |
| MGD-014 Per-window intent ownership | §§9.2 | WH,WX | V-UI-UNIT-01,V-E2E-01 | R-PATCH-WINDOW | F4–F5 | A-UI | Planned/Unverified |
| MGD-015 100k/query-scoped work | §§6,10,17 | WR,W2,WX | V-PERF-01,V-RESOURCE-01 | R-VECTOR-SCALE,R-UI-PERF-CHEAT | F3–F5 | A-PERF,A-MAN | Planned/Unverified |
| MGD-016 3D evidence-or-delete | §15 | W3 | V-3D-01 | R-3D-OPTIONAL,R-SUPPLY | F6 | A-PERF,A-A11Y,A-SBOM,A-REV | Planned/Unverified |
| MGD-017 Relative prediction scores | §§6,11 | WR,WH | V-RET-01,V-RET-02,V-RET-03,V-VIS-01 | R-TRUTH-LAUNDER,R-RETRIEVAL-REGRESSION | F3–F5 | A-RET,A-VIS | Planned/Unverified |
| MGD-018 Current vs Planned truth | §§1,16,20 | WE | orphan/claim lint,V-REG-01 | R-TRUTH-LAUNDER | F0–F5 | A-MAN,A-REV | Planned/Unverified |
| MGD-019 Hard migration/no dual authority | §§1,4,14 | WA,WX | V-SCHEMA-01,V-IO-01 | R-MIGRATION-LOSS,R-AUTH-SPLIT | F1–F5 | A-DB,A-IO | Planned/Unverified |
| MGD-020 Forward-only security rollback | §§5,8.3,13 | WS,WL | V-POLICY-01,V-POLICY-02,V-FAULT-01,V-REC-01 | R-POLICY-LEAK,R-CORRUPT-AUTH | F1–F5 | A-SEC,A-REC | Planned/Unverified |
| MGD-021 Current SVG/dormant 3D truth | §§1,15 | WE,W2,W3 | claim inventory,V-REG-01,V-3D-01 | R-DT-FALSE,R-3D-OPTIONAL | F0/F6 | A-MAN,A-REV | Planned/Unverified |
| MGD-022 Documentation purpose split | §§1,16,20 | WE | orphan/doc lint | R-TRUTH-LAUNDER | F0–F5 | A-MAN | Planned/Unverified |
| MGD-023 SQLite v2 sole authority | §§3–5 | WA | V-SCHEMA-01,V-AUTH-01,V-AUTH-02,V-AUTH-03 | R-AUTH-SPLIT,R-MIGRATION-LOSS | F1 | A-DB,A-PROP | Planned/Unverified |
| MGD-024 Exact SQLiteVectorStore; no ANN | §§3,4.4,6.1 | WR | V-VECTOR-01,V-PERF-01 | R-VECTOR-SCALE,R-MODEL-PIN | F3–F5 | A-RET,A-PERF | Planned/Unverified |
| MGD-025 Five-strategy weighted RRF | §6 | WR | V-RET-01,V-RET-02,V-RET-03 | R-RETRIEVAL-REGRESSION | F3–F5 | A-RET,A-REV | Planned/Unverified |
| MGD-026 Conditional Canvas2D + DOM list | §§10–12 | WH,W2 | V-UI-UNIT-01,V-DT-01,V-A11Y-01,V-RESOURCE-01 | R-CANVAS-WEBKIT,R-A11Y-DRIFT | F4–F5 | A-UI,A-A11Y,A-PERF | Planned/Unverified |
| MGD-027 Crypto-shred unavailable until proven | §5.4 | WL,WS | V-CRYPTO-01,V-LIFE-01 | R-EVENT-PLAINTEXT,R-DELETE-RESIDUE | F1–F5 | A-SEC,A-REC,A-REV | Planned/Unverified |
| MGD-028 3D only after F5 | §§15,18 | W3,WE | predecessor lint,V-3D-01 | R-3D-OPTIONAL | F6 | A-MAN,A-REV | Planned/Unverified |
| MGD-029 Exact license/SBOM review | §§15,17–18 | WX | V-SBOM-01 | R-SUPPLY,R-PROJECT-LICENSE | F1–F6 | A-SBOM,A-REV | Planned/Unverified |
| MGD-030 Seven-destination Digital Twin | §§9–12 | WH | V-DT-01,V-E2E-01,V-VIS-01 | R-DT-FALSE,R-A11Y-DRIFT | F4–F5 | A-UI,A-VIS,A-REV | Planned/Unverified |
| MGD-031 Tool observations cannot escalate | §§4.3,7.4 | WC,WS | V-TOOL-01,V-POLICY-01 | R-TOOL-ESCALATE | F3–F5 | A-PROP,A-SEC | Planned/Unverified |
| MGD-032 Versioned schema/model/interchange | §§4,6,14 | WA,WR,WX | V-SCHEMA-01,V-VECTOR-01,V-IO-01 | R-MIGRATION-LOSS,R-INTERCHANGE-LOSS | F1–F5 | A-DB,A-RET,A-IO | Planned/Unverified |
| MGD-033 One-way core; trace writes governed | §§3,5–8 | WA,WP,WR | V-AUTH-01,V-AUTH-02,V-AUTH-03,V-XPORT-01,V-RET-02 | R-AUTH-SPLIT,R-AUTH-ATOMIC | F1–F3 | A-DB,A-API,A-RET | Planned/Unverified |
| MGD-034 Policy as fail-closed meet | §§4.1,13 | WS | V-POLICY-01,V-POLICY-02 | R-POLICY-LEAK,R-TIMING-LEAK | F1–F5 | A-PROP,A-SEC | Planned/Unverified |
| MGD-035 Semantic revision vs Health status | §§5.2,9–10 | WA,WP,WH | V-AUTH-01,V-UI-UNIT-01,V-DT-01 | R-PATCH-WINDOW,R-DT-FALSE | F1–F5 | A-DB,A-UI | Planned/Unverified |
| MGD-036 Hard Delete is not crypto erasure | §5.4 | WL,WH | V-LIFE-01,V-CRYPTO-01,V-VIS-01 | R-EVENT-PLAINTEXT,R-DELETE-RESIDUE | F1–F5 | A-REC,A-VIS,A-REV | Planned/Unverified |
| MGD-037 Remote profile fail-closed | §§8.3,13 | WS,WP | V-POLICY-02,V-XPORT-01,V-FAULT-01 | R-POLICY-LEAK,R-TIMING-LEAK | F1–F5 | A-SEC,A-API,A-REC | Planned/Unverified |
| MGD-038 One SQLite owns all durable records/events | §§3–5 | WA,WS | V-AUTH-01,V-AUTH-02,V-AUTH-03,V-SCHEMA-01,V-POLICY-01 | R-AUTH-SPLIT,R-AUTH-ATOMIC | F1–F3 | A-DB,A-PROP,A-SEC | Planned/Unverified |
| MGD-039 Pinned 384d model identity | §§4.4,6.1 | WR,WX | V-VECTOR-01,V-SBOM-01 | R-MODEL-PIN,R-VECTOR-SCALE | F3–F5 | A-RET,A-SBOM | Planned/Unverified |
| MGD-040 Canonical Memory Links | §§4.2,7 | WM,WA | V-SEM-01,V-AUTH-01 | R-AUTH-SPLIT,R-TRUTH-LAUNDER | F2 | A-PROP,A-DB | Planned/Unverified |
| MGD-041 Authority corruption→Recovery; derived→Partial | §§5.3–5.4 | WL | V-REC-01,V-REBUILD-01,V-FAULT-01 | R-CORRUPT-AUTH,R-CORRUPT-DERIVED | F1–F5 | A-REC,A-REV | Planned/Unverified |
| MGD-042 Backend-first F0→F6 | §§1–3,18–20 | all | predecessor/orphan lint,all V-* | R-DT-FALSE,R-3D-OPTIONAL | F0–F6 | A-MAN,A-REV | Planned/Unverified |
| MGD-043 Governed consolidation/tool learning | §§4.3,7.3–7.4 | WC,WS | V-CONS-01,V-TOOL-01 | R-CONS-ESCALATE,R-TOOL-ESCALATE | F3–F5 | A-PROP,A-REC,A-REV | Planned/Unverified |
| MGD-044 Checksummed atomic interchange | §14 | WX,WA | V-IO-01,V-SCHEMA-01 | R-INTERCHANGE-LOSS,R-MIGRATION-LOSS | F2–F5 | A-IO,A-DB | Planned/Unverified |
| MGD-045 Supply-chain release gate | §§15,17–18 | WX,W3 | V-SBOM-01,V-3D-01 | R-SUPPLY,R-PROJECT-LICENSE | F5–F6 | A-SBOM,A-REV | Planned/Unverified |
| MGD-046 Canvas evidence-conditional/list complete/3D delete | §§10–12,15 | WH,W2,W3 | V-DT-01,V-A11Y-01,V-RESOURCE-01,V-3D-01 | R-CANVAS-WEBKIT,R-A11Y-DRIFT,R-3D-OPTIONAL | F4–F6 | A-UI,A-A11Y,A-PERF,A-REV | Planned/Unverified |

## 4. Audit Finding Ledger — All 65 Preserved

Each audit row inherits suites, risks, gates, and artifacts through its mapped MGR rows above. Status remains Planned/Unverified until those linked artifacts exist.

### Critical — 7/7

| ID | MGR mapping | Planned disposition | Status |
|---|---|---|---|
| MG-C01 | MGR-001,030,047–048 | truthful renderer state; F6 GO/delete; supply-chain gate | Planned/Unverified |
| MG-C02 | MGR-001,024,034 | provenance-required claims and inspector | Planned/Unverified |
| MG-C03 | MGR-001–002,012,026 | policy-safe scene; generated navigation excluded | Planned/Unverified |
| MG-C04 | MGR-001,025 | Stored/Recalled/Used separation and exact injection trace | Planned/Unverified |
| MG-C05 | MGR-003,020,043 | loopback/remote fail-closed transport matrix | Planned/Unverified |
| MG-C06 | MGR-004,033–035,043 | restrictive policy and cross-path non-interference | Planned/Unverified |
| MG-C07 | MGR-005,033–035 | atomic governed relationship write/idempotency/reversal | Planned/Unverified |

### High — 17/17

| ID | MGR mapping | Planned disposition | Status |
|---|---|---|---|
| MG-H01 | MGR-006,036 | full-corpus five-strategy judged/100k gates | Planned/Unverified |
| MG-H02 | MGR-012,030 | one scene/action model; optional 3D parity/delete | Planned/Unverified |
| MG-H03 | MGR-009,028,045 | blocking, scheduler, observability overhead | Planned/Unverified |
| MG-H04 | MGR-006–007,023 | honest totals/truncation/cursor/query subgraphs | Planned/Unverified |
| MG-H05 | MGR-011,032 | component/community metadata and invalidation | Planned/Unverified |
| MG-H06 | MGR-013,021 | generation/revision/policy-guarded focus | Planned/Unverified |
| MG-H07 | MGR-010,037 | centralized validity and dual-time tests | Planned/Unverified |
| MG-H08 | MGR-005,018 | semantic uniqueness plus multi-evidence | Planned/Unverified |
| MG-H09 | MGR-007,023,032 | bounded ≤3-hop and 100k/evolution seam | Planned/Unverified |
| MG-H10 | MGR-014–016 | composite tab stop, list parity, Orca | Planned/Unverified |
| MG-H11 | MGR-014,024 | inspector focus containment/return | Planned/Unverified |
| MG-H12 | MGR-014,031 | implemented semantic controls only | Planned/Unverified |
| MG-H13 | MGR-002,018,026,034 | visible kind/authority/evidence/provenance | Planned/Unverified |
| MG-H14 | MGR-015,022,045 | Canvas scene/frame/idle/pressure budgets | Planned/Unverified |
| MG-H15 | MGR-015–016,031 | exact responsive/input matrix | Planned/Unverified |
| MG-H16 | MGR-007,017,042 | typed faults, Recovery, derived rebuild | Planned/Unverified |
| MG-H17 | MGR-008,021 | monotonic revisions and patch/window convergence | Planned/Unverified |

### Medium — 28/28

| ID | MGR mapping | Planned disposition | Status |
|---|---|---|---|
| MG-M01 | MGR-015–016 | centroid zoom, bounded pan, fit actions | Planned/Unverified |
| MG-M02 | MGR-015–016,031 | inspector composition at exact dimensions | Planned/Unverified |
| MG-M03 | MGR-013,031 | disjoint single/double activation | Planned/Unverified |
| MG-M04 | MGR-002,007 | endpoint-complete projection | Planned/Unverified |
| MG-M05 | MGR-005,018 | registry direction and identity properties | Planned/Unverified |
| MG-M06 | MGR-013 | focus-scoped prediction race handling | Planned/Unverified |
| MG-M07 | MGR-001,017–018 | relative score/calibration semantics | Planned/Unverified |
| MG-M08 | MGR-010,031 | real timeline capability or omitted control | Planned/Unverified |
| MG-M09 | MGR-012,020,031 | typed capability-authorized actions | Planned/Unverified |
| MG-M10 | MGR-015,031 | distinct camera actions | Planned/Unverified |
| MG-M11 | MGR-011,026 | generated facets not ontology | Planned/Unverified |
| MG-M12 | MGR-005,018 | relation write validation | Planned/Unverified |
| MG-M13 | MGR-019,024 | conservative merge/split/reversal | Planned/Unverified |
| MG-M14 | MGR-019,034 | mention locator/extractor/version provenance | Planned/Unverified |
| MG-M15 | MGR-032,042 | stable graph/analytics/vector ports | Planned/Unverified |
| MG-M16 | MGR-009 | batched endpoint/evidence reads | Planned/Unverified |
| MG-M17 | MGR-007,009 | bounded cycle-safe traversal | Planned/Unverified |
| MG-M18 | MGR-020 | canonical DTO/error/capability parity | Planned/Unverified |
| MG-M19 | MGR-021 | per-window ownership/cache keys | Planned/Unverified |
| MG-M20 | MGR-012,030 | optional 3D same scene or deletion | Planned/Unverified |
| MG-M21 | MGR-030,047 | integrated LOD/culling/license gate | Planned/Unverified |
| MG-M22 | MGR-022,030 | packed worker/allocation/idle profile | Planned/Unverified |
| MG-M23 | MGR-022,030 | bounded label collision/dirty update | Planned/Unverified |
| MG-M24 | MGR-014–016,026 | size/zoom/touch/typography matrix | Planned/Unverified |
| MG-M25 | MGR-014,026 | redundant encoding/CVD/forced colors | Planned/Unverified |
| MG-M26 | MGR-022 | finite motion and idle stop | Planned/Unverified |
| MG-M27 | MGR-027–028,047 | manifests, visual/perf/SBOM suites | Planned/Unverified |
| MG-M28 | MGR-029,048 | documentation authority/audit continuity | Planned/Unverified |

### Low — 13/13

| ID | MGR mapping | Planned disposition | Status |
|---|---|---|---|
| MG-L01 | MGR-001–002,034 | entity/memory terminology and typed records | Planned/Unverified |
| MG-L02 | MGR-017,031 | exact stale/offline/error states | Planned/Unverified |
| MG-L03 | MGR-018,026 | present-only semantic legend | Planned/Unverified |
| MG-L04 | MGR-014,031 | compact controls with accessible names/state | Planned/Unverified |
| MG-L05 | MGR-006,016,031 | platform-correct implemented shortcuts | Planned/Unverified |
| MG-L06 | MGR-031,046 | goal-led consent-aware onboarding | Planned/Unverified |
| MG-L07 | MGR-001,008,031 | exact revision/time status | Planned/Unverified |
| MG-L08 | MGR-026 | ≥14px body/≥12px graph labels | Planned/Unverified |
| MG-L09 | MGR-015–016 | ultrawide adaptive layout | Planned/Unverified |
| MG-L10 | MGR-014,026 | ≥2px AA focus and forced-color tokens | Planned/Unverified |
| MG-L11 | MGR-017,031 | retry/preserved intent/correlation ID | Planned/Unverified |
| MG-L12 | MGR-017,030 | context/theme fallback or 3D deletion | Planned/Unverified |
| MG-L13 | MGR-015,031 | fit visible/selection/neighborhood | Planned/Unverified |
## 5. Opportunity Ledger — All 31 Preserved

Opportunities remain Planned/Unverified and cannot bypass prerequisite MGR gates.

| ID | MGR mapping | Future work/disposition | Status |
|---|---|---|---|
| MG-O01 | MGR-025,031 | answer-to-memory trace workflow | Planned/Unverified |
| MG-O02 | MGR-018,024 | evidence-backed relationship inspector | Planned/Unverified |
| MG-O03 | MGR-019,024,034 | entity-to-source drill-down | Planned/Unverified |
| MG-O04 | MGR-001,024,037 | verification/resolution actions | Planned/Unverified |
| MG-O05 | MGR-006,036 | five-strategy full-corpus search | Planned/Unverified |
| MG-O06 | MGR-007,023 | query-defined subgraphs | Planned/Unverified |
| MG-O07 | MGR-007,023 | bounded ego-depth controls | Planned/Unverified |
| MG-O08 | MGR-007,018,024 | evidence-bearing path explanation | Planned/Unverified |
| MG-O09 | MGR-010,032 | revision/valid-time diff | Planned/Unverified |
| MG-O10 | MGR-024,026,031 | grounded Health overlays | Planned/Unverified |
| MG-O11 | MGR-005,024,037 | contradiction resolution | Planned/Unverified |
| MG-O12 | MGR-019,024 | merge/split workspace | Planned/Unverified |
| MG-O13 | MGR-018 | relation registry manager | Planned/Unverified |
| MG-O14 | MGR-011,023 | validated community summaries after quality gate | Planned/Unverified |
| MG-O15 | MGR-011,023,032 | bridge/hub/orphan analytics | Planned/Unverified |
| MG-O16 | MGR-017–018,036 | prediction rationale; calibration only with corpus | Planned/Unverified |
| MG-O17 | MGR-015,021 | saved-view/bookmark seam after core | Planned/Unverified |
| MG-O18 | MGR-013,015 | camera/navigation history | Planned/Unverified |
| MG-O19 | MGR-012 | renderer-neutral scene | Planned/Unverified |
| MG-O20 | MGR-030,047–048 | F6 true-3D GO/delete | Planned/Unverified |
| MG-O21 | MGR-023,032 | semantic aggregation/zoom | Planned/Unverified |
| MG-O22 | MGR-008,020 | incremental revision patches | Planned/Unverified |
| MG-O23 | MGR-015,022,045 | low-power quality ladder | Planned/Unverified |
| MG-O24 | MGR-028 | redacted observability | Planned/Unverified |
| MG-O25 | MGR-014 | equivalent graph composite/list | Planned/Unverified |
| MG-O26 | MGR-016 | touch-first scoped 2D | Planned/Unverified |
| MG-O27 | MGR-004,026 | non-leaking policy cues | Planned/Unverified |
| MG-O28 | MGR-004,023,043 | namespace/project/source queries | Planned/Unverified |
| MG-O29 | MGR-032 | versioned interchange round trip | Planned/Unverified |
| MG-O30 | MGR-010,034 | source-moment replay only with evidence links | Planned/Unverified |
| MG-O31 | MGR-024,031,044 | grounded gap-to-action after validated analytics | Planned/Unverified |

## 6. Coverage Totals and Orphan Checks

| Register | Expected IDs | Present mappings | Verified by linked evidence now | Missing/orphan allowed |
|---|---:|---:|---:|---:|
| Requirements MGR-001–048 | 48 | 48 | 0 | 0 |
| Decisions MGD-001–046 | 46 | 46 | 0 | 0 |
| Critical findings MG-C01–07 | 7 | 7 | 0 | 0 |
| High findings MG-H01–17 | 17 | 17 | 0 | 0 |
| Medium findings MG-M01–28 | 28 | 28 | 0 | 0 |
| Low findings MG-L01–13 | 13 | 13 | 0 | 0 |
| **All findings** | **65** | **65** | **0** | **0** |
| Opportunities MG-O01–31 | 31 | 31 | 0 | 0 |

The planned coverage linter must parse this ledger plus requirements, decisions, design §20, validation, risk, and roadmap and fail when: an expected ID is absent/duplicated/out of range; any MGR or MGD lacks design/workstream/suite/risk/gate/artifact mappings; any suite/risk/workstream/artifact code is undefined; an audit ID maps to no MGR; a later gate lacks predecessor evidence; a status other than Planned/Unverified lacks a valid manifest path/hash; or a claimed pass points only to a checklist/manual assertion. It must also report reverse orphans: suites, risks, artifact classes, and workstreams with no governing MGR/MGD.

## 7. Status Transition Rule

`Planned/Unverified → In progress → Implemented/Unverified → Verified` requires linked code, exact executable suite output, artifact checksums, quantitative thresholds, required manual reviews, predecessor gate manifests, and no blocking risk. `Deferred` cannot hide a P0 or launch-blocking Critical/High item. `Rejected` requires an explicit replacement decision and complete remapping. An F6 NO-GO remains a verified decision only after optional code, controls, dependencies, assets, tests, and claims are removed.
