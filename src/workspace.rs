use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct TreeNode {
    pub name: String,
    pub path: PathBuf,
    pub is_repo: bool,
    pub children: Vec<TreeNode>,
}

/// Build a recursive tree of directories and repos under `dir`.
/// Repo nodes (have .git) are leaves; non-repo dirs recurse.
/// Hidden directories (`.` prefix) are skipped.
/// Children sorted alphabetically (case-insensitive).
pub fn scan_tree(dir: &Path) -> TreeNode {
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let mut root = TreeNode {
        name,
        path: dir.to_path_buf(),
        is_repo: false,
        children: Vec::new(),
    };
    scan_tree_recursive(dir, &mut root);
    root
}

fn scan_tree_recursive(dir: &Path, node: &mut TreeNode) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    let mut children: Vec<TreeNode> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Skip hidden directories
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }

        let child_name = entry
            .file_name()
            .to_str()
            .unwrap_or("")
            .to_string();

        if path.join(".git").exists() {
            // Repo leaf node
            children.push(TreeNode {
                name: child_name,
                path,
                is_repo: true,
                children: Vec::new(),
            });
        } else {
            // Directory node — recurse
            let mut child = TreeNode {
                name: child_name,
                path: path.clone(),
                is_repo: false,
                children: Vec::new(),
            };
            scan_tree_recursive(&path, &mut child);
            // Only include directory if it has descendants
            if !child.children.is_empty() {
                children.push(child);
            }
        }
    }

    // Sort alphabetically, case-insensitive
    children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    node.children = children;
}
