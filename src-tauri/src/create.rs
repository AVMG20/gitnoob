//! Bringing a repository into existence: cloning one, or starting a new one.
//!
//! Neither happens inside an open repository — they are how one arrives — so
//! both take the destination's parent folder rather than reading a path from
//! `AppState`. Like every write, they go through the `git` CLI, so a clone over
//! ssh uses the profile's key and a clone over https uses the machine's own
//! credential helper.

use std::path::Path;

use serde::Serialize;

use crate::git_cmd;

/// What the caller needs to open what was just made.
#[derive(Serialize, Debug)]
pub struct NewRepo {
    pub path: String,
    pub name: String,
    /// Why there is no first commit, when there is not one. Only a new
    /// repository can lack one: nothing anywhere said who to commit as.
    pub note: Option<String>,
}

/// The folder a clone lands in: the repository's own name, the way `git clone`
/// itself names it. Worked out here so the destination can be checked before
/// anything is fetched.
pub fn folder_name(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    // Drop the scheme if there is one, so `ssh://` does not survive as part of
    // the name; whatever separated host from path also separates the name.
    let without_scheme = trimmed.rsplit("://").next().unwrap_or(trimmed);
    // A colon separates host from path in `git@host:owner/repo.git`, and names
    // a drive in `C:\src\widget`. Told apart, because splitting a local path on
    // its colon leaves the whole path as the folder's name — which is what
    // cloning by pasting a Windows path used to do.
    let name = if local_path(trimmed) {
        without_scheme.rsplit(['/', '\\']).next()
    } else {
        without_scheme.rsplit(['/', ':']).next()
    };
    name.unwrap_or(without_scheme)
        .trim_end_matches(".git")
        .to_string()
}

/// Whether this is a path on this machine rather than an address on a forge.
fn local_path(input: &str) -> bool {
    let bytes = input.as_bytes();
    // `C:\src\widget` or `C:/src/widget`: a drive letter, not a host.
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    drive || input.contains('\\') || input.starts_with('/') || input.starts_with('.')
}

/// Clones `url` into a folder named after the repository, inside `parent`.
pub fn clone(url: &str, parent: &Path) -> Result<NewRepo, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Paste the repository's address first".to_string());
    }
    let name = folder_name(url);
    if name.is_empty() {
        return Err(format!(
            "Could not tell what to call a folder cloned from {url}"
        ));
    }
    let dest = parent.join(&name);
    if dest.exists() {
        return Err(format!(
            "{} already has a folder named {name}",
            parent.display()
        ));
    }
    // `--` keeps an address beginning with a dash from being read as a flag,
    // the same discipline every other ref-taking command follows.
    git_cmd::run_checked(parent, &["clone", "--", url, &name])?;
    Ok(NewRepo {
        path: dest.to_string_lossy().into_owned(),
        name,
        note: None,
    })
}

/// A modest starter `.gitignore`: the noise every machine makes whatever the
/// language. What a project's own build produces is the project's to say.
const STARTER_GITIGNORE: &str = "\
# macOS
.DS_Store
# Windows
Thumbs.db
# Editors
*.swp
.idea/
.vscode/
# Local secrets; the example is for committing
.env
.env.*
!.env.example
";

/// Creates a repository with a first commit, so it opens with a history.
///
/// `identity` is the active profile's name and email, applied to the new
/// repository's local config the way opening an existing one applies it. Given
/// no identity to commit under — neither a profile nor the machine's global
/// config — the first commit is left unmade and the `.gitignore` sits untracked
/// in the changes panel, which is as good a place as any to write your first
/// commit message.
pub fn init(
    parent: &Path,
    name: &str,
    identity: Option<(String, String)>,
) -> Result<NewRepo, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Name the repository first".to_string());
    }
    if name.starts_with('.') || name.contains(['/', '\\', ':']) {
        return Err(format!("\"{name}\" cannot be used as a folder name"));
    }
    let root = parent.join(name);
    if root.exists() {
        return Err(format!(
            "{} already has a folder named {name}",
            parent.display()
        ));
    }
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("Could not create {}: {e}", root.display()))?;
    git_cmd::run_checked(&root, &["init", "-q", "-b", "main"])?;
    std::fs::write(root.join(".gitignore"), STARTER_GITIGNORE)
        .map_err(|e| format!("Could not write the starter .gitignore: {e}"))?;
    if let Some((who, email)) = identity {
        git_cmd::run_checked(&root, &["config", "--local", "user.name", &who])?;
        git_cmd::run_checked(&root, &["config", "--local", "user.email", &email])?;
    }
    git_cmd::run_checked(&root, &["add", "--", ".gitignore"])?;
    let committed = git_cmd::run(
        &root,
        &["commit", "-q", "-m", "First commit: a starter .gitignore"],
    )?;
    let note = if committed.ok {
        None
    } else {
        // Undo the staging, so the repository opens clean rather than
        // half-committed.
        let _ = git_cmd::run(&root, &["reset", "-q"]);
        Some("No identity is configured, so the new repository has no first commit; its .gitignore is waiting in the changes panel".to_string())
    };
    Ok(NewRepo {
        path: root.to_string_lossy().into_owned(),
        name: name.to_string(),
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clone_is_named_after_the_repository() {
        assert_eq!(folder_name("git@github.com:acme/widget.git"), "widget");
        assert_eq!(folder_name("https://github.com/acme/widget.git"), "widget");
        assert_eq!(
            folder_name("ssh://git@git.example.com:2222/acme/widget.git"),
            "widget"
        );
        assert_eq!(
            folder_name("https://gitlab.com/group/subgroup/widget/"),
            "widget"
        );

        // A path on this machine is a path, not an address: the colon after a
        // drive letter separates nothing.
        assert_eq!(folder_name("C:\\Users\\robin\\src\\widget"), "widget");
        assert_eq!(folder_name("C:/Users/robin/src/widget/"), "widget");
        assert_eq!(folder_name("/home/robin/src/widget"), "widget");
        assert_eq!(folder_name("../widget"), "widget");
        assert_eq!(folder_name("file:///C:/src/widget.git"), "widget");
    }

    #[test]
    fn a_name_that_is_only_a_host_is_left_alone() {
        assert_eq!(folder_name("https://github.com"), "github.com");
    }
}
