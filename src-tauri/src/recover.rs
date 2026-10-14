//! Getting things back.
//!
//! Two halves. The first is the safety net under every discard: before a file
//! is thrown away, what it held is written into the object database, and the
//! undo step that follows can write it back byte for byte. The second is the
//! reflog view behind "More history…" in the history menu — where HEAD has
//! been, and the stashes that fell off the list — read-only until something
//! in it is picked.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::git_cmd;
use crate::journal::{FileKind, FileSave, Saved};
use crate::state::AppState;

// --- saving files before they are thrown away -------------------------------

/// What each of `paths` holds right now, as blobs. With `write` the blobs go
/// into the object database, so they outlive the file; without, they are only
/// named, which is how a file is compared against one saved earlier.
///
/// Raw bytes, with no filters: `--no-filters` hashes the file as it is on
/// disk, line endings and all, so what is written back is what was there and
/// not what git would have checked in. A directory — a submodule, a nested
/// repository — is not something a discard can lose, and is left out.
pub fn look(
    root: &Path,
    paths: &[String],
    write: bool,
) -> Result<Vec<(String, Option<Saved>)>, String> {
    let mut out: Vec<(String, Option<Saved>)> = Vec::with_capacity(paths.len());
    // Files hashed in one go, by where their answer goes in `out`.
    let mut batch: Vec<(usize, String)> = Vec::new();

    for path in paths {
        let full = root.join(path);
        let Ok(meta) = std::fs::symlink_metadata(&full) else {
            out.push((path.clone(), None));
            continue;
        };
        if meta.file_type().is_symlink() && cfg!(unix) {
            let target = std::fs::read_link(&full)
                .map_err(|e| format!("Could not read the link {path}: {e}"))?;
            let blob = hash_text(root, &target.to_string_lossy(), write)?;
            out.push((
                path.clone(),
                Some(Saved {
                    blob,
                    kind: FileKind::Link,
                }),
            ));
        } else if meta.is_dir() {
            continue;
        } else {
            let kind = if is_executable(&meta) {
                FileKind::Exec
            } else {
                FileKind::File
            };
            out.push((
                path.clone(),
                Some(Saved {
                    blob: String::new(),
                    kind,
                }),
            ));
            batch.push((out.len() - 1, path.clone()));
        }
    }

    // `--stdin-paths` reads one path a line, so a name with a line break in it
    // is hashed on its own. So is one that starts with a quote, which git
    // reads as a C-quoted name and unquotes — `"x"` would hash the file `x` —
    // and one that ends in a carriage return, which it strips as a line end.
    let (odd, plain): (Vec<_>, Vec<_>) = batch
        .into_iter()
        .partition(|(_, path)| needs_its_own_line(path));
    if !plain.is_empty() {
        let mut args = vec!["hash-object"];
        if write {
            args.push("-w");
        }
        args.extend(["--no-filters", "--stdin-paths"]);
        let input: String = plain.iter().map(|(_, path)| format!("{path}\n")).collect();
        let said = git_cmd::run_with_input(root, &args, &input)?;
        if !said.ok {
            return Err(format!(
                "Could not save the files first: {}",
                said.stderr.trim()
            ));
        }
        let oids: Vec<&str> = said.stdout.lines().map(str::trim).collect();
        if oids.len() != plain.len() {
            return Err(
                "Could not save the files first: git answered for the wrong number of files"
                    .to_string(),
            );
        }
        for ((at, _), oid) in plain.iter().zip(oids) {
            if let Some((_, Some(saved))) = out.get_mut(*at) {
                saved.blob = oid.to_string();
            }
        }
    }
    for (at, path) in odd {
        let mut args = vec!["hash-object"];
        if write {
            args.push("-w");
        }
        args.extend(["--no-filters", "--", path.as_str()]);
        let oid = git_cmd::run_checked(root, &args)?.trim().to_string();
        if let Some((_, Some(saved))) = out.get_mut(at) {
            saved.blob = oid;
        }
    }
    Ok(out)
}

/// Whether `--stdin-paths` would read `path` as something else.
fn needs_its_own_line(path: &str) -> bool {
    path.contains('\n') || path.starts_with('"') || path.ends_with('\r')
}

