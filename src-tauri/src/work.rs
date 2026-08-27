use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;

use crate::git_cmd;
use crate::journal::{self, Mode};
use crate::state::AppState;

#[derive(Serialize)]
pub struct AmendDraft {
    pub summary: String,
    pub body: String,
    /// True when the commit being amended has already been pushed, which makes
    /// the amend a history rewrite that needs a force push to publish.
    pub is_pushed: bool,
    pub short: String,
}

/// Wraps a user-given path in git's literal pathspec magic.
///
/// A pathspec after `--` still wildmatches by default — `--` only tells git
/// where the revisions end and the paths begin, it does not make what follows
/// literal. A file named `a*.txt`, `f[1].js`, or one starting with `:` would
/// otherwise match more, or something else entirely, than the one path it
/// names.
fn literal_pathspecs(paths: &[String]) -> Vec<String> {
    paths.iter().map(|p| format!(":(literal){p}")).collect()
}

pub fn stage(state: &AppState, paths: &[String]) -> Result<String, String> {
    let root = state.path()?;
    let pathspecs = literal_pathspecs(paths);
    let mut args = vec!["add", "--"];
    args.extend(pathspecs.iter().map(String::as_str));
    git_cmd::run_checked(&root, &args)
}

pub fn stage_all(state: &AppState) -> Result<String, String> {
    let root = state.path()?;
    git_cmd::run_checked(&root, &["add", "--all"])
}

pub fn unstage(state: &AppState, paths: &[String]) -> Result<String, String> {
    let root = state.path()?;
    let pathspecs = literal_pathspecs(paths);
    let mut args = vec!["restore", "--staged", "--"];
    args.extend(pathspecs.iter().map(String::as_str));
    git_cmd::run_checked(&root, &args)
}

/// Throws away working-tree changes to the given paths. Untracked files are left
/// alone: deleting a file the user never committed is not something to do as a
/// side effect of "discard changes".
pub fn discard(state: &AppState, paths: &[String]) -> Result<String, String> {
    let root = state.path()?;
    let pathspecs = literal_pathspecs(paths);
    let mut args = vec!["restore", "--worktree", "--"];
    args.extend(pathspecs.iter().map(String::as_str));
    git_cmd::run_checked(&root, &args)
}

/// Deletes files git is not tracking, which is what discarding one comes to.
///
/// `git clean -f` rather than removing them here: it refuses to touch anything
/// tracked, so a path that turns out to be in the index is left alone instead
/// of being deleted by a menu item that promised something else. `-d` for the
/// directories an untracked folder full of files shows up as.
pub fn delete_untracked(state: &AppState, paths: &[String]) -> Result<String, String> {
    if paths.is_empty() {
        return Err("Nothing to delete".to_string());
    }
    let root = state.path()?;
    let pathspecs = literal_pathspecs(paths);
    let mut args = vec!["clean", "-f", "-d", "--"];
    args.extend(pathspecs.iter().map(String::as_str));
    git_cmd::run_checked(&root, &args)?;
    Ok(format!(
        "Deleted {} {}",
        paths.len(),
        if paths.len() == 1 { "file" } else { "files" }
    ))
}

pub fn commit(state: &AppState, message: &str, amend: bool) -> Result<String, String> {
    let root = state.path()?;
    if message.trim().is_empty() {
        return Err("A commit needs a message".to_string());
    }
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    let mut args = vec!["commit", "-m", message];
    if amend {
        args.push("--amend");
    }
    let output = git_cmd::run_checked(&root, &args)?;

    // Undo is a soft reset back to `before`: for an amend that is the commit
    // that was replaced, which still exists as an object.
    let summary = message.lines().next().unwrap_or("").trim().to_string();
    journal::record(
        state,
        if amend { "amend" } else { "commit" },
        format!("{}: {summary}", if amend { "Amend" } else { "Commit" }),
        branch,
        before,
        journal::head_oid(state),
        Mode::Soft,
        false,
    );
    Ok(output)
}

/// Loads HEAD's message so the amend dialog can start from it, and reports
/// whether amending would rewrite published history.
pub fn amend_draft(state: &AppState) -> Result<AmendDraft, String> {
    let repo = state.repo()?;
    let head = repo
        .head()
        .map_err(|_| "There is no commit to amend".to_string())?;
    let commit = head.peel_to_commit().map_err(|e| e.message().to_string())?;
    let oid = commit.id();

    let (summary, body) = split_message(commit.message().unwrap_or(""));

    Ok(AmendDraft {
        summary,
        body,
        is_pushed: published(&repo, oid),
        short: oid.to_string()[..7].to_string(),
    })
}

/// What rewording a particular commit would involve, asked before the editor
/// opens so the panel can say no rather than letting the user type a message
/// that cannot be applied.
#[derive(Serialize)]
pub struct RewordCheck {
    pub summary: String,
    pub body: String,
    /// False when this commit is not the one an amend would rewrite, in which
    /// case `reason` says so and the editor stays shut.
    pub can: bool,
    pub reason: Option<String>,
    /// True when a remote-tracking branch already has it: rewording is then a
    /// history rewrite that needs a force push to publish.
    pub is_pushed: bool,
}

