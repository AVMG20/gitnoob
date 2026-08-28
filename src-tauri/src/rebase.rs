//! Interactive rebase: the plan, and running it.
//!
//! `git rebase -i` works by opening an editor on a todo list. There is no
//! editor here, so the app puts itself in that slot: `GIT_SEQUENCE_EDITOR`
//! names this same binary with `--write-todo`, which copies the list the
//! window built over the one git offered and exits. That is the whole trick —
//! everything after it is git's own rebase, with git's own conflict handling,
//! `git rebase --continue` and `git rebase --abort`.
//!
//! Squashing a run of commits (`squash` below) is the same machinery with the
//! plan written for it rather than by hand, and one `exec` line that puts the
//! message the user typed onto the commit the fold leaves.
//!
//! `reword` is the one action not handed straight to git. Git would open the
//! message editor itself, which is the thing there is none of, so a reword is
//! written into the todo as `edit` and the oid is remembered on the side. When
//! the rebase stops at one, the window recognises it and asks for the message
//! rather than showing the "make your changes" strip an ordinary `edit` gets.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git_cmd;
use crate::journal::{self, Mode};
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
///
/// `GITNOOB_SEQUENCE_EDITOR` stands in for the binary half, and exists for the
/// test suite: the test binary is not this app and cannot answer
/// `--write-todo`, so without it nothing that starts a rebase could be covered
/// end to end. Nothing in the app sets it.
fn sequence_editor(list: &Path) -> Result<String, String> {
    let head = match std::env::var("GITNOOB_SEQUENCE_EDITOR") {
        Ok(over) if !over.trim().is_empty() => over,
        _ => {
            let exe = std::env::current_exe()
                .map_err(|e| format!("Could not find gitnoob's own path: {e}"))?;
            format!("{} --write-todo", shell_quote(&exe))
        }
    };
    Ok(format!("{head} {}", shell_quote(list)))
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

    // The oids the window called rewords, which go in as `edit`. Remembered on
    // disk rather than in memory so that closing the window part-way through a
    // rebase does not lose which stop is which.
    let rewords: Vec<&str> = steps
        .iter()
        .filter(|s| s.action == Action::Reword)
        .map(|s| s.oid.as_str())
        .collect();

    run_todo(
        &root,
        Some(onto),
        &todo,
        &rewords,
        format!("Rebased onto {onto}"),
    )
}

/// Hands git a todo list and lets it replay.
///
/// Everything that starts a rebase here goes through this: the plan the pane
/// built, and the fold a squash asks for. `onto` is `None` for a rebase that
/// reaches the repository's first commit, which git spells `--root`.
fn run_todo(
    root: &Path,
    onto: Option<&str>,
    todo: &str,
    rewords: &[&str],
    done: String,
) -> Result<String, String> {
    let git_dir = crate::remote::git_dir(root)?;
    let list = git_dir.join("gitnoob-rebase-todo");
    std::fs::write(&list, todo).map_err(|e| format!("Could not write the plan: {e}"))?;
    let _ = std::fs::write(git_dir.join("gitnoob-rebase-rewords"), rewords.join("\n"));

    let editor = sequence_editor(&list)?;

    let mut args: Vec<&str> = vec!["rebase", "-i", "--autostash"];
    match onto {
        Some(onto) => args.push(onto),
        None => args.push("--root"),
    }

    let out = git_cmd::run_with_env(
        root,
        &args,
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
        return Ok(done);
    }
    // A rebase that stops is not a rebase that failed: it stopped for an edit,
    // a reword or a conflict, and the window has a strip for each of those.
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        return Ok("Rebase stopped".to_string());
    }
    clear_marks(&git_dir);
    Err(one_message(&out))
}

// --- squash ------------------------------------------------------------------
//
// Folding a run of commits into one is a rebase with the second and later ones
// written as `fixup`, and one `exec` after them that puts the message the user
// wrote onto what is left. The message goes in through a file rather than an
// editor for the same reason everything else here does: there is no editor.
//
// Only a *run* can be folded. Git replays a todo list in order, so commits with
// others in between could only be folded by moving them together first — which
// is a different operation with a different answer to "what did that do to the
// commits I did not pick". The pane's rebase plan is where reordering lives;
// this is the one-gesture version for commits that already sit together.

