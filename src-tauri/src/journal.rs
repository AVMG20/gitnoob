//! Undo and redo.
//!
//! Every repository — every work tree, to be exact — has a history of its own.
//! It used to be one list for the whole window, so a second tab's history menu
//! listed the first tab's steps and its undo took them; with two worktrees of
//! one repository open, where the stash and the objects are shared, that undid
//! a stash into the wrong folder. Now the list is looked up by the work tree's
//! canonical path, every step remembers which work tree it was recorded in,
//! and an undo is pinned to the folder it was asked about from start to end.
//!
//! The history is also written to disk, next to git's own state for that work
//! tree, so it survives a restart. A step whose recorded state no longer
//! matches the repository — the branch moved in a terminal, the stash was
//! dropped by hand — is listed as unavailable rather than attempted.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git_cmd;
use crate::state::AppState;

/// How an entry is stepped back and forth.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Move the branch pointer, keep the changes staged. For commits and amends.
    Soft,
    /// Move the branch pointer and the working tree. For merges. A branch that
    /// is not checked out anywhere is moved on its own, ref only.
    Hard,
    /// A stash push: undoing pops it back, redoing stashes again.
    Stash,
    /// A stash applied onto the tree while the entry stayed on the list:
    /// undoing takes those files back off, redoing puts them on again.
    Apply,
    /// A stash popped cleanly: undoing takes the files back off and puts the
    /// stash back on the list, redoing pops it again.
    Pop,
    /// A stash dropped from the list: undoing stores it again.
    Drop,
    /// Working-tree files thrown away — a discard, a hunk discard, deleting
    /// untracked files. What was there was saved first; undoing writes it back.
    Files,
    /// A branch deleted: undoing makes it again where it pointed.
    Branch,
}

/// How a saved file was kept on disk.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    File,
    /// A file with the executable bit set.
    Exec,
    /// A symbolic link; the blob holds where it points.
    Link,
}

/// One version of a file, saved as a blob in the object database.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Saved {
    pub blob: String,
    pub kind: FileKind,
}

/// A file a step changed: what it was before and what it was after. `None`
/// means the file was not there.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileSave {
    pub path: String,
    pub before: Option<Saved>,
    pub after: Option<Saved>,
}

/// What a deleted branch tracked, so undoing the deletion puts that back too.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Upstream {
    pub remote: String,
    pub merge: String,
}

/// What the index held for a path before a stash went on: `None` when the
/// path was not in it. Undoing the apply puts exactly this back, so work that
/// was staged before the stash came off is not traded for the committed copy.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Staged {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<IndexSaved>,
}

/// One index entry: its mode as git writes it (`100644`) and its blob.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct IndexSaved {
    pub mode: String,
    pub blob: String,
}

/// One reversible operation.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    #[serde(default)]
    pub id: usize,
    /// What the user did, in their words: "Commit: Tidy the parser".
    pub label: String,
    pub kind: String,
    /// The branch the operation moved, so undo can refuse if HEAD has moved on.
    pub branch: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub mode: Mode,
    /// True when stepping back or forth touches the working tree.
    pub destructive: bool,
    pub at: i64,
    /// The work tree the step was recorded in, canonical.
    #[serde(default)]
    pub repo: String,
    /// The stash's own line in the list, for the stash modes, so a stash put
    /// back reads exactly as it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Where HEAD stood when the step was recorded, for the modes that are
    /// only safe to reverse while it still stands there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// The files a `Files` step changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileSave>,
    /// What a deleted branch tracked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<Upstream>,
    /// Every branch a tidy deleted, for a `Branch` step that took more than
    /// one. A single deletion keeps using `branch`, `before` and `upstream`,
    /// so a history written by an older version still reads the same.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<BranchSnapshot>,
    /// For a stash apply or pop: what the index held for each path the stash
    /// touched, just before it went on. `files` holds the working tree's side,
    /// before and after.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub staged: Vec<Staged>,
    /// Filled in when the history is read: why this step cannot be taken as
    /// things stand. Never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

