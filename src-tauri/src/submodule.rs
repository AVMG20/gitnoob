//! The repositories kept inside this one.
//!
//! Read by shelling out rather than through libgit2. `git submodule status`
//! answers in one line each the only three questions the sidebar asks — is it
//! cloned, is it on the commit the parent pins, is it in the middle of a
//! conflict — and it answers them the same way the command line would, which
//! is the answer the user can check.
//!
//! What `.gitmodules` says is read separately and joined on, because a
//! submodule's name and its path are two different things and only the file
//! knows the name.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::git_cmd;
use crate::state::AppState;

/// Where a submodule stands, from the prefix character `git submodule status`
/// puts in front of the line.
#[derive(Serialize, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Cloned, and sitting on the commit the parent records.
    Ready,
    /// Named in `.gitmodules` but never checked out: the folder is empty.
    Absent,
    /// Cloned, but on some other commit than the parent records.
    Moved,
    /// A merge left the parent's idea of which commit it should be at conflicted.
    Conflicted,
}

/// One entry of `.gitmodules`, joined to what is actually on disk.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Submodule {
    /// The name in `.gitmodules`, which is not always the path.
    pub name: String,
    /// Where it sits, relative to the repository root.
    pub path: String,
    /// The same place, absolute, so it can be opened as a repository of its own.
    pub abs: String,
    pub url: Option<String>,
    /// The branch `.gitmodules` pins it to, when it pins one.
    pub branch: Option<String>,
    /// The commit the parent repository records for it.
    pub oid: String,
    pub short: String,
    /// What `git describe` made of that commit, when git offered anything.
    pub described: Option<String>,
    pub state: State,
}

/// Every submodule this repository declares.
///
/// A repository without a `.gitmodules` is the common case and is answered
/// without running git at all: this is on the refresh path, which already runs
/// nine commands.
pub fn list(state: &AppState) -> Result<Vec<Submodule>, String> {
    let root = state.path()?;
    if !root.join(".gitmodules").is_file() {
        return Ok(Vec::new());
    }
    let status = git_cmd::run(&root, &["submodule", "status"])?;
    let declared = git_cmd::run(
        &root,
        &[
            "config",
            "--file",
            ".gitmodules",
            "--get-regexp",
            "^submodule\\.",
        ],
    )?;
    Ok(join(&status.stdout, &declared.stdout, &root))
}

/// Joins what `.gitmodules` declares to what is on disk.
///
/// Split out from the two git calls so it can be tested against the output
/// git actually prints, which is the part with the edge cases in it.
fn join(status_raw: &str, declared_raw: &str, root: &Path) -> Vec<Submodule> {
    let mut on_disk = read_status(status_raw);
    let mut out = Vec::new();

    for (name, fields) in read_declared(declared_raw) {
        let Some(path) = fields.get("path").cloned() else {
            // A stanza with no path is not a submodule git would act on.
            continue;
        };
        let found = on_disk.remove(&path);
        let (state, oid, described) = found.unwrap_or((State::Absent, String::new(), None));
        out.push(Submodule {
            name,
            abs: root.join(&path).to_string_lossy().into_owned(),
            path,
            url: fields.get("url").cloned(),
            branch: fields.get("branch").cloned(),
            short: oid.chars().take(7).collect(),
            oid,
            described,
            state,
        });
    }

    // Anything git reports that the file no longer declares. It is still a
    // submodule as far as the index is concerned, and leaving it out would be
    // hiding the very thing that needs cleaning up.
    let mut orphans: Vec<_> = on_disk.into_iter().collect();
    orphans.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, (state, oid, described)) in orphans {
        out.push(Submodule {
            name: path.clone(),
            abs: root.join(&path).to_string_lossy().into_owned(),
            path,
            url: None,
            branch: None,
            short: oid.chars().take(7).collect(),
            oid,
            described,
            state,
        });
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// `git submodule status`, whose lines are `<mark><oid> <path> (<describe>)`.
fn read_status(raw: &str) -> HashMap<String, (State, String, Option<String>)> {
    let mut found = HashMap::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut chars = line.chars();
        let Some(mark) = chars.next() else { continue };
        let state = match mark {
            '-' => State::Absent,
            '+' => State::Moved,
            'U' => State::Conflicted,
            ' ' => State::Ready,
            // Not a status line at all; git prints nothing else here, but a
            // line we cannot read is better skipped than guessed at.
            _ => continue,
        };
        let Some((oid, rest)) = chars.as_str().split_once(' ') else {
            continue;
        };
        // The description git appends is parenthesised and last. A path may
        // hold spaces and brackets of its own, so it is read from the right.
        let (path, described) = match rest.rfind(" (") {
            Some(at) if rest.ends_with(')') => (
                &rest[..at],
                Some(rest[at + 2..rest.len() - 1].trim().to_string()),
            ),
            _ => (rest, None),
        };
        found.insert(
            path.trim().to_string(),
            (state, oid.trim().to_string(), described),
        );
    }
    found
}

