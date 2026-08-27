//! Worktrees: the same repository checked out into more than one folder.
//!
//! A worktree is how you stand on two branches at once — a hotfix in one
//! folder while a feature keeps its half-finished state in another — without
//! the stash-and-switch dance. Each one is an ordinary folder, so the rest of
//! the app can open it as a project tab and never know it is special.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::git_cmd;
use crate::state::AppState;

#[derive(Serialize)]
pub struct Worktree {
    /// Absolute path of the folder.
    pub path: String,
    /// The folder's own name, which is how the row reads.
    pub name: String,
    /// The branch checked out there; `None` for a detached HEAD.
    pub branch: Option<String>,
    pub oid: String,
    /// The original checkout — the folder the repository itself lives in,
    /// which `git worktree` refuses to remove.
    pub is_main: bool,
    /// The folder this window has open right now.
    pub is_current: bool,
    pub locked: bool,
}

/// Every folder this repository is checked out into, this one included.
pub fn list(state: &AppState) -> Result<Vec<Worktree>, String> {
    let root = state.path()?;
    let raw = git_cmd::run_checked(&root, &["worktree", "list", "--porcelain"])?;
    let here = canonical(&root);
    let mut trees = parse(&raw);
    for tree in &mut trees {
        tree.is_current = canonical(Path::new(&tree.path)) == here;
    }
    Ok(trees)
}

/// Reads `--porcelain` output: one paragraph per worktree — a `worktree
/// <path>` line, then `HEAD`, `branch` or `detached`, and `locked` when it is.
/// The first paragraph is always the main worktree.
fn parse(raw: &str) -> Vec<Worktree> {
    let mut out = Vec::new();
    for block in raw.split("\n\n") {
        let mut path = None;
        let mut oid = String::new();
        let mut branch = None;
        let mut locked = false;
        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("worktree ") {
                path = Some(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("HEAD ") {
                oid = rest.to_string();
            } else if let Some(rest) = line.strip_prefix("branch ") {
                branch = Some(rest.strip_prefix("refs/heads/").unwrap_or(rest).to_string());
            } else if line == "locked" || line.starts_with("locked ") {
                locked = true;
            }
        }
        let Some(path) = path else { continue };
        let name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        out.push(Worktree {
            path,
            name,
            branch,
            oid,
            is_main: out.is_empty(),
            is_current: false,
            locked,
        });
    }
    out
}

/// A path in the one spelling the OS resolves it to, so two names for the same
/// folder compare equal — macOS temp folders live behind a `/private` symlink.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Checks a branch out into a new folder.
///
/// With `track`, the branch does not exist here yet: it is created following
/// that remote-tracking ref, the same branch an ordinary checkout of the
/// remote branch would have made. Without it, the branch is checked out as it
/// stands — and git refuses when some other worktree already has it, which is
/// the point of the rule: two folders writing to one branch would fight over
/// it.
pub fn add(
    state: &AppState,
    path: &str,
    branch: &str,
    track: Option<&str>,
) -> Result<String, String> {
    let root = state.path()?;
    if Path::new(path).exists() {
        return Err(format!("{path} already exists"));
    }
    match track {
        Some(remote_ref) => {
            git_cmd::run_checked(
                &root,
                &["worktree", "add", "--track", "-b", branch, path, remote_ref],
            )?;
        }
        None => {
            git_cmd::run_checked(&root, &["worktree", "add", path, branch])?;
        }
    }
    Ok(format!("Created a worktree at {path}, on {branch}"))
}

/// Removes a worktree's folder and the bookkeeping that made it one.
///
/// Without `force`, git refuses over uncommitted work in it — the right
/// default, because a folder someone forgot about is exactly where changes get
/// lost. The main worktree is refused too: that one is the repository.
pub fn remove(state: &AppState, path: &str, force: bool) -> Result<String, String> {
    let root = state.path()?;
    if canonical(Path::new(path)) == canonical(&root) {
        return Err(
            "This folder is the one the window has open. Switch to another worktree's tab first."
                .to_string(),
        );
    }
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(path);
    git_cmd::run_checked(&root, &args)?;
    Ok(format!("Removed the worktree at {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORCELAIN: &str = "\
worktree /home/me/widget
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree /home/me/widget-fix
HEAD 2222222222222222222222222222222222222222
branch refs/heads/hotfix/crash
locked reason of its own

worktree /home/me/widget-old
HEAD 3333333333333333333333333333333333333333
detached
";

    #[test]
    fn reads_the_porcelain_paragraphs() {
        let trees = parse(PORCELAIN);
        assert_eq!(trees.len(), 3);

        assert_eq!(trees[0].name, "widget");
        assert!(trees[0].is_main, "the first paragraph is the repository itself");
        assert_eq!(trees[0].branch.as_deref(), Some("main"));
        assert!(!trees[0].locked);

        // A branch with slashes keeps them: the folder name is the short part.
        assert_eq!(trees[1].branch.as_deref(), Some("hotfix/crash"));
        assert!(trees[1].locked);
        assert!(!trees[1].is_main);

        assert_eq!(trees[2].branch, None, "a detached HEAD names no branch");
        assert_eq!(trees[2].oid, "3333333333333333333333333333333333333333");
    }
}