impl Entry {
    /// A step with nothing filled in but what every step has. The caller sets
    /// the rest before handing it to [`record_entry`].
    pub fn new(kind: &str, label: String, mode: Mode, destructive: bool) -> Self {
        Entry {
            id: 0,
            label,
            kind: kind.to_string(),
            branch: None,
            before: None,
            after: None,
            mode,
            destructive,
            at: now(),
            repo: String::new(),
            message: None,
            head: None,
            files: Vec::new(),
            upstream: None,
            branches: Vec::new(),
            staged: Vec::new(),
            unavailable: None,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct Stacks {
    pub undo: Vec<Entry>,
    pub redo: Vec<Entry>,
}

/// Enough history to cover a session's mistakes without growing unbounded.
const LIMIT: usize = 100;

/// The most the file on disk may hold. A discard of a few thousand files is
/// one step with a line for each, and a history of those is not worth a slow
/// start: the oldest steps go first until it fits.
const MAX_BYTES: usize = 2 * 1024 * 1024;

/// The file a work tree's history is kept in, inside git's own directory for
/// it — a linked worktree has a directory of its own there, so two worktrees
/// of one repository never share it.
const FILE: &str = "gitnoob-journal.json";

#[derive(Default, Serialize, Deserialize)]
pub struct Journal {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    next_id: usize,
    /// Where it is kept; `None` for one that is not written anywhere.
    #[serde(skip)]
    file: Option<PathBuf>,
    #[serde(skip)]
    dirty: bool,
    /// The work tree's key, stamped on every step read from disk.
    #[serde(skip)]
    key: String,
    /// The file as this process last read or wrote it. Another window of the
    /// app — a second process with the same repository open — writes the
    /// same file, and anything there that is not this is its doing.
    #[serde(skip)]
    seen: Option<String>,
    /// The steps that were in the file as last seen. One here that is not is
    /// a step this process made and could not write out yet.
    #[serde(skip)]
    synced: HashSet<usize>,
    /// The file is there but could not be read the last time it was looked
    /// at: held open by another program on Windows, or its permissions gone
    /// wrong. Nothing is written over it until it can be read again, since
    /// what is in memory may be missing every step it holds.
    #[serde(skip)]
    blocked: bool,
}

/// What reading the history file came to.
enum Disk {
    Missing,
    /// There, but it could not be read just now.
    Busy,
    /// There, but not something this version can read: written by a newer
    /// one, or broken.
    Unreadable,
    Read(Box<Journal>, String),
}

fn read_disk(file: &Path) -> Disk {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Disk::Missing,
        Err(_) => return Disk::Busy,
    };
    parse_disk(text)
}

fn parse_disk(text: String) -> Disk {
    match serde_json::from_str::<Journal>(&text) {
        Ok(journal) => Disk::Read(Box::new(journal), text),
        Err(_) => Disk::Unreadable,
    }
}

/// Moves a history file this version cannot read out of the way, so the next
/// write does not destroy it. A newer version that wrote it can still read it
/// back from there; nothing is lost by starting a fresh one beside it.
fn set_aside(file: &Path) {
    let mut aside = file.as_os_str().to_owned();
    aside.push(format!(".bad-{}", now()));
    let _ = std::fs::rename(file, PathBuf::from(aside));
}

impl Journal {
    /// The history kept for `root`, or an empty one when there is none yet or
    /// the file cannot be read. A history that will not load is not a reason
    /// to refuse to open the repository — but it is set aside rather than
    /// written over.
    pub fn load(root: &Path, key: &Path) -> Journal {
        let file = crate::remote::git_dir(root).ok().map(|dir| dir.join(FILE));
        let mut journal = match file.as_deref().map(read_disk) {
            Some(Disk::Read(mut journal, text)) => {
                journal.seen = Some(text);
                journal.synced = journal.ids();
                *journal
            }
            Some(Disk::Unreadable) => {
                if let Some(file) = &file {
                    set_aside(file);
                }
                Journal::default()
            }
            // Read again on the first use, and not written to before then.
            Some(Disk::Busy) => Journal {
                blocked: true,
                ..Journal::default()
            },
            Some(Disk::Missing) | None => Journal::default(),
        };
        // The file sits inside this work tree's own git directory, so whatever
        // is in it was recorded here — under whatever path the folder had at
        // the time. A folder that was moved keeps its history.
        journal.key = key.to_string_lossy().into_owned();
        journal.stamp();
        journal.next_id = journal.next_id.max(journal.highest());
        journal.file = file;
        journal
    }

    fn ids(&self) -> HashSet<usize> {
        self.undo
            .iter()
            .chain(self.redo.iter())
            .map(|entry| entry.id)
            .collect()
    }

    fn highest(&self) -> usize {
        self.ids().into_iter().max().unwrap_or(0)
    }

    fn stamp(&mut self) {
        for entry in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            entry.repo = self.key.clone();
            entry.unavailable = None;
        }
    }

    /// Catches up with what another process wrote to the file since this one
    /// last read or wrote it.
    ///
    /// Two windows of the app on one repository each hold the history in
    /// memory, and each used to write all of it out: the second one to save
    /// wiped every step the first had made. Now the file is read again before
    /// every use. What it says wins for every step both have seen; a step
    /// this process made and could not write out yet is kept, numbered after
    /// the file's own.
    pub fn refresh(&mut self) {
        let Some(file) = self.file.clone() else {
            return;
        };
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                // Keep what is in memory and write nothing until it reads.
                self.blocked = true;
                return;
            }
        };
        if text == self.seen && !self.blocked {
            return;
        }
        self.blocked = false;
        let disk = match text.clone().map(parse_disk).unwrap_or(Disk::Missing) {
            Disk::Read(journal, _) => *journal,
            Disk::Missing | Disk::Busy => Journal::default(),
            Disk::Unreadable => {
                // Not ours to read and not ours to write over. What is in
                // memory is written out fresh beside it.
                set_aside(&file);
                self.seen = None;
                self.dirty = true;
                return;
            }
        };
        let synced = std::mem::take(&mut self.synced);
        let unsaved_undo: Vec<Entry> = std::mem::take(&mut self.undo)
            .into_iter()
            .filter(|entry| !synced.contains(&entry.id))
            .collect();
        let unsaved_redo: Vec<Entry> = std::mem::take(&mut self.redo)
            .into_iter()
            .filter(|entry| !synced.contains(&entry.id))
            .collect();
        self.undo = disk.undo;
        self.redo = disk.redo;
        self.synced = self.ids();
        self.seen = text;
        self.next_id = self.next_id.max(disk.next_id).max(self.highest());
        for (mut entry, undo) in unsaved_undo
            .into_iter()
            .map(|entry| (entry, true))
            .chain(unsaved_redo.into_iter().map(|entry| (entry, false)))
        {
            if self.synced.contains(&entry.id) {
                self.next_id += 1;
                entry.id = self.next_id;
            }
            if undo {
                self.undo.push(entry);
            } else {
                self.redo.push(entry);
            }
            self.dirty = true;
        }
        self.stamp();
    }

    /// Writes the history out if it changed. Written beside and renamed over,
    /// so a crash half-way leaves the old file rather than half of a new one.
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(file) = self.file.clone() else {
            self.dirty = false;
            return;
        };
        if self.blocked {
            // Still dirty, so the next use that can read the file writes it.
            return;
        }
        let mut text = serde_json::to_string(self).unwrap_or_default();
        while text.len() > MAX_BYTES && (!self.undo.is_empty() || !self.redo.is_empty()) {
            // A discard's saved copy is reachable from its step and nowhere
            // else, so every other kind of step goes first.
            // The newest undo step stays for the same reason as in `evict`.
            let older = self.undo.len().saturating_sub(1);
            if let Some(at) = self.undo[..older]
                .iter()
                .position(|entry| entry.mode != Mode::Files)
            {
                self.undo.remove(at);
            } else if let Some(at) = self.redo.iter().position(|entry| entry.mode != Mode::Files) {
                self.redo.remove(at);
            } else if !self.undo.is_empty() {
                self.undo.remove(0);
            } else {
                self.redo.remove(0);
            }
            text = serde_json::to_string(self).unwrap_or_default();
        }
        let temp = file.with_extension("json.tmp");
        if std::fs::write(&temp, &text).is_ok() && std::fs::rename(&temp, &file).is_ok() {
            self.dirty = false;
            self.synced = self.ids();
            self.seen = Some(text);
        }
    }

    /// Adds a step, handing back the id it was given.
    pub fn push(&mut self, mut entry: Entry) -> usize {
        self.next_id += 1;
        entry.id = self.next_id;
        // A fresh operation invalidates anything that was undone.
        self.redo.clear();
        self.undo.push(entry);
        if self.undo.len() > LIMIT {
            evict(&mut self.undo);
        }
        self.dirty = true;
        self.next_id
    }

    pub fn stacks(&self) -> Stacks {
        Stacks {
            // Newest first: that is the order the history menu reads in.
            undo: self.undo.iter().rev().cloned().collect(),
            redo: self.redo.iter().rev().cloned().collect(),
        }
    }

    pub fn take_undo(&mut self) -> Option<Entry> {
        self.dirty = true;
        self.undo.pop()
    }

    pub fn take_redo(&mut self) -> Option<Entry> {
        self.dirty = true;
        self.redo.pop()
    }

    pub fn put_redo(&mut self, entry: Entry) {
        self.dirty = true;
        self.redo.push(entry);
        if self.redo.len() > LIMIT {
            evict(&mut self.redo);
        }
    }

    pub fn put_undo(&mut self, entry: Entry) {
        self.dirty = true;
        self.undo.push(entry);
        if self.undo.len() > LIMIT {
            evict(&mut self.undo);
        }
    }

    /// Takes one step off the undo stack by id, with where it stood, for
    /// [`undo_step`].
    fn take_undo_id(&mut self, id: usize) -> Option<(usize, Entry)> {
        let at = self.undo.iter().position(|entry| entry.id == id)?;
        self.dirty = true;
        Some((at, self.undo.remove(at)))
    }

    /// Puts a step taken by [`Journal::take_undo_id`] back where it stood.
    fn restore_undo(&mut self, at: usize, entry: Entry) {
        self.dirty = true;
        let at = at.min(self.undo.len());
        self.undo.insert(at, entry);
    }

    /// Takes one step out of the history, wherever it is.
    pub fn remove(&mut self, id: usize) -> Option<Entry> {
        for stack in [&mut self.undo, &mut self.redo] {
            if let Some(at) = stack.iter().position(|entry| entry.id == id) {
                self.dirty = true;
                return Some(stack.remove(at));
            }
        }
        None
    }
}