fn hash_text(root: &Path, text: &str, write: bool) -> Result<String, String> {
    let mut args = vec!["hash-object"];
    if write {
        args.push("-w");
    }
    args.push("--stdin");
    let said = git_cmd::run_with_input(root, &args, text)?;
    if !said.ok {
        return Err(said.stderr.trim().to_string());
    }
    Ok(said.stdout.trim().to_string())
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    false
}

/// The files that changed between two looks at the same paths. Anything the
/// first look skipped — a directory — is not a file to put back.
pub fn changes(
    before: Vec<(String, Option<Saved>)>,
    after: Vec<(String, Option<Saved>)>,
) -> Vec<FileSave> {
    let mut later: HashMap<String, Option<Saved>> = after.into_iter().collect();
    before
        .into_iter()
        .filter_map(|(path, was)| {
            let now = later.remove(&path).unwrap_or(None);
            (was != now).then_some(FileSave {
                path,
                before: was,
                after: now,
            })
        })
        .collect()
}

/// Writes saved files back: their `before` when `back`, else their `after`.
///
/// Every file is checked first, and nothing is written unless all of them are
/// exactly as the step left them. A file worked on since holds something that
/// is saved nowhere, and writing over it would trade one lost change for
/// another.
pub fn put_files(root: &Path, files: &[FileSave], back: bool) -> Result<usize, String> {
    if files.is_empty() {
        return Err("Nothing was saved, so there is nothing to put back".to_string());
    }
    check_files(root, files, back)?;
    write_files(root, files, back)
}

/// Refuses unless every file reads exactly as the step left it: its `after`
/// when going `back`, else its `before`. See [`put_files`].
pub fn check_files(root: &Path, files: &[FileSave], back: bool) -> Result<(), String> {
    let paths: Vec<String> = files.iter().map(|file| file.path.clone()).collect();
    let now: HashMap<String, Option<Saved>> = look(root, &paths, false)?.into_iter().collect();
    for file in files {
        let expected = if back { &file.after } else { &file.before };
        let current = match now.get(&file.path) {
            Some(current) => current.clone(),
            // Skipped by the look: it is a directory now.
            None => {
                return Err(format!(
                    "{} is a folder now, so the file cannot be put back there",
                    file.path
                ))
            }
        };
        if &current != expected {
            return Err(format!(
                "{} has changed since, so putting it back would throw that away. Nothing was \
                 changed.",
                file.path
            ));
        }
    }
    Ok(())
}

/// Writes saved files back without looking first: their `before` when `back`,
/// else their `after`. Every blob is read before the first file is touched.
pub fn write_files(root: &Path, files: &[FileSave], back: bool) -> Result<usize, String> {
    let repo = git2::Repository::open(root).map_err(|e| e.message().to_string())?;
    // Read every blob before writing anything, so a missing object stops it
    // before the first file is touched.
    let mut contents: Vec<(&FileSave, Option<Content>)> = Vec::new();
    for file in files {
        let wanted = if back { &file.before } else { &file.after };
        let content = match wanted {
            None => None,
            Some(saved) => {
                let oid = git2::Oid::from_str(&saved.blob).map_err(|e| e.message().to_string())?;
                let blob = repo.find_blob(oid).map_err(|_| {
                    format!(
                        "The saved copy of {} is gone from the repository",
                        file.path
                    )
                })?;
                Some((blob.content().to_vec(), saved.kind))
            }
        };
        contents.push((file, content));
    }

    for (file, content) in &contents {
        let full = root.join(&file.path);
        match content {
            None => {
                remove_entry(&full)?;
                tidy_parents(root, &full);
            }
            Some((bytes, kind)) => write_file(&full, bytes, *kind)
                .map_err(|e| format!("Could not write {}: {e}", file.path))?,
        }
    }
    Ok(contents.len())
}

/// The bytes a saved file is written back with, and how.
type Content = (Vec<u8>, FileKind);

fn remove_entry(full: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(full) {
        Ok(_) => std::fs::remove_file(full)
            .map_err(|e| format!("Could not remove {}: {e}", full.display())),
        Err(_) => Ok(()),
    }
}

