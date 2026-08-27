use std::path::Path;

use serde::Serialize;

use crate::git_cmd;
use crate::state::AppState;

/// How an entry is stepped back and forth.
#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Move the branch pointer, keep the changes staged. For commits and amends.
    Soft,
    /// Move the branch pointer and the working tree. For merges.
    Hard,
    /// A stash push: undoing pops it back, redoing stashes again.
    Stash,
}

/// One reversible operation.
#[derive(Serialize, Clone)]
pub struct Entry {
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
}

#[derive(Serialize, Clone)]
pub struct Stacks {
    pub undo: Vec<Entry>,
    pub redo: Vec<Entry>,
}

/// Enough history to cover a session's mistakes without growing unbounded.
const LIMIT: usize = 100;

#[derive(Default)]
pub struct Journal {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    next_id: usize,
}

impl Journal {
    pub fn push(&mut self, mut entry: Entry) {
        self.next_id += 1;
        entry.id = self.next_id;
        // A fresh operation invalidates anything that was undone.
        self.redo.clear();
        self.undo.push(entry);
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn stacks(&self) -> Stacks {
        Stacks {
            // Newest first: that is the order the history menu reads in.
            undo: self.undo.iter().rev().cloned().collect(),
            redo: self.redo.iter().rev().cloned().collect(),
        }
    }

    pub fn take_undo(&mut self) -> Option<Entry> {
        self.undo.pop()
    }

    pub fn take_redo(&mut self) -> Option<Entry> {
        self.redo.pop()
    }

    pub fn put_redo(&mut self, entry: Entry) {
        self.redo.push(entry);
    }

    pub fn put_undo(&mut self, entry: Entry) {
        self.undo.push(entry);
    }
}

/// The current HEAD commit, or None in a repository with no commits.
pub fn head_oid(state: &AppState) -> Option<String> {
    state
        .repo()
        .ok()
        .and_then(|repo| repo.head().ok().and_then(|h| h.target()))
        .map(|oid| oid.to_string())
}

/// The checked out branch, or None when HEAD is detached.
pub fn current_branch(state: &AppState) -> Option<String> {
    let repo = state.repo().ok()?;
    if repo.head_detached().unwrap_or(false) {
        return None;
    }
    // Copy the name out before `repo` is dropped at the end of the function.
    let head = repo.head().ok()?;
    let name = head.shorthand()?.to_string();
    Some(name)
}

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
    // An operation that changed nothing is not worth an undo step.
    if mode != Mode::Stash && before == after {
        return;
    }
    state.journal(|journal| {
        journal.push(Entry {
            id: 0,
            label,
            kind: kind.to_string(),
            branch,
            before,
            after,
            mode,
            destructive,
            at: now(),
        })
    });
}

pub fn stacks(state: &AppState) -> Stacks {
    state.journal(|journal| journal.stacks())
}

pub fn undo(state: &AppState) -> Result<String, String> {
    let Some(mut entry) = state.journal(|journal| journal.take_undo()) else {
        return Err("Nothing to undo".to_string());
    };
    match step(state, &mut entry, Direction::Back) {
        Ok(message) => {
            state.journal(|journal| journal.put_redo(entry));
            Ok(message)
        }
        Err(error) => {
            // Put it back so the stack still reflects reality.
            state.journal(|journal| journal.put_undo(entry));
            Err(error)
        }
    }
}

pub fn redo(state: &AppState) -> Result<String, String> {
    let Some(mut entry) = state.journal(|journal| journal.take_redo()) else {
        return Err("Nothing to redo".to_string());
    };
    match step(state, &mut entry, Direction::Forward) {
        Ok(message) => {
            state.journal(|journal| journal.put_undo(entry));
            Ok(message)
        }
        Err(error) => {
            state.journal(|journal| journal.put_redo(entry));
            Err(error)
        }
    }
}