/// One commit a squash would fold, with the message it brings to the join.
#[derive(Serialize, Debug, PartialEq, Eq, Clone)]
pub struct Folded {
    pub oid: String,
    pub short: String,
    pub summary: String,
    /// The whole message, which is what the joined default is built from.
    pub message: String,
    pub author: String,
    /// When it was committed, for the line the dialog draws.
    pub time: i64,
    /// Already on a remote, so folding it means a force push afterwards.
    pub pushed: bool,
}

/// What the window shows before asking for the fold.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct SquashPreview {
    /// The commits to be folded, oldest first — the order the join reads in.
    pub commits: Vec<Folded>,
    /// The joined message, for the box to start from.
    pub message: String,
    /// The commit the fold lands on, short, or `None` at the root.
    pub onto: Option<String>,
    /// How many commits sit above the run and get replayed onto the fold.
    pub above: usize,
    /// The branch this would rewrite, or `None` on a detached HEAD.
    pub branch: Option<String>,
    /// Why it cannot be done, in the words the dialog shows. `None` when it can.
    pub refusal: Option<String>,
}

/// What the range looks like, worked out once and used by both calls.
struct Survey {
    /// The commits asked for, with any repeat taken out.
    wanted: Vec<String>,
    /// Every commit from the one under the run up to HEAD, oldest first.
    chain: Vec<String>,
    /// How many of the leading ones are being folded.
    fold: usize,
    /// The commit under the run, or `None` when the run starts at the root.
    base: Option<String>,
    refusal: Option<String>,
}

/// Reads the range a squash would touch and decides whether it can be done.
///
/// Everything it refuses on is said in full rather than left for git to fail
/// on: the selection is the user's, and "those two are not next to each other"
/// is an answer they can act on where `error: cannot squash without a previous
/// commit` is not.
fn survey(state: &AppState, oids: &[String]) -> Result<Survey, String> {
    let root = state.path()?;

    // The window can hand the same commit twice — a shift-range over a ctrl
    // click — and git counts a repeated oid as a second commit to fold.
    let mut wanted: Vec<String> = Vec::new();
    for oid in oids {
        if !wanted.contains(oid) {
            wanted.push(oid.clone());
        }
    }
    if wanted.len() < 2 {
        return Err("Squashing folds commits together, so it takes at least two.".to_string());
    }

    // What the fold lands on. A first commit has no parent, and git spells
    // rebasing from there `--root` rather than naming a commit.
    let oldest = oldest_of(&root, &wanted)?;
    let base = git_cmd::run(&root, &["rev-parse", "--verify", &format!("{oldest}^")])
        .ok()
        .filter(|out| out.ok)
        .map(|out| out.stdout.trim().to_string())
        .filter(|oid| !oid.is_empty());

    let range = match &base {
        Some(base) => format!("{base}..HEAD"),
        None => "HEAD".to_string(),
    };
    let listed = git_cmd::run_checked(
        &root,
        &["log", "--reverse", "--topo-order", "--format=%H %P", &range],
    )?;

    let mut chain: Vec<String> = Vec::new();
    let mut merged = false;
    for line in listed.lines() {
        let mut parts = line.split_whitespace();
        let Some(oid) = parts.next() else { continue };
        // Everything after the commit is its parents; two or more is a merge.
        if parts.count() > 1 {
            merged = true;
        }
        chain.push(oid.to_string());
    }

    let mut refusal = None;
    if wanted.iter().any(|oid| !chain.contains(oid)) {
        refusal = Some(
            "Some of those commits are not on the branch you are on. Squashing folds commits \
             that already sit together on this branch."
                .to_string(),
        );
    } else if merged {
        refusal = Some(
            "There is a merge commit between these and the tip of the branch. Replaying over \
             one would flatten it, so this is not offered here."
                .to_string(),
        );
    } else {
        // With no merges the range is a straight line, so the run is contiguous
        // exactly when the chosen commits are the first few of it — `base` is
        // the parent of the oldest, which puts that one at the front.
        let mut positions: Vec<usize> = wanted
            .iter()
            .filter_map(|oid| chain.iter().position(|one| one == oid))
            .collect();
        positions.sort_unstable();
        let run = positions.first() == Some(&0)
            && positions.windows(2).all(|pair| pair[1] == pair[0] + 1);
        if !run {
            let between = positions.last().copied().unwrap_or(0) + 1 - positions.len();
            refusal = Some(format!(
                "Those commits are not next to each other: {between} other {} between them. \
                 Squashing folds a run with nothing in the middle — use the rebase plan to move \
                 them together first.",
                if between == 1 {
                    "commit sits"
                } else {
                    "commits sit"
                }
            ));
        }
    }

    Ok(Survey {
        fold: wanted.len(),
        wanted,
        chain,
        base,
        refusal,
    })
}