/// Removes the folders a deleted file leaves empty, the way `git clean -d`
/// would have, up to but never including the work tree itself.
fn tidy_parents(root: &Path, full: &Path) {
    let mut dir = full.parent();
    while let Some(at) = dir {
        if at == root || !at.starts_with(root) {
            break;
        }
        if std::fs::remove_dir(at).is_err() {
            break;
        }
        dir = at.parent();
    }
}

fn write_file(full: &Path, bytes: &[u8], kind: FileKind) -> std::io::Result<()> {
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Written through, a link would carry the bytes to wherever it points.
    if std::fs::symlink_metadata(full).is_ok_and(|meta| meta.file_type().is_symlink()) {
        std::fs::remove_file(full)?;
    }
    if kind == FileKind::Link {
        #[cfg(unix)]
        {
            if std::fs::symlink_metadata(full).is_ok() {
                std::fs::remove_file(full)?;
            }
            let target = String::from_utf8_lossy(bytes).into_owned();
            return std::os::unix::fs::symlink(target, full);
        }
    }
    std::fs::write(full, bytes)?;
    set_executable(full, kind == FileKind::Exec)
}

#[cfg(unix)]
fn set_executable(full: &Path, executable: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(full)?.permissions();
    let mode = permissions.mode();
    let wanted = if executable {
        // Execute wherever it can be read, as git checks such a file out.
        mode | ((mode & 0o444) >> 2)
    } else {
        mode & !0o111
    };
    if wanted != mode {
        permissions.set_mode(wanted);
        std::fs::set_permissions(full, permissions)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_: &Path, _: bool) -> std::io::Result<()> {
    Ok(())
}

/// Every untracked file at or under each of `paths`: what `git clean -f -d`
/// on those paths removes. Ignored files are left out because clean leaves
/// them too.
pub fn untracked_under(root: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    let specs: Vec<String> = paths
        .iter()
        .map(|path| format!(":(literal){path}"))
        .collect();
    let mut args = vec!["ls-files", "-z", "--others", "--exclude-standard", "--"];
    args.extend(specs.iter().map(String::as_str));
    let listed = git_cmd::run_checked(root, &args)?;
    Ok(listed
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect())
}

// --- the reflog, behind "More history…" --------------------------------------

/// One place HEAD or the branch has been.
#[derive(Serialize, Debug)]
pub struct Moment {
    pub oid: String,
    pub short: String,
    /// Where it was before this move, when there was anywhere.
    pub previous: Option<String>,
    pub time: i64,
    /// What git wrote down about the move: "commit: Tidy", "checkout: moving
    /// from a to b", "reset: moving to HEAD~1".
    pub action: String,
    /// The subject of the commit it landed on.
    pub subject: String,
    /// Which logs carry it: "HEAD", the branch's name, or both.
    pub logs: Vec<String>,
}

/// HEAD's reflog and the current branch's, newest first, as one list.
///
/// The two say mostly the same thing — every commit is in both — so a move
/// both logs carry is one row. Only reads: nothing here changes anything.
pub fn moments(state: &AppState, limit: usize) -> Result<Vec<Moment>, String> {
    let repo = state.repo()?;
    let mut logs: Vec<(String, String)> = vec![("HEAD".to_string(), "HEAD".to_string())];
    if !repo.head_detached().unwrap_or(false) {
        if let Some(name) = repo
            .head()
            .ok()
            .and_then(|head| head.shorthand().map(str::to_string))
        {
            logs.push((format!("refs/heads/{name}"), name));
        }
    }

    let mut out: Vec<Moment> = Vec::new();
    let mut seen: HashMap<(String, i64, String), usize> = HashMap::new();
    for (reference, label) in logs {
        let Ok(reflog) = repo.reflog(&reference) else {
            continue;
        };
        for entry in reflog.iter().take(limit) {
            let oid = entry.id_new();
            if oid.is_zero() {
                continue;
            }
            let time = entry.committer().when().seconds();
            let action = entry.message().unwrap_or("").to_string();
            let key = (oid.to_string(), time, action.clone());
            if let Some(&at) = seen.get(&key) {
                if let Some(moment) = out.get_mut(at) {
                    moment.logs.push(label.clone());
                }
                continue;
            }
            let previous = Some(entry.id_old())
                .filter(|old| !old.is_zero())
                .map(|old| old.to_string());
            let subject = repo
                .find_commit(oid)
                .ok()
                .and_then(|commit| commit.summary().map(str::to_string))
                .unwrap_or_default();
            seen.insert(key, out.len());
            out.push(Moment {
                short: oid.to_string().chars().take(7).collect(),
                oid: oid.to_string(),
                previous,
                time,
                action,
                subject,
                logs: vec![label.clone()],
            });
        }
    }
    // Stable, so moves made in the same second keep the order the log has.
    out.sort_by_key(|one| std::cmp::Reverse(one.time));
    out.truncate(limit);
    Ok(out)
}

/// A stash commit no longer on the list.
#[derive(Serialize, Debug)]
pub struct LostStash {
    pub oid: String,
    pub short: String,
    /// The stash's own message: "On main: tidy", "WIP on main: 1234abc …".
    pub message: String,
    pub time: i64,
}

/// Stashes that were dropped or popped and are still in the object database.
///
/// `git fsck --unreachable --no-reflogs` lists every commit nothing points at,
/// counting the stash list's own entries below the top as unreachable; those
/// are still listed, and are left out. What is left is narrowed to commits
/// shaped like a stash: two or three parents, a "WIP on" or "On" message, and
/// an "index on" commit as the second parent. Slow on a big repository, which
/// is why it only runs when asked.
pub fn lost_stashes(state: &AppState) -> Result<Vec<LostStash>, String> {
    let root = state.path()?;
    let said = git_cmd::run(
        &root,
        &["fsck", "--unreachable", "--no-reflogs", "--no-progress"],
    )?;
    // fsck exits non-zero over a broken object somewhere else in the
    // repository; what it did find is still worth showing.
    let listed: Vec<String> = git_cmd::run_checked(&root, &["stash", "list", "--format=%H"])
        .unwrap_or_default()
        .lines()
        .map(|line| line.trim().to_string())
        .collect();
    let repo = state.repo()?;
    let mut out = Vec::new();
    for line in said.stdout.lines() {
        let Some(oid) = line.strip_prefix("unreachable commit ") else {
            continue;
        };
        let oid = oid.trim();
        if listed.iter().any(|one| one == oid) {
            continue;
        }
        let Some(found) = stash_shaped(&repo, oid) else {
            continue;
        };
        out.push(found);
    }
    out.sort_by_key(|one| std::cmp::Reverse(one.time));
    out.truncate(100);
    Ok(out)
}

fn stash_shaped(repo: &git2::Repository, oid: &str) -> Option<LostStash> {
    let commit = repo.find_commit(git2::Oid::from_str(oid).ok()?).ok()?;
    if !(2..=3).contains(&commit.parent_count()) {
        return None;
    }
    let message = commit.summary()?.to_string();
    if !(message.starts_with("WIP on ") || message.starts_with("On ")) {
        return None;
    }
    let index = commit.parent(1).ok()?;
    if !index.summary().unwrap_or("").starts_with("index on ") {
        return None;
    }
    Some(LostStash {
        oid: oid.to_string(),
        short: oid.chars().take(7).collect(),
        message,
        time: commit.time().seconds(),
    })
}

/// Puts a stash commit back on the list, at the top, under its own message.
pub fn restore_stash(state: &AppState, oid: &str) -> Result<String, String> {
    let root = state.path()?;
    let repo = state.repo()?;
    let found = stash_shaped(&repo, oid).ok_or_else(|| {
        "That commit is not a stash, so it cannot go on the stash list".to_string()
    })?;
    if crate::journal::stash_index_at(&root, &found.oid).is_some() {
        return Err("That stash is on the list already".to_string());
    }
    git_cmd::run_checked(&root, &["stash", "store", "-m", &found.message, &found.oid])?;
    Ok(format!("Put {} back on the stash list", found.message))
}
