//! Git LFS: the files that are not really in the repository.
//!
//! What LFS puts in a commit is a three-line pointer, and what the checkout
//! puts on disk is either the real file or — when the objects were never
//! fetched, or `git lfs` is not installed — that same pointer. A viewer that
//! does not know the difference shows three lines of metadata and calls it the
//! file, which is the one thing worth getting right here.

use serde::Serialize;

use crate::git_cmd;
use crate::state::AppState;

/// Whether this repository has anything to do with LFS.
#[derive(Serialize, Debug, PartialEq, Eq, Default)]
pub struct Status {
    /// `.gitattributes` sends something through the LFS filter.
    pub in_use: bool,
    /// `git lfs` answers, so the files can actually be fetched.
    pub installed: bool,
}

/// Read on every refresh, so it stays cheap: a repository with no
/// `.gitattributes` is answered without running anything at all.
pub fn status(state: &AppState) -> Result<Status, String> {
    let root = state.path()?;
    let attributes = root.join(".gitattributes");
    let in_use = std::fs::read_to_string(&attributes)
        .map(|text| uses_lfs(&text))
        .unwrap_or(false);
    if !in_use {
        return Ok(Status::default());
    }
    Ok(Status {
        in_use,
        installed: installed(&root),
    })
}

/// Whether any attribute line sends its files through the LFS filter.
fn uses_lfs(attributes: &str) -> bool {
    attributes.lines().any(|line| {
        let line = line.trim();
        !line.starts_with('#') && line.contains("filter=lfs")
    })
}

fn installed(root: &std::path::Path) -> bool {
    git_cmd::run(root, &["lfs", "version"]).is_ok_and(|out| out.ok)
}

/// Fetches the real contents, for one file or for the lot.
pub fn pull(state: &AppState, path: Option<&str>) -> Result<String, String> {
    let root = state.path()?;
    if !installed(&root) {
        return Err(
            "git-lfs is not installed on this machine. Install it, then run this again."
                .to_string(),
        );
    }
    let include;
    let mut args = vec!["lfs", "pull"];
    if let Some(path) = path {
        include = format!("--include={path}");
        args.push(&include);
    }
    git_cmd::run_checked(&root, &args)?;
    Ok(match path {
        Some(path) => format!("Fetched {path} from LFS"),
        None => "Fetched every LFS file".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_attributes_that_matter_and_ignores_the_rest() {
        assert!(uses_lfs("*.psd filter=lfs diff=lfs merge=lfs -text\n"));
        assert!(!uses_lfs("* text=auto\n"));
        assert!(!uses_lfs("# *.psd filter=lfs diff=lfs\n"));
        assert!(!uses_lfs(""));
    }
}
