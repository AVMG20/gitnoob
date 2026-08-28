//! Interactive rebase: the plan, and running it.
//!
//! `git rebase -i` works by opening an editor on a todo list. There is no
//! editor here, so the app puts itself in that slot: `GIT_SEQUENCE_EDITOR`
//! names this same binary with `--write-todo`, which copies the list the
//! window built over the one git offered and exits. That is the whole trick —
//! everything after it is git's own rebase, with git's own conflict handling,
//! `git rebase --continue` and `git rebase --abort`.
//!
//! `reword` is the one action not handed straight to git. Git would open the
//! message editor itself, which is the thing there is none of, so a reword is
//! written into the todo as `edit` and the oid is remembered on the side. When
//! the rebase stops at one, the window recognises it and asks for the message
//! rather than showing the "make your changes" strip an ordinary `edit` gets.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git_cmd;
use crate::state::AppState;

/// What to do with one commit. The names are git's own.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Pick,
    Reword,
    Squash,
    Fixup,
    Edit,
    Drop,
}

impl Action {
    /// The word git's todo list uses.
    ///
    /// A reword goes in as `edit`: see the note at the top of the file.
    fn word(self) -> &'static str {
        match self {
            Action::Pick => "pick",
            Action::Reword | Action::Edit => "edit",
            Action::Squash => "squash",
            Action::Fixup => "fixup",
            Action::Drop => "drop",
        }
    }
}

/// One line of the plan, as the window sends it back.
#[derive(Deserialize, Debug, Clone)]
pub struct Step {
    pub oid: String,
    pub action: Action,
}

/// One commit offered in the plan, with what the window needs to draw it.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub oid: String,
    pub short: String,
    pub summary: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    /// Already on a remote, so rewriting it means a force push afterwards.
    pub pushed: bool,
}

/// The commits between `onto` and HEAD, oldest first — the order git replays
/// them in, and so the order the plan is read in.
pub fn plan(state: &AppState, onto: &str) -> Result<Vec<Candidate>, String> {
    let root = state.path()?;
    let range = format!("{onto}..HEAD");
    let raw = git_cmd::run_checked(
        &root,
        &[
            "log",
            "--reverse",
            "--no-merges",
            "--no-show-signature",
            "--format=%H%x1f%an%x1f%ae%x1f%at%x1f%s",
            &range,
        ],
    )?;
    let mut found = read_plan(&raw);
    if found.is_empty() {
        return Err(format!("Nothing to rebase: HEAD is already at {onto}"));
    }

    // Which of them a remote already has. One command for the lot rather than
    // one per commit, and a repository with no remotes simply answers none.
    let published = published_commits(&root);
    for one in &mut found {
        one.pushed = published.contains(&one.oid);
    }
    Ok(found)
}

fn read_plan(raw: &str) -> Vec<Candidate> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let oid = parts.next()?.trim().to_string();
            if oid.is_empty() {
                return None;
            }
            let author = parts.next().unwrap_or("").to_string();
            let email = parts.next().unwrap_or("").to_string();
            let time = parts.next().unwrap_or("0").trim().parse().unwrap_or(0);
            let summary = parts.collect::<Vec<_>>().join("\u{1f}");
            Some(Candidate {
                short: oid.chars().take(7).collect(),
                oid,
                summary,
                author,
                email,
                time,
                pushed: false,
            })
        })
        .collect()
}

/// Every commit reachable from a remote-tracking branch, so the plan can say
/// which of its entries a rewrite would strand.
fn published_commits(root: &Path) -> Vec<String> {
    let Ok(out) = git_cmd::run(
        root,
        &["log", "--format=%H", "--remotes", "--max-count=2000"],
    ) else {
        return Vec::new();
    };
    if !out.ok {
        return Vec::new();
    }
    out.stdout.lines().map(|l| l.trim().to_string()).collect()
}

/// The todo list git will be handed.
///
/// A `squash` or `fixup` with nothing above it to fold into is not a plan git
/// will accept, so it is refused here where it can be explained rather than
/// there where it cannot.
pub fn todo_text(steps: &[Step]) -> Result<String, String> {
    let mut lines = Vec::new();
    let mut has_root = false;
    for step in steps {
        if matches!(step.action, Action::Squash | Action::Fixup) && !has_root {
            return Err(
                "The first commit of a rebase has nothing above it to fold into. Move it down, or pick it."
                    .to_string(),
            );
        }
        if step.action == Action::Drop {
            // A dropped commit is left out rather than written as `drop`: both
            // work, and a shorter list is a list with less to go wrong in it.
            continue;
        }
        if !matches!(step.action, Action::Squash | Action::Fixup) {
            has_root = true;
        }
        lines.push(format!("{} {}", step.action.word(), step.oid));
    }
    if lines.is_empty() {
        return Err("That plan drops every commit. Use reset instead.".to_string());
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}

/// The command git will run in place of an editor.
///
/// This app's own binary, which answers `--write-todo` by copying and exiting.
/// Git hands the value to a shell and appends the todo path, so what runs is
/// `gitnoob --write-todo <ours> <git's>`.
fn sequence_editor(list: &Path) -> Result<String, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Could not find gitnoob's own path: {e}"))?;
    Ok(format!(
        "{} --write-todo {}",
        shell_quote(&exe),
        shell_quote(list)
    ))
}