/// The oldest of a set of commits: the one all the others have as an ancestor.
///
/// Asked of git rather than worked out from commit dates. Two commits made in
/// the same second are not ordered by their dates at all, and a rebase todo
/// written in the wrong order folds the wrong commit into the wrong one.
fn oldest_of(root: &Path, oids: &[String]) -> Result<String, String> {
    let mut oldest = oids.first().cloned().unwrap_or_default();
    for other in oids.iter().skip(1) {
        let out = git_cmd::run(root, &["merge-base", "--is-ancestor", other, &oldest])?;
        if out.ok {
            oldest = other.clone();
        }
    }
    Ok(oldest)
}

/// The commits a squash would fold, the message it would start from, and what
/// would stop it.
pub fn squash_preview(state: &AppState, oids: &[String]) -> Result<SquashPreview, String> {
    let root = state.path()?;
    let found = survey(state, oids)?;
    // A refused selection is not a run, so there is no front of the chain to
    // read it off: what is described is what was picked, oldest first by date,
    // which is only ever a list to look at rather than an order to fold in.
    let subjects: Vec<String> = if found.refusal.is_some() {
        found.wanted.clone()
    } else {
        found.chain.iter().take(found.fold).cloned().collect()
    };

    let published = published_commits(&root);
    // Anything git cannot read is left out rather than made into an error: it
    // is already the reason for a refusal, and the dialog has more to say about
    // that than "bad object" does.
    let mut commits: Vec<Folded> = subjects
        .iter()
        .filter_map(|oid| read_folded(&root, oid, &published))
        .collect();
    if found.refusal.is_some() {
        commits.sort_by_key(|one| one.time);
    }

    Ok(SquashPreview {
        message: joined_message(&commits),
        onto: found.base.as_ref().map(|oid| oid.chars().take(7).collect()),
        above: found.chain.len().saturating_sub(found.fold),
        branch: journal::current_branch(state),
        refusal: found.refusal,
        commits,
    })
}

fn read_folded(root: &Path, oid: &str, published: &[String]) -> Option<Folded> {
    let raw = git_cmd::run_checked(
        root,
        &[
            "log",
            "-1",
            "--no-show-signature",
            "--format=%H%x1f%an%x1f%ct%x1f%s%x1f%B",
            oid,
        ],
    )
    .ok()?;
    let mut parts = raw.split('\u{1f}');
    let full = parts.next().unwrap_or(oid).trim().to_string();
    let author = parts.next().unwrap_or("").to_string();
    let time = parts.next().unwrap_or("0").trim().parse().unwrap_or(0);
    let summary = parts.next().unwrap_or("").to_string();
    let message = parts.next().unwrap_or("").trim().to_string();
    Some(Folded {
        short: full.chars().take(7).collect(),
        pushed: published.contains(&full),
        oid: full,
        summary,
        message,
        author,
        time,
    })
}

