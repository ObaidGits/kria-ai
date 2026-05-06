use std::path::{Component, Path, PathBuf};

pub fn resolve_path(original_path: &str) -> PathBuf {
    match std::env::var("KRIA_EVAL_FS_ROOT") {
        Ok(sandbox_root) if !sandbox_root.is_empty() => {
            let mut relative = PathBuf::new();

            for component in Path::new(original_path).components() {
                match component {
                    Component::Normal(segment) => relative.push(segment),
                    Component::ParentDir => {
                        let _ = relative.pop();
                    }
                    Component::CurDir | Component::RootDir => {}
                    _ => {}
                }
            }

            Path::new(&sandbox_root).join(relative)
        }
        _ => PathBuf::from(original_path),
    }
}