/// Makes room on a stack that is over its limit.
///
/// The oldest step goes, unless it is a discard: the saved copy of what was
/// thrown away is reachable from that step and from nowhere else, so a tidy
/// that deleted a hundred branches, or a long afternoon of commits, must not
/// push it off. Only a stack of nothing but discards loses one of those.
///
/// The newest step is never the one picked: it is the one just recorded, and
/// a history full of discards would otherwise let go of every commit, merge
/// or rebase the moment it was made.
fn evict(stack: &mut Vec<Entry>) {
    let older = stack.len().saturating_sub(1);
    let at = stack[..older]
        .iter()
        .position(|entry| entry.mode != Mode::Files)
        .unwrap_or(0);
    stack.remove(at);
}

/// What a history is filed under: the work tree's canonical path, so the same
/// folder reached through a symlink, or written with a separator on the end,
/// is the same history.
pub fn key(root: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    crate::state::trim_separator(&canonical)
}

// --- reading the repository at a pinned path ---------------------------------
//
// An undo reads the path once and uses it to the end. Asking the state again
// half-way would follow a tab switched in the meantime to another repository.

fn open(root: &Path) -> Option<git2::Repository> {
    git2::Repository::open(root).ok()
}

/// The commit HEAD points at in `root`, or `None` with no commits yet.
pub fn head_at(root: &Path) -> Option<String> {
    let repo = open(root)?;
    let oid = repo.head().ok()?.target()?;
    Some(oid.to_string())
}

/// The branch checked out in `root`, or `None` when HEAD is detached.
pub fn branch_at(root: &Path) -> Option<String> {
    let repo = open(root)?;
    if repo.head_detached().unwrap_or(false) {
        return None;
    }
    let head = repo.head().ok()?;
    let name = head.shorthand()?.to_string();
    Some(name)
}

/// Where a local branch points, or `None` when there is no such branch.
pub fn tip_at(root: &Path, branch: &str) -> Option<String> {
    let repo = open(root)?;
    let found = repo.find_reference(&format!("refs/heads/{branch}")).ok()?;
    let oid = found.target()?;
    Some(oid.to_string())
}

/// The current HEAD commit, or None in a repository with no commits.
pub fn head_oid(state: &AppState) -> Option<String> {
    head_at(&state.path().ok()?)
}

/// The checked out branch, or None when HEAD is detached.
pub fn current_branch(state: &AppState) -> Option<String> {
    branch_at(&state.path().ok()?)
}

/// The commit `refs/stash` currently points at, or `None` when there is no
/// stash at all.
///
/// `git stash push` exits 0 whether or not it made anything — a clean tree
/// prints "No local changes to save" and leaves `refs/stash` untouched. Reading
/// this before and after a push is the only reliable way to tell "it made a
/// stash" from "there was nothing to stash", since matching that message would
/// break the moment git's wording changes or the locale isn't English.
pub(crate) fn stash_ref(root: &Path) -> Option<String> {
    git_cmd::run_checked(root, &["rev-parse", "--quiet", "--verify", "refs/stash"])
        .ok()
        .map(|out| out.trim().to_string())
        .filter(|oid| !oid.is_empty())
}

/// Every stash on the list, newest first.
fn stashes_at(root: &Path) -> Vec<String> {
    git_cmd::run_checked(root, &["stash", "list", "--format=%H"])
        .map(|listed| {
            listed
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The index a stash commit sits at in `root`'s list, or `None` if it is not
/// listed.
pub fn stash_index_at(root: &Path, oid: &str) -> Option<usize> {
    stashes_at(root).iter().position(|line| line == oid)
}

/// The index a stash commit sits at now, or `None` if it is no longer listed.
///
/// `stash@{0}` is whatever was stashed last, which is not the same thing as the
/// stash you mean: anything stashed since — here, or in a terminal — sits above
/// it. Naming the commit is the only way to be sure.
pub fn stash_index(state: &AppState, oid: &str) -> Option<usize> {
    stash_index_at(&state.path().ok()?, oid)
}

/// The line a stash has in the list — "On main: tidy" — which is what
/// `git stash store -m` needs to put it back looking the same.
pub fn stash_message_at(root: &Path, oid: &str) -> Option<String> {
    let listed = git_cmd::run_checked(root, &["stash", "list", "--format=%H%x00%gs"]).ok()?;
    listed.lines().find_map(|line| {
        let (id, said) = line.split_once('\0')?;
        (id.trim() == oid).then(|| said.to_string())
    })
}

/// What git is part-way through in `root`, in words, or `None`.
pub fn running_at(root: &Path) -> Option<&'static str> {
    let dir = crate::remote::git_dir(root).ok()?;
    if dir.join("rebase-merge").exists() || dir.join("rebase-apply").exists() {
        Some("rebase")
    } else if dir.join("MERGE_HEAD").exists() {
        Some("merge")
    } else if dir.join("CHERRY_PICK_HEAD").exists() {
        Some("cherry-pick")
    } else if dir.join("REVERT_HEAD").exists() {
        Some("revert")
    } else {
        None
    }
}

/// The merge or rebase git is part-way through, as something that tells one
/// from another: the commit being merged, or where a rebase started and what
/// it is going onto. The same on every stop of one rebase, and gone the moment
/// it ends.
fn underway_at(root: &Path) -> Option<String> {
    let dir = crate::remote::git_dir(root).ok()?;
    let read = |name: &str| {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|text| !text.is_empty())
    };
    if let Some(head) = read("MERGE_HEAD") {
        return Some(format!("merge {head}"));
    }
    ["rebase-merge", "rebase-apply"].iter().find_map(|sub| {
        let started = read(&format!("{sub}/orig-head"))?;
        let onto = read(&format!("{sub}/onto")).unwrap_or_default();
        Some(format!("rebase {started} {onto}"))
    })
}

/// The folder a branch is checked out in, other than `root`, if any.
fn checked_out_elsewhere(root: &Path, branch: &str) -> Option<String> {
    let raw = git_cmd::run_checked(root, &["worktree", "list", "--porcelain"]).ok()?;
    let here = key(root);
    let wanted = format!("branch refs/heads/{branch}");
    raw.split("\n\n").find_map(|block| {
        let path = block
            .lines()
            .find_map(|line| line.strip_prefix("worktree "))?;
        let holds = block.lines().any(|line| line.trim() == wanted);
        (holds && key(Path::new(path)) != here).then(|| path.to_string())
    })
}

// --- recording ---------------------------------------------------------------

/// Records an operation. Called after it succeeded, never before.
// One entry in the undo history, described in full at the call site so that
// nothing is recorded by accident. A struct here would be the same list.
#[allow(clippy::too_many_arguments)]
pub fn record(
    state: &AppState,
    kind: &str,
    label: String,
    branch: Option<String>,
    before: Option<String>,
    after: Option<String>,
    mode: Mode,
    destructive: bool,
) {
    let mut entry = Entry::new(kind, label, mode, destructive);
    entry.branch = branch;
    entry.before = before;
    entry.after = after;
    record_entry(state, entry);
}

/// Records a step built with [`Entry::new`], for the modes that carry more
/// than a branch and two commits.
pub fn record_entry(state: &AppState, entry: Entry) {
    if let Ok(root) = state.path() {
        record_entry_at(state, &root, entry);
    }
}

/// [`record_entry`] for a work tree read once at the start of an operation.
/// Hands back the id the step was given, or `None` when nothing was worth
/// recording.
pub fn record_entry_at(state: &AppState, root: &Path, mut entry: Entry) -> Option<usize> {
    // An operation that changed nothing is not worth an undo step.
    if matches!(entry.mode, Mode::Soft | Mode::Hard) && entry.before == entry.after {
        return None;
    }
    if entry.mode == Mode::Files && entry.files.is_empty() {
        return None;
    }
    entry.repo = key(root).to_string_lossy().into_owned();
    entry.unavailable = None;
    Some(state.journal_at(root, |journal| journal.push(entry)))
}

/// A branch as it stood just before it was deleted.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BranchSnapshot {
    pub name: String,
    pub oid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<Upstream>,
}

