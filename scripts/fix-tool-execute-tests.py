#!/usr/bin/env python3
"""Point test files at `execute_with_context`, the interface the agent actually uses.

    python3 scripts/fix-tool-execute-tests.py <file.rs> [...]

# Why

`ToolHandler` has two methods and both have default bodies:

    execute(params)                 -> defaults to an ERROR
    execute_with_context(params, ctx) -> defaults to calling execute

Handlers that need the environment (filesystem, shell, packages) implement only
`execute_with_context`. Calling `execute` on them hits the erroring default and
reports "tool does not implement execute", which reads exactly like the tool is
broken when it is fine.

The erroring default is the RIGHT design and must not be changed: a handler that
needs the governed OS runtime has to refuse when called without one, rather than
quietly fabricating a local environment and reaching the host ungoverned. So the
tests move to the real interface instead.

# Why balanced-paren scanning rather than a regex

The call sites are written several ways — one-liners, multi-line `json!` blocks,
nested parens inside the arguments. A regex either misses those or corrupts them.
Scanning for the matching close paren is exact.
"""
from __future__ import annotations

import pathlib
import sys

HELPER = '''
// `execute_with_context` is what `loop_engine` and `resume_executor` call, so tests
// go through it too. `execute` has an erroring default body — see
// scripts/fix-tool-execute-tests.py for why that default is correct and these calls
// were the thing that was wrong.
fn test_ctx() -> kria_core::tools::ToolContext {
    use std::collections::HashMap;
    use std::sync::Arc;
    kria_core::tools::ToolContext::new(
        Arc::new(kria_core::infra::environment::LocalEnvironment::new()),
        Arc::new(tokio::sync::Mutex::new(
            kria_core::infra::environment::ShellState {
                cwd: std::env::current_dir().expect("a working directory"),
                env_vars: HashMap::new(),
                generation: 0,
            },
        )),
        tokio_util::sync::CancellationToken::new(),
    )
}
'''

MARKER = ".execute("


def convert(text: str) -> tuple[str, int]:
    out = []
    i = 0
    count = 0
    while True:
        idx = text.find(MARKER, i)
        if idx == -1:
            out.append(text[i:])
            break
        # Skip an existing `execute_with_context` — `.execute(` is a prefix of it only
        # if the source already says so, which this guards against.
        if text.startswith(".execute_with_context(", idx):
            out.append(text[i : idx + 1])
            i = idx + 1
            continue

        open_paren = idx + len(MARKER) - 1
        depth = 0
        j = open_paren
        in_str = False
        while j < len(text):
            c = text[j]
            if in_str:
                if c == "\\":
                    j += 2
                    continue
                if c == '"':
                    in_str = False
            elif c == '"':
                in_str = True
            elif c == "(":
                depth += 1
            elif c == ")":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        if j >= len(text):
            raise SystemExit(f"unbalanced parentheses after offset {idx}")

        out.append(text[i:idx])
        out.append(".execute_with_context(")
        out.append(text[open_paren + 1 : j])
        out.append(", test_ctx())")
        count += 1
        i = j + 1
    return "".join(out), count


for name in sys.argv[1:]:
    path = pathlib.Path(name)
    src = path.read_text(encoding="utf-8")
    converted, n = convert(src)
    if "fn test_ctx()" not in converted:
        converted = converted.rstrip() + "\n" + HELPER
    path.write_text(converted, encoding="utf-8")
    print(f"{path.name}: converted {n} call site(s)")