/// Splits a commit message the way git reads it: first line, then the rest.
fn split_message(message: &str) -> (String, String) {
    let mut lines = message.lines();
    let summary = lines.next().unwrap_or("").trim().to_string();
    let body = lines
        .collect::<Vec<_>>()
        .join("\n")
        .trim_start_matches('\n')
        .trim()
        .to_string();
    (summary, body)
}

/// Whether any remote-tracking branch already contains this commit.
fn published(repo: &git2::Repository, oid: git2::Oid) -> bool {
    let Ok(branches) = repo.branches(Some(git2::BranchType::Remote)) else {
        return false;
    };
    for branch in branches.flatten() {
        if let Some(target) = branch.0.get().target() {
            if target == oid || repo.graph_descendant_of(target, oid).unwrap_or(false) {
                return true;
            }
        }
    }
    false
}

pub fn reword_check(state: &AppState, oid: &str) -> Result<RewordCheck, String> {
    let repo = state.repo()?;
    let commit = repo
        .revparse_single(oid)
        .and_then(|object| object.peel_to_commit())
        .map_err(|e| e.message().to_string())?;
    let (summary, body) = split_message(commit.message().unwrap_or(""));

    let head = repo
        .head()
        .ok()
        .and_then(|head| head.peel_to_commit().ok())
        .map(|head| head.id());

    // Only the newest commit. Anything older would have to be replayed, which
    // rewrites every commit above it, and the mistake this is for — a message
    // typed in haste and regretted a second later — is always on the newest.
    let reason = (head != Some(commit.id()))
        .then(|| "Only the newest commit can be given a new message.".to_string());

    Ok(RewordCheck {
        summary,
        body,
        can: reason.is_none(),
        reason,
        is_pushed: published(&repo, commit.id()),
    })
}

/// Gives the newest commit a new message, keeping everything else about it.
///
/// This is `git commit --amend` with `--only`, which is git's own way of
/// saying "the message and nothing else": whatever is staged stays staged
/// rather than being swept into the commit being reworded.
pub fn reword(state: &AppState, oid: &str, message: &str) -> Result<String, String> {
    let root = state.path()?;
    if message.trim().is_empty() {
        return Err("A commit needs a message".to_string());
    }

    let check = reword_check(state, oid)?;
    if !check.can {
        return Err(check.reason.unwrap_or_else(|| "Cannot reword that commit".to_string()));
    }

    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    let (summary, _) = split_message(message);

    git_cmd::run_checked(
        &root,
        &["commit", "--amend", "--only", "--allow-empty", "-m", message],
    )?;

    let after = journal::head_oid(state);
    journal::record(
        state,
        "reword",
        format!("Reword: {summary}"),
        branch,
        before,
        after.clone(),
        Mode::Soft,
        false,
    );
    after.ok_or_else(|| "The reworded commit went missing".to_string())
}

pub fn stash_push(
    state: &AppState,
    message: Option<&str>,
    include_untracked: bool,
) -> Result<String, String> {
    let root = state.path()?;
    let before = journal::stash_ref(&root);

    let mut args = vec!["stash", "push"];
    if include_untracked {
        args.push("--include-untracked");
    }
    if let Some(message) = message {
        args.push("-m");
        args.push(message);
    }
    let output = git_cmd::run_checked(&root, &args)?;

    // Which stash this is, not just that there was one: undo pops the entry it
    // made, and by the time anyone reaches for undo there may be others on top
    // of it.
    let made = journal::stash_ref(&root);
    // A clean tree makes `stash push` exit 0 with "No local changes to save"
    // rather than failing, and never touches `refs/stash`. Recording it anyway
    // would journal whatever stash was already on top — somebody else's — as
    // this operation's own, and undo would later pop that.
    if made == before {
        return Ok(output);
    }

    journal::record(
        state,
        "stash",
        format!("Stash: {}", message.unwrap_or("uncommitted changes")),
        journal::current_branch(state),
        None,
        made,
        Mode::Stash,
        true,
    );
    Ok(output)
}

pub fn stash_pop(state: &AppState, index: usize) -> Result<String, String> {
    let root = state.path()?;
    let name = format!("stash@{{{index}}}");
    let oid = git_cmd::run_checked(&root, &["rev-parse", &name])
        .map_err(|_| "There is no stash at that position".to_string())?
        .trim()
        .to_string();
    // `pop` only understands `stash@{n}`, not a commit id, so the position is
    // looked up again right before acting rather than trusted from the read
    // above: a push or drop landing on the list in between, here or in another
    // window, shifts what that number means.
    let at = journal::stash_index(state, &oid)
        .ok_or_else(|| "That stash is no longer there".to_string())?;
    git_cmd::run_checked(&root, &["stash", "pop", &format!("stash@{{{at}}}")])
}

// --- stash -----------------------------------------------------------------

