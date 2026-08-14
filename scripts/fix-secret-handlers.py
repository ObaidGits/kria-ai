"""Replace the closure-based macro with inline sealing in each secret handler.

The `AdmittedMutationContext<'a>` borrows from the call, grant and lease set, so it
cannot be produced by a helper that returns it — the borrows would not outlive the
call. Inlining per handler is the shape the lifetimes actually allow.
"""

import pathlib
import re

p = pathlib.Path("/media/obaid/SSD/KRIA/crates/kria-core/src/tools/secrets.rs")
s = p.read_text(encoding="utf-8")

# 1. next_cursor is a SafeText, not a String.
s = s.replace(
    '"next_cursor": page.next_cursor.as_deref(),',
    '"next_cursor": page.next_cursor.as_ref().map(|c| c.as_str()),',
)

# 2. Drop the macro entirely.
start = s.index("/// Seal a mutation context and hand it to `body`")
end = s.index("struct StoreSecret;")
s = s[:start] + s[end:]

PRELUDE = """        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let Some(grant) = call.grant() else {
            return ToolResult::err("a secret mutation requires a grant");
        };
        let Some(leases) = call.leases() else {
            return ToolResult::err("a secret mutation requires held leases");
        };
        let binding = call.binding();
        // Sealing proves grant, leases, audit admission and observation all came
        // from the SAME admission before the keyring is touched.
        let sealed = match resolved.runtime.seal_mutation_context(
            call.observation(),
            grant,
            leases,
            call.admission(),
            &binding,
        ) {
            Ok(sealed) => sealed,
            Err(error) => return gov::os_error(&error),
        };
        let store = match resolved.runtime.secrets(tool) {
            Ok(store) => store,
            Err(error) => return gov::os_error(&error),
        };
"""

BODIES = {
    "store_secret": PRELUDE
    + """        match store
            .store(
                &sealed,
                purpose,
                SecretScope::new(scope_raw),
                SafeText::new(label),
                input,
            )
            .await
        {
            // Only the reference and metadata come back — never the value.
            Ok(metadata) => ToolResult::ok(serde_json::json!({
                "reference": metadata.reference.as_str(),
                "purpose": format!("{:?}", metadata.purpose),
                "scope": metadata.scope.as_str(),
                "created_unix": metadata.created_unix,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}""",
    "replace_secret": PRELUDE
    + """        // The previous value is not recoverable, so no rollback is claimed.
        match store
            .replace(&sealed, &SecretRef::new(reference), input)
            .await
        {
            Ok(metadata) => ToolResult::ok(serde_json::json!({
                "reference": metadata.reference.as_str(),
                "created_unix": metadata.created_unix,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}""",
    "delete_secret": PRELUDE
    + """        // Irreversible: the value is gone and cannot be restored.
        match store.delete(&sealed, &SecretRef::new(reference)).await {
            Ok(()) => ToolResult::ok(serde_json::json!({ "deleted": true })),
            Err(error) => gov::os_error(&error),
        }
    }
}""",
}

for tool, body in BODIES.items():
    # Replace from the macro invocation through the end of that impl block.
    pattern = re.compile(
        r"        sealed_secret_mutation!\(ctx, tool, \|store: &dyn crate::os_control::secrets::CredentialStore,.*?\n        \}\)\n    \}\n\}",
        re.S,
    )
    m = pattern.search(s)
    if not m:
        raise SystemExit(f"MISS for {tool}")
    s = s[: m.start()] + body + s[m.end() :]

p.write_text(s, encoding="utf-8")
print("inlined sealing in 3 secret handlers")
