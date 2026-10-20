//! Policy: no Python in this repo. Fails when any `*.py` file exists in the source tree.

use std::path::Path;

fn find_py(dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !matches!(name.as_ref(), ".git" | "target" | "node_modules" | ".svelte-kit") {
                find_py(&path, out);
            }
        } else if name.ends_with(".py") {
            out.push(path.display().to_string());
        }
    }
}

#[test]
fn no_python_files_in_repo() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = Vec::new();
    find_py(&root, &mut found);
    assert!(found.is_empty(), "Python files are not allowed: {found:?}");
}