/// One stash entry, with enough detail to tell two of them apart.
#[derive(Serialize)]
pub struct StashEntry {
    pub index: usize,
    pub oid: String,
    pub message: String,
    /// The branch that was checked out when it was made.
    pub branch: Option<String>,
    pub time: i64,
    pub files: usize,
}

pub fn stash_list(state: &AppState) -> Result<Vec<StashEntry>, String> {
    let root = state.path()?;
    // A record per line: index, hash, subject, branch, unix time.
    let raw = git_cmd::run_checked(
        &root,
        &[
            "stash",
            "list",
            "--format=%gd%x00%H%x00%gs%x00%at",
        ],
    )?;

    let mut out = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let mut parts = line.split('\0');
        let _selector = parts.next().unwrap_or("");
        let oid = parts.next().unwrap_or("").to_string();
        let subject = parts.next().unwrap_or("").to_string();
        let time = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);

        // `git stash list` subjects read "WIP on main: 1234abc Message" or
        // "On main: my message"; pull the branch out and keep the rest.
        let (branch, message) = split_subject(&subject);
        let files = file_count(&root, &oid, index);

        out.push(StashEntry {
            index,
            oid,
            message,
            branch,
            time,
            files,
        });
    }
    Ok(out)
}

/// How many files a stash entry touches, asked once per entry ever.
///
/// The count needs its own `git stash show`, and the stash list is re-read on
/// every refresh — a file saved in an editor was enough to spawn one process
/// per stash. A stash's oid names its content, so the answer for an oid can
/// never change; only entries seen for the first time cost anything.
fn file_count(root: &Path, oid: &str, index: usize) -> usize {
    let known = FILE_COUNTS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|seen| seen.get(oid).copied());
    if let Some(known) = known {
        return known;
    }
    let count = git_cmd::run_checked(
        root,
        &["stash", "show", "--name-only", &format!("stash@{{{index}}}")],
    )
    .map(|text| text.lines().filter(|l| !l.trim().is_empty()).count())
    .unwrap_or(0);
    // A stash with no oid is a line git did not print the way we expect;
    // remembering the fallback under an empty key would spread it.
    if !oid.is_empty() {
        FILE_COUNTS
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(oid.to_string(), count);
    }
    count
}

static FILE_COUNTS: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

fn split_subject(subject: &str) -> (Option<String>, String) {
    let trimmed = subject
        .strip_prefix("WIP on ")
        .or_else(|| subject.strip_prefix("On "))
        .unwrap_or(subject);
    match trimmed.split_once(": ") {
        Some((branch, rest)) => (Some(branch.to_string()), rest.to_string()),
        None => (None, trimmed.to_string()),
    }
}

/// The commit a stash entry points at, so its diff can be shown like any other.
pub fn stash_oid(state: &AppState, index: usize) -> Result<String, String> {
    let root = state.path()?;
    let name = format!("stash@{{{index}}}");
    Ok(git_cmd::run_checked(&root, &["rev-parse", &name])?
        .trim()
        .to_string())
}

/// Gives a stash a new description, leaving it where it is in the list.
///
/// Git has no command for this. `git stash store` can put a stash commit back
/// under a new message, but only on top of the list — renaming the third stash
/// would silently make it the first, which is not what renaming means. The
/// message lives in the reflog for `refs/stash`, one line per entry, so that
/// line is what gets rewritten.
pub fn stash_rename(state: &AppState, index: usize, message: &str) -> Result<String, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("A stash needs a name".to_string());
    }
    // The message becomes one line of `logs/refs/stash`; a newline in it would
    // write an extra, malformed line and corrupt the reflog everything else
    // here reads the stash list from.
    if message.contains(['\n', '\r']) {
        return Err("A stash name can't contain a line break".to_string());
    }
    let root = state.path()?;
    let selector = format!("stash@{{{index}}}");
    let oid = git_cmd::run_checked(&root, &["rev-parse", &selector])?
        .trim()
        .to_string();

    // `--git-path` answers correctly inside a worktree or a submodule, where
    // `.git` is a file pointing somewhere else entirely.
    let logs = git_cmd::run_checked(&root, &["rev-parse", "--git-path", "logs/refs/stash"])?
        .trim()
        .to_string();
    let logs = root.join(logs);
    let text = fs::read_to_string(&logs).map_err(|e| format!("Could not read the stash log: {e}"))?;

    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // The newest entry is the last line, and `stash@{0}` is the newest.
    let at = lines
        .len()
        .checked_sub(index + 1)
        .ok_or("There is no stash at that position")?;
    let line = lines.get(at).ok_or("There is no stash at that position")?;

    let renamed = renamed_entry(line, &oid, message)
        .ok_or("The stash list moved while it was being renamed")?;
    lines[at] = renamed;

    let mut out = lines.join("\n");
    out.push('\n');
    fs::write(&logs, out).map_err(|e| format!("Could not write the stash log: {e}"))?;
    Ok(format!("Renamed {selector}"))
}