enum Direction {
    Back,
    Forward,
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

/// The index a stash commit sits at now, or `None` if it is no longer listed.
///
/// `stash@{0}` is whatever was stashed last, which is not the same thing as the
/// stash you mean: anything stashed since — here, or in a terminal — sits above
/// it. Naming the commit is the only way to be sure.
pub fn stash_index(state: &AppState, oid: &str) -> Option<usize> {
    let root = state.path().ok()?;
    let listed = git_cmd::run_checked(&root, &["stash", "list", "--format=%H"]).ok()?;
    listed.lines().position(|line| line.trim() == oid)
}

fn step(state: &AppState, entry: &mut Entry, direction: Direction) -> Result<String, String> {
    let root = state.path()?;
    let target = match direction {
        Direction::Back => entry.before.as_deref(),
        Direction::Forward => entry.after.as_deref(),
    };

    match entry.mode {
        Mode::Stash => match direction {
            // Undoing a stash means putting those changes back — that stash,
            // not whatever is on top of the list now. Anything stashed since,
            // here or in a terminal, sits above it.
            Direction::Back => {
                let made = entry
                    .after
                    .as_deref()
                    .ok_or_else(|| "Nothing recorded to put back".to_string())?;
                let at = stash_index(state, made).ok_or_else(|| {
                    "That stash is not in the list any more, so there is nothing to put back"
                        .to_string()
                })?;
                git_cmd::run_checked(&root, &["stash", "pop", &format!("stash@{{{at}}}")])?;
                Ok(format!("Restored: {}", entry.label))
            }
            Direction::Forward => {
                let before = stash_ref(&root);
                git_cmd::run_checked(&root, &["stash", "push", "--include-untracked"])?;
                let after = stash_ref(&root);
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
        },
        Mode::Soft | Mode::Hard => {
            let oid = target.ok_or_else(|| "Nothing recorded to move back to".to_string())?;
            // Refuse if the branch has moved since, which means something the
            // journal never saw — a commit in a terminal, a rebase in another
            // window — and moving it back from here would take that with it.
            let standing = match direction {
                Direction::Back => entry.after.as_deref(),
                Direction::Forward => entry.before.as_deref(),
            };
            if let (Some(expected), Some(actual)) = (standing, head_oid(state)) {
                if expected != actual {
                    return Err(format!(
                        "{} is not where that step left it, so stepping it back would take \
                         something else with it. Whatever moved it happened outside this history.",
                        entry
                            .branch
                            .clone()
                            .unwrap_or_else(|| "The branch".to_string())
                    ));
                }
            }
            // Refuse if the user has since switched branches; resetting the wrong
            // branch to this commit would be worse than doing nothing.
            if let Some(expected) = &entry.branch {
                match current_branch(state) {
                    Some(actual) if &actual == expected => {}
                    Some(actual) => {
                        return Err(format!(
                            "That step belongs to {expected}, but {actual} is checked out"
                        ))
                    }
                    None => return Err("HEAD is detached; check out a branch first".to_string()),
                }
            }
            let flag = if entry.mode == Mode::Hard {
                "--hard"
            } else {
                "--soft"
            };
            git_cmd::run_checked(&root, &["reset", flag, oid])?;
            Ok(match direction {
                Direction::Back => {
                    let mut said = format!("Undid: {}", entry.label);
                    // Undoing moves the branch here; it cannot move the remote,
                    // which still has the commit. Left unsaid, the window simply
                    // reports the branch as behind — and pulling, the obvious
                    // thing to do about that, brings the undone commit back.
                    if let Some(remote) = still_published(state, entry) {
                        said.push_str(&format!(
                            " — but {remote} still has it. Push to undo it there too; pull, and the commit comes back."
                        ));
                    }
                    said
                }
                Direction::Forward => format!("Redid: {}", entry.label),
            })
        }
    }
}

/// The upstream that still carries the commit an undo has just moved off, if
/// there is one.
///
/// An undo is local by definition — it moves a branch, and a branch is the only
/// thing it can move. Whether that matters depends on whether the commit ever
/// left this machine.
pub fn still_published(state: &AppState, entry: &Entry) -> Option<String> {
    let root = state.path().ok()?;
    let undone = entry.after.as_deref()?;
    let branch = entry.branch.clone().or_else(|| current_branch(state))?;

    let upstream = git_cmd::run(
        &root,
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

    let contains = git_cmd::run(&root, &["merge-base", "--is-ancestor", undone, &upstream])
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