/// Quotes a path for the shell git runs the sequence editor through.
///
/// Git hands `GIT_SEQUENCE_EDITOR` to a shell, so a path with a space in it —
/// `C:\Program Files\...`, `/Applications/...` — has to arrive as one word.
fn shell_quote(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Starts the rebase, with the window's plan in place of git's editor.
pub fn start(state: &AppState, onto: &str, steps: Vec<Step>) -> Result<String, String> {
    let root = state.path()?;
    let todo = todo_text(&steps)?;

    let git_dir = crate::remote::git_dir(&root)?;
    let list = git_dir.join("gitnoob-rebase-todo");
    std::fs::write(&list, &todo).map_err(|e| format!("Could not write the plan: {e}"))?;

    // The oids the window called rewords, which go in as `edit`. Remembered on
    // disk rather than in memory so that closing the window part-way through a
    // rebase does not lose which stop is which.
    let rewords: Vec<&str> = steps
        .iter()
        .filter(|s| s.action == Action::Reword)
        .map(|s| s.oid.as_str())
        .collect();
    let _ = std::fs::write(git_dir.join("gitnoob-rebase-rewords"), rewords.join("\n"));

    let editor = sequence_editor(&list)?;

    let out = git_cmd::run_with_env(
        &root,
        &["rebase", "-i", "--autostash", onto],
        &[
            ("GIT_SEQUENCE_EDITOR", editor.as_str()),
            // Nothing else may open an editor: a squash would otherwise sit
            // waiting on a message box that does not exist. `true` accepts
            // whatever message git prepared, which for a squash is both
            // messages joined — the same thing saving an unedited editor does.
            ("GIT_EDITOR", "true"),
        ],
    )?;

    if out.ok {
        clear_marks(&git_dir);
        return Ok(format!("Rebased onto {onto}"));
    }
    // A rebase that stops is not a rebase that failed: it stopped for an edit,
    // a reword or a conflict, and the window has a strip for each of those.
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        return Ok("Rebase stopped".to_string());
    }
    clear_marks(&git_dir);
    Err(one_message(&out))
}

/// Where a rebase has got to, or `None` when none is running.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Which step it is on, counting from one.
    pub at: usize,
    pub total: usize,
    /// The original commit it stopped at, when it stopped at one.
    pub stopped: Option<String>,
    pub summary: Option<String>,
    /// True when this stop is one the window asked for as a reword.
    pub rewording: bool,
    /// The message that commit carries now, to start the box off with.
    pub message: Option<String>,
}

pub fn progress(state: &AppState) -> Result<Option<Progress>, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let dir = git_dir.join("rebase-merge");
    if !dir.exists() {
        // `rebase-apply` is the old non-interactive machinery. It is a rebase,
        // but not one of ours, and it has no todo list to report against.
        return Ok(None);
    }

    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
    let number = |name: &str| {
        read(name)
            .and_then(|text| text.trim().parse::<usize>().ok())
            .unwrap_or(0)
    };

    let stopped = read("stopped-sha").map(|text| text.trim().to_string());
    let rewording = stopped
        .as_deref()
        .is_some_and(|oid| is_reword(&git_dir, oid));

    let (summary, message) = match stopped.as_deref() {
        Some(oid) => (
            subject_of(&root, oid),
            // The message to start the box off with is the one on the commit
            // that has just been applied, not the original: an `edit` that
            // amended something has already moved it on.
            rewording.then(|| message_of(&root, "HEAD")).flatten(),
        ),
        None => (None, None),
    };

    Ok(Some(Progress {
        at: number("msgnum"),
        total: number("end"),
        stopped,
        summary,
        rewording,
        message,
    }))
}

fn is_reword(git_dir: &Path, oid: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(git_dir.join("gitnoob-rebase-rewords")) else {
        return false;
    };
    text.lines().any(|line| line.trim() == oid)
}

fn subject_of(root: &Path, oid: &str) -> Option<String> {
    let out = git_cmd::run(root, &["log", "-1", "--format=%s", oid]).ok()?;
    out.ok
        .then(|| out.stdout.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn message_of(root: &Path, oid: &str) -> Option<String> {
    let out = git_cmd::run(root, &["log", "-1", "--format=%B", oid]).ok()?;
    out.ok
        .then(|| out.stdout.trim_end().to_string())
        .filter(|s| !s.is_empty())
}

/// Carries on from wherever the rebase stopped.
pub fn resume(state: &AppState) -> Result<String, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let out = git_cmd::run_with_env(
        &root,
        &["rebase", "--continue"],
        &[("GIT_EDITOR", "true")],
    )?;
    settle(&git_dir, out, "Rebase finished")
}

/// Leaves the commit it stopped at out and moves on.
pub fn skip(state: &AppState) -> Result<String, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let out = git_cmd::run_with_env(&root, &["rebase", "--skip"], &[("GIT_EDITOR", "true")])?;
    settle(&git_dir, out, "Rebase finished")
}