/// One reflog line with its description replaced, or `None` if it is not the
/// entry it was meant to be.
///
/// A line reads `<before> <after> <who> <when> <zone>\t<message>`. Only the
/// message changes, and the "On main: " part of it stays: it says where the
/// stash was made, which is not the user's to rename.
fn renamed_entry(line: &str, oid: &str, message: &str) -> Option<String> {
    let (meta, said) = line.split_once('\t')?;
    // The second field is the commit the entry points at. If it is not the
    // stash being renamed, the list is not what it was when it was read.
    if meta.split_whitespace().nth(1)? != oid {
        return None;
    }
    let prefix = said
        .find(": ")
        .filter(|_| said.starts_with("On ") || said.starts_with("WIP on "))
        .map(|at| &said[..at + 2])
        .unwrap_or("");
    Some(format!("{meta}\t{prefix}{message}"))
}

/// Applies a stash and keeps it in the list.
pub fn stash_apply(state: &AppState, index: usize) -> Result<String, String> {
    let root = state.path()?;
    let name = format!("stash@{{{index}}}");
    let oid = git_cmd::run_checked(&root, &["rev-parse", &name])
        .map_err(|_| "There is no stash at that position".to_string())?
        .trim()
        .to_string();
    // Applying the commit id itself, rather than `stash@{index}` again, means
    // the list shifting under this call — a push or drop elsewhere — cannot
    // land it on an entry other than the one just looked up.
    git_cmd::run_checked(&root, &["stash", "apply", &oid])?;
    Ok(format!("Applied {name}"))
}

pub fn stash_drop(state: &AppState, index: usize) -> Result<String, String> {
    let root = state.path()?;
    let name = format!("stash@{{{index}}}");
    let oid = git_cmd::run_checked(&root, &["rev-parse", &name])
        .map_err(|_| "There is no stash at that position".to_string())?
        .trim()
        .to_string();
    // `drop`, like `pop`, only understands `stash@{n}`; the position is
    // re-resolved from the oid right before dropping so a shift in between
    // cannot make this drop the wrong entry.
    let at = journal::stash_index(state, &oid)
        .ok_or_else(|| "That stash is no longer there".to_string())?;
    git_cmd::run_checked(&root, &["stash", "drop", &format!("stash@{{{at}}}")])?;
    Ok(format!("Dropped {name}"))
}

/// Turns a stash into a branch, which is the safe way out of a stash that no
/// longer applies cleanly to the current branch.
pub fn stash_branch(state: &AppState, index: usize, name: &str) -> Result<String, String> {
    let root = state.path()?;
    let selector = format!("stash@{{{index}}}");
    // A branch fetched as `origin/-f` is real, and without this a name
    // beginning with `-` is parsed as a flag rather than the branch to create.
    git_cmd::run_checked(&root, &["stash", "branch", "--end-of-options", name, &selector])?;
    Ok(format!("Created {name} from {selector}"))
}

// --- auto-stash ------------------------------------------------------------

/// Prefix every auto-stash carries, so the ones this app made can be told from
/// the ones the user made by hand.
pub const AUTO_STASH: &str = "gitnoob auto-stash";

/// Marker returned by [`stash_before`], to be handed back to [`restore_after`].
pub struct Held {
    pub stashed: bool,
}

fn is_dirty(state: &AppState) -> bool {
    state
        .path()
        .ok()
        .and_then(|root| git_cmd::run_checked(&root, &["status", "--porcelain"]).ok())
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false)
}

/// Stashes uncommitted work so an operation that needs a clean tree can run.
///
/// This is the trick that makes switching branches with local edits feel
/// ordinary: stash, switch, put the edits back.
pub fn stash_before(state: &AppState, reason: &str) -> Result<Held, String> {
    if !state.config().global.auto_stash || !is_dirty(state) {
        return Ok(Held { stashed: false });
    }
    let root = state.path()?;
    // The branch is part of the message because it is the only way back: when
    // putting the work down again goes wrong, the way out is to return to the
    // branch it was taken from, and by then HEAD has already moved.
    let message = match crate::journal::current_branch(state) {
        Some(branch) => format!("{AUTO_STASH} on {branch}: {reason}"),
        None => format!("{AUTO_STASH}: {reason}"),
    };
    git_cmd::run_checked(
        &root,
        &["stash", "push", "--include-untracked", "-m", &message],
    )?;
    Ok(Held { stashed: true })
}

/// Puts auto-stashed work back. A conflict here is reported, not swallowed: the
/// changes are still in the stash and the user needs to know that.
pub fn restore_after(state: &AppState, held: Held) -> Result<Option<String>, String> {
    if !held.stashed {
        return Ok(None);
    }
    let root = state.path()?;
    match git_cmd::run_checked(&root, &["stash", "pop"]) {
        Ok(_) => Ok(Some("Local changes were stashed and put back".to_string())),
        Err(error) => Err(format!(
            "Your changes are safe in the stash, but putting them back hit a problem: {error}"
        )),
    }
}

