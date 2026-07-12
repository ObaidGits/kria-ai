# Design — Settings Intelligence (production NL settings OS)

Reuses the shipped backbone (`ConfigService`, `FieldMeta`/schema, `SettingsHandler`, `HitlGateway`,
`AuditLogger`, `RequestOverride`, `run_settings_stage`, `config_prompt` command). This design adds
the intelligence + generalization layers on top, with strict no-hardcoding.

## Architecture (layers)

```
 message + session
   │
   ├─(0) Cheap domain gate ── semantic score vs schema corpus; below floor ⇒ NotSettings (fast out)
   │
   ├─(1) Evidence collectors (parallel, all optional, degrade gracefully)
   │      • SchemaEntityIndex  : field candidates (tier-A lexical + tier-B embedding)
   │      • ValueEngine        : typed value extraction (kind/bounds/aliases)  ← R2
   │      • ConversationEvidence: topic embedding + subject scope from `messages`
   │      • MemoryEvidence     : recalled facts/RAG topic (optional)           ← A2
   │      • IntentKind         : imperative / interrogative / undo / help / catalog / config-flow
   │
   ├─(2) Confidence fusion ── documented weighted model → confidence ∈[0,1] + SettingsIntentTrace
   │      bands: ≥act ⇒ act ; clarify-band ⇒ ONE question ; <clarify ⇒ NotSettings (fail→convo)
   │
   ├─(3) Decision  ─ Change | ReadBack | Undo | TempOverride | Help | Catalog | Explain
   │                 | ConfigFlow(step) | Clarify | NotSettings
   │
   ├─(4) SlotFiller (multi-turn) ── for ConfigFlow: accumulate known fields, resolve target/provider,
   │      ask only missing, confirm, commit; secrets→vault; per-session, resumable, expiring  ← R4
   │
   └─(5) SettingsHandler ── validate(ValueEngine) → availability → injection wall → risk gate
          (GREEN auto / else NeedsApproval→caller HITL→apply_approved) → ConfigService → audit → event
          ReadBack/Catalog/Help/Explain ⇒ read-only answer from schema + ConfigService + audit
```

## Components

### C1. `FieldMeta` v2 (schema metadata is the single knowledge source — no hardcoding)
Add to `config/schema.rs::FieldMeta`: `value_kind: ValueKind`, `min`/`max: Option<f64>`,
`aliases: &[(&str,&str)]` (natural→canonical value), `label: &str`, `description: &str`,
`help: &str`, `subject_scope` (does this field make sense as "yours/the app's"). Keep it `const`.
`ValueKind = Bool | Int | Float | Enum | Str | Url | Path | Duration | LangCode | ModelId |
ProviderName | List(Box<ValueKind>)`. To avoid rewriting 22 literals, give `FieldMeta` a
`const fn base()` + builder-style `const` updates, or a macro `field!{…}`; unannotated ⇒ fail-closed.
Coverage task fills ALL user-facing fields.

### C2. `ValueEngine` (`config/nl/value.rs` — new; universal, schema-driven)
`extract(field_meta, text) -> Result<Value, ValueError>` and `validate(field_meta, value)`.
Type-aware parsing: numbers/floats (with range), booleans (generic on/off/enable/disable/yes/no),
enums (normalize spacing/underscore/case + alias table), URLs/paths (format check), durations
("30s","5 min"), lang codes ("French"→fr via alias), lists ("a, b and c"). Coercion for wrong-typed
JSON. NO per-field branches — everything reads `FieldMeta`. Optional LLM-constrained fallback
(grammar/structured) behind a trait when tier-A fails (R2.4).

### C3. Evidence-based intent (`config/nl/pipeline.rs` rework + `evidence.rs`)
Replace ad-hoc branch scoring with the layered collectors + documented weighted fusion (R1.3).
`ConversationEvidence`/`MemoryEvidence` provide topic + subject scope via **semantic similarity**
(embedder when present) with the lexical markers demoted to a low-weight fallback, not the authority.
Emits `SettingsIntentTrace{stage scores, evidence, band, decision}` — persisted (R7.3).

### C4. `SlotFiller` (`config/nl/flow.rs` — new; multi-turn config)
Per-session `ConfigFlowState{ target (provider/section), known: Map<field,Value>, missing: Vec,
pending_secret: bool }`. Resolves target from context, merges each turn's extracted fields, computes
missing from the target's required schema, asks only for missing, confirms, commits atomically via
`patch_batch` (+ vault for secrets). Resumable within session; TTL-expires; never persists partial.
Providers: model the `providers: Vec<ProviderConfig>` as an addressable target set.

### C5. `SettingsCatalog` (`config/nl/catalog.rs` — new; answer-from-system)
Catalog/Help/Explain/Read-all built from `full_schema_json()` + `FieldMeta.label/description/help` +
`AuditLogger.config_change_history`. No LLM for these (R5). Groups by section; lists providers/models
from config; explains valid values/restart/env-lock/why-locked.

### C6. Minimal routing / no interference (`agent/loop_engine` + routing)
Settings turn ⇒ claim before forcing/semantic/LLM (already done). Add a **negative-evidence gate**
so knowledge tools (`search_marketplace`, `recall_fact`) require minimum routing confidence and do
not fire on general questions (D1). Keep the cheap domain gate so non-settings pays ~nothing (R6.2).

### C7. Locking/observability (`handler.rs`, `service.rs`)
Optional expected_version threading (R7.1); pending-approval TTL/GC (E2); HITL timeout + turn release
(E3); persist `SettingsIntentTrace` to a bounded diagnostics ring + audit (E4/R7.3).

## No-hardcoding guarantees
- All field/value/subject/help knowledge lives in `FieldMeta`/schema, not in routing code.
- The synthetic-field test (add a field with metadata only) must prove set/read/help/catalog all
  work with zero routing edits (R3.2).

## Correctness properties (extend prior P1–P10)
- **P11 Evidence separation:** identical phrase resolves differently given different conversation/
  memory evidence (KRIA vs topic) — proven by paired tests.
- **P12 Universal values:** int/float/bool/enum/url/path/lang/list all extract+validate from natural
  text with no per-field code.
- **P13 Slot-filling convergence:** one-at-a-time and all-at-once provider config converge to the
  same committed state; partial never persists.
- **P14 Answer-from-system:** catalog/help/explain/read-back never invoke the LLM and never
  hallucinate; values match ConfigService/schema/audit.
- **P15 No interference:** a curated general-query set triggers ZERO settings mutations/reads and no
  unnecessary tools.

## Testing
Backend golden + P1–P15; real-frontend WebDriver across all categories + providers/keys/models +
restart; adversarial (ambiguous/multilingual/Hinglish/typo/incomplete/chained/pronoun/memory-ref).
Mark on-box-unverifiable (keychain) honestly.
