use std::path::Path;
use std::process::Command;

pub fn is_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Create a worktree in detached-HEAD state at the repo's current HEAD.
/// The caller can `git switch -c <any-name>` inside to attach their own
/// branch — scrn never claims a branch name.
pub fn create_worktree(repo: &Path, wt_path: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "add", "--detach"])
        .arg(wt_path)
        .output()
        .map_err(|e| format!("git worktree add failed to execute: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("git worktree add: {}", stderr.trim()))
    }
}

pub fn remove_worktree(wt_path: &Path, force: bool) -> Result<(), String> {
    let mut cmd = Command::new("git");
    cmd.args(["worktree", "remove"]);
    if force {
        cmd.arg("--force");
    }
    cmd.arg(wt_path);
    let output = cmd
        .output()
        .map_err(|e| format!("git worktree remove failed to execute: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("git worktree remove: {}", stderr.trim()))
    }
}

pub fn is_worktree_dirty(wt_path: &Path) -> bool {
    let output = Command::new("git")
        .arg("-C")
        .arg(wt_path)
        .args(["status", "--porcelain"])
        .output();
    matches!(output, Ok(o) if o.status.success() && !o.stdout.is_empty())
}

