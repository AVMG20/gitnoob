//! What the Files tree changes on disk: new files and folders, renames and
//! moves, copies, deleting to the Trash, files dropped in from outside, and
//! taking a file out of git's hands.
//!
//! Two rules hold here as everywhere else:
//!
//! - Every path from the window is checked against the work tree before
//!   anything is touched. An existing one through [`existing`], which keeps a
//!   link as the link rather than following it, and a new one through
//!   [`fresh`], whose folders may not exist yet.
//! - What git has to know about goes through the `git` CLI. A rename or move of
//!   anything git tracks is `git mv`, so the commit records it as a rename and
//!   not as one file deleted and another added. Everything git does not track
//!   is plain file I/O, since git has nothing to say about it.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use base64::Engine;
use serde::Serialize;

use crate::files;
use crate::git_cmd;
use crate::state::AppState;

/// The most files a delete saves for undo. A folder with more than this in it
/// still goes to the Trash, which is where it can be brought back from.
const SAVE_LIMIT: usize = 10_000;

/// The biggest file a drop from outside may bring in, decoded.
const IMPORT_LIMIT: usize = 100 * 1024 * 1024;

/// The parts of a relative path, refused unless every one is a plain name and
/// none is `.git`.
fn parts(relative: &str) -> Result<Vec<String>, String> {
    if relative.starts_with(['/', '\\']) || Path::new(relative).is_absolute() {
        return Err(format!("{relative} is not inside the work tree"));
    }
    let trimmed = relative.trim_matches('/');
    if trimmed.is_empty() {
        return Err("A name is needed".to_string());
    }
    let mut out = Vec::new();
    for part in Path::new(trimmed).components() {
        match part {
            Component::Normal(name) => {
                let name = name.to_string_lossy().into_owned();
                if name.eq_ignore_ascii_case(".git") {
                    return Err(format!("{relative} would be inside .git"));
                }
                out.push(name);
            }
            Component::CurDir => {}
            _ => return Err(format!("{relative} is not inside the work tree")),
        }
    }
    if out.is_empty() {
        return Err("A name is needed".to_string());
    }
    Ok(out)
}

/// A path that is there now, as itself: its folder is checked against the
/// work tree, and the last part is not followed, so renaming or deleting a
/// link acts on the link and not on what it points at.
pub fn existing(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let names = parts(relative)?;
    let (name, folders) = names.split_last().expect("parts is never empty");
    let parent = files::resolve(root, &folders.join("/"), true)?;
    let full = parent.join(name);
    std::fs::symlink_metadata(&full).map_err(|_| format!("{relative} is not there any more"))?;
    Ok(full)
}

/// Where a new path would go. The folders on the way may not exist yet; the
/// nearest one that does is checked against the work tree, and the rest are
/// plain names under it.
pub fn fresh(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let names = parts(relative)?;
    let real_root = root
        .canonicalize()
        .map_err(|e| format!("Could not read the work tree: {e}"))?;
    let mut have = names.len() - 1;
    loop {
        let folder = names[..have].join("/");
        let exists = real_root.join(&folder).exists();
        if exists || have == 0 {
            let base = files::resolve(root, &folder, true)?;
            let mut full = base;
            for name in &names[have..] {
                full.push(name);
            }
            return Ok(full);
        }
        have -= 1;
    }
}

/// `a/b/c` as `a/b`, and a top-level name as `""`.
fn parent_of(relative: &str) -> &str {
    relative.rfind('/').map_or("", |at| &relative[..at])
}

fn name_of(relative: &str) -> &str {
    relative
        .rfind('/')
        .map_or(relative, |at| &relative[at + 1..])
}