/// Every path a stash entry touches, tracked or not.
///
/// Plain `git stash show` leaves out anything that only lives in the third,
/// untracked-files parent an auto-stash always has, since it was pushed with
/// `--include-untracked`; asking for that parent too is what `--include-untracked`
/// on `show` itself is for.
fn stash_touched_paths(root: &Path, selector: &str) -> Result<HashSet<String>, String> {
    let raw = git_cmd::run_checked(
        root,
        &["stash", "show", "--include-untracked", "--name-only", selector],
    )?;
    Ok(raw.lines().map(str::to_string).filter(|l| !l.is_empty()).collect())
}

/// The branch an auto-stash was taken on, read back out of its message.
fn stashed_on(message: &str) -> Option<&str> {
    let rest = message.split_once(AUTO_STASH)?.1;
    let rest = rest.strip_prefix(" on ")?;
    rest.split_once(':').map(|(branch, _)| branch.trim())
}

/// Undoes an auto-stash that would not go back on.
///
/// A `stash pop` that hits a conflict leaves files conflicted with no merge
/// running, so "abort merge" has nothing to abort and git says so. What the
/// user meant is: forget this, put me back where I was. The stash survives a
/// conflicted pop untouched, which is what makes the reset safe — everything
/// being thrown away here is still in it.
pub fn undo_restore(state: &AppState) -> Result<String, String> {
    let root = state.path()?;
    let list = git_cmd::run_checked(&root, &["stash", "list"])?;
    let top = list.lines().next().unwrap_or_default().to_string();
    if !top.contains(AUTO_STASH) {
        return Err(
            "The stash this would put back is not there any more, so undoing the switch would \
             throw the conflicted files away. Resolve them, or stash them, instead."
                .to_string(),
        );
    }

    // The message alone only says a matching stash exists, not that today's
    // mess is what it made — a failed restore earlier leaves the stash on the
    // list exactly as the guard above expects, and anything typed into the
    // tree since then would be reset away with it. Every dirty path has to be
    // one the stash itself touched, or there is no telling the two apart.
    let touched = stash_touched_paths(&root, "stash@{0}")?;
    let status = crate::refs::status(state)?;
    let mut dirty = status
        .staged
        .iter()
        .chain(status.unstaged.iter())
        .map(|entry| &entry.path)
        .chain(status.conflicted.iter());
    if let Some(path) = dirty.find(|path| !touched.contains(path.as_str())) {
        return Err(format!(
            "{path} has changes the stash never made, so undoing would throw them away instead \
             of just putting the stash back. Resolve or stash that first."
        ));
    }

    // Clears the half-applied pop: conflicted files, and whatever else came
    // out of the stash cleanly.
    git_cmd::run_checked(&root, &["reset", "--hard", "HEAD"])?;

    let home = stashed_on(&top).map(str::to_string);
    let moved = match (&home, crate::journal::current_branch(state)) {
        (Some(home), Some(here)) if home != &here => {
            git_cmd::run_checked(&root, &["checkout", home, "--"])?;
            true
        }
        // An older stash from before the branch was recorded, or a pull rather
        // than a switch: there is nowhere else to go, so put the work back here.
        _ => false,
    };

    git_cmd::run_checked(&root, &["stash", "pop"])?;
    Ok(match (moved, home) {
        (true, Some(branch)) => format!("Back on {branch}, with your changes"),
        _ => "Your changes are back".to_string(),
    })
}

// --- moving a branch and replaying commits ---------------------------------

/// How much a reset touches.
#[derive(serde::Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResetMode {
    /// Move the branch; leave the index and working tree alone. Changes end up
    /// staged.
    Soft,
    /// Move the branch and the index. Changes end up unstaged.
    Mixed,
    /// Move everything. Uncommitted work is destroyed.
    Hard,
}

impl ResetMode {
    fn flag(&self) -> &'static str {
        match self {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        }
    }
}

/// What a reset would do, so the UI can say it before it happens.
#[derive(Serialize)]
pub struct ResetPreview {
    pub target: String,
    pub short: String,
    pub summary: String,
    pub branch: Option<String>,
    /// Commits that would no longer be on the branch.
    pub dropped: Vec<crate::remote::CommitSummary>,
    /// True when the branch is not an ancestor, i.e. this moves sideways.
    pub diverges: bool,
    pub staged_files: usize,
    pub unstaged_files: usize,
}

