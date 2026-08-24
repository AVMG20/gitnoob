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
    /// `before` and `after` hold branch names rather than object ids.
    Checkout,
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
    let Some(entry) = state.journal(|journal| journal.take_undo()) else {
        return Err("Nothing to undo".to_string());
    };
    match step(state, &entry, Direction::Back) {
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
    let Some(entry) = state.journal(|journal| journal.take_redo()) else {
        return Err("Nothing to redo".to_string());
    };
    match step(state, &entry, Direction::Forward) {
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

fn step(state: &AppState, entry: &Entry, direction: Direction) -> Result<String, String> {
    let root = state.path()?;
    let target = match direction {
        Direction::Back => entry.before.as_deref(),
        Direction::Forward => entry.after.as_deref(),
    };

    match entry.mode {
        Mode::Checkout => {
            let branch = target.ok_or_else(|| "Nothing recorded to switch back to".to_string())?;
            git_cmd::run_checked(&root, &["checkout", branch, "--"])?;
            Ok(format!("Switched to {branch}"))
        }
        Mode::Stash => match direction {
            // Undoing a stash means putting the changes back.
            Direction::Back => {
                git_cmd::run_checked(&root, &["stash", "pop"])?;
                Ok(format!("Restored: {}", entry.label))
            }
            Direction::Forward => {
                git_cmd::run_checked(&root, &["stash", "push", "--include-untracked"])?;
                Ok(format!("Stashed again: {}", entry.label))
            }
        },
        Mode::Soft | Mode::Hard => {
            let oid = target.ok_or_else(|| "Nothing recorded to move back to".to_string())?;
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
                Direction::Back => format!("Undid: {}", entry.label),
                Direction::Forward => format!("Redid: {}", entry.label),
            })
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