/// Reads what deleting a branch would lose, to hand to
/// [`record_branch_deleted`] once it is gone.
pub fn snapshot_branch(root: &Path, name: &str) -> Option<BranchSnapshot> {
    let oid = tip_at(root, name)?;
    let read = |key: &str| {
        git_cmd::run_checked(root, &["config", "--get", &format!("branch.{name}.{key}")])
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let upstream = match (read("remote"), read("merge")) {
        (Some(remote), Some(merge)) => Some(Upstream { remote, merge }),
        _ => None,
    };
    Some(BranchSnapshot {
        name: name.to_string(),
        oid,
        upstream,
    })
}

/// Puts a deleted branch on the history. Undoing makes it again at the commit
/// it pointed at, tracking what it tracked; it refuses if a branch of that name
/// has been made since.
///
/// `root` is the work tree the deletion ran in, read once at its start.
pub fn record_branch_deleted(
    state: &AppState,
    root: &Path,
    snapshot: BranchSnapshot,
) -> Option<usize> {
    record_branches_deleted(state, root, vec![snapshot])
}

/// Puts a batch of deleted branches on the history as one step.
///
/// A tidy can delete a hundred branches, and a step for each pushed everything
/// older off the history — a discard's saved copy with it. One step undoes
/// the lot. A batch of one is written the way a single deletion always was.
pub fn record_branches_deleted(
    state: &AppState,
    root: &Path,
    mut snapshots: Vec<BranchSnapshot>,
) -> Option<usize> {
    let mut entry = match snapshots.len() {
        0 => return None,
        1 => {
            let snapshot = snapshots.remove(0);
            let mut entry = Entry::new(
                "delete-branch",
                format!("Delete branch {}", snapshot.name),
                Mode::Branch,
                false,
            );
            entry.branch = Some(snapshot.name);
            entry.before = Some(snapshot.oid);
            entry.upstream = snapshot.upstream;
            entry
        }
        n => {
            let mut entry = Entry::new(
                "delete-branches",
                format!("Delete {n} branches"),
                Mode::Branch,
                false,
            );
            entry.branches = snapshots;
            entry
        }
    };
    entry.unavailable = None;
    record_entry_at(state, root, entry)
}

/// The branches a `Branch` step deleted, whichever way it was written.
fn deleted_branches(entry: &Entry) -> Vec<BranchSnapshot> {
    if !entry.branches.is_empty() {
        return entry.branches.clone();
    }
    match (&entry.branch, &entry.before) {
        (Some(name), Some(oid)) => vec![BranchSnapshot {
            name: name.clone(),
            oid: oid.clone(),
            upstream: entry.upstream.clone(),
        }],
        _ => Vec::new(),
    }
}

// --- steps that stopped part-way ---------------------------------------------
//
// A merge, pull or rebase that stops on a conflict has not finished, so there
// is nothing whole to record yet. What it started from is written down beside
// git's own state, and whatever finishes it — a commit, a continue, a skip —
// records the step then. An abort finishes it too, with the branch back where
// it began, which records nothing.

const PENDING: &str = "gitnoob-pending-step";

/// A step waiting for git to finish what it started.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Pending {
    pub kind: String,
    pub label: String,
    pub branch: Option<String>,
    pub before: Option<String>,
    /// What git was part-way through when it was written. A pending step
    /// belongs to that merge or rebase and no other.
    pub underway: String,
}

fn pending_file(root: &Path) -> Option<PathBuf> {
    crate::remote::git_dir(root)
        .ok()
        .map(|dir| dir.join(PENDING))
}

/// Writes down a step that stopped on conflicts, for whatever finishes it to
/// record. Does nothing if git is not part-way through anything.
pub fn begin_pending(
    state: &AppState,
    kind: &str,
    label: String,
    branch: Option<String>,
    before: Option<String>,
) {
    let Ok(root) = state.path() else { return };
    let (Some(underway), Some(file)) = (underway_at(&root), pending_file(&root)) else {
        return;
    };
    let pending = Pending {
        kind: kind.to_string(),
        label,
        branch,
        before,
        underway,
    };
    if let Ok(text) = serde_json::to_string(&pending) {
        let _ = std::fs::write(file, text);
    }
}

/// The step waiting on what git is part-way through right now, read before
/// something that may finish it. A note left by a merge or rebase that was
/// finished somewhere else is stale, and is cleared.
pub fn pending(state: &AppState) -> Option<Pending> {
    pending_at(&state.path().ok()?)
}

/// [`pending`] in a work tree read once by the caller.
pub fn pending_at(root: &Path) -> Option<Pending> {
    let root = root.to_path_buf();
    let file = pending_file(&root)?;
    let text = std::fs::read_to_string(&file).ok()?;
    let found: Option<Pending> = serde_json::from_str(&text).ok();
    match (found, underway_at(&root)) {
        (Some(found), Some(now)) if now == found.underway => Some(found),
        _ => {
            let _ = std::fs::remove_file(&file);
            None
        }
    }
}

/// Records the step that was waiting, once git has nothing left to finish.
///
/// Called after a commit, continue, skip or abort with what [`pending`] said
/// before it ran. While git is still part-way through, it keeps waiting.
pub fn finish_pending(state: &AppState, waiting: Option<Pending>) {
    if let Ok(root) = state.path() {
        finish_pending_at(state, &root, waiting);
    }
}

/// [`finish_pending`] in a work tree read once by the caller.
pub fn finish_pending_at(state: &AppState, root: &Path, waiting: Option<Pending>) {
    let Some(waiting) = waiting else { return };
    let root = root.to_path_buf();
    if running_at(&root).is_some() {
        return;
    }
    if let Some(file) = pending_file(&root) {
        let _ = std::fs::remove_file(file);
    }
    let after = match &waiting.branch {
        Some(branch) => tip_at(&root, branch),
        None => head_at(&root),
    };
    let mut entry = Entry::new(&waiting.kind, waiting.label, Mode::Hard, true);
    entry.branch = waiting.branch;
    entry.before = waiting.before;
    entry.after = after;
    record_entry_at(state, &root, entry);
}

// --- reading the history -----------------------------------------------------

/// Both stacks, newest first, each step marked with whether it can be taken
/// as the repository stands.
pub fn stacks(state: &AppState) -> Stacks {
    let Ok(root) = state.path() else {
        return Stacks {
            undo: Vec::new(),
            redo: Vec::new(),
        };
    };
    let mut stacks = state.journal_at(&root, |journal| journal.stacks());
    mark(&root, &mut stacks.undo, Direction::Back);
    mark(&root, &mut stacks.redo, Direction::Forward);
    stacks
}