pub fn reset_preview(state: &AppState, oid: &str) -> Result<ResetPreview, String> {
    let root = state.path()?;
    let branch = journal::current_branch(state);
    let head = journal::head_oid(state);

    let summary = git_cmd::run_checked(&root, &["log", "-1", "--format=%s", oid])?
        .trim()
        .to_string();

    // Commits on the branch now that would not be on it afterwards.
    let raw = git_cmd::run_checked(
        &root,
        &["log", "--format=%H%x00%h%x00%s%x00%an%x00%at", &format!("{oid}..HEAD")],
    )
    .unwrap_or_default();
    let dropped: Vec<crate::remote::CommitSummary> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split('\0');
            crate::remote::CommitSummary {
                oid: parts.next().unwrap_or("").to_string(),
                short: parts.next().unwrap_or("").to_string(),
                summary: parts.next().unwrap_or("").to_string(),
                author: parts.next().unwrap_or("").to_string(),
                time: parts.next().unwrap_or("0").parse().unwrap_or(0),
            }
        })
        .collect();

    // If the target is not an ancestor of HEAD, this is not a plain rewind.
    let diverges = head
        .as_deref()
        .map(|head| {
            git_cmd::run_checked(&root, &["merge-base", "--is-ancestor", oid, head]).is_err()
        })
        .unwrap_or(false);

    let status = crate::refs::status(state)?;

    Ok(ResetPreview {
        target: oid.to_string(),
        short: oid.chars().take(7).collect(),
        summary,
        branch,
        dropped,
        diverges,
        staged_files: status.staged.len(),
        unstaged_files: status.unstaged.len(),
    })
}

/// Moves the current branch to a commit.
pub fn reset(state: &AppState, oid: &str, mode: ResetMode) -> Result<String, String> {
    let root = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    git_cmd::run_checked(&root, &["reset", mode.flag(), oid])?;

    let short: String = oid.chars().take(7).collect();
    journal::record(
        state,
        "reset",
        format!("Reset to {short}"),
        branch,
        before,
        journal::head_oid(state),
        // Undoing a hard reset has to put the working tree back too.
        if mode == ResetMode::Hard {
            Mode::Hard
        } else {
            Mode::Soft
        },
        mode == ResetMode::Hard,
    );
    Ok(format!("Branch moved to {short}"))
}

/// How a cherry-pick should behave.
#[derive(serde::Deserialize, Clone, Copy, Default)]
pub struct CherryPickOptions {
    /// Apply the changes and stage them, but stop short of committing, so the
    /// work can be re-split or amended into something else first.
    #[serde(default)]
    pub no_commit: bool,
    /// Append "(cherry picked from commit …)" to the message. Worth having when
    /// the commit also lives on a branch someone else reads.
    #[serde(default)]
    pub record_origin: bool,
}

/// Copies commits onto the current branch, oldest first.
///
/// Git applies a list in the order given, so the caller's selection is sorted
/// into history order before it is handed over — picking newest-first would
/// conflict against a tree that does not have the earlier change yet.
pub fn cherry_pick(
    state: &AppState,
    oids: &[String],
    options: CherryPickOptions,
) -> Result<String, String> {
    if oids.is_empty() {
        return Err("No commits to cherry-pick".to_string());
    }
    let root = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    let ordered = in_history_order(state, oids)?;

    let mut args: Vec<&str> = vec!["cherry-pick"];
    if options.no_commit {
        args.push("--no-commit");
    }
    if options.record_origin {
        args.push("-x");
    }
    args.extend(ordered.iter().map(String::as_str));

    let out = git_cmd::run(&root, &args)?;
    if !out.ok {
        let conflicts = crate::conflict::list(state).unwrap_or_default();
        if conflicts.is_empty() {
            return Err(if out.stderr.trim().is_empty() {
                out.stdout
            } else {
                out.stderr
            });
        }
        return Ok(format!(
            "Cherry-pick stopped with conflicts in {}",
            conflicts.join(", ")
        ));
    }

    let shorts: Vec<String> = ordered
        .iter()
        .map(|oid| oid.chars().take(7).collect())
        .collect();
    let label = shorts.join(", ");

    // With --no-commit nothing was committed, so there is no new HEAD to undo
    // back from; the change sits in the index for the user to deal with.
    if !options.no_commit {
        journal::record(
            state,
            "cherry-pick",
            format!("Cherry-pick {label}"),
            branch,
            before,
            journal::head_oid(state),
            Mode::Hard,
            true,
        );
    }

    Ok(if options.no_commit {
        format!("Applied {label} without committing — the changes are staged")
    } else {
        format!("Cherry-picked {label}")
    })
}

/// Sorts commits oldest first, using the order git itself walks them in.
///
/// `rev-list --topo-order` lists newest first over the whole repository, so the
/// chosen commits keep their relative history order once reversed. Anything git
/// does not report back is appended in the order it was given, rather than
/// dropped.
fn in_history_order(state: &AppState, oids: &[String]) -> Result<Vec<String>, String> {
    if oids.len() < 2 {
        return Ok(oids.to_vec());
    }
    let root = state.path()?;

    let mut args: Vec<&str> = vec!["rev-list", "--topo-order", "--no-walk"];
    args.extend(oids.iter().map(String::as_str));
    let listed = git_cmd::run_checked(&root, &args)?;

    let mut ordered: Vec<String> = Vec::with_capacity(oids.len());
    for line in listed.lines().rev() {
        let line = line.trim();
        if let Some(found) = oids.iter().find(|o| line.starts_with(o.as_str())) {
            if !ordered.contains(found) {
                ordered.push(found.clone());
            }
        }
    }
    for oid in oids {
        if !ordered.contains(oid) {
            ordered.push(oid.clone());
        }
    }
    Ok(ordered)
}