/// The messages of the folded commits, joined the way git's own squash joins
/// them: each one whole, in history order, a blank line between.
fn joined_message(commits: &[Folded]) -> String {
    commits
        .iter()
        .map(|one| one.message.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The todo list that folds the first `fold` commits of `chain` into one.
///
/// `fixup` rather than `squash` for the ones being folded: their messages are
/// already in the text the user has edited, and a `squash` would open the
/// editor that does not exist. The `exec` after them is what puts that text on.
fn squash_todo(chain: &[String], fold: usize, message: &Path) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(chain.len() + 1);
    for (at, oid) in chain.iter().enumerate() {
        if at == 0 || at >= fold {
            lines.push(format!("pick {oid}"));
        } else {
            lines.push(format!("fixup {oid}"));
        }
        if at + 1 == fold {
            // Straight after the last fold and before anything above it is
            // replayed, so the commits on top land on a finished commit.
            lines.push(format!(
                "exec git commit --amend --no-verify --file={}",
                shell_quote(message)
            ));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Folds a run of commits into one carrying `message`.
pub fn squash(state: &AppState, oids: &[String], message: &str) -> Result<String, String> {
    let text = message.trim();
    if text.is_empty() {
        return Err("A commit needs a message".to_string());
    }
    let found = survey(state, oids)?;
    if let Some(why) = found.refusal {
        return Err(why);
    }

    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let note = git_dir.join("gitnoob-squash-message");
    std::fs::write(&note, format!("{text}\n"))
        .map_err(|e| format!("Could not write the message: {e}"))?;

    let todo = squash_todo(&found.chain, found.fold, &note);
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    let folded = found.fold;

    let said = run_todo(
        &root,
        found.base.as_deref(),
        &todo,
        &[],
        format!("Squashed {folded} commits into one"),
    )?;

    // A rebase that stopped has not finished folding, so there is nothing whole
    // to step back from yet; the strip under the toolbar takes it from here.
    if said == "Rebase stopped" {
        return Ok("The squash stopped part-way — resolve it, then continue".to_string());
    }

    // Soft rather than hard: the fold leaves the same tree it started with, so
    // moving the branch back is all an undo has to do, and doing it softly
    // keeps whatever is uncommitted out of it.
    journal::record(
        state,
        "squash",
        format!("Squash {folded} commits"),
        branch,
        before,
        journal::head_oid(state),
        Mode::Soft,
        false,
    );
    Ok(said)
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
    let out = git_cmd::run_with_env(&root, &["rebase", "--continue"], &[("GIT_EDITOR", "true")])?;
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
    let _ = std::fs::remove_file(git_dir.join("gitnoob-squash-message"));
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

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_squash_folds_the_run_and_picks_what_sits_above_it() {
        let todo = squash_todo(
            &ids(&["aaa", "bbb", "ccc", "ddd"]),
            3,
            Path::new("/tmp/msg"),
        );
        assert_eq!(
            todo,
            "pick aaa\n\
             fixup bbb\n\
             fixup ccc\n\
             exec git commit --amend --no-verify --file='/tmp/msg'\n\
             pick ddd\n"
        );
    }

    #[test]
    fn the_message_is_put_on_before_anything_is_replayed_over_it() {
        // The exec has to sit between the last fold and the first commit above
        // it: run at the end instead, it would amend the wrong commit.
        let todo = squash_todo(&ids(&["aaa", "bbb", "ccc"]), 2, Path::new("/tmp/msg"));
        let lines: Vec<&str> = todo.lines().collect();
        assert_eq!(lines[1], "fixup bbb");
        assert!(lines[2].starts_with("exec git commit --amend"));
        assert_eq!(lines[3], "pick ccc");
    }

    #[test]
    fn folding_the_whole_branch_leaves_nothing_to_replay() {
        let todo = squash_todo(&ids(&["aaa", "bbb"]), 2, Path::new("/tmp/msg"));
        assert_eq!(
            todo,
            "pick aaa\nfixup bbb\nexec git commit --amend --no-verify --file='/tmp/msg'\n"
        );
    }

    #[test]
    fn a_message_path_with_a_space_reaches_the_shell_as_one_word() {
        let todo = squash_todo(&ids(&["aaa", "bbb"]), 2, Path::new("/a b/msg"));
        assert!(todo.contains("--file='/a b/msg'"), "{todo}");
    }

    fn folded(message: &str) -> Folded {
        Folded {
            oid: "a".repeat(40),
            short: "aaaaaaa".to_string(),
            summary: message.lines().next().unwrap_or("").to_string(),
            message: message.to_string(),
            author: "Ramon".to_string(),
            time: 1756000000,
            pushed: false,
        }
    }

    #[test]
    fn the_joined_message_keeps_every_message_whole_and_in_order() {
        let joined = joined_message(&[
            folded("feat: the first\n\nwith a body"),
            folded("wip"),
            folded("typo"),
        ]);
        assert_eq!(joined, "feat: the first\n\nwith a body\n\nwip\n\ntypo");
    }

    #[test]
    fn a_commit_with_no_message_adds_no_blank_lines_to_the_join() {
        let joined = joined_message(&[folded("feat: one"), folded("   "), folded("feat: two")]);
        assert_eq!(joined, "feat: one\n\nfeat: two");
    }

    #[test]
    fn a_path_with_a_space_reaches_the_shell_as_one_word() {
        let quoted = shell_quote(Path::new("/Applications/git noob.app/gitnoob"));
        assert_eq!(quoted, "'/Applications/git noob.app/gitnoob'");
        assert_eq!(shell_quote(Path::new("/it's/here")), r"'/it'\''s/here'");
    }
}