/// Fills in `unavailable` for every step that no longer fits.
///
/// Read top down, as if each step above had been taken: a branch moves back
/// one step at a time, so the second step on a branch is checked against
/// where the first one would leave it. A step that does not fit blocks
/// everything below it on the same branch, because there is no getting there
/// without it.
fn mark(root: &Path, entries: &mut [Entry], direction: Direction) {
    let mut tips: HashMap<String, Option<String>> = HashMap::new();
    let here = branch_at(root);
    let mut stashes: Option<Vec<String>> = None;
    let mut listed = |oid: &str| {
        stashes
            .get_or_insert_with(|| stashes_at(root))
            .iter()
            .any(|one| one == oid)
    };
    for entry in entries.iter_mut() {
        let (standing, landing) = match direction {
            Direction::Back => (entry.after.clone(), entry.before.clone()),
            Direction::Forward => (entry.before.clone(), entry.after.clone()),
        };
        entry.unavailable = match entry.mode {
            Mode::Soft | Mode::Hard => {
                let name = entry.branch.clone().unwrap_or_else(|| "HEAD".to_string());
                let now = tips
                    .entry(name.clone())
                    .or_insert_with(|| match &entry.branch {
                        Some(branch) => tip_at(root, branch),
                        None => head_at(root),
                    })
                    .clone();
                if now.as_deref() == Some("blocked") {
                    Some(format!("An earlier step on {name} has to be taken first"))
                } else if now != standing {
                    tips.insert(name.clone(), Some("blocked".to_string()));
                    Some(format!(
                        "{name} has moved since, outside this history, so this step no longer fits"
                    ))
                } else if entry.mode == Mode::Soft && entry.branch.is_some() && entry.branch != here
                {
                    tips.insert(name.clone(), Some("blocked".to_string()));
                    Some(format!("Check out {name} to step this"))
                } else {
                    tips.insert(name, landing);
                    None
                }
            }
            Mode::Stash => {
                let made = entry.after.clone().unwrap_or_default();
                match direction {
                    Direction::Back if !listed(&made) => {
                        Some("That stash is no longer on the list".to_string())
                    }
                    _ => None,
                }
            }
            Mode::Apply => None,
            Mode::Pop | Mode::Drop => {
                let oid = entry.after.clone().unwrap_or_default();
                let on = listed(&oid);
                match direction {
                    Direction::Back if on => Some("That stash is on the list already".to_string()),
                    Direction::Forward if !on => {
                        Some("That stash is no longer on the list".to_string())
                    }
                    _ => None,
                }
            }
            Mode::Files => None,
            Mode::Branch => {
                let all = deleted_branches(entry);
                let back: Vec<&str> = all
                    .iter()
                    .filter(|one| tip_at(root, &one.name).is_some())
                    .map(|one| one.name.as_str())
                    .collect();
                match direction {
                    Direction::Back if !back.is_empty() => Some(format!(
                        "There is a branch called {} again",
                        back.join(", ")
                    )),
                    Direction::Forward if back.is_empty() => Some(match all.as_slice() {
                        [one] => format!("{} is gone already", one.name),
                        _ => "Those branches are gone already".to_string(),
                    }),
                    _ => None,
                }
            }
        };
    }
}

/// Takes a step that cannot be taken out of the history.
///
/// Only one marked unavailable: anything else might still be wanted, and a
/// step forgotten is a way back lost. A stale step on top otherwise sits in
/// front of every older one for good.
pub fn forget(state: &AppState, id: usize) -> Result<String, String> {
    let stacks = stacks(state);
    let found = stacks
        .undo
        .iter()
        .chain(stacks.redo.iter())
        .find(|entry| entry.id == id)
        .ok_or_else(|| "That step is not in the history any more".to_string())?;
    if found.unavailable.is_none() {
        return Err(format!(
            "{} can still be taken, so it stays in the history",
            found.label
        ));
    }
    let label = found.label.clone();
    let root = state.path()?;
    state.journal_at(&root, |journal| journal.remove(id));
    Ok(format!("Forgot: {label}"))
}

// --- undo and redo -----------------------------------------------------------

/// Refuses while git is part-way through something.
///
/// Moving a branch out from under a merge or a rebase leaves git's own state
/// pointing at commits that are no longer where it thinks they are.
fn refuse_while_running(root: &Path, verb: &str) -> Result<(), String> {
    match running_at(root) {
        Some(what) => Err(format!(
            "A {what} is part-way through. Finish or abort it before you {verb} anything else."
        )),
        None => Ok(()),
    }
}

pub fn undo(state: &AppState) -> Result<String, String> {
    let root = state.path()?;
    refuse_while_running(&root, "undo")?;
    // Work that would not go back on after the last step is that step's mess:
    // conflicted files, nothing running, the carried stash still on the list.
    // Undo takes the step and the mess back together, which is the only order
    // that works — the step cannot be moved off from under an unmerged index.
    // The same test `in_progress` offers "Put it back" on.
    if crate::work::has_unmerged(&root) && crate::work::auto_stash_on_top(&root) {
        return crate::work::undo_restore(state);
    }
    let Some(mut entry) = state.journal_at(&root, |journal| journal.take_undo()) else {
        return Err("Nothing to undo".to_string());
    };
    let outcome =
        check_repo(&root, &entry).and_then(|_| step(state, &root, &mut entry, Direction::Back));
    match outcome {
        Ok(message) => {
            state.journal_at(&root, |journal| journal.put_redo(entry));
            Ok(message)
        }
        Err(error) => {
            // Put it back so the stack still reflects reality.
            state.journal_at(&root, |journal| journal.put_undo(entry));
            Err(error)
        }
    }
}

pub fn redo(state: &AppState) -> Result<String, String> {
    let root = state.path()?;
    refuse_while_running(&root, "redo")?;
    let Some(mut entry) = state.journal_at(&root, |journal| journal.take_redo()) else {
        return Err("Nothing to redo".to_string());
    };
    let outcome =
        check_repo(&root, &entry).and_then(|_| step(state, &root, &mut entry, Direction::Forward));
    match outcome {
        Ok(message) => {
            state.journal_at(&root, |journal| journal.put_undo(entry));
            Ok(message)
        }
        Err(error) => {
            state.journal_at(&root, |journal| journal.put_redo(entry));
            Err(error)
        }
    }
}

/// Undoes one step by id, for a notice's own Undo.
///
/// The notice after a tidy used to put the branches back by itself and leave
/// the history's step on top, marked unavailable, in the way of every older
/// one. Now it asks the history: the same step, the same checks, and it lands
/// on the redo stack like any undo. Only the newest step can be taken this
/// way, except a deleted branch: making a branch again moves nothing else, so
/// that step does not have to wait for the ones above it.
pub fn undo_step(state: &AppState, id: usize) -> Result<String, String> {
    let root = state.path()?;
    let Some((at, mut entry)) = state.journal_at(&root, |journal| journal.take_undo_id(id)) else {
        return Err("That step is not in the history any more".to_string());
    };
    let newest = state.journal_at(&root, |journal| journal.undo.len()) == at;
    if !newest && entry.mode != Mode::Branch {
        let label = entry.label.clone();
        state.journal_at(&root, |journal| journal.restore_undo(at, entry));
        return Err(format!(
            "{label} is not the newest step any more. Undo from the history instead."
        ));
    }
    let outcome = check_repo(&root, &entry)
        .and_then(|_| match entry.mode {
            Mode::Branch => Ok(()),
            _ => refuse_while_running(&root, "undo"),
        })
        .and_then(|_| step(state, &root, &mut entry, Direction::Back));
    match outcome {
        Ok(message) => {
            state.journal_at(&root, |journal| journal.put_redo(entry));
            Ok(message)
        }
        Err(error) => {
            state.journal_at(&root, |journal| journal.restore_undo(at, entry));
            Err(error)
        }
    }
}

