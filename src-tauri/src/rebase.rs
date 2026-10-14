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
///
/// A range with a merge in it is refused rather than listed. The plan is
/// replayed without `--rebase-merges`, which lays the merged commits out in a
/// line and throws the merge away, along with whatever was resolved inside
/// it. Leaving the merge out of the list used to hide exactly that.
pub fn plan(state: &AppState, onto: &str) -> Result<Vec<Candidate>, String> {
    let root = state.path()?;
    let range = format!("{onto}..HEAD");
    if let Some(why) = merge_in_range(&root, &range) {
        return Err(why);
    }
    let raw = git_cmd::run_checked(
        &root,
        &[
            "log",
            "--reverse",
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
    let unpushed = unpushed_in(&root, &range);
    for one in &mut found {
        one.pushed = !unpushed.contains(&one.oid);
    }
    Ok(found)
}

/// Why a range cannot be replayed as a straight line, or `None` when it can.
///
/// Every rewrite here — the plan, a fixup, a split, a message change — runs
/// without `--rebase-merges`, so a merge inside the range would come out the
/// other side flattened. Said up front, naming the merge, so the answer is
/// "start at or above that one" rather than a history quietly redrawn.
pub fn merge_in_range(root: &Path, range: &str) -> Option<String> {
    let newest = git_cmd::run(
        root,
        &[
            "log",
            "--merges",
            "-1",
            "--no-show-signature",
            "--format=%h%x1f%s",
            range,
        ],
    )
    .ok()
    .filter(|out| out.ok)
    .map(|out| out.stdout.trim().to_string())
    .filter(|line| !line.is_empty())?;
    let (short, subject) = newest.split_once('\u{1f}').unwrap_or((&newest, ""));
    Some(format!(
        "There is a merge commit in the way ({short} {subject}). Replaying the commits around \
         it would flatten the merge and lose whatever was resolved inside it, so this only works \
         on the commits above the newest merge."
    ))
}

/// The commits in `range` that no remote-tracking branch has.
///
/// `--not --remotes` asks the whole question, however long ago the commit was
/// pushed. It used to be a list of the newest two thousand remote commits,
/// which in a busy repository missed anything older and called it unpushed —
/// and the warning about a force push went missing exactly when it mattered.
pub fn unpushed_in(root: &Path, range: &str) -> Vec<String> {
    git_cmd::run(root, &["rev-list", range, "--not", "--remotes"])
        .ok()
        .filter(|out| out.ok)
        .map(|out| out.stdout.lines().map(|l| l.trim().to_string()).collect())
        .unwrap_or_default()
}

/// Whether some remote-tracking branch already has this commit.
pub fn is_published(root: &Path, oid: &str) -> bool {
    // A remote has it exactly when nothing is left of it once everything the
    // remotes reach is taken away.
    git_cmd::run(root, &["rev-list", "-n", "1", oid, "--not", "--remotes"])
        .ok()
        .filter(|out| out.ok)
        .is_some_and(|out| out.stdout.trim().is_empty())
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

/// The todo list git will be handed.
///
/// A `squash` or `fixup` with nothing above it to fold into is not a plan git
/// will accept, so it is refused here where it can be explained rather than
/// there where it cannot.
pub fn todo_text(steps: &[Step]) -> Result<String, String> {
    todo_with_refs(steps, &[])
}

/// The todo list, with an `update-ref` line for every stacked branch.
///
/// `--update-refs` would have git write those lines itself, but the list git
/// writes is the one the window's plan is copied over, so they are put back
/// here: each branch goes after the commit it pointed at, or after the fold
/// that commit ends up inside, which is where git puts them too. A branch on a
/// dropped commit stays at that place in the list and so lands on whatever
/// was kept before it.
pub fn todo_with_refs(steps: &[Step], stacked: &[(String, String)]) -> Result<String, String> {
    let mut lines = Vec::new();
    let mut has_root = false;
    let mut pending: Vec<&str> = Vec::new();
    let flush = |lines: &mut Vec<String>, pending: &mut Vec<&str>| {
        for name in pending.drain(..) {
            lines.push(format!("update-ref refs/heads/{name}"));
        }
    };
    for step in steps {
        let folds = matches!(step.action, Action::Squash | Action::Fixup);
        if folds && !has_root {
            return Err(
                "The first commit of a rebase has nothing above it to fold into. Move it down, or pick it."
                    .to_string(),
            );
        }
        if step.action != Action::Drop {
            // A ref is never put between a commit and what folds into it:
            // the branch would be left on the half-made commit.
            if !folds {
                flush(&mut lines, &mut pending);
                has_root = true;
            }
            // A dropped commit is left out rather than written as `drop`: both
            // work, and a shorter list is a list with less to go wrong in it.
            lines.push(format!("{} {}", step.action.word(), step.oid));
        }
        pending.extend(
            stacked
                .iter()
                .filter(|(_, oid)| *oid == step.oid)
                .map(|(name, _)| name.as_str()),
        );
    }
    if !lines.iter().any(|line| !line.starts_with("update-ref")) {
        return Err("That plan drops every commit. Use reset instead.".to_string());
    }
    flush(&mut lines, &mut pending);
    lines.push(String::new());
    Ok(lines.join("\n"))
}

// --- what every rewrite runs with --------------------------------------------

/// The settings every command that can stop on a conflict runs with.
///
/// `diff3` puts the merge base into the markers, which is the third pane of
/// the resolver; without it a cherry-pick or a squash that stopped showed only
/// two sides of a three-sided question. `rerere` remembers how a conflict was
/// resolved and answers the same conflict the same way next time. Both are
/// given on the command line, so the user's own git config is never written.
pub fn conflict_flags(state: &AppState) -> Vec<String> {
    let mut flags = vec!["-c".to_string(), "merge.conflictStyle=diff3".to_string()];
    flags.extend(rerere_flags(state));
    flags
}

/// `-c rerere.enabled=…` saying what the app's setting says, or nothing when
/// the user set `rerere.enabled` in their own git config.
///
/// Off has to be said out loud: once `.git/rr-cache` exists, git takes rerere
/// to be on whenever the config is silent, so leaving the flag out did not
/// turn it off at all. A value the user configured themselves, at any level,
/// is theirs, and is left to stand.
pub fn rerere_flags(state: &AppState) -> Vec<String> {
    let Ok(root) = state.path() else {
        return Vec::new();
    };
    if user_set_rerere(&root) {
        return Vec::new();
    }
    let value = if rerere_on(state) { "true" } else { "false" };
    vec!["-c".to_string(), format!("rerere.enabled={value}")]
}

/// Whether `rerere.enabled` is set anywhere in the user's git config.
fn user_set_rerere(root: &Path) -> bool {
    git2::Repository::open(root)
        .and_then(|repo| repo.config())
        .and_then(|mut config| config.snapshot())
        .is_ok_and(|config| config.get_bool("rerere.enabled").is_ok())
}

/// Whether this repository has rerere turned on in the app. Off unless it
/// was turned on for this one.
pub fn rerere_on(state: &AppState) -> bool {
    let Ok(root) = state.path() else {
        return false;
    };
    let here = root.to_string_lossy().to_string();
    state.config().global.rerere_on.contains(&here)
}

/// Turns rerere on or off for the repository that is open.
pub fn set_rerere(state: &AppState, enabled: bool) -> Result<bool, String> {
    let here = state.path()?.to_string_lossy().to_string();
    state.update_config(|config| {
        let list = &mut config.global.rerere_on;
        list.retain(|path| *path != here);
        if enabled {
            list.push(here.clone());
        }
    })?;
    Ok(enabled)
}

/// The version of git on the path, as `(major, minor)`, read once.
pub fn git_version() -> Option<(u32, u32)> {
    static FOUND: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        let mut command = std::process::Command::new("git");
        git_cmd::quiet(&mut command);
        let out = command.arg("version").output().ok()?;
        parse_version(&String::from_utf8_lossy(&out.stdout))
    })
}

/// `git version 2.50.1 (Apple Git-155)`, `git version 2.45.1.windows.1`.
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let number = text
        .split_whitespace()
        .find(|word| word.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut parts = number.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Whether stacked branches should move with a rebase: the setting is on and
/// git is new enough to do it (2.38 brought `--update-refs`).
///
/// Never on a detached HEAD: with no branch being rebased there is nothing for
/// the others to be stacked on, and moving them would be a surprise.
pub fn update_refs_on(state: &AppState) -> bool {
    state.config().global.update_refs
        && git_version().is_some_and(|found| found >= (2, 38))
        && journal::current_branch(state).is_some()
}

/// The local branches a rebase of `range` would carry along: every one whose
/// tip is among the commits being replayed, except the one checked out here
/// and any checked out in another worktree, which git will not move.
pub fn stacked_branches(state: &AppState, range: &str) -> Vec<(String, String)> {
    stacked_and_held(state, range).0
}

/// [`stacked_branches`], and beside it the branches on the replayed commits
/// that are left out of it on purpose: a backup at the tip, a colleague's
/// branch. Git's own `--update-refs` would move those too.
pub fn stacked_and_held(state: &AppState, range: &str) -> (Vec<(String, String)>, Vec<String>) {
    let none = || (Vec::new(), Vec::new());
    let Ok(root) = state.path() else {
        return none();
    };
    let replayed: std::collections::HashSet<String> = git_cmd::run(&root, &["rev-list", range])
        .ok()
        .filter(|out| out.ok)
        .map(|out| out.stdout.lines().map(|l| l.trim().to_string()).collect())
        .unwrap_or_default();
    if replayed.is_empty() {
        return none();
    }
    let elsewhere: Vec<String> = git_cmd::run(&root, &["worktree", "list", "--porcelain"])
        .ok()
        .filter(|out| out.ok)
        .map(|out| {
            out.stdout
                .lines()
                .filter_map(|line| line.strip_prefix("branch refs/heads/"))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    // Nothing rides along with a detached HEAD: there is no branch being
    // rebased for the others to be stacked on.
    let Some(here) = journal::current_branch(state) else {
        return none();
    };
    let Ok(repo) = state.repo() else {
        return none();
    };
    let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) else {
        return none();
    };
    let head_tip = journal::head_at(&root);
    let mut found = Vec::new();
    let mut held = Vec::new();
    for (branch, _) in branches.flatten() {
        let Some(name) = branch.name().ok().flatten().map(str::to_string) else {
            continue;
        };
        let Some(oid) = branch.get().target().map(|oid| oid.to_string()) else {
            continue;
        };
        if name == here || elsewhere.contains(&name) || !replayed.contains(&oid) {
            continue;
        }
        // A branch on the very tip is not stacked on anything: it is the
        // same work under another name — a backup made before the rebase,
        // most often — and moving it takes away the very copy it was kept
        // as. git's own `--update-refs` moves it; this list leaves it be.
        if head_tip.as_deref() == Some(oid.as_str()) {
            held.push(name);
            continue;
        }
        // One that follows a remote branch of another name is somebody
        // else's branch, checked out here to look at: not ours to rewrite.
        if crate::refs::configured_upstream(&repo, &name)
            .is_some_and(|(remote, merge)| remote != "." && merge != name)
        {
            held.push(name);
            continue;
        }
        found.push((name, oid));
    }
    found.sort();
    held.sort();
    (found, held)
}

/// Where a rebase that may stop remembers which branches it was carrying, so
/// the one that finishes it — a continue, maybe after a restart — can say
/// which of them moved.
const CARRIED: &str = "gitnoob-update-refs";

pub fn remember_carried(git_dir: &Path, stacked: &[(String, String)]) {
    let file = git_dir.join(CARRIED);
    if stacked.is_empty() {
        let _ = std::fs::remove_file(file);
        return;
    }
    let text: Vec<String> = stacked
        .iter()
        .map(|(name, oid)| format!("{oid} {name}"))
        .collect();
    let _ = std::fs::write(file, text.join("\n") + "\n");
}

/// The branches the rebase that just ended moved, said once and forgotten.
///
/// Their old tips stay in each branch's reflog (`<branch>@{1}`, entry
/// "rebase (update-refs)"), which is the way back for any of them; the note
/// names the old commit so it can be found without knowing that.
pub fn carried_note(state: &AppState, git_dir: &Path) -> Option<String> {
    let file = git_dir.join(CARRIED);
    let text = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    let repo = state.repo().ok()?;
    let mut moved = Vec::new();
    for line in text.lines() {
        let Some((old, name)) = line.split_once(' ') else {
            continue;
        };
        let now = repo
            .find_branch(name, git2::BranchType::Local)
            .ok()
            .and_then(|branch| branch.get().target())
            .map(|oid| oid.to_string());
        if now.as_deref().is_some_and(|now| now != old) {
            moved.push(format!("{name} (was {})", &old[..old.len().min(7)]));
        }
    }
    (!moved.is_empty()).then(|| {
        format!(
            "Also moved {}. The old tips are in each branch's reflog.",
            moved.join(", ")
        )
    })
}

/// `said`, with the stacked branches that moved underneath it.
pub fn with_carried(state: &AppState, git_dir: &Path, said: String) -> String {
    match carried_note(state, git_dir) {
        Some(note) => format!("{}\n{note}", said.trim_end()),
        None => said,
    }
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
pub(crate) fn sequence_editor(list: &Path) -> Result<String, String> {
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
    // Asked again here rather than trusted from the plan: the branch can have
    // moved between the pane opening and the button being pressed.
    if let Some(why) = merge_in_range(&root, &format!("{onto}..HEAD")) {
        return Err(why);
    }
    let stacked = carried_by(state, Some(onto));
    let todo = todo_with_refs(&steps, &stacked)?;

    // The oids the window called rewords, which go in as `edit`. Remembered on
    // disk rather than in memory so that closing the window part-way through a
    // rebase does not lose which stop is which.
    let rewords: Vec<&str> = steps
        .iter()
        .filter(|s| s.action == Action::Reword)
        .map(|s| s.oid.as_str())
        .collect();

    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    let short: String = onto.chars().take(7).collect();
    let label = format!("Interactive rebase onto {short}");

    let said = run_todo(
        state,
        Some(onto),
        &todo,
        &rewords,
        "true",
        &stacked,
        format!("Rebased onto {onto}"),
    )?;
    // One that stopped — for a conflict, an edit or a reword — is recorded
    // whole by whatever finishes it.
    if said == "Rebase stopped" {
        journal::begin_pending(state, "rebase", label, branch, before);
    } else {
        journal::record(
            state,
            "rebase",
            label,
            branch,
            before,
            journal::head_oid(state),
            Mode::Hard,
            true,
        );
    }
    Ok(said)
}

/// The branches a rewrite from `base` would carry, when that is switched on.
pub(crate) fn carried_by(state: &AppState, base: Option<&str>) -> Vec<(String, String)> {
    carried_and_held(state, base).0
}

/// [`carried_by`], with the branches on those commits it leaves alone.
pub(crate) fn carried_and_held(
    state: &AppState,
    base: Option<&str>,
) -> (Vec<(String, String)>, Vec<String>) {
    if !update_refs_on(state) {
        return (Vec::new(), Vec::new());
    }
    let range = match base {
        Some(base) => format!("{base}..HEAD"),
        None => "HEAD".to_string(),
    };
    stacked_and_held(state, &range)
}

/// Hands git a todo list and lets it replay.
///
/// Everything that starts a rebase here goes through this: the plan the pane
/// built, and the fold a squash asks for. `onto` is `None` for a rebase that
/// reaches the repository's first commit, which git spells `--root`.
pub(crate) fn run_todo(
    state: &AppState,
    onto: Option<&str>,
    todo: &str,
    rewords: &[&str],
    editor: &str,
    stacked: &[(String, String)],
    done: String,
) -> Result<String, String> {
    let root = state.path()?;
    let root = root.as_path();
    let git_dir = crate::remote::git_dir(root)?;
    // Refused here, before anything is written: git would refuse anyway, but
    // by then the todo and rewords files of the rebase that IS running would
    // have been overwritten — and the exists-afterwards check below would read
    // that old rebase as this one having stopped.
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        return Err(
            "A rebase is already part-way through. Continue or abort that one first.".to_string(),
        );
    }
    let list = git_dir.join("gitnoob-rebase-todo");
    std::fs::write(&list, todo).map_err(|e| format!("Could not write the plan: {e}"))?;
    let _ = std::fs::write(git_dir.join("gitnoob-rebase-rewords"), rewords.join("\n"));

    let list_editor = sequence_editor(&list)?;
    remember_carried(&git_dir, stacked);

    let flags = conflict_flags(state);
    let mut args: Vec<&str> = flags.iter().map(String::as_str).collect();
    args.extend(["rebase", "-i", "--autostash"]);
    match onto {
        Some(onto) => args.push(onto),
        None => args.push("--root"),
    }

    let out = git_cmd::run_with_env(
        root,
        &args,
        &[
            ("GIT_SEQUENCE_EDITOR", list_editor.as_str()),
            // No editor may sit open waiting on a window that does not exist.
            // For the plan this is `true` — a squash there takes the joined
            // message git prepared, the same thing saving an unedited editor
            // does. The one-gesture squash instead points it at the message
            // the user already wrote, through the same copy-and-exit trick
            // the todo list itself arrives by.
            ("GIT_EDITOR", editor),
        ],
    )?;

    if out.ok && !still_rebasing(&git_dir) {
        clear_marks(&git_dir);
        return Ok(with_carried(state, &git_dir, done));
    }
    // A rebase that stops is not a rebase that failed: it stopped for an edit,
    // a reword or a conflict, and the window has a strip for each of those.
    if still_rebasing(&git_dir) {
        return Ok("Rebase stopped".to_string());
    }
    clear_marks(&git_dir);
    let _ = carried_note(state, &git_dir);
    Err(one_message(&out))
}

fn still_rebasing(git_dir: &Path) -> bool {
    git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists()
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
    // rebasing from there `--root` rather than naming a commit. The oldest is
    // asked of ancestry, the same way cherry-pick orders its picks: dates tie,
    // lines of history do not.
    let oldest = crate::work::in_history_order(state, &wanted)?
        .into_iter()
        .next()
        .expect("checked non-empty above");
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

    // Anything git cannot read is left out rather than made into an error: it
    // is already the reason for a refusal, and the dialog has more to say about
    // that than "bad object" does.
    let mut commits: Vec<Folded> = subjects
        .iter()
        .filter_map(|oid| read_folded(&root, oid))
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

fn read_folded(root: &Path, oid: &str) -> Option<Folded> {
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
        pushed: is_published(root, &full),
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
/// `squash` rather than `fixup` plus an `exec git commit --amend`: git opens
/// the message editor once for the whole run, and the editor slot is already
/// ours, so the user's text goes in through the front door. An exec used to do
/// it, and had a failure mode all its own — an exec that fails (a signing
/// hiccup, say) is *not* re-run by `git rebase --continue`, so the fold kept
/// the wrong message without a word said. A squash-commit that fails is
/// retried on continue, message and all, because the message is git's own
/// state by then.
#[cfg(test)]
fn squash_todo(chain: &[String], fold: usize) -> String {
    squash_todo_with(chain, fold, &[])
}

/// The same list, carrying the stacked branches along.
fn squash_todo_with(chain: &[String], fold: usize, stacked: &[(String, String)]) -> String {
    let steps: Vec<Step> = chain
        .iter()
        .enumerate()
        .map(|(at, oid)| Step {
            oid: oid.clone(),
            action: if at == 0 || at >= fold {
                Action::Pick
            } else {
                Action::Squash
            },
        })
        .collect();
    // The first step is always a pick, so this cannot be refused.
    todo_with_refs(&steps, stacked).unwrap_or_default()
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

    let stacked = carried_by(state, found.base.as_deref());
    let todo = squash_todo_with(&found.chain, found.fold, &stacked);
    let editor = sequence_editor(&note)?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    let folded = found.fold;

    let said = run_todo(
        state,
        found.base.as_deref(),
        &todo,
        &[],
        &editor,
        &stacked,
        format!("Squashed {folded} commits into one"),
    )?;

    // A rebase that stopped has not finished folding, so there is nothing whole
    // to step back from yet; the strip under the toolbar takes it from here.
    if said == "Rebase stopped" {
        journal::begin_pending(
            state,
            "squash",
            format!("Squash {folded} commits"),
            branch,
            before,
        );
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
    /// The subject of the commit being split, when this stop is a split.
    pub splitting: Option<String>,
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
    let splitting = crate::rewrite::splitting(&git_dir)
        .filter(|oid| stopped.as_deref() == Some(oid.as_str()))
        .and_then(|_| summary.clone());

    Ok(Some(Progress {
        at: number("msgnum"),
        total: number("end"),
        stopped,
        summary,
        rewording,
        message,
        splitting,
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
///
/// A rebase a visit left stopped on another branch's behalf ends here as
/// surely as from the conflict sheet, so this one goes home too.
pub fn resume(state: &AppState) -> Result<String, String> {
    step_on(state, "--continue")
}

/// Leaves the commit it stopped at out and moves on.
pub fn skip(state: &AppState) -> Result<String, String> {
    step_on(state, "--skip")
}

/// `rebase --continue` or `--skip`, with the same settings the rebase started
/// with: the next commit can stop on a conflict as well, and that one deserves
/// its base pane and its remembered resolution as much as the first did.
fn step_on(state: &AppState, how: &str) -> Result<String, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let leg = crate::remote::trip(state);
    let waiting = journal::pending(state);
    let flags = conflict_flags(state);
    let mut args: Vec<&str> = flags.iter().map(String::as_str).collect();
    args.extend(["rebase", how]);
    let out = git_cmd::run_with_env(&root, &args, &[("GIT_EDITOR", "true")])?;
    journal::finish_pending(state, waiting);
    settle(state, &git_dir, out, "Rebase finished")
        .map(|said| crate::remote::and_home(state, leg, said))
}

/// Puts the branch back exactly as it was before the rebase started.
pub fn abort(state: &AppState) -> Result<String, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let leg = crate::remote::trip(state);
    let waiting = journal::pending(state);
    // A split's files are where its work is; the abort resets them.
    let kept = crate::rewrite::keep_for_abort(state)?;
    crate::rewrite::clear_for_abort(state);
    let out = git_cmd::run(&root, &["rebase", "--abort"])?;
    journal::finish_pending(state, waiting);
    clear_marks(&git_dir);
    // Nothing was moved: git only moves the stacked branches at the very end.
    let _ = std::fs::remove_file(git_dir.join(CARRIED));
    if out.ok {
        let mut said = "Rebase abandoned; the branch is as it was".to_string();
        if let Some(kept) = kept {
            said = format!("{said}\n{kept}");
        }
        Ok(crate::remote::and_home(state, leg, said))
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
fn settle(
    state: &AppState,
    git_dir: &Path,
    out: git_cmd::CmdOutput,
    done: &str,
) -> Result<String, String> {
    if still_rebasing(git_dir) {
        return Ok("Rebase stopped".to_string());
    }
    clear_marks(git_dir);
    let said = carried_note(state, git_dir);
    if out.ok {
        Ok(match said {
            Some(note) => format!("{done}\n{note}"),
            None => done.to_string(),
        })
    } else {
        Err(one_message(&out))
    }
}

/// The files this app leaves beside git's own, once there is no rebase left.
pub(crate) fn clear_marks(git_dir: &Path) {
    let _ = std::fs::remove_file(git_dir.join("gitnoob-rebase-todo"));
    let _ = std::fs::remove_file(git_dir.join("gitnoob-rebase-rewords"));
    let _ = std::fs::remove_file(git_dir.join("gitnoob-squash-message"));
    let _ = std::fs::remove_file(git_dir.join(crate::rewrite::SPLIT));
}

pub(crate) fn one_message(out: &git_cmd::CmdOutput) -> String {
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
    fn reads_the_git_version_whatever_the_platform_adds_to_it() {
        assert_eq!(
            parse_version("git version 2.50.1 (Apple Git-155)\n"),
            Some((2, 50))
        );
        assert_eq!(parse_version("git version 2.45.1.windows.1"), Some((2, 45)));
        assert_eq!(parse_version("git version 2.37.0"), Some((2, 37)));
        assert_eq!(parse_version("nonsense"), None);
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
        let todo = squash_todo(&ids(&["aaa", "bbb", "ccc", "ddd"]), 3);
        assert_eq!(todo, "pick aaa\nsquash bbb\nsquash ccc\npick ddd\n");
    }

    #[test]
    fn folding_the_whole_branch_leaves_nothing_to_replay() {
        let todo = squash_todo(&ids(&["aaa", "bbb"]), 2);
        assert_eq!(todo, "pick aaa\nsquash bbb\n");
    }

    #[test]
    fn the_fold_is_squashes_not_fixups_so_the_editor_is_asked_once() {
        // The editor slot is ours, so "asked" means the user's message file is
        // copied in. A fixup would skip the editor and keep the first commit's
        // message; an exec after it was the old way, and an exec that fails is
        // not re-run on continue — the message would go missing in silence.
        let todo = squash_todo(&ids(&["aaa", "bbb", "ccc"]), 3);
        assert!(!todo.contains("fixup"), "{todo}");
        assert!(!todo.contains("exec"), "{todo}");
        assert_eq!(todo.matches("squash").count(), 2, "{todo}");
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