/// Adds a commit that undoes an earlier one, leaving history intact.
pub fn revert(state: &AppState, oid: &str) -> Result<String, String> {
    let root = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    let out = git_cmd::run(&root, &["revert", "--no-edit", oid])?;
    if !out.ok {
        let conflicts = crate::conflict::list(state).unwrap_or_default();
        if conflicts.is_empty() {
            return Err(if out.stderr.trim().is_empty() {
                out.stdout
            } else {
                out.stderr
            });
        }
        return Ok(format!(
            "Revert stopped with conflicts in {}",
            conflicts.join(", ")
        ));
    }

    let short: String = oid.chars().take(7).collect();
    journal::record(
        state,
        "revert",
        format!("Revert {short}"),
        branch,
        before,
        journal::head_oid(state),
        Mode::Hard,
        true,
    );
    Ok(format!("Reverted {short}"))
}

/// Creates a tag, annotated when a message is given.
pub fn create_tag(
    state: &AppState,
    name: &str,
    oid: &str,
    message: Option<&str>,
) -> Result<String, String> {
    let root = state.path()?;
    let mut args = vec!["tag"];
    if let Some(message) = message.filter(|m| !m.trim().is_empty()) {
        args.extend(["-a", name, "-m", message]);
    } else {
        args.push(name);
    }
    args.push(oid);
    git_cmd::run_checked(&root, &args)?;
    Ok(format!("Tagged {} as {name}", &oid[..oid.len().min(7)]))
}

pub fn delete_tag(state: &AppState, name: &str) -> Result<String, String> {
    let root = state.path()?;
    git_cmd::run_checked(&root, &["tag", "-d", name])?;
    Ok(format!("Deleted tag {name}"))
}

/// Full message of one commit, for the copy-to-clipboard actions.
pub fn commit_message_text(state: &AppState, oid: &str) -> Result<String, String> {
    let root = state.path()?;
    Ok(git_cmd::run_checked(&root, &["log", "-1", "--format=%B", oid])?
        .trim_end()
        .to_string())
}

/// The full patch for a commit, for the "copy patch" action.
pub fn commit_patch(state: &AppState, oid: &str) -> Result<String, String> {
    let root = state.path()?;
    git_cmd::run_checked(&root, &["show", "--no-color", "--patch", oid])
}

/// Hands a path to the desktop's file manager.
pub fn reveal(state: &AppState, relative: &str) -> Result<(), String> {
    let root = state.path()?;
    let target = root.join(relative);
    if !target.exists() {
        return Err(format!("{} is not there any more", target.display()));
    }
    let (program, args): (&str, Vec<String>) = if cfg!(target_os = "macos") {
        ("open", vec!["-R".into(), target.to_string_lossy().into_owned()])
    } else if cfg!(target_os = "windows") {
        ("explorer", vec![format!("/select,{}", target.display())])
    } else {
        // No portable "reveal" on Linux; open the containing directory.
        let dir = target.parent().unwrap_or(&root);
        ("xdg-open", vec![dir.to_string_lossy().into_owned()])
    };
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not reveal {}: {e}", target.display()))
}

// --- hunk-level staging ----------------------------------------------------

/// What to do with a single hunk.
#[derive(serde::Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HunkAction {
    /// Move it into the index.
    Stage,
    /// Take it back out of the index.
    Unstage,
    /// Throw it away in the working tree.
    Discard,
}

/// Applies one hunk of a file's diff.
///
/// Rather than reimplementing patch arithmetic, this asks git for the same diff
/// the UI is showing, keeps the file header and the one hunk the user picked,
/// and feeds that back to `git apply`. The index and the working tree are then
/// changed by git itself, which is the only way to be sure the result matches
/// what a command-line `git add -p` would have produced.
pub fn apply_hunk(
    state: &AppState,
    path: &str,
    hunk_index: usize,
    action: HunkAction,
) -> Result<String, String> {
    let root = state.path()?;

    // Unstaging reads the index-versus-HEAD diff; the other two read the
    // working tree against the index.
    let pathspec = format!(":(literal){path}");
    let diff = if action == HunkAction::Unstage {
        git_cmd::run_checked(
            &root,
            &["diff", "--cached", "--no-color", "--unified=3", "--", &pathspec],
        )?
    } else {
        git_cmd::run_checked(&root, &["diff", "--no-color", "--unified=3", "--", &pathspec])?
    };

    if diff.trim().is_empty() {
        return Err(format!(
            "No {} changes left in {path}",
            if action == HunkAction::Unstage {
                "staged"
            } else {
                "unstaged"
            }
        ));
    }

    let patch = single_hunk_patch(&diff, hunk_index)?;

    let args: &[&str] = match action {
        HunkAction::Stage => &["apply", "--cached", "--whitespace=nowarn", "-"],
        HunkAction::Unstage => &["apply", "--cached", "--reverse", "--whitespace=nowarn", "-"],
        HunkAction::Discard => &["apply", "--reverse", "--whitespace=nowarn", "-"],
    };

    let out = git_cmd::run_with_input(&root, args, &patch)?;
    if !out.ok {
        let detail = if out.stderr.trim().is_empty() {
            out.stdout
        } else {
            out.stderr
        };
        return Err(format!("git could not apply that hunk: {}", detail.trim()));
    }

    Ok(match action {
        HunkAction::Stage => format!("Staged one hunk of {path}"),
        HunkAction::Unstage => format!("Unstaged one hunk of {path}"),
        HunkAction::Discard => format!("Discarded one hunk of {path}"),
    })
}