/// Refuses a step recorded in another work tree.
fn check_repo(root: &Path, entry: &Entry) -> Result<(), String> {
    if entry.repo.is_empty() || Path::new(&entry.repo) == key(root) {
        Ok(())
    } else {
        Err(format!(
            "That step was recorded in {}, not in {}",
            entry.repo,
            root.display()
        ))
    }
}

/// Takes the newest step off the history if it is the one that left the
/// branch where it stands, handing back where it came from and what it was.
///
/// For `undo_restore`, which puts a branch back by hand and has to keep the
/// history true while it does: the step goes onto the redo stack the way any
/// undone step does.
pub fn take_step_standing_here(state: &AppState) -> Option<(String, String)> {
    let root = state.path().ok()?;
    let head = head_at(&root)?;
    let here = branch_at(&root);
    let entry = state.journal_at(&root, |journal| journal.take_undo())?;
    let fits = entry.mode == Mode::Hard
        && entry.after.as_deref() == Some(head.as_str())
        && entry.branch == here;
    let Some(before) = entry.before.clone().filter(|_| fits) else {
        state.journal_at(&root, |journal| journal.put_undo(entry));
        return None;
    };
    let label = entry.label.clone();
    state.journal_at(&root, |journal| journal.put_redo(entry));
    Some((before, label))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Back,
    Forward,
}

fn step(
    state: &AppState,
    root: &Path,
    entry: &mut Entry,
    direction: Direction,
) -> Result<String, String> {
    match entry.mode {
        Mode::Stash => step_stash(root, entry, direction),
        Mode::Apply => step_apply(root, entry, direction),
        Mode::Pop => step_pop(root, entry, direction),
        Mode::Drop => step_drop(root, entry, direction),
        Mode::Files => {
            let count =
                crate::recover::put_files(root, &entry.files, direction == Direction::Back)?;
            let files = if count == 1 { "file" } else { "files" };
            Ok(match direction {
                Direction::Back => format!("Undid: {} — {count} {files} put back", entry.label),
                Direction::Forward => format!("Redid: {}", entry.label),
            })
        }
        Mode::Branch => step_branch(root, entry, direction),
        Mode::Soft | Mode::Hard => step_branch_move(state, root, entry, direction),
    }
}

fn step_stash(root: &Path, entry: &mut Entry, direction: Direction) -> Result<String, String> {
    match direction {
        // Undoing a stash means putting those changes back — that stash,
        // not whatever is on top of the list now. Anything stashed since,
        // here or in a terminal, sits above it.
        Direction::Back => {
            let made = entry
                .after
                .as_deref()
                .ok_or_else(|| "Nothing recorded to put back".to_string())?;
            let at = stash_index_at(root, made).ok_or_else(|| {
                "That stash is not in the list any more, so there is nothing to put back"
                    .to_string()
            })?;
            git_cmd::run_checked(root, &["stash", "pop", &format!("stash@{{{at}}}")])?;
            Ok(format!("Restored: {}", entry.label))
        }
        Direction::Forward => {
            let before = stash_ref(root);
            git_cmd::run_checked(root, &["stash", "push", "--include-untracked"])?;
            let after = stash_ref(root);
            // A clean tree makes the push above exit 0 without creating a
            // stash. `after` would then still name whatever `refs/stash`
            // pointed at before — someone else's stash, not this redo's —
            // and recording it would hand a later undo something to pop
            // that was never this operation's to touch.
            if after == before {
                return Err(
                    "There is nothing to stash again; the working tree is already clean."
                        .to_string(),
                );
            }
            // It is a different stash now, and undoing again has to find
            // this one rather than the one that is gone.
            entry.after = after;
            Ok(format!("Stashed again: {}", entry.label))
        }
    }
}

fn step_apply(root: &Path, entry: &mut Entry, direction: Direction) -> Result<String, String> {
    let oid = entry
        .after
        .clone()
        .ok_or_else(|| "Nothing recorded to put back".to_string())?;
    match direction {
        // The tree goes back to what the branch has. The stash is still on
        // the list — an apply leaves it, and so does a pop that stopped — so
        // nothing of the work is lost.
        Direction::Back => {
            // `before` holds what the apply left, for one that landed
            // cleanly: putting it back is only safe while the files still
            // look like that. An apply that stopped carries nothing there and
            // is always backed out of.
            crate::work::undo_applied_at(
                root,
                &oid,
                entry.before.as_deref(),
                &entry.staged,
                &entry.files,
            )?;
            Ok(format!("Undid: {}", entry.label))
        }
        Direction::Forward => {
            let at = stash_index_at(root, &oid).ok_or_else(|| {
                "That stash is not in the list any more, so there is nothing to put on".to_string()
            })?;
            // What the paths hold now is what undoing this redo puts back.
            let held = crate::work::before_stash(root, &oid);
            let said = git_cmd::run_checked(root, &["stash", "apply", &format!("stash@{{{at}}}")]);
            if said.is_ok() || crate::work::has_unmerged(root) {
                crate::work::keep_before(root, held, entry);
            }
            // A redo that stops on a conflict still put the stash on:
            // treated as a failure the step would go back on the redo
            // stack, leaving the mess it made with no way back.
            if said.is_err() && crate::work::has_unmerged(root) {
                entry.before = None;
                return Ok(format!(
                    "{} — it stopped on a conflict. Resolve it, or undo again.",
                    entry.label
                ));
            }
            said?;
            // What it left now, so the next undo checks against this apply
            // rather than the last one.
            entry.before = crate::work::applied_fingerprint(root, &oid);
            Ok(format!("Redid: {}", entry.label))
        }
    }
}

/// A clean pop, stepped back and forth.
///
/// Back: the files the stash brought in go back to what the branch has, and
/// the stash goes back on the list. The stash is stored first — if anything
/// after that fails, the work is in the stash and in the tree, never in
/// neither.
fn step_pop(root: &Path, entry: &mut Entry, direction: Direction) -> Result<String, String> {
    let oid = entry
        .after
        .clone()
        .ok_or_else(|| "Nothing recorded to put back".to_string())?;
    match direction {
        Direction::Back => {
            if stash_index_at(root, &oid).is_some() {
                return Err("That stash is on the list already".to_string());
            }
            // A commit since the pop could have taken the popped changes with
            // it; taking the files back to the branch would then hold the
            // same work twice, once committed and once stashed.
            if entry.head.is_some() && head_at(root) != entry.head {
                return Err(
                    "The branch has moved since the stash came off, so putting it back would \
                     not be the same thing any more. Stash the changes yourself if that is what \
                     you meant."
                        .to_string(),
                );
            }
            crate::work::applied_unchanged(root, &oid, entry.before.as_deref())?;
            if entry.before.is_some() {
                crate::recover::check_files(root, &entry.files, true)?;
            }
            let message = entry.message.clone().unwrap_or_else(|| entry.label.clone());
            git_cmd::run_checked(root, &["stash", "store", "-m", &message, &oid])?;
            crate::work::undo_applied_at(
                root,
                &oid,
                entry.before.as_deref(),
                &entry.staged,
                &entry.files,
            )?;
            Ok(format!(
                "Undid: {} — the stash is back on the list",
                entry.label
            ))
        }
        Direction::Forward => {
            let at = stash_index_at(root, &oid).ok_or_else(|| {
                "That stash is not in the list any more, so there is nothing to pop".to_string()
            })?;
            let held = crate::work::before_stash(root, &oid);
            let said = git_cmd::run_checked(root, &["stash", "pop", &format!("stash@{{{at}}}")]);
            if said.is_ok() || crate::work::has_unmerged(root) {
                crate::work::keep_before(root, held, entry);
            }
            if said.is_err() && crate::work::has_unmerged(root) {
                // A pop that stopped leaves the stash on the list: from here
                // it is an apply that stopped, and is backed out of like one.
                entry.mode = Mode::Apply;
                entry.before = None;
                return Ok(format!(
                    "{} — it stopped on a conflict. Resolve it, or undo again.",
                    entry.label
                ));
            }
            said?;
            entry.before = crate::work::applied_fingerprint(root, &oid);
            entry.head = head_at(root);
            Ok(format!("Redid: {}", entry.label))
        }
    }
}

