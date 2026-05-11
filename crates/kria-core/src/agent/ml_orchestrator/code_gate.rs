// crates/kria-core/src/agent/ml_orchestrator/code_gate.rs
//
// Strict capability allowlist using tree-sitter AST parsing.
// Bans dangerous modules at the import_statement level regardless of aliasing.

use tree_sitter::{Parser, Node};

/// Unconditionally banned modules — blocked at import_statement level.
/// Banned regardless of aliasing (import X as Y).
const BANNED_MODULES: &[&str] = &[
    // System interaction
    "os", "subprocess", "shutil", "sys",
    // Code execution
    "code", "codeop", "compile", "ast",
    // Deserialization attacks
    "pickle", "joblib", "shelve", "marshal",
    // Network exfiltration
    "socket", "http", "urllib", "requests", "http.client",
    "ftplib", "smtplib", "paramiko", "telnetlib", "xmlrpc",
    // Native code
    "ctypes", "cffi",
    // Dynamic import
    "importlib", "imp",
    // Introspection (sandbox escape)
    "__builtin__", "__builtins__", "builtins",
];

/// Banned call targets — blocked at call AST node.
const BANNED_CALLS: &[&str] = &[
    "eval", "exec", "compile", "globals", "locals", "vars",
    "getattr", "setattr", "delattr",
    "__import__", "breakpoint", "exit", "quit",
];

/// Check a code block against the capability allowlist.
/// Returns Ok(()) if safe, Err with details if blocked.
pub fn capability_check(code: &str) -> Result<(), CapabilityError> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_python::language())
        .expect("Failed to load Python grammar for tree-sitter");

    let tree = parser.parse(code, None)
        .ok_or(CapabilityError::ParseError)?;

    walk_node(tree.root_node(), code, 0)
}

fn walk_node(node: Node, source: &str, depth: usize) -> Result<(), CapabilityError> {
    if depth > 200 {
        return Err(CapabilityError::AstTooDeep);
    }

    match node.kind() {
        "import_statement" => {
            // "import os as harmless" → AST: import_statement > aliased_import > dotted_name
            // "import os" → AST: import_statement > dotted_name
            // Recurse to find dotted_name at any depth
            fn find_dotted_names(node: Node, source: &str) -> Vec<String> {
                let mut names = Vec::new();
                if node.kind() == "dotted_name" {
                    names.push(text(node, source));
                }
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    names.extend(find_dotted_names(child, source));
                }
                names
            }
            for module in find_dotted_names(node, source) {
                let root = module.split('.').next().unwrap_or(&module);
                if BANNED_MODULES.contains(&root) {
                    return Err(CapabilityError::BannedModule {
                        module: module.clone(),
                        line: node.start_position().row + 1,
                    });
                }
            }
        }

        "import_from_statement" => {
            for child in node.children(&mut node.walk()) {
                if child.kind() == "dotted_name" || child.kind() == "module_name" {
                    let module = text(child, source);
                    let root = module.split('.').next().unwrap_or(&module);
                    if BANNED_MODULES.contains(&root) {
                        return Err(CapabilityError::BannedModule {
                            module: module.clone(),
                            line: child.start_position().row + 1,
                        });
                    }
                }
            }
        }

        "call" => {
            if let Some(name) = extract_call_target(node, source) {
                if BANNED_CALLS.iter().any(|b| name == *b) {
                    return Err(CapabilityError::BannedCall {
                        call: name,
                        line: node.start_position().row + 1,
                    });
                }
            }
        }

        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, source, depth + 1)?;
    }

    Ok(())
}

fn text(node: Node, source: &str) -> String {
    source[node.start_byte()..node.end_byte()].to_string()
}

fn extract_call_target(node: Node, source: &str) -> Option<String> {
    for child in node.children(&mut node.walk()) {
        match child.kind() {
            "identifier" => return Some(text(child, source)),
            "attribute" => {
                let mut parts = Vec::new();
                for c in child.children(&mut child.walk()) {
                    if c.kind() == "identifier" {
                        parts.push(text(c, source));
                    }
                }
                if !parts.is_empty() {
                    return Some(parts.join("."));
                }
            }
            _ => {}
        }
    }
    None
}

