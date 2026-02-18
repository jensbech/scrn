use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct RepoEntry {
    pub name: String,
    pub path: PathBuf,
    pub group: String,
}

pub fn scan_repos(dir: &Path) -> Vec<RepoEntry> {
    let mut repos = Vec::new();
    scan_recursive(dir, dir, &mut repos);
    repos.sort_by(|a, b| {
        let ga = a.group.to_lowercase();
        let gb = b.group.to_lowercase();
        ga.cmp(&gb).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    repos
}

fn scan_recursive(root: &Path, dir: &Path, repos: &mut Vec<RepoEntry>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

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

        if path.join(".git").exists() {
            // It's a repo — record it with the group being the relative path from root to parent
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                let group = path
                    .parent()
                    .and_then(|p| p.strip_prefix(root).ok())
                    .map(|rel| rel.to_string_lossy().into_owned())
                    .unwrap_or_default();
                repos.push(RepoEntry {
                    name: name.to_string(),
                    path,
                    group,
                });
            }
        } else {
            // Not a repo — descend into it
            scan_recursive(root, &path, repos);
        }
    }
}
