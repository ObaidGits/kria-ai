# GRAPHIFY — ESSENTIAL DEPLOYMENT & USAGE GUIDE
## Minimal Documentation for Fast Setup (Ubuntu / Linux)

Graphify provides **codebase graph intelligence** using:
- **Static AST parsing** → free, local
- **Semantic embeddings** → local, zero API cost
- **LLM extraction** → optional, paid, deeper summaries

Use this when you want:
- codebase mapping
- dependency tracing
- architecture overview
- semantic search
- AI-assisted navigation of large repositories

---

# 1) INSTALL GRAPHIFY

## Core Install
```bash
pip install graphifyy
```

Initialize Graphify:
```bash
graphify install
```

This sets up:
- default config
- runtime environment
- Graphify skill files
- command integration

Verify:
```bash
graphify --help
```

---

## Optional: Local Semantic Engine (Recommended)
For zero-token semantic understanding:

```bash
pip install "graphifyy[embeddings]"
```

Enables:
- semantic similarity mapping
- intent clustering
- code meaning search
- local vector generation

No API usage.

---

# 2) OPTIONAL: INSTALL OLLAMA (FOR LOCAL EMBEDDINGS)

Required only for semantic graph generation.

Install:
```bash
curl -fsSL https://ollama.com/install.sh | sh
```

Pull embedding model:
```bash
ollama run nomic-embed-text
```

Alternative models:
```bash
ollama pull mxbai-embed-large
ollama pull snowflake-arctic-embed
```

Recommended:
```bash
nomic-embed-text
```

Lightweight + fast.

Verify:
```bash
ollama list
```

---

# 3) IDE INTEGRATION

Install integration from project root.

---

## Cursor
```bash
graphify cursor install
```

Creates:
```text
.cursor/rules/graphify.mdc
```

Purpose:
- AI understands graph structure
- improves repo navigation
- better contextual answers

---

## VS Code / GitHub Copilot
```bash
graphify vscode install
```

Creates:
```text
.github/copilot-instructions.md
```

Purpose:
- injects Graphify workflow into Copilot
- enables graph-aware prompts

---

## Windsurf / Trae
```bash
graphify trae install
```

Creates:
```text
AGENTS.md
```

Purpose:
- gives AI project graph instructions

---

## Claude Code
```bash
graphify claude install
```

Creates:
```text
CLAUDE.md
```

---

## Reinstall / Refresh IDE Rules
If instructions become outdated:

```bash
graphify cursor install --force
graphify vscode install --force
graphify trae install --force
graphify claude install --force
```

---

# 4) INITIAL GRAPH BUILD

From project root:

```bash
graphify update .
```

Builds:
- dependency graph
- AST relationships
- imports
- symbols
- module clusters

Output:

```text
graphify-out/
```

Contains:
- graph report
- JSON graph
- HTML visualization

Run after first install.

---

# 5) WATCH MODE (AUTO UPDATE)

Continuous sync:

```bash
graphify watch .
```

Whenever file changes:
- graph updates
- nodes refresh
- dependencies re-linked

Best during active development.

Stop:

```bash
CTRL+C
```

---

# 6) LOCAL SEMANTIC GRAPH (ZERO TOKEN)

Best mode.

Build semantic understanding locally:

```bash
graphify update . --embeddings
```

Graphify adds:
- semantic edges
- intent similarity
- logical grouping
- meaning-based search

No API.

No token cost.

Fully local.

---

## More Precise Semantic Matching
```bash
graphify update . --embeddings --embed-threshold 0.85
```

Threshold guide:

| Value | Meaning |
|---|---|
| 0.65 | loose match |
| 0.75 | balanced |
| 0.85 | strict |
| 0.90 | very strict |

Recommended:

```bash
0.85
```

---

# 7) FULL AI EXTRACTION (OPTIONAL)

Deep logic summarization:

```bash
graphify extract .
```