fn join(dir: &str, name: &str) -> String {
    let dir = dir.trim_matches('/');
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Whether git tracks the path, or anything under it when it is a folder.
/// Read from the index with libgit2.
fn tracked(root: &Path, relative: &str) -> bool {
    let Ok(repo) = git2::Repository::open(root) else {
        return false;
    };
    let Ok(index) = repo.index() else {
        return false;
    };
    // On a filesystem that ignores case, the index may spell a name one way
    // and the disk another; git itself goes by `core.ignorecase`.
    let fold = ignores_case(&repo);
    let norm = |text: &str| {
        if fold {
            text.to_lowercase()
        } else {
            text.to_string()
        }
    };
    let relative = norm(relative);
    let inside = format!("{relative}/");
    index.iter().any(|entry| {
        let path = norm(&String::from_utf8_lossy(&entry.path));
        path == relative || path.starts_with(&inside)
    })
}

fn ignores_case(repo: &git2::Repository) -> bool {
    repo.config()
        .and_then(|config| config.get_bool("core.ignorecase"))
        .unwrap_or(false)
}

/// The pathspec magic for a literal path, case-blind where git is.
fn literal_magic(root: &Path) -> &'static str {
    let fold = git2::Repository::open(root).is_ok_and(|repo| ignores_case(&repo));
    if fold {
        ":(literal,icase)"
    } else {
        ":(literal)"
    }
}

/// Whether two paths are the same entry on disk, links not followed: what a
/// change of case alone looks like on a filesystem that ignores case.
fn same_entry(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (a.canonicalize(), b.canonicalize()) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
    }
}

/// Makes the folders missing above `full`, and says which it made, so a
/// move that fails can take them away again.
fn make_parents(full: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    let mut at = full.parent();
    while let Some(dir) = at {
        if std::fs::symlink_metadata(dir).is_ok() {
            break;
        }
        missing.push(dir.to_path_buf());
        at = dir.parent();
    }
    missing.reverse();
    for dir in &missing {
        std::fs::create_dir(dir)?;
    }
    Ok(missing)
}

fn remove_made(made: &[PathBuf]) {
    for dir in made.iter().rev() {
        let _ = std::fs::remove_dir(dir);
    }
}

/// Makes an empty file, or a folder, with any folders on the way. Refused
/// when something of that name is there already.
pub fn create(state: &AppState, relative: &str, dir: bool) -> Result<String, String> {
    let root = state.path()?;
    let relative = parts(relative)?.join("/");
    let full = fresh(&root, &relative)?;
    if std::fs::symlink_metadata(&full).is_ok() {
        return Err(format!("{relative} is already there"));
    }
    if dir {
        std::fs::create_dir_all(&full).map_err(|e| format!("Could not make {relative}: {e}"))?;
    } else {
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Could not make the folder for {relative}: {e}"))?;
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&full)
            .map_err(|e| format!("Could not make {relative}: {e}"))?;
    }
    Ok(relative)
}

/// Renames or moves one path. `git mv` when git tracks it (or anything in it,
/// for a folder), so the commit records a rename; a plain rename otherwise.
///
/// A change of case only (`readme.md` to `README.md`) is allowed on a
/// filesystem that would call the two the same file.
pub fn rename(state: &AppState, from: &str, to: &str) -> Result<String, String> {
    let root = state.path()?;
    let from = parts(from)?.join("/");
    let to = parts(to)?.join("/");
    if from == to {
        return Ok(to);
    }
    if to.starts_with(&format!("{from}/")) {
        return Err(format!("{from} cannot be moved into itself"));
    }
    let from_full = existing(&root, &from)?;
    let to_full = fresh(&root, &to)?;
    let same_file = from.eq_ignore_ascii_case(&to) && same_entry(&from_full, &to_full);
    if std::fs::symlink_metadata(&to_full).is_ok() && !same_file {
        return Err(format!("{to} is already there"));
    }
    let made =
        make_parents(&to_full).map_err(|e| format!("Could not make the folder for {to}: {e}"))?;
    let moved = if tracked(&root, &from) {
        if same_file {
            // `git mv d D` on a folder fails where the filesystem ignores
            // case, so a change of case goes through a name of its own.
            let step = format!("{to}.gitnoob-rename-{}", std::process::id());
            git_mv(&root, &from, &step).and_then(|()| git_mv(&root, &step, &to))
        } else {
            git_mv(&root, &from, &to)
        }
    } else {
        std::fs::rename(&from_full, &to_full)
            .map_err(|e| format!("Could not move {from} to {to}: {e}"))
    };
    if let Err(error) = moved {
        remove_made(&made);
        return Err(error);
    }
    // `-k` skips what git cannot move rather than stopping; the source still
    // being there under its own spelling means nothing moved.
    if !same_file
        && std::fs::symlink_metadata(&from_full).is_ok()
        && std::fs::symlink_metadata(&to_full).is_err()
    {
        remove_made(&made);
        return Err(format!("Git could not move {from}"));
    }
    Ok(to)
}