#[derive(Debug, thiserror::Error)]
pub enum CapabilityError {
    #[error("Python AST parse error — code may be malformed")]
    ParseError,
    #[error("AST too deeply nested (possible adversarial input)")]
    AstTooDeep,
    #[error("Banned module: '{module}' (line {line})")]
    BannedModule { module: String, line: usize },
    #[error("Banned call: '{call}' (line {line})")]
    BannedCall { call: String, line: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_safe_imports() {
        assert!(capability_check("import torch\nimport pandas as pd\nimport sklearn").is_ok());
    }

    #[test]
    fn blocks_os_import() {
        assert!(matches!(
            capability_check("import os"),
            Err(CapabilityError::BannedModule { module, .. }) if module == "os"
        ));
    }

    #[test]
    fn blocks_os_alias() {
        assert!(matches!(
            capability_check("import os as harmless"),
            Err(CapabilityError::BannedModule { module, .. }) if module == "os"
        ));
    }

    #[test]
    fn blocks_from_os_import() {
        assert!(matches!(
            capability_check("from os import system"),
            Err(CapabilityError::BannedModule { module, .. }) if module == "os"
        ));
    }

    #[test]
    fn blocks_subprocess() {
        assert!(matches!(
            capability_check("import subprocess"),
            Err(CapabilityError::BannedModule { module, .. }) if module == "subprocess"
        ));
    }

    #[test]
    fn blocks_pickle() {
        assert!(matches!(
            capability_check("import pickle"),
            Err(CapabilityError::BannedModule { module, .. }) if module == "pickle"
        ));
    }

    #[test]
    fn blocks_eval_call() {
        assert!(matches!(
            capability_check("eval('1+1')"),
            Err(CapabilityError::BannedCall { call, .. }) if call == "eval"
        ));
    }

    #[test]
    fn blocks_exec_call() {
        assert!(matches!(
            capability_check("exec('import os')"),
            Err(CapabilityError::BannedCall { call, .. }) if call == "exec"
        ));
    }

    #[test]
    fn blocks_getattr_call() {
        assert!(matches!(
            capability_check("getattr(obj, 'name')"),
            Err(CapabilityError::BannedCall { call, .. }) if call == "getattr"
        ));
    }

    #[test]
    fn allows_torch_save() {
        assert!(capability_check("torch.save(model, 'path')").is_ok());
    }

    #[test]
    fn allows_job_paths() {
        assert!(capability_check("job_paths.safe_save_model(model, '04_train/model.pth')").is_ok());
    }

    #[test]
    fn allows_job_progress() {
        assert!(capability_check("job_progress.report(progress=0.5)").is_ok());
    }

    #[test]
    fn blocks_socket() {
        assert!(matches!(
            capability_check("import socket"),
            Err(CapabilityError::BannedModule { module, .. }) if module == "socket"
        ));
    }

    #[test]
    fn blocks_importlib() {
        assert!(matches!(
            capability_check("import importlib"),
            Err(CapabilityError::BannedModule { module, .. }) if module == "importlib"
        ));
    }

    #[test]
    fn allows_complex_ml_code() {
        let code = r#"
import torch
import pandas as pd
from transformers import BertTokenizer, BertForSequenceClassification
from torch.utils.data import DataLoader, Dataset

df = pd.read_parquet(job_paths.input("load_data", "data.parquet"))
tokenizer = BertTokenizer.from_pretrained("bert-base-uncased")

model = BertForSequenceClassification.from_pretrained("bert-base-uncased", num_labels=2)
optimizer = torch.optim.AdamW(model.parameters(), lr=2e-5)

for epoch in range(3):
    for batch in dataloader:
        loss = model(**batch).loss
        loss.backward()
        optimizer.step()
        job_progress.report(progress=0.5, metrics={"loss": loss.item()})

job_paths.safe_save_model(model, "04_train/model.pth")
job_progress.complete()
"#;
        assert!(capability_check(code).is_ok());
    }
}