/// `git config --file .gitmodules --get-regexp ^submodule\.`, whose keys are
/// `submodule.<name>.<field>` — and a name is allowed to hold dots, so the
/// field is taken from the right-hand end.
fn read_declared(raw: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut order: Vec<String> = Vec::new();
    let mut stanzas: HashMap<String, HashMap<String, String>> = HashMap::new();

    for line in raw.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let Some(rest) = key.strip_prefix("submodule.") else {
            continue;
        };
        let Some((name, field)) = rest.rsplit_once('.') else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let entry = stanzas.entry(name.to_string()).or_insert_with(|| {
            order.push(name.to_string());
            HashMap::new()
        });
        entry.insert(field.to_string(), value.trim().to_string());
    }

    order
        .into_iter()
        .filter_map(|name| stanzas.remove(&name).map(|fields| (name, fields)))
        .collect()
}

/// Clones what is missing and moves each one onto the commit the parent pins.
///
/// `--init` is always passed: an update that skips the submodules nobody has
/// cloned yet is the one thing a person clicking "Update" never means.
pub fn update(state: &AppState, path: Option<&str>, recursive: bool) -> Result<String, String> {
    let root = state.path()?;
    let mut args = vec!["submodule", "update", "--init"];
    if recursive {
        args.push("--recursive");
    }
    if let Some(path) = path {
        args.push("--");
        args.push(path);
    }
    git_cmd::run_checked(&root, &args)?;
    Ok(match path {
        Some(path) => format!("{path} is at the commit this repository records"),
        None => "Every submodule is at the commit this repository records".to_string(),
    })
}

/// Copies the URLs in `.gitmodules` over the ones each submodule was cloned
/// with — what you run after a remote moves.
pub fn sync(state: &AppState, path: Option<&str>) -> Result<String, String> {
    let root = state.path()?;
    let mut args = vec!["submodule", "sync"];
    if let Some(path) = path {
        args.push("--");
        args.push(path);
    }
    git_cmd::run_checked(&root, &args)?;
    Ok(match path {
        Some(path) => format!("{path} now points where .gitmodules says"),
        None => "Every submodule now points where .gitmodules says".to_string(),
    })
}

/// Adds a repository as a submodule at `path`, cloning it there.
pub fn add(state: &AppState, url: &str, path: &str) -> Result<String, String> {
    let root = state.path()?;
    if root.join(path).exists() {
        return Err(format!("{path} already exists"));
    }
    git_cmd::run_checked(&root, &["submodule", "add", "--", url, path])?;
    Ok(format!("Added {url} at {path}"))
}

/// Empties a submodule's folder, keeping it declared.
///
/// `force` is for one that has work in it: without it git refuses, which is
/// the refusal worth passing on rather than pre-empting.
pub fn deinit(state: &AppState, path: &str, force: bool) -> Result<String, String> {
    let root = state.path()?;
    let mut args = vec!["submodule", "deinit"];
    if force {
        args.push("--force");
    }
    args.push("--");
    args.push(path);
    git_cmd::run_checked(&root, &args)?;
    Ok(format!(
        "Emptied {path}; it is still declared in .gitmodules"
    ))
}

