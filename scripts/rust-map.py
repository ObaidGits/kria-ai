#!/usr/bin/env python3
"""Rust code map generator for KRIA — the Rust counterpart to the CodeScout TS graph.

CodeScout only analyses TypeScript/JavaScript, so ~660k lines of Rust in crates/
are invisible to it. This script builds a compact module index so an agent can
read ONE file (.codescout/RUST_MAP.md) instead of grepping 1200+ .rs files.

Pure static analysis: standard library only, no network, no LLM, no API key.

Usage:
    python3 scripts/rust-map.py            # rebuild both outputs
    python3 scripts/rust-map.py --json-only
    python3 scripts/rust-map.py --quiet
    python3 scripts/rust-map.py --check     # exit 1 if map is stale (for cron)
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CRATES = REPO / "crates"
OUT_DIR = REPO / ".codescout"
JSON_OUT = OUT_DIR / "rust-map.json"
MD_OUT = OUT_DIR / "RUST_MAP.md"

MD_SIZE_BUDGET = 200 * 1024
HOTSPOT_COUNT = 40

# --- regexes (line-oriented; cheap and good enough for an index) -------------
RE_MOD_DECL = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
RE_USE = re.compile(r"^\s*(?:pub\s+)?use\s+([^;]+);")
RE_PUB_FN = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\"[^\"]*\"\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)"
)
RE_PUB_STRUCT = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)struct\s+([A-Za-z_][A-Za-z0-9_]*)")
RE_PUB_ENUM = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)enum\s+([A-Za-z_][A-Za-z0-9_]*)")
RE_PUB_TRAIT = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)(?:unsafe\s+)?trait\s+([A-Za-z_][A-Za-z0-9_]*)")
RE_PUB_TYPE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)type\s+([A-Za-z_][A-Za-z0-9_]*)")
RE_IMPL_FOR = re.compile(r"^\s*impl(?:\s*<[^>]*>)?\s+([A-Za-z_][\w:<>, ]*?)\s+for\s+([A-Za-z_][\w:<>, ]*?)\s*(?:\{|where)")
RE_TAURI_CMD = re.compile(r"^\s*#\[\s*tauri::command")
RE_ANY_FN = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")
RE_CFG_TEST = re.compile(r"^\s*#\[\s*cfg\s*\(\s*test\s*\)")
RE_TEST_FN = re.compile(r"^\s*#\[\s*(?:tokio::)?test")
RE_CARGO_NAME = re.compile(r'^\s*name\s*=\s*"([^"]+)"')


def crate_names() -> dict[str, Path]:
    """Map crate name (from Cargo.toml) -> crate dir."""
    out: dict[str, Path] = {}
    if not CRATES.is_dir():
        return out
    for d in sorted(CRATES.iterdir()):
        manifest = d / "Cargo.toml"
        if not (d.is_dir() and manifest.is_file()):
            continue
        name = d.name
        try:
            in_pkg = False
            for line in manifest.read_text(encoding="utf-8", errors="replace").splitlines():
                s = line.strip()
                if s.startswith("["):
                    in_pkg = s == "[package]"
                    continue
                if in_pkg:
                    m = RE_CARGO_NAME.match(line)
                    if m:
                        name = m.group(1)
                        break
        except OSError:
            pass
        out[name] = d
    return out


def module_key(crate: str, src_root: Path, path: Path) -> str:
    """crates/kria-core/src/agent/loop_engine/mod.rs -> kria-core::agent::loop_engine"""
    rel = path.relative_to(src_root)
    parts = list(rel.parts)
    stem = Path(parts[-1]).stem
    if stem in ("mod", "lib", "main"):
        parts = parts[:-1]
    else:
        parts[-1] = stem
    return "::".join([crate] + parts) if parts else crate


def norm_use(raw: str) -> list[str]:
    """Flatten a use statement into leading paths. `a::{b::c, d}` -> [a::b::c, a::d]"""
    raw = " ".join(raw.split()).replace(" as ", "::")
    results: list[str] = []
    if "{" in raw:
        prefix, _, rest = raw.partition("{")
        prefix = prefix.strip()
        inner = rest.rsplit("}", 1)[0]
        depth = 0
        buf = ""
        for ch in inner:
            if ch == "," and depth == 0:
                results.append(prefix + buf.strip())
                buf = ""
                continue
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
            buf += ch
        if buf.strip():
            results.append(prefix + buf.strip())
    else:
        results.append(raw)
    cleaned = []
    for r in results:
        r = r.replace("{", "").replace("}", "").strip().rstrip(":")
        if r:
            cleaned.append(r)
    return cleaned


def scan_file(path: Path, crate: str, src_root: Path) -> dict | None:
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    lines = text.splitlines()
    rec: dict = {
        "module": module_key(crate, src_root, path),
        "crate": crate,
        "file": str(path.relative_to(REPO)),
        "lines": len(lines),
        "mods": [],
        "intra": [],
        "cross": [],
        "external": [],
        "symbols": [],
        "tauri": [],
        "hasTests": False,
        "testCount": 0,
    }
    pending_tauri = False
    for i, line in enumerate(lines, 1):
        if RE_CFG_TEST.match(line):
            rec["hasTests"] = True
        if RE_TEST_FN.match(line):
            rec["testCount"] += 1
        m = RE_MOD_DECL.match(line)
        if m:
            rec["mods"].append(m.group(1))
            continue
        m = RE_USE.match(line)
        if m:
            for p in norm_use(m.group(1)):
                head = p.split("::", 1)[0]
                if head in ("crate", "self", "super"):
                    rec["intra"].append(p)
                elif head.startswith("kria_"):
                    rec["cross"].append(p)
                else:
                    rec["external"].append(head)
            continue
        if RE_TAURI_CMD.match(line):
            pending_tauri = True
            continue
        for rx, kind in (
            (RE_PUB_FN, "fn"),
            (RE_PUB_STRUCT, "struct"),
            (RE_PUB_ENUM, "enum"),
            (RE_PUB_TRAIT, "trait"),
            (RE_PUB_TYPE, "type"),
        ):
            m = rx.match(line)
            if m:
                rec["symbols"].append({"n": m.group(1), "k": kind, "l": i})
                break
        m = RE_IMPL_FOR.match(line)
        if m:
            rec["symbols"].append(
                {"n": f"{m.group(1).strip()} for {m.group(2).strip()}", "k": "impl", "l": i}
            )
        if pending_tauri:
            m = RE_ANY_FN.search(line)
            if m:
                rec["tauri"].append({"n": m.group(1), "l": i})
                pending_tauri = False
            elif line.strip() and not line.strip().startswith("#["):
                pending_tauri = False
    return rec


def resolve_intra(dep: str, module: str, crate: str, known: set[str]) -> str | None:
    """Resolve a crate::/self::/super:: use path to the longest matching module key."""
    head, _, rest = dep.partition("::")
    if head == "crate":
        base = crate
        segs = rest.split("::") if rest else []
    elif head == "self":
        base = module
        segs = rest.split("::") if rest else []
    elif head == "super":
        parts = module.split("::")
        segs = rest.split("::") if rest else []
        while dep.startswith("super::") or dep == "super":
            parts = parts[:-1] if len(parts) > 1 else parts
            if not rest.startswith("super"):
                break
            dep = rest
            rest = dep.partition("::")[2]
            segs = rest.split("::") if rest else []
        base = "::".join(parts)
    else:
        return None
    candidate = base
    for seg in segs:
        nxt = f"{candidate}::{seg}"
        if nxt in known or any(k.startswith(nxt + "::") for k in known):
            candidate = nxt
        else:
            break
    return candidate if candidate in known and candidate != module else None


def resolve_cross(dep: str, known: set[str]) -> str | None:
    head, _, rest = dep.partition("::")
    base = head.replace("_", "-")
    candidate = base
    for seg in (rest.split("::") if rest else []):
        nxt = f"{candidate}::{seg}"
        if nxt in known or any(k.startswith(nxt + "::") for k in known):
            candidate = nxt
        else:
            break
    return candidate if candidate in known else None


def build() -> tuple[dict, int]:
    crates = crate_names()
    records: list[dict] = []
    skipped = 0
    for crate, cdir in crates.items():
        src = cdir / "src"
        if not src.is_dir():
            continue
        for f in sorted(src.rglob("*.rs")):
            rec = scan_file(f, crate, src)
            if rec is None:
                skipped += 1
            else:
                records.append(rec)

    known = {r["module"] for r in records}
    by_module = {r["module"]: r for r in records}
    edges: list[list[str]] = []
    dependents: dict[str, set[str]] = defaultdict(set)
    seen: set[tuple[str, str]] = set()
    for r in records:
        for dep in r["intra"]:
            tgt = resolve_intra(dep, r["module"], r["crate"], known)
            if tgt and (r["module"], tgt) not in seen:
                seen.add((r["module"], tgt))
                edges.append([r["module"], tgt])
                dependents[tgt].add(r["module"])
        for dep in r["cross"]:
            tgt = resolve_cross(dep, known)
            if tgt and (r["module"], tgt) not in seen:
                seen.add((r["module"], tgt))
                edges.append([r["module"], tgt])
                dependents[tgt].add(r["module"])

    for r in records:
        deps = len({e[1] for e in edges if e[0] == r["module"]})
        r["dependents"] = len(dependents.get(r["module"], ()))
        r["imports"] = deps
        r["importance"] = 5 * r["dependents"] + 2 * deps
        r["externalCount"] = len(set(r.pop("external")))

    crate_stats: dict[str, dict] = {}
    for crate, cdir in crates.items():
        mine = [r for r in records if r["crate"] == crate]
        root = by_module.get(crate)
        crate_stats[crate] = {
            "dir": str(cdir.relative_to(REPO)),
            "files": len(mine),
            "lines": sum(r["lines"] for r in mine),
            "topModules": sorted(root["mods"]) if root else [],
            "tests": sum(r["testCount"] for r in mine),
        }

    tauri = [
        {"fn": t["n"], "file": r["file"], "line": t["l"], "module": r["module"]}
        for r in records
        for t in r["tauri"]
    ]

    data = {
        "schemaVersion": 1,
        "generatedAt": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "totals": {
            "crates": len(crate_stats),
            "files": len(records),
            "lines": sum(r["lines"] for r in records),
            "modules": len(records),
            "edges": len(edges),
            "tauriCommands": len(tauri),
            "skipped": skipped,
        },
        "crates": crate_stats,
        "modules": [
            {
                "module": r["module"],
                "crate": r["crate"],
                "file": r["file"],
                "lines": r["lines"],
                "dependents": r["dependents"],
                "imports": r["imports"],
                "importance": r["importance"],
                "externalCrates": r["externalCount"],
                "hasTests": r["hasTests"],
                "testCount": r["testCount"],
                "symbols": r["symbols"],
                "childMods": r["mods"],
            }
            for r in records
        ],
        "edges": edges,
        "tauriCommands": tauri,
    }
    return data, skipped


def sym_line(symbols: list[dict], budget: int) -> str:
    order = {"trait": 0, "struct": 1, "enum": 2, "fn": 3, "type": 4, "impl": 5}
    ranked = sorted(symbols, key=lambda s: order.get(s["k"], 9))
    names: list[str] = []
    used = 0
    for s in ranked:
        if s["k"] == "impl":
            continue
        n = s["n"]
        if used + len(n) + 2 > budget:
            names.append("…")
            break
        names.append(n)
        used += len(n) + 2
    return ", ".join(names) if names else "—"


def render_md(data: dict) -> str:
    t = data["totals"]
    L: list[str] = []
    L.append("# KRIA Rust Code Map")
    L.append("")
    L.append("## HOW TO USE THIS FILE")
    L.append("")
    L.append("This is an **index, not the code**. It exists so you do not have to grep")
    L.append(f"{t['files']} Rust files. Workflow:")
    L.append("")
    L.append("1. Find the module you need below (HOTSPOTS for the load-bearing ones,")
    L.append("   MODULE INDEX for everything, TAURI COMMANDS for the frontend contract).")
    L.append("2. Note its file path and the line number of the symbol you care about.")
    L.append("3. Open **only that file's relevant line range** — never the whole file.")
    L.append("")
    L.append("`dependents` = how many modules import it (high = changing it is risky).")
    L.append("The CodeScout graph (`.codescout/graph.json`) does NOT cover Rust — use this.")
    L.append(f"Refresh: `python3 scripts/rust-map.py` · generated {data['generatedAt']}")
    L.append("")
    L.append(
        f"**Totals:** {t['crates']} crates · {t['files']} files · {t['lines']:,} lines · "
        f"{t['edges']} module edges · {t['tauriCommands']} Tauri commands"
    )
    L.append("")
    L.append("## CRATES")
    L.append("")
    L.append("| Crate | Files | Lines | Tests | Top-level modules |")
    L.append("|---|---:|---:|---:|---|")
    for name, c in sorted(data["crates"].items(), key=lambda kv: -kv[1]["lines"]):
        mods = ", ".join(c["topModules"][:14])
        if len(c["topModules"]) > 14:
            mods += f", … (+{len(c['topModules']) - 14})"
        L.append(f"| `{name}` | {c['files']} | {c['lines']:,} | {c['tests']} | {mods or '—'} |")
    L.append("")

    mods = data["modules"]
    hot = sorted(mods, key=lambda m: (-m["importance"], -m["lines"]))[:HOTSPOT_COUNT]
    L.append(f"## HOTSPOTS (top {len(hot)} by importance = 5×dependents + 2×imports)")
    L.append("")
    L.append("| Module | File | Lines | Dependents | Key public symbols |")
    L.append("|---|---|---:|---:|---|")
    for m in hot:
        L.append(
            f"| `{m['module']}` | `{m['file']}` | {m['lines']} | {m['dependents']} | "
            f"{sym_line(m['symbols'], 90)} |"
        )
    L.append("")

    L.append(f"## TAURI COMMANDS ({t['tauriCommands']}) — frontend/backend contract, never rename")
    L.append("")
    if data["tauriCommands"]:
        L.append("| Command fn | File | Line |")
        L.append("|---|---|---:|")
        for c in sorted(data["tauriCommands"], key=lambda c: (c["file"], c["line"])):
            L.append(f"| `{c['fn']}` | `{c['file']}` | {c['line']} |")
    else:
        L.append("_none found_")
    L.append("")

    L.append("## MODULE INDEX")
    L.append("")
    L.append("Paths are relative to each crate's `src/`. `p`=public symbols, `l`=lines,")
    L.append("`d`=modules it imports, `D`=modules importing it, `T`=has tests.")
    for crate in sorted(data["crates"], key=lambda k: -data["crates"][k]["lines"]):
        mine = [m for m in mods if m["crate"] == crate]
        if not mine:
            continue
        prefix = f"crates/{Path(data['crates'][crate]['dir']).name}/src/"
        L.append("")
        L.append(f"### {crate} ({len(mine)} modules) — paths under `{prefix}`")
        L.append("")
        L.append("```")
        for m in sorted(mine, key=lambda m: m["file"]):
            pub = len([s for s in m["symbols"] if s["k"] != "impl"])
            short = m["file"][len(prefix):] if m["file"].startswith(prefix) else m["file"]
            flag = " T" if m["hasTests"] else ""
            L.append(
                f"{short} p={pub} l={m['lines']} d={m['imports']} D={m['dependents']}{flag}"
            )
        L.append("```")
    L.append("")
    return "\n".join(L)


def newest_rs_mtime() -> float:
    newest = 0.0
    if CRATES.is_dir():
        for f in CRATES.rglob("*.rs"):
            try:
                newest = max(newest, f.stat().st_mtime)
            except OSError:
                continue
    return newest


def main() -> int:
    ap = argparse.ArgumentParser(description="Build the KRIA Rust code map.")
    ap.add_argument("--json-only", action="store_true", help="skip RUST_MAP.md")
    ap.add_argument("--quiet", action="store_true", help="suppress the summary line")
    ap.add_argument("--check", action="store_true", help="exit 1 if the map is stale")
    args = ap.parse_args()

    if args.check:
        if not JSON_OUT.is_file():
            print("rust-map: MISSING")
            return 1
        stale = newest_rs_mtime() > JSON_OUT.stat().st_mtime
        print("rust-map: STALE" if stale else "rust-map: fresh")
        return 1 if stale else 0

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    data, skipped = build()
    JSON_OUT.write_text(json.dumps(data, separators=(",", ":")), encoding="utf-8")
    md_size = 0
    if not args.json_only:
        md = render_md(data)
        MD_OUT.write_text(md, encoding="utf-8")
        md_size = MD_OUT.stat().st_size

    if not args.quiet:
        t = data["totals"]
        print(
            f"rust-map: {t['files']} files, {t['modules']} modules, {t['edges']} edges, "
            f"{t['tauriCommands']} tauri cmds, {skipped} skipped · "
            f"json={JSON_OUT.stat().st_size / 1024:.0f}KB md={md_size / 1024:.0f}KB"
            + ("  ⚠️ md over budget" if md_size > MD_SIZE_BUDGET else "")
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