/// `git mv -k`: a tracked file deleted from disk inside a folder is skipped
/// rather than stopping the whole move, and stays an unstaged deletion where
/// it was. Paths to `git mv` are paths, not pathspecs, so nothing in them is a
/// wildcard; `--` keeps a name starting with a dash from being an option.
fn git_mv(root: &Path, from: &str, to: &str) -> Result<(), String> {
    git_cmd::run_checked(root, &["mv", "-k", "--", from, to]).map(|_| ())
}

/// What a move of several paths did: each one that moved, from where to
/// where, and why it stopped when it did not get through them all.
#[derive(Serialize, Debug, Default)]
pub struct Moved {
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub error: Option<String>,
}

/// Moves each path into a folder, under its own name, stopping at the first
/// that cannot go. A path already in that folder stays where it is.
pub fn move_into(state: &AppState, paths: &[String], dest: &str) -> Result<Moved, String> {
    let dest = dest.trim_matches('/');
    let mut out = Moved::default();
    for path in paths {
        let path = parts(path)?.join("/");
        if parent_of(&path) == dest {
            continue;
        }
        if dest == path || dest.starts_with(&format!("{path}/")) {
            out.error = Some(format!("{path} cannot be moved into itself"));
            break;
        }
        let to = join(dest, name_of(&path));
        match rename(state, &path, &to) {
            Ok(now) => {
                out.from.push(path);
                out.to.push(now);
            }
            Err(error) => {
                out.error = Some(error);
                break;
            }
        }
    }
    Ok(out)
}

/// A name in `dir` nothing has yet: `name` itself when it is free, else
/// `stem copy.ext`, `stem copy 2.ext` and so on.
pub fn free_name(state: &AppState, dir: &str, name: &str) -> Result<String, String> {
    let root = state.path()?;
    free_name_at(&root, dir, name)
}

fn free_name_at(root: &Path, dir: &str, name: &str) -> Result<String, String> {
    let name = name_of(name.trim_matches('/'));
    parts(name)?;
    let taken = |candidate: &str| -> Result<bool, String> {
        let full = fresh(root, &join(dir, candidate))?;
        Ok(std::fs::symlink_metadata(full).is_ok())
    };
    if !taken(name)? {
        return Ok(join(dir, name));
    }
    // `.env` has no extension, `archive.tar.gz` keeps only `.gz` apart.
    let (stem, ext) = match name.rfind('.') {
        Some(at) if at > 0 => (&name[..at], &name[at..]),
        _ => (name, ""),
    };
    for n in 1..10_000 {
        let candidate = if n == 1 {
            format!("{stem} copy{ext}")
        } else {
            format!("{stem} copy {n}{ext}")
        };
        if !taken(&candidate)? {
            return Ok(join(dir, &candidate));
        }
    }
    Err(format!("Could not find a free name for {name}"))
}