/// Takes a submodule out altogether: out of the working tree, out of the
/// index, and out of `.gitmodules`.
///
/// Three commands because git has never had one. The deinit is forced —
/// somebody who has confirmed removing the whole submodule has already
/// answered the question `--force` asks — but the `git rm` is not, so a
/// surprise it finds still stops the removal rather than shredding it.
pub fn remove(state: &AppState, path: &str) -> Result<String, String> {
    let root = state.path()?;
    git_cmd::run_checked(&root, &["submodule", "deinit", "--force", "--", path])?;
    git_cmd::run_checked(&root, &["rm", "--", path])?;
    Ok(format!("Removed {path}. Commit to record it."))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = concat!(
        " 0e8a1f2c4b6d8e0a2c4e6a8c0e2a4c6e8a0c2e4a libs/shared (v1.4.0)\n",
        "-9f1c3e5a7b9d1f3a5c7e9a1c3e5a7b9d1f3a5c7e vendor/theme\n",
        "+2b4d6f8a0c2e4a6c8e0a2c4e6a8c0e2a4c6e8a0c tools/cli (v0.2.1-3-gab12cd3)\n",
        "U7c9e1a3c5e7a9c1e3a5c7e9a1c3e5a7b9d1f3a5c apps/web\n"
    );

    const DECLARED: &str = concat!(
        "submodule.libs/shared.path libs/shared\n",
        "submodule.libs/shared.url git@github.com:acme/shared.git\n",
        "submodule.vendor/theme.path vendor/theme\n",
        "submodule.vendor/theme.url https://example.com/theme.git\n",
        "submodule.vendor/theme.branch main\n",
        "submodule.tools/cli.path tools/cli\n",
        "submodule.tools/cli.url git@github.com:acme/cli.git\n",
        "submodule.apps/web.path apps/web\n",
        "submodule.apps/web.url git@github.com:acme/web.git\n"
    );

    fn joined() -> Vec<Submodule> {
        join(STATUS, DECLARED, Path::new("/repo"))
    }

    #[test]
    fn reads_the_four_states_git_marks() {
        let found = joined();
        let states: Vec<_> = found.iter().map(|s| (s.path.as_str(), s.state)).collect();
        assert_eq!(
            states,
            vec![
                ("apps/web", State::Conflicted),
                ("libs/shared", State::Ready),
                ("tools/cli", State::Moved),
                ("vendor/theme", State::Absent),
            ]
        );
    }

    #[test]
    fn joins_the_url_and_branch_onto_the_commit() {
        let found = joined();
        let theme = found.iter().find(|s| s.path == "vendor/theme").unwrap();
        assert_eq!(theme.url.as_deref(), Some("https://example.com/theme.git"));
        assert_eq!(theme.branch.as_deref(), Some("main"));

        let shared = found.iter().find(|s| s.path == "libs/shared").unwrap();
        assert_eq!(shared.branch, None);
        assert_eq!(shared.short, "0e8a1f2");
        assert_eq!(shared.described.as_deref(), Some("v1.4.0"));
        // Rooted at the repository. Compared as a path rather than as a
        // string: Windows joins with a backslash, and what is under test is
        // where the folder is, not which character the platform spells a
        // separator with.
        assert_eq!(
            Path::new(&shared.abs),
            Path::new("/repo").join("libs/shared")
        );
    }

    #[test]
    fn a_description_with_brackets_is_not_read_as_the_path() {
        let found = join(
            " 0e8a1f2c4b6d8e0a2c4e6a8c0e2a4c6e8a0c2e4a some (odd) name (v2.0)\n",
            "submodule.odd.path some (odd) name\n",
            Path::new("/repo"),
        );
        assert_eq!(found[0].path, "some (odd) name");
        assert_eq!(found[0].described.as_deref(), Some("v2.0"));
        assert_eq!(found[0].state, State::Ready);
    }

    #[test]
    fn a_name_may_hold_a_dot() {
        let found = join(
            " 0e8a1f2c4b6d8e0a2c4e6a8c0e2a4c6e8a0c2e4a libs/v1.2\n",
            "submodule.libs/v1.2.path libs/v1.2\nsubmodule.libs/v1.2.url git@x:y.git\n",
            Path::new("/repo"),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "libs/v1.2");
        assert_eq!(found[0].url.as_deref(), Some("git@x:y.git"));
    }

    #[test]
    fn one_git_still_reports_but_the_file_has_forgotten_is_kept() {
        let found = join(
            STATUS,
            "submodule.libs/shared.path libs/shared\n",
            Path::new("/repo"),
        );
        let paths: Vec<_> = found.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["apps/web", "libs/shared", "tools/cli", "vendor/theme"]
        );
        let orphan = found.iter().find(|s| s.path == "apps/web").unwrap();
        assert_eq!(orphan.url, None);
    }

    #[test]
    fn nothing_declared_is_nothing_at_all() {
        assert!(join("", "", Path::new("/repo")).is_empty());
    }
}