fn step_drop(root: &Path, entry: &mut Entry, direction: Direction) -> Result<String, String> {
    let oid = entry
        .after
        .clone()
        .ok_or_else(|| "Nothing recorded to put back".to_string())?;
    match direction {
        Direction::Back => {
            if stash_index_at(root, &oid).is_some() {
                return Err("That stash is on the list already".to_string());
            }
            let message = entry.message.clone().unwrap_or_else(|| entry.label.clone());
            git_cmd::run_checked(root, &["stash", "store", "-m", &message, &oid])?;
            Ok(format!(
                "Undid: {} — it is back, at the top of the stash list",
                entry.label
            ))
        }
        Direction::Forward => {
            let at = stash_index_at(root, &oid).ok_or_else(|| {
                "That stash is not in the list any more, so there is nothing to drop".to_string()
            })?;
            git_cmd::run_checked(root, &["stash", "drop", &format!("stash@{{{at}}}")])?;
            Ok(format!("Redid: {}", entry.label))
        }
    }
}

/// Deleted branches, made again or deleted again.
///
/// `update-ref` with an old value on both sides: an empty one says "must not
/// exist yet", so a branch made under the same name in the meantime is never
/// overwritten, and deleting again only goes ahead while it still points
/// where it did.
///
/// A step that deleted several makes them all again in one transaction, or
/// none: half a tidy put back is harder to reason about than a refusal that
/// names the branch in the way. Deleting them again goes one by one, and a
/// branch that has moved since is left where it is and dropped from the
/// step — it is not this step's to take any more.
fn step_branch(root: &Path, entry: &mut Entry, direction: Direction) -> Result<String, String> {
    let all = deleted_branches(entry);
    if all.is_empty() {
        return Err("Nothing recorded to put back".to_string());
    }
    match direction {
        Direction::Back => {
            let taken: Vec<&str> = all
                .iter()
                .filter(|one| tip_at(root, &one.name).is_some())
                .map(|one| one.name.as_str())
                .collect();
            if !taken.is_empty() {
                return Err(format!(
                    "There is a branch called {} again, so the old one cannot be put back \
                     under that name. Rename or delete the new one first.",
                    taken.join(", ")
                ));
            }
            let input: String = all
                .iter()
                .map(|one| format!("create refs/heads/{}\0{}\0", one.name, one.oid))
                .collect();
            let said = git_cmd::run_with_input(
                root,
                &[
                    "update-ref",
                    "-m",
                    &format!("gitnoob: undo {}", entry.label),
                    "-z",
                    "--stdin",
                ],
                &input,
            )?;
            if !said.ok {
                return Err(said.stderr.trim().to_string());
            }
            for one in &all {
                if let Some(upstream) = &one.upstream {
                    let _ = git_cmd::run_checked(
                        root,
                        &[
                            "config",
                            &format!("branch.{}.remote", one.name),
                            &upstream.remote,
                        ],
                    );
                    let _ = git_cmd::run_checked(
                        root,
                        &[
                            "config",
                            &format!("branch.{}.merge", one.name),
                            &upstream.merge,
                        ],
                    );
                }
            }
            Ok(match all.as_slice() {
                [_] => format!("Undid: {} — it is back", entry.label),
                _ => format!("Undid: {} — they are back", entry.label),
            })
        }
        Direction::Forward => {
            let here = branch_at(root);
            let mut gone = Vec::new();
            let mut left = Vec::new();
            for one in all {
                let refusal = if here.as_deref() == Some(one.name.as_str()) {
                    Some(format!("{} is checked out here", one.name))
                } else {
                    checked_out_elsewhere(root, &one.name)
                        .map(|at| format!("{} is checked out in {at}", one.name))
                };
                if let Some(refusal) = refusal {
                    left.push(refusal);
                    continue;
                }
                match delete_branch_at(root, &one.name, &one.oid, &format!("redo {}", entry.label))
                {
                    Ok(()) => gone.push(one),
                    Err(error) => left.push(format!("{}: {error}", one.name)),
                }
            }
            if gone.is_empty() {
                return Err(left.join("\n"));
            }
            if entry.branches.is_empty() {
                // A single deletion: it went, or it would have said so above.
            } else {
                entry.branches = gone;
            }
            let mut said = format!("Redid: {}", entry.label);
            if !left.is_empty() {
                said = format!("{said}\nLeft alone: {}", left.join("; "));
            }
            Ok(said)
        }
    }
}

/// Deletes a branch only while it points at `oid`, with its section of the
/// config, the way `git branch -D` does — but by commit rather than by name,
/// so a branch that moved after it was looked at is refused instead of being
/// deleted at its new tip.
pub fn delete_branch_at(root: &Path, name: &str, oid: &str, why: &str) -> Result<(), String> {
    git_cmd::run_checked(
        root,
        &[
            "update-ref",
            "-m",
            &format!("gitnoob: {why}"),
            "-d",
            &format!("refs/heads/{name}"),
            oid,
        ],
    )
    .map_err(|error| error.trim().to_string())?;
    let _ = git_cmd::run_checked(
        root,
        &["config", "--remove-section", &format!("branch.{name}")],
    );
    Ok(())
}