/// Copies each path into a folder. A name that is taken there gets ` copy`
/// added, so copying into the folder a file is in is a duplicate. Answers
/// with the new paths.
pub fn copy_into(state: &AppState, paths: &[String], dest: &str) -> Result<Vec<String>, String> {
    let root = state.path()?;
    let dest = dest.trim_matches('/');
    let dest_full = files::resolve(&root, dest, true)?;
    if !dest_full.is_dir() {
        return Err(format!("{dest} is not a folder"));
    }
    let mut made = Vec::new();
    for path in paths {
        let path = parts(path)?.join("/");
        let from_full = existing(&root, &path)?;
        if let Ok(real) = from_full.canonicalize() {
            if from_full.is_dir() && dest_full.starts_with(&real) {
                return Err(format!("{path} cannot be copied into itself"));
            }
        }
        let to = free_name_at(&root, dest, name_of(&path))?;
        let to_full = fresh(&root, &to)?;
        copy_entry(&from_full, &to_full).map_err(|e| format!("Could not copy {path}: {e}"))?;
        made.push(to);
    }
    Ok(made)
}

/// Copies a file, a link or a folder with everything in it. A link is copied
/// as the link where the platform allows it, never as what it points at.
fn copy_entry(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        {
            let target = std::fs::read_link(from)?;
            return std::os::unix::fs::symlink(target, to);
        }
        #[cfg(not(unix))]
        {
            if std::fs::metadata(from)?.is_dir() {
                return Ok(());
            }
            std::fs::copy(from, to)?;
            return Ok(());
        }
    }
    if meta.is_dir() {
        std::fs::create_dir(to)?;
        for item in std::fs::read_dir(from)? {
            let item = item?;
            if item.file_name() == ".git" {
                continue;
            }
            copy_entry(&item.path(), &to.join(item.file_name()))?;
        }
        return Ok(());
    }
    std::fs::copy(from, to)?;
    Ok(())
}

/// Every file at or under `paths` that git tracks or would list as untracked:
/// what a delete is saved for, so undo can put it back. What git ignores is
/// left to the Trash.
fn saveable(root: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    let magic = literal_magic(root);
    let specs: Vec<String> = paths.iter().map(|path| format!("{magic}{path}")).collect();
    let mut args = vec![
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
        "--",
    ];
    args.extend(specs.iter().map(String::as_str));
    let listed = git_cmd::run_checked(root, &args)?;
    let found: BTreeSet<String> = listed
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect();
    Ok(found.into_iter().collect())
}

/// Deletes files and folders, tracked or not. With `to_trash` they go to the
/// Trash (the Recycle Bin on Windows); without, they are removed, which only
/// the tests ask for. What git tracks shows up as deleted afterwards, so
/// discarding brings it back, and the delete is a step on the undo history
/// either way.
pub fn delete(state: &AppState, paths: &[String], to_trash: bool) -> Result<String, String> {
    if paths.is_empty() {
        return Err("Nothing to delete".to_string());
    }
    let root = state.path()?;
    let mut clean = Vec::new();
    let mut fulls = Vec::new();
    for path in paths {
        let path = parts(path)?.join("/");
        fulls.push(existing(&root, &path)?);
        clean.push(path);
    }
    let label = match clean.as_slice() {
        [one] => format!("Delete {one}"),
        many => format!("Delete {}", plural(many.len(), "item")),
    };
    let act = || -> Result<(), String> {
        if to_trash {
            to_the_trash(&fulls)
        } else {
            for full in &fulls {
                let gone = if std::fs::symlink_metadata(full).is_ok_and(|m| m.is_dir()) {
                    std::fs::remove_dir_all(full)
                } else {
                    std::fs::remove_file(full)
                };
                gone.map_err(|e| format!("Could not delete {}: {e}", full.display()))?;
            }
            Ok(())
        }
    };
    let saved = saveable(&root, &clean)?;
    if saved.len() > SAVE_LIMIT {
        act()?;
    } else {
        crate::work::throw_away_saved(state, &root, &saved, "delete", label, act)?;
    }
    Ok(match clean.as_slice() {
        [one] if to_trash => format!("Moved {one} to the {}", trash_name()),
        [one] => format!("Deleted {one}"),
        many if to_trash => format!(
            "Moved {} to the {}",
            plural(many.len(), "item"),
            trash_name()
        ),
        many => format!("Deleted {}", plural(many.len(), "item")),
    })
}