Adds:
- module summaries
- intent summaries
- architecture explanation
- richer node metadata

Uses LLM.

Usually paid / API-backed.

Use only when needed.

---

# 8) CLEAN REBUILD

If graph becomes stale:

```bash
graphify update . --force
```

Rebuilds from scratch.

Useful after:
- major refactor
- deleted modules
- moved folders
- renamed packages

---

# 9) CLUSTER-ONLY MODE

For huge repos:

```bash
graphify cluster-only .
```

Produces:
- architecture grouping
- community clusters
- reduced graph complexity

Useful for:
- monorepos
- 5k+ nodes
- enterprise codebases

---

# 10) LARGE GRAPH VISUALIZATION

Increase visualization limit:

```bash
export GRAPHIFY_VIZ_NODE_LIMIT=10000
```

Then:

```bash
graphify cluster-only .
```

Useful when default HTML graph is truncated.

---

# 11) OUTPUT FILES

Everything goes into:

```text
graphify-out/
```

Important files:

---

## GRAPH_REPORT.md
Main file.

Contains:
- module map
- cluster summary
- dependency overview
- important nodes

Use in prompts:

```text
Using @GRAPH_REPORT.md explain auth flow
```

or

```text
Using @GRAPH_REPORT.md locate renderer entrypoint
```

---

## graph.json
Machine-readable graph.

Useful for:
- scripts
- tooling
- analytics
- custom visualization

---

## graph.html
Interactive graph.

Open:

```bash
xdg-open graphify-out/graph.html
```

Best for:
- dependency inspection
- architecture browsing
- visual clusters

---

# 12) .GRAPHIFYIGNORE (IMPORTANT)

Create:

```text
.graphifyignore
```

Example:

```text
node_modules/
target/
.git/
__pycache__/
build/
dist/
coverage/
.cache/
.next/
out/
*.min.js
*.lock
```

Benefits:
- faster scan
- cleaner graph
- less noise
- lower memory use

Recommended for every project.

---

# 13) COMMON WORKFLOW

## Minimal
```bash
pip install graphifyy
graphify install
graphify update .
```

Done.

---

## Recommended
```bash
pip install graphifyy
pip install "graphifyy[embeddings]"
graphify install

ollama run nomic-embed-text

graphify update . --embeddings
graphify watch .
```

Best setup.

---

## Large Project
```bash
graphify update . --embeddings --embed-threshold 0.85
graphify cluster-only .
```

Best for monorepo / enterprise.

---

# 14) USEFUL PROMPTS

Navigation:
```text
Using @GRAPH_REPORT.md find service entrypoint
```

Dependency:
```text
Using @GRAPH_REPORT.md trace payment dependencies
```

Architecture:
```text
Summarize major clusters from @GRAPH_REPORT.md
```

Refactor:
```text
Using @GRAPH_REPORT.md identify tightly coupled modules
```

Impact analysis:
```text
Which modules break if cache layer changes?
```

---

# 15) QUICK COMMAND REFERENCE

| Task | Command |
|---|---|
| install | `pip install graphifyy` |
| initialize | `graphify install` |
| AST graph | `graphify update .` |
| semantic graph | `graphify update . --embeddings` |
| strict semantic | `graphify update . --embeddings --embed-threshold 0.85` |
| watch | `graphify watch .` |
| clean rebuild | `graphify update . --force` |
| AI extraction | `graphify extract .` |
| cluster only | `graphify cluster-only .` |
| Cursor install | `graphify cursor install` |
| VS Code install | `graphify vscode install` |
| Windsurf install | `graphify trae install` |
| Claude install | `graphify claude install` |

---

# RECOMMENDED DEFAULT SETUP

Run:

```bash
pip install graphifyy
pip install "graphifyy[embeddings]"

graphify install

ollama run nomic-embed-text

graphify cursor install      # or vscode / trae / claude

graphify update . --embeddings --embed-threshold 0.85

graphify watch .
```

This is the practical full setup.