fn step_branch_move(
    state: &AppState,
    root: &Path,
    entry: &mut Entry,
    direction: Direction,
) -> Result<String, String> {
    let target = match direction {
        Direction::Back => entry.before.clone(),
        Direction::Forward => entry.after.clone(),
    };
    let oid = target.ok_or_else(|| "Nothing recorded to move back to".to_string())?;
    // Where the branch has to be standing for this step to be the last thing
    // that moved it.
    let standing = match direction {
        Direction::Back => entry.after.clone(),
        Direction::Forward => entry.before.clone(),
    };
    let moved_on = |name: &str| {
        format!(
            "{name} is not where that step left it, so stepping it back would take something \
             else with it. Whatever moved it happened outside this history."
        )
    };

    // A step that moved a branch which is not checked out here — a merge made
    // into another branch, a fast-forward of one — moves that branch's ref and
    // nothing else, as long as no folder has it checked out. A soft step keeps
    // its changes staged, and there is no index for a branch that is not
    // checked out, so those wait until it is.
    let here = branch_at(root);
    if let Some(name) = entry
        .branch
        .clone()
        .filter(|name| entry.mode == Mode::Hard && here.as_deref() != Some(name.as_str()))
    {
        if let Some(at) = checked_out_elsewhere(root, &name) {
            return Err(format!(
                "{name} is checked out in {at}, so moving it from here would change the files \
                 under that folder. Switch that folder to another branch first."
            ));
        }
        let tip = tip_at(root, &name).ok_or_else(|| format!("{name} does not exist any more"))?;
        if Some(&tip) != standing.as_ref() {
            return Err(moved_on(&name));
        }
        let verb = match direction {
            Direction::Back => "undo",
            Direction::Forward => "redo",
        };
        // The old value makes git itself refuse if the branch moved between
        // the check above and now.
        git_cmd::run_checked(
            root,
            &[
                "update-ref",
                "-m",
                &format!("gitnoob: {verb} {}", entry.label),
                &format!("refs/heads/{name}"),
                &oid,
                &tip,
            ],
        )?;
        let short: String = oid.chars().take(7).collect();
        return Ok(match direction {
            Direction::Back => {
                let mut said = format!("Undid: {} — {name} is back at {short}", entry.label);
                if entry.kind != "pull" {
                    if let Some(remote) = still_published_at(root, entry) {
                        said.push_str(&format!(
                            ", but {remote} still has it. Push to undo it there too."
                        ));
                    }
                }
                said
            }
            Direction::Forward => format!("Redid: {} — {name} is at {short} again", entry.label),
        });
    }

    // Refuse if the branch has moved since, which means something the
    // journal never saw — a commit in a terminal, a rebase in another
    // window — and moving it back from here would take that with it.
    if let (Some(expected), Some(actual)) = (standing.as_deref(), head_at(root)) {
        if expected != actual {
            return Err(moved_on(
                &entry
                    .branch
                    .clone()
                    .unwrap_or_else(|| "The branch".to_string()),
            ));
        }
    }
    // Refuse if the user has since switched branches; resetting the wrong
    // branch to this commit would be worse than doing nothing.
    if let Some(expected) = &entry.branch {
        match &here {
            Some(actual) if actual == expected => {}
            Some(actual) => {
                return Err(format!(
                    "That step belongs to {expected}, but {actual} is checked out"
                ))
            }
            None => return Err("HEAD is detached; check out a branch first".to_string()),
        }
    }
    let note = if entry.mode == Mode::Hard {
        let verb = match direction {
            Direction::Back => "undoing",
            Direction::Forward => "redoing",
        };
        move_keeping(state, root, &oid, &format!("{verb} {}", entry.label))?
    } else {
        git_cmd::run_checked(root, &["reset", "--soft", &oid])?;
        None
    };
    let mut said = match direction {
        Direction::Back => {
            let mut said = format!("Undid: {}", entry.label);
            // Undoing moves the branch here; it cannot move the remote,
            // which still has the commit. Left unsaid, the window simply
            // reports the branch as behind — and pulling, the obvious
            // thing to do about that, brings the undone commit back.
            // A pull is the one step that was never ours to publish:
            // undoing it only puts the branch behind again.
            if entry.kind != "pull" {
                if let Some(remote) = still_published_at(root, entry) {
                    said.push_str(&format!(
                        " — but {remote} still has it. Push to undo it there too; pull, and the commit comes back."
                    ));
                }
            }
            said
        }
        Direction::Forward => format!("Redid: {}", entry.label),
    };
    if let Some(note) = note {
        said = format!("{said}\n{note}");
    }
    Ok(said)
}

/// Moves the branch and the working tree to `oid` without losing uncommitted
/// work.
///
/// `reset --hard` would take every open change with it, which turns the undo
/// of a merge into the loss of an afternoon's edits in a file the merge never
/// touched. `reset --keep` is the same move made carefully: files the step
/// changed go back, files with uncommitted edits are left alone, and a file
/// that is both makes git refuse rather than choose. That refusal is met the
/// way every other step here meets open work — set it down, move on a clean
/// tree, put it back — and only with auto-stash off is it passed on in words.
///
/// The note, when there is one, says what happened to the work.
fn move_keeping(
    state: &AppState,
    root: &Path,
    oid: &str,
    reason: &str,
) -> Result<Option<String>, String> {
    match git_cmd::run_checked(root, &["reset", "--keep", oid]) {
        Ok(_) => Ok(None),
        Err(error) if refused_over_local_changes(&error) => {
            if !state.config().global.auto_stash {
                return Err(format!(
                    "Your uncommitted changes are in the way of this step. Commit or stash them \
                     first, or turn auto-stash on in settings.\n{error}"
                ));
            }
            // Set down by hand rather than through the state, so the stash
            // is made and put back in this folder whatever else happens.
            let message = match branch_at(root) {
                Some(branch) => format!("{} on {branch}: {reason}", crate::work::AUTO_STASH),
                None => format!("{}: {reason}", crate::work::AUTO_STASH),
            };
            let before = stash_ref(root);
            git_cmd::run_checked(
                root,
                &["stash", "push", "--include-untracked", "-m", &message],
            )?;
            let held = stash_ref(root).filter(|held| Some(held) != before.as_ref());
            let moved = git_cmd::run_checked(root, &["reset", "--keep", oid]);
            let restored = match held.as_deref().and_then(|held| stash_index_at(root, held)) {
                Some(at) => crate::work::pop_stash(root, &format!("stash@{{{at}}}"))
                    .map(|_| "Local changes were stashed and put back".to_string())
                    .map_err(|error| {
                        format!(
                            "Your changes are safe in the stash, but putting them back hit a \
                             problem: {error}"
                        )
                    }),
                None => Ok(String::new()),
            };
            moved?;
            // The step itself went through; how the work came back is a note
            // on it, not a reason to pretend it did not happen.
            Ok(match restored {
                Ok(note) if note.is_empty() => None,
                Ok(note) => Some(note),
                Err(note) => Some(note),
            })
        }
        Err(error) => Err(error),
    }
}

/// Whether `reset --keep` turned the move down over uncommitted work, as
/// opposed to failing for some other reason.
fn refused_over_local_changes(error: &str) -> bool {
    error.contains("not uptodate") || error.contains("would be overwritten")
}

/// The upstream that still carries the commit an undo has just moved off, if
/// there is one.
///
/// An undo is local by definition — it moves a branch, and a branch is the only
/// thing it can move. Whether that matters depends on whether the commit ever
/// left this machine.
pub fn still_published(state: &AppState, entry: &Entry) -> Option<String> {
    still_published_at(&state.path().ok()?, entry)
}

fn still_published_at(root: &Path, entry: &Entry) -> Option<String> {
    let undone = entry.after.as_deref()?;
    let branch = entry.branch.clone().or_else(|| branch_at(root))?;

    let upstream = git_cmd::run(
        root,
        &[
            "rev-parse",
            "--abbrev-ref",
            &format!("{branch}@{{upstream}}"),
        ],
    )
    .ok()
    .filter(|out| out.ok)
    .map(|out| out.stdout.trim().to_string())
    .filter(|name| !name.is_empty())?;

    let contains = git_cmd::run(root, &["merge-base", "--is-ancestor", undone, &upstream])
        .map(|out| out.ok)
        .unwrap_or(false);
    contains.then_some(upstream)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