fn to_the_trash(fulls: &[PathBuf]) -> Result<(), String> {
    #[allow(unused_mut)]
    let mut context = trash::TrashContext::default();
    // The Finder way plays the sound and asks for permission to script the
    // Finder; the file manager call does neither.
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        context.set_delete_method(DeleteMethod::NsFileManager);
    }
    context
        .delete_all(fulls)
        .map_err(|e| format!("Could not move it to the {}: {e}", trash_name()))
}

fn trash_name() -> &'static str {
    if cfg!(windows) {
        "Recycle Bin"
    } else {
        "Trash"
    }
}

/// Takes paths out of the index and leaves them on disk: `git rm --cached`.
/// The next commit deletes them from the repository, and they show up as
/// untracked (or not at all, once ignored).
pub fn untrack(state: &AppState, paths: &[String]) -> Result<String, String> {
    if paths.is_empty() {
        return Err("Nothing to stop tracking".to_string());
    }
    let root = state.path()?;
    if let Some(loose) = paths
        .iter()
        .find(|path| !tracked(&root, path.trim_matches('/')))
    {
        return Err(format!(
            "Git does not track {loose}, so there is nothing to stop"
        ));
    }
    let specs: Vec<String> = paths
        .iter()
        .map(|path| format!(":(literal){}", path.trim_matches('/')))
        .collect();
    let mut args = vec!["rm", "--cached", "-r", "--quiet", "--"];
    args.extend(specs.iter().map(String::as_str));
    git_cmd::run_checked(&root, &args)?;
    Ok(match paths {
        [one] => format!("Git no longer tracks {one}; it is still on disk"),
        many => format!(
            "Git no longer tracks {}; they are still on disk",
            plural(many.len(), "path")
        ),
    })
}

/// Writes a file dropped in from outside the window, as base64. Refused when
/// something of that name is there: [`free_name`] finds one that is not.
pub fn import(state: &AppState, relative: &str, data: &str) -> Result<String, String> {
    let root = state.path()?;
    let relative = parts(relative)?.join("/");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| format!("Could not read the dropped file {relative}: {e}"))?;
    if bytes.len() > IMPORT_LIMIT {
        return Err(format!(
            "{relative} is over 100 MB, so it was not copied in"
        ));
    }
    let full = fresh(&root, &relative)?;
    if std::fs::symlink_metadata(&full).is_ok() {
        return Err(format!("{relative} is already there"));
    }
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Could not make the folder for {relative}: {e}"))?;
    }
    std::fs::write(&full, bytes).map_err(|e| format!("Could not write {relative}: {e}"))?;
    Ok(relative)
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        format!("1 {word}")
    } else {
        format!("{count} {word}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_plain_names_only() {
        assert_eq!(parts("a/b/c.txt").unwrap(), ["a", "b", "c.txt"]);
        assert_eq!(parts("a/").unwrap(), ["a"]);
        assert!(parts("/etc/x").is_err());
        assert!(parts("").is_err());
        assert!(parts("a/../b").is_err());
        assert!(parts(".git/config").is_err());
        assert!(parts("a/.GIT/x").is_err());
    }

    #[test]
    fn parents_and_names() {
        assert_eq!(parent_of("a/b/c"), "a/b");
        assert_eq!(parent_of("c"), "");
        assert_eq!(name_of("a/b/c"), "c");
        assert_eq!(join("", "x"), "x");
        assert_eq!(join("a/", "x"), "a/x");
    }
}
