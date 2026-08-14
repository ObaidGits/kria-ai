#!/usr/bin/env python3
"""Point `semantic_parser.rs` at the inverted LLM trait.

The import was already swapped by the extraction script; this replaces the three
uses of the old types:

* the field and constructor types `Arc<dyn LlmBackend>` -> `Arc<dyn SemanticExtractionLlm>`
* the two `ChatMessage` literals and the `.chat(...)` call -> one `.extract(...)`

The prompts are unchanged, so extraction behaviour is identical; only the seam it
travels through is narrower.
"""
import pathlib
import re
import sys

PATH = (
    pathlib.Path(__file__).resolve().parent.parent
    / "crates/kria-memory/src/semantic_parser.rs"
)
text = PATH.read_text(encoding="utf-8")
before = text

text = text.replace("Arc<dyn LlmBackend>", "Arc<dyn SemanticExtractionLlm>")

# Replace the message construction plus the chat call with one extract call.
old_call = re.compile(
    r"""        let messages = vec!\[.*?\];\n\n        let response = self\n            \.parser_client\n            \.chat\(&messages, None, 0\.0, self\.max_tokens\)\n            \.await\n            \.ok\(\)\?;\n\n        let parsed = parse_memory_extraction_payload\(&response\.content\)\?;""",
    re.S,
)
new_call = """        // One narrow call through the inverted seam: the crate asks for text and
        // does not know which model produced it. `None` means "no extraction
        // available" — enrichment is best-effort and must never fail the write.
        let response = self
            .parser_client
            .extract(system_prompt, &extraction_prompt, self.max_tokens)
            .await?;

        let parsed = parse_memory_extraction_payload(&response)?;"""

text, count = old_call.subn(new_call, text)
if count == 0:
    print("PATTERN NOT MATCHED — inspect the call site manually", file=sys.stderr)
    sys.exit(1)

PATH.write_text(text, encoding="utf-8")
print(f"rewrote {count} call site; ChatMessage remaining: {text.count('ChatMessage')}")