/// Puts the branch back exactly as it was before the rebase started.
pub fn abort(state: &AppState) -> Result<String, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let out = git_cmd::run(&root, &["rebase", "--abort"])?;
    clear_marks(&git_dir);
    if out.ok {
        Ok("Rebase abandoned; the branch is as it was".to_string())
    } else {
        Err(one_message(&out))
    }
}

/// Gives the commit the rebase stopped at a new message, then carries on.
pub fn reword(state: &AppState, message: &str) -> Result<String, String> {
    let root = state.path()?;
    let text = message.trim();
    if text.is_empty() {
        return Err("A commit needs a message".to_string());
    }
    let out = git_cmd::run_with_input(&root, &["commit", "--amend", "--file=-"], text)?;
    if !out.ok {
        return Err(one_message(&out));
    }
    resume(state)
}

/// What a `--continue` or `--skip` left behind.
fn settle(git_dir: &Path, out: git_cmd::CmdOutput, done: &str) -> Result<String, String> {
    let still_going =
        git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists();
    if still_going {
        return Ok("Rebase stopped".to_string());
    }
    clear_marks(git_dir);
    if out.ok {
        Ok(done.to_string())
    } else {
        Err(one_message(&out))
    }
}

/// The files this app leaves beside git's own, once there is no rebase left.
fn clear_marks(git_dir: &Path) {
    let _ = std::fs::remove_file(git_dir.join("gitnoob-rebase-todo"));
    let _ = std::fs::remove_file(git_dir.join("gitnoob-rebase-rewords"));
}

fn one_message(out: &git_cmd::CmdOutput) -> String {
    let text = if out.stderr.trim().is_empty() {
        out.stdout.trim()
    } else {
        out.stderr.trim()
    };
    if text.is_empty() {
        format!("git exited {}", out.code)
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(oid: &str, action: Action) -> Step {
        Step {
            oid: oid.to_string(),
            action,
        }
    }

    #[test]
    fn writes_the_list_git_expects() {
        let todo = todo_text(&[
            step("aaa", Action::Pick),
            step("bbb", Action::Fixup),
            step("ccc", Action::Edit),
        ])
        .unwrap();
        assert_eq!(todo, "pick aaa\nfixup bbb\nedit ccc\n");
    }

    #[test]
    fn a_reword_goes_in_as_an_edit_so_the_window_can_ask() {
        let todo = todo_text(&[step("aaa", Action::Pick), step("bbb", Action::Reword)]).unwrap();
        assert_eq!(todo, "pick aaa\nedit bbb\n");
    }

    #[test]
    fn a_dropped_commit_is_simply_left_out() {
        let todo = todo_text(&[
            step("aaa", Action::Pick),
            step("bbb", Action::Drop),
            step("ccc", Action::Pick),
        ])
        .unwrap();
        assert_eq!(todo, "pick aaa\npick ccc\n");
    }

    #[test]
    fn folding_the_first_commit_into_nothing_is_refused_here() {
        let refused = todo_text(&[step("aaa", Action::Squash), step("bbb", Action::Pick)]);
        assert!(refused.unwrap_err().contains("nothing above it"));

        // Including when everything above it was dropped.
        let refused = todo_text(&[step("aaa", Action::Drop), step("bbb", Action::Fixup)]);
        assert!(refused.is_err());
    }

    #[test]
    fn a_plan_that_drops_everything_is_refused() {
        let refused = todo_text(&[step("aaa", Action::Drop), step("bbb", Action::Drop)]);
        assert!(refused.unwrap_err().contains("drops every commit"));
    }

    #[test]
    fn reads_the_commits_oldest_first_as_git_listed_them() {
        const RAW: &str = concat!(
            "aaa\u{1f}Ramon\u{1f}r@x\u{1f}1756000000\u{1f}feat: one\n",
            "bbb\u{1f}Ramon\u{1f}r@x\u{1f}1756000100\u{1f}fix: two\n"
        );
        let found = read_plan(RAW);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].oid, "aaa");
        assert_eq!(found[0].summary, "feat: one");
        assert_eq!(found[1].short, "bbb");
        assert!(!found[0].pushed);
    }

    #[test]
    fn the_editor_git_runs_is_this_binary_answering_write_todo() {
        let line = sequence_editor(Path::new("/tmp/plan")).unwrap();
        let exe = std::env::current_exe().unwrap();
        assert!(line.starts_with(&shell_quote(&exe)), "{line}");
        assert!(line.ends_with("--write-todo '/tmp/plan'"), "{line}");
    }

    #[test]
    fn a_path_with_a_space_reaches_the_shell_as_one_word() {
        let quoted = shell_quote(Path::new("/Applications/git noob.app/gitnoob"));
        assert_eq!(quoted, "'/Applications/git noob.app/gitnoob'");
        assert_eq!(shell_quote(Path::new("/it's/here")), r"'/it'\''s/here'");
    }
}