/// Rebuilds a patch containing the file header and exactly one of its hunks.
fn single_hunk_patch(diff: &str, hunk_index: usize) -> Result<String, String> {
    let mut header: Vec<&str> = Vec::new();
    let mut hunks: Vec<Vec<&str>> = Vec::new();

    for line in diff.lines() {
        if line.starts_with("@@") {
            hunks.push(vec![line]);
        } else if let Some(current) = hunks.last_mut() {
            current.push(line);
        } else {
            header.push(line);
        }
    }

    let hunk = hunks.get(hunk_index).ok_or_else(|| {
        format!(
            "That hunk is no longer there — the file has {} {}",
            hunks.len(),
            if hunks.len() == 1 { "hunk" } else { "hunks" }
        )
    })?;

    let mut patch = header.join("\n");
    patch.push('\n');
    patch.push_str(&hunk.join("\n"));
    // `git apply` insists on a trailing newline.
    patch.push('\n');
    Ok(patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line as git writes it into `.git/logs/refs/stash`.
    const ENTRY: &str = "0000000000000000000000000000000000000000 abc123 A <a@b.c> 1700000000 +0100\tOn main: the old name";

    #[test]
    fn renaming_keeps_the_branch_the_stash_was_made_on() {
        let out = renamed_entry(ENTRY, "abc123", "the new name").expect("renamed");
        assert!(out.ends_with("\tOn main: the new name"));
        // Everything before the message is the reflog's own bookkeeping and is
        // left exactly as it was.
        assert_eq!(
            out.split('\t').next(),
            ENTRY.split('\t').next()
        );
    }

    #[test]
    fn a_message_with_no_branch_in_it_is_replaced_whole() {
        let line = "0000 abc123 A <a@b.c> 1700000000 +0100\twhatever this was";
        let out = renamed_entry(line, "abc123", "named").expect("renamed");
        assert!(out.ends_with("\tnamed"));
    }

    #[test]
    fn a_line_pointing_somewhere_else_is_refused() {
        // The list changed under us: renaming it would rename another stash.
        assert!(renamed_entry(ENTRY, "def456", "nope").is_none());
    }

    const DIFF: &str = "\
diff --git a/a.txt b/a.txt
index 1234567..89abcde 100644
--- a/a.txt
+++ b/a.txt
@@ -1,3 +1,4 @@
 one
+first addition
 two
 three
@@ -10,3 +11,4 @@
 ten
+second addition
 eleven
 twelve
";

    #[test]
    fn keeps_the_header_and_only_the_chosen_hunk() {
        let first = single_hunk_patch(DIFF, 0).unwrap();
        assert!(first.contains("--- a/a.txt"));
        assert!(first.contains("+++ b/a.txt"));
        assert!(first.contains("@@ -1,3 +1,4 @@"));
        assert!(first.contains("+first addition"));
        assert!(!first.contains("+second addition"));
        assert!(!first.contains("@@ -10,3 +11,4 @@"));
        assert!(first.ends_with('\n'));

        let second = single_hunk_patch(DIFF, 1).unwrap();
        assert!(second.contains("@@ -10,3 +11,4 @@"));
        assert!(second.contains("+second addition"));
        assert!(!second.contains("+first addition"));
    }

    #[test]
    fn reports_a_hunk_that_is_no_longer_there() {
        let error = single_hunk_patch(DIFF, 7).unwrap_err();
        assert!(error.contains("2 hunks"), "unexpected message: {error}");
    }

    #[test]
    fn reads_the_branch_out_of_an_auto_stash() {
        // What `git stash list` actually prints: its own "On <branch>:" prefix
        // in front of the message this app gave it.
        assert_eq!(
            stashed_on("stash@{0}: On main: gitnoob auto-stash on main: switching to other"),
            Some("main")
        );
        // A branch with slashes in it, which is most of them.
        assert_eq!(
            stashed_on("stash@{0}: On x: gitnoob auto-stash on feature/ASANA-12: pulling"),
            Some("feature/ASANA-12")
        );
        // Stashes from before the branch was recorded, and the user's own.
        assert_eq!(stashed_on("stash@{0}: On main: gitnoob auto-stash: pulling"), None);
        assert_eq!(stashed_on("stash@{0}: WIP on main: 1234567 something"), None);
    }
}
