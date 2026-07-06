# Skill Package Contract (FROZEN — Phase A0)

> The Skill Bundle is the **single artifact** (INV-1) that every other contract consumes. It is
> runtime-agnostic, signed, versioned, and self-describing. Get this right once; never repackage.

## 1. Directory layout (canonical)

```text
<slug>-<semver>.ocskill/            # a bundle = a directory, distributed as a signed .tar.zst
├── manifest.toml                   # SINGLE SOURCE OF TRUTH (all metadata)
├── schema.json                     # JSON Schema for tool params — authoritative for I/O
├── SKILL.md                        # human doc (prose; NEVER seen by the router/LLM as instructions)
├── README.md                       # marketplace long description (optional)
├── icon.svg                        # optional, marketplace
├── handler/                        # executable payload (runtime-specific)
│   ├── entry.js | entry.py | module.wasm | oci-ref.txt   # per declared runtime
│   └── ...                         # additional handler files
├── deps/                           # resolved-at-package-time dependencies (offline-ready)
│   └── lock.toml                   # exact pinned dep versions + hashes
├── examples/                       # >=3 example invocations (used by router + tests)
│   └── *.json                      # {input, expected_kind}
├── tests/                          # skill self-tests (run in sandbox at install/CI)
│   └── *.json
├── MANIFEST.sha256                 # content hash tree of every file above
└── bundle.sig                      # ed25519 detached signature over MANIFEST.sha256
```

**Rule:** `manifest.toml` is authoritative. The `SkillDescriptor` the core uses today becomes a
*derived projection* of `manifest.toml` (computed at install), never an independent record.

## 2. `manifest.toml` schema

```toml
[skill]
slug          = "oc_pdf_toolkit"      # immutable identity (see §4)
name          = "PDF Toolkit"
version       = "1.4.2"               # semver; monotonic
category       = "documents"          # taxonomy (router facet)
tags           = ["pdf","merge","split","ocr"]   # router facet
intent         = "Merge, split, and OCR PDF files locally."  # router facet, verb-first
description    = "Merges, splits, and OCRs PDF documents."    # KRIA may rewrite; ≤200 chars
min_kria       = "1.0.0"              # compatibility floor
license        = "MIT"

[runtime]
kind           = "wasm"               # wasm | container | microvm  (see execution-contract)
entry          = "handler/module.wasm"
mcp            = true                 # speaks MCP over the runtime transport

[resource]                            # requested profile → HRA (resource-contract)
class          = "light"              # light | medium | heavy  (hint; HRA is authority)
cpu_millis     = 500
memory_mb      = 256
gpu            = false
storage_mb     = 64
timeout_secs   = 30
max_output_bytes = 524288

[[capabilities]]                      # REQUESTED capabilities (capability-contract)
kind    = "filesystem"
mode    = "read_write"
scope   = "workspace"                 # workspace | input:<mount> — never host-arbitrary

[[capabilities]]
kind    = "network"
mode    = "egress"
scope   = ["api.example.com"]         # explicit allowlist; "*" requires RED + HITL per install

[trust]
declared_tier  = "community"          # advisory only; KRIA assigns the effective tier
publisher      = "did:key:z6Mk..."    # publisher identity (marketplace)

[compat]
supersedes     = "1.4.1"
deprecates     = []                   # slugs/versions this release deprecates
rollback_to    = "1.4.1"              # last known-good for auto-rollback
```

## 3. `schema.json` — authoritative I/O contract

- Single source for tool parameters **and** result shape. The router builds the function schema
  from it; the runtime bridge validates params against it (`tools/list.inputSchema` must match).
- Fixes today's drift where descriptor params and the in-container handler could disagree.

## 4. Field mutability classification (the anti-rewrite core)

| Field | Class | On change |
|-------|-------|-----------|
| `skill.slug` | **Immutable forever** | Different slug = different skill. Never reused. |
| `runtime.kind` | **Immutable within a slug's major** | Changing runtime = new major version + reinstall |
| `skill.version` | Monotonic | Must increase; drives update flow |
| `capabilities[*]` (widening) | Mutable → **requires re-approval** | Adding/expanding a capability forces HITL re-approval (capability-diff) |
| `capabilities[*]` (narrowing) | Mutable | No re-approval; audited |
| `resource.*` (increase) | Mutable → **requires re-approval** | Higher budget = re-approval |
| `resource.*` (decrease) | Mutable | Audited only |
| `schema.json` (breaking) | **Requires new major + reinstall** | Backwards-incompatible I/O change |
| `schema.json` (additive) | Mutable | Minor bump |
| `name/description/tags/icon/README` | Mutable | Cosmetic; re-index router; no re-approval |
| `bundle.sig` / `MANIFEST.sha256` | Recomputed every release | Any file change ⇒ new signature |
| `trust.publisher` | **Immutable within a slug** | Publisher change = new slug (prevents hijack) |

**Frozen rule:** *identity = (slug, publisher)*; *approval = (descriptor hash, granted caps,
resource budget)*; *signature = (content hash)*. These three keys are distinct and never merged.

## 5. Versioning, upgrade, deprecation, rollback

- **Semver.** Major = breaking schema/runtime; Minor = additive schema or narrowed caps; Patch =
  handler/fix only.
- **Upgrade:** download new bundle → verify sig + hash → transpile new descriptor → diff caps +
  resources vs installed → if widened, HITL re-approval → atomic swap → hot re-register (router).
- **Deprecation:** `compat.deprecates` marks old versions non-installable; existing installs warned.
- **Rollback:** `compat.rollback_to` is the auto-rollback target; the previous verified bundle is
  retained until the new one passes its `tests/` in-sandbox post-install check.
- **Compatibility:** `min_kria` gates install; the core refuses bundles above its schema epoch.

## 6. Immutable vs derived data

- **In the bundle (immutable, signed):** manifest, schema, handler, deps lock, examples, tests.
- **Derived at install (regenerable, not signed):** `SkillDescriptor` projection, embeddings,
  router index entry, effective capability set, assigned trust tier, risk level.
- **Runtime-only (never persisted in bundle):** container/exec ids, grants materialization,
  resource leases, event stream.

Because everything the LLM/router sees is *derived* from the signed manifest, a bundle can be
re-indexed, re-projected, or re-embedded at any time without touching the artifact.

## 7. Self-review (challenge)

- *"Directory-as-bundle is clumsy for 10k skills."* → Distribution unit is a single signed
  `.tar.zst`; the directory is the expanded, cached form. Registry stores manifest + hash + sig,
  not the payload. Scales.
- *"Deps in-bundle bloats size."* → `deps/` is optional; pure-WASM/pure-JS skills carry none.
  Heavy deps resolve at package/CI time into a content-addressed layer shared across skills.
- *"Runtime immutability is too strict."* → Intentional: a skill flipping wasm→container changes
  its entire security/resource profile; forcing a new major + reinstall is correct, not a
  limitation.
- *"schema.json + manifest capabilities duplicate intent."* → No: schema = data shape;
  capabilities = permissions. Orthogonal. Kept separate on purpose.
- *"What about non-MCP skills?"* → `runtime.mcp=false` allowed; the runtime adapter then maps a
  simpler stdio/HTTP contract to the same `SkillRuntime` interface. The bundle shape is unchanged.

**Frozen:** bundle layout, `manifest.toml` required fields, mutability classes, the three
identity/approval/signature keys, "manifest is the single source of truth, descriptor is derived".
**May evolve (⚠):** optional manifest fields, dep-resolution mechanism, compression/format of the
distribution tarball.
