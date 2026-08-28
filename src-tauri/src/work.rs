use std::collections::{BTreeSet, HashMap};
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
    // A file moved in the working tree is one row but two things to stage: the
    // deletion where it was, and the file where it is now. Staging only the new
    // name leaves the old one hanging as an unstaged deletion, and git never
    // sees the pair as the move it is.
    let both = with_both_halves(state, paths, Side::Unstaged);
    let pathspecs = literal_pathspecs(&both);
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
    // Same as staging, the other way round: taking back only the name the row
    // carries leaves the file both moved and deleted in the index.
    let both = with_both_halves(state, paths, Side::Staged);
    let pathspecs = literal_pathspecs(&both);
    let mut args = vec!["restore", "--staged", "--"];
    args.extend(pathspecs.iter().map(String::as_str));
    git_cmd::run_checked(&root, &args)
}

/// Which side of the index a path was read off.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Staged,
    Unstaged,
}

/// The moves among `paths`, as the name a file has now and the one it had.
///
/// A move is two entries — a deletion and an arrival — drawn as one row, so a
/// command given the row's path is only ever given half of it.
fn moves(state: &AppState, paths: &[String], side: Side) -> Vec<(String, String)> {
    let Ok(status) = crate::refs::status(state) else {
        return Vec::new();
    };
    let list = if side == Side::Staged {
        status.staged
    } else {
        status.unstaged
    };
    list.into_iter()
        .filter(|entry| paths.contains(&entry.path))
        .filter_map(|entry| entry.from.map(|from| (entry.path, from)))
        .collect()
}

/// `paths` with the other half of every move among them added.
fn with_both_halves(state: &AppState, paths: &[String], side: Side) -> Vec<String> {
    let mut all = paths.to_vec();
    for (_, from) in moves(state, paths, side) {
        if !all.contains(&from) {
            all.push(from);
        }
    }
    all
}

/// Throws away working-tree changes to the given paths. Untracked files are left
/// alone: deleting a file the user never committed is not something to do as a
/// side effect of "discard changes".
pub fn discard(state: &AppState, paths: &[String]) -> Result<String, String> {
    let root = state.path()?;
    // Undoing a move that was never staged means putting the file back where it
    // was — that half is a deletion the index can restore. The copy at the new
    // name is untracked, and `git restore` has nothing to say about it; it is
    // left where it is, to be deleted deliberately or not at all, which is the
    // same promise discarding makes about every other untracked file.
    let moved = moves(state, paths, Side::Unstaged);
    let mut aimed: Vec<String> = paths
        .iter()
        .filter(|path| !moved.iter().any(|(to, _)| &to == path))
        .cloned()
        .collect();
    for (_, from) in moved {
        if !aimed.contains(&from) {
            aimed.push(from);
        }
    }
    if aimed.is_empty() {
        return Ok(String::new());
    }
    let pathspecs = literal_pathspecs(&aimed);
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
        return Err(check
            .reason
            .unwrap_or_else(|| "Cannot reword that commit".to_string()));
    }

    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    let (summary, _) = split_message(message);

    git_cmd::run_checked(
        &root,
        &[
            "commit",
            "--amend",
            "--only",
            "--allow-empty",
            "-m",
            message,
        ],
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
    // A pop that conflicts does not drop the entry, so this is as undoable as
    // an apply — and worth remembering for the same reason.
    let said = git_cmd::run_checked(&root, &["stash", "pop", &format!("stash@{{{at}}}")]);
    remember_applied(&root, &oid, said.is_err());
    // Only a pop that stopped: one that went through took the entry off the
    // list with it, and putting the files back would leave the work nowhere.
    if said.is_err() && has_unmerged(&root) {
        record_applied(state, &oid, true);
    }
    said
}

/// What the files a stash touched look like right now, one line each.
///
/// Taken the moment an apply lands, and again when its undo is asked for. An
/// undo puts every one of those paths back to what the branch has; if any of
/// them has moved since, that is work done after the apply and putting it back
/// would throw the work away rather than the stash.
///
/// `None` when it cannot be read — a fingerprint nobody can compare is not a
/// reason to refuse an undo, only a reason not to promise the check.
fn fingerprint(root: &Path, paths: &BTreeSet<String>) -> Option<String> {
    let mut lines = Vec::new();
    for path in paths {
        let full = root.join(path);
        let mark = if full.exists() {
            // A filename, not a pathspec: `hash-object` reads the file itself,
            // so the literal magic other commands need would be read as part
            // of the name. `--` is what keeps a leading dash out of trouble.
            git_cmd::run_checked(root, &["hash-object", "--", path])
                .ok()?
                .trim()
                .to_string()
        } else {
            // Gone is a state like any other: the apply may have deleted it.
            "absent".to_string()
        };
        lines.push(format!("{mark} {path}"));
    }
    Some(lines.join("\n"))
}

/// The first path whose content is not what the apply left, if any.
fn moved_since(root: &Path, paths: &BTreeSet<String>, taken: &str) -> Option<String> {
    let now = fingerprint(root, paths)?;
    if now == taken {
        return None;
    }
    let then: HashMap<&str, &str> = taken
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(mark, path)| (path, mark))
        .collect();
    now.lines()
        .filter_map(|line| line.split_once(' '))
        .find(|(mark, path)| then.get(path) != Some(mark))
        .map(|(_, path)| path.to_string())
}

/// Puts an apply on the undo stack, named by the stash it came from.
///
/// The top-right undo is where every other step back lives, and a stash that
/// went on badly is the one people reach for it hardest.
///
/// A stash that went on cleanly is remembered with a fingerprint of what it
/// left: undo it straight away and the files go back, but once they have been
/// worked on the step refuses rather than taking that work with it. One that
/// stopped on a conflict carries none — a half-merged file is git's mess, not
/// anybody's work, and backing out of it has to stay possible however much
/// resolving has been half-done.
fn record_applied(state: &AppState, oid: &str, conflicted: bool) {
    let label = stash_list(state)
        .ok()
        .and_then(|listed| {
            listed
                .into_iter()
                .find(|entry| entry.oid == oid)
                .map(|entry| entry.message)
        })
        .unwrap_or_else(|| "a stash".to_string());
    let taken = (!conflicted)
        .then(|| {
            let root = state.path().ok()?;
            let touched = stash_touched_paths(&root, oid).ok()?;
            fingerprint(&root, &touched)
        })
        .flatten();
    journal::record(
        state,
        "stash-apply",
        format!("Apply: {label}"),
        journal::current_branch(state),
        taken,
        Some(oid.to_string()),
        Mode::Apply,
        true,
    );
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
        &["stash", "list", "--format=%gd%x00%H%x00%gs%x00%at"],
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
        &[
            "stash",
            "show",
            "--name-only",
            &format!("stash@{{{index}}}"),
        ],
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
    let text =
        fs::read_to_string(&logs).map_err(|e| format!("Could not read the stash log: {e}"))?;

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
    let said = git_cmd::run_checked(&root, &["stash", "apply", &oid]);
    remember_applied(&root, &oid, said.is_err());
    let conflicted = said.is_err() && has_unmerged(&root);
    // An apply that git refused outright changed nothing, and there is
    // nothing to step back out of.
    if said.is_ok() || conflicted {
        record_applied(state, &oid, conflicted);
    }
    said?;
    Ok(format!("Applied {name}"))
}

/// Where the stash a conflicted apply came from is written down.
///
/// In git's own directory rather than in memory: the conflict outlives the
/// window, and the way back out has to outlive it too.
fn applied_marker(root: &Path) -> Option<std::path::PathBuf> {
    crate::remote::git_dir(root)
        .ok()
        .map(|dir| dir.join("gitnoob-applied-stash"))
}

/// Notes which stash left the tree in this state, or clears the note.
///
/// Only a run that stopped is worth remembering: one that went on cleanly has
/// nothing to undo here — the changes are ordinary working-tree changes now,
/// and discarding them is what the file menu is for.
fn remember_applied(root: &Path, oid: &str, conflicted: bool) {
    let Some(marker) = applied_marker(root) else {
        return;
    };
    if conflicted && has_unmerged(root) {
        let _ = std::fs::write(&marker, format!("{oid}\n"));
    } else {
        let _ = std::fs::remove_file(&marker);
    }
}

pub(crate) fn has_unmerged(root: &Path) -> bool {
    git_cmd::run_checked(root, &["ls-files", "--unmerged"])
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false)
}

/// The stash a conflicted apply or pop came from, while its mess is still here.
///
/// The note is cleared as soon as nothing is conflicted any more, so a stale
/// one from an argument that was settled long ago cannot offer to undo work
/// that has since been done by hand.
pub fn applied_stash(root: &Path) -> Option<String> {
    let marker = applied_marker(root)?;
    let oid = std::fs::read_to_string(&marker).ok()?.trim().to_string();
    if !has_unmerged(root) {
        let _ = std::fs::remove_file(&marker);
        return None;
    }
    // The stash itself has to still be there: putting the tree back is only
    // half of it, and the point of undoing is that the work is still on the
    // list afterwards.
    if oid.is_empty() || git_cmd::run_checked(root, &["cat-file", "-e", &oid]).is_err() {
        return None;
    }
    Some(oid)
}

/// Takes a conflicted stash apply back off, leaving the stash where it was.
///
/// A stash that would not go on is a dead end: the files are half from the
/// branch and half from the stash, and git offers nothing to step back out of
/// it. This puts every path the stash touched back to what the branch has, so
/// the tree reads exactly as it did before the apply — and the stash, which
/// both `apply` and a conflicted `pop` leave alone, is still on the list.
pub fn undo_stash_apply(state: &AppState) -> Result<String, String> {
    let root = state.path()?;
    let oid =
        applied_stash(&root).ok_or_else(|| "There is no stash apply to undo here".to_string())?;
    // The marker is only written for an apply that stopped, which is backed
    // out of whatever state the resolving got to.
    undo_applied(state, &oid, None)
}

/// Takes a named stash back off the tree. See [`undo_stash_apply`].
///
/// `taken` is the fingerprint from when the apply landed, for one that landed
/// cleanly; `None` for one that stopped on a conflict, which is always backed
/// out of. See [`record_applied`].
pub fn undo_applied(state: &AppState, oid: &str, taken: Option<&str>) -> Result<String, String> {
    let root = state.path()?;

    // Only the paths the stash brought in. A reset of the whole tree would be
    // simpler and would take everything else standing in it — a file edited
    // beside the apply, work staged before it — with no way to tell any of it
    // apart afterwards. Git refuses to apply a stash onto a file that is
    // already dirty, so every path here is one the apply itself wrote.
    let touched = stash_touched_paths(&root, oid)?;
    if touched.is_empty() {
        return Err("That stash touched nothing, so there is nothing to take back off".to_string());
    }

    // Work done on those files since the apply is work this would throw away,
    // and unlike the stash there is nowhere it is kept.
    if let Some(taken) = taken {
        if let Some(path) = moved_since(&root, &touched, taken) {
            return Err(format!(
                "{path} has been worked on since the stash went on, so taking it back off would \
                 throw that away. Discard it yourself if that is what you meant."
            ));
        }
    }

    let mut put_back = 0usize;
    for path in &touched {
        // Literal, so a stashed file whose name reads as a glob — `a*.txt`,
        // `f[1].js` — takes only itself back off.
        let spec = format!(":(literal){path}");
        if in_head(&root, path) {
            // Index and working tree together: an apply that stopped leaves
            // both sides of the conflict in the index, and only the committed
            // copy makes the path whole again.
            git_cmd::run_checked(
                &root,
                &[
                    "restore",
                    "--source=HEAD",
                    "--staged",
                    "--worktree",
                    "--",
                    &spec,
                ],
            )?;
        } else if tracked(&root, path) {
            // In the index but never committed — the stash was adding it.
            git_cmd::run_checked(&root, &["rm", "-f", "--quiet", "--", &spec])?;
        } else if root.join(path).exists() {
            // Only on disk: something the stash carried as an untracked file.
            git_cmd::run_checked(&root, &["clean", "-f", "--", &spec])?;
        } else {
            continue;
        }
        put_back += 1;
    }

    if let Some(marker) = applied_marker(&root) {
        let _ = std::fs::remove_file(marker);
    }
    Ok(format!(
        "Put {put_back} {} back — the stash is still on the list",
        if put_back == 1 { "file" } else { "files" }
    ))
}

/// Whether the commit that is checked out has this path in it.
fn in_head(root: &Path, path: &str) -> bool {
    git_cmd::run_checked(root, &["cat-file", "-e", &format!("HEAD:{path}")]).is_ok()
}

/// Whether the index knows this path at all, committed or not.
fn tracked(root: &Path, path: &str) -> bool {
    let spec = format!(":(literal){path}");
    git_cmd::run_checked(root, &["ls-files", "--error-unmatch", "--", &spec]).is_ok()
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

/// What a run over several stashes did.
#[derive(Serialize, Debug)]
pub struct StashRun {
    /// The ones that went on, oldest first, as they went on.
    pub applied: Vec<String>,
    /// The one that stopped the run, when one did.
    pub stopped: Option<StashStop>,
    /// Files left conflicted by the stash that stopped it.
    pub conflicted: Vec<String>,
}

#[derive(Serialize, Debug)]
pub struct StashStop {
    pub message: String,
    /// Git's own words about why it would not go on.
    pub reason: String,
}

/// Applies several stashes one after another, and optionally drops each one
/// that went on cleanly.
///
/// Oldest first, so the newest ends up on top — which is the order they were
/// made in and so the order they were meant to be replayed in. Uncommitted
/// work is not in the way of this: `git stash apply` merges into the working
/// tree, and only stops when two changes actually meet.
///
/// The whole list is resolved to commit ids before anything is applied.
/// Dropping renumbers every entry below it, so acting on positions read at the
/// start would act on the wrong stashes from the second one onwards.
pub fn stash_apply_many(
    state: &AppState,
    indexes: Vec<usize>,
    drop_after: bool,
) -> Result<StashRun, String> {
    let root = state.path()?;
    let listed = stash_list(state)?;

    let mut picked: Vec<(usize, String, String)> = Vec::new();
    for index in indexes {
        let found = listed
            .iter()
            .find(|entry| entry.index == index)
            .ok_or_else(|| format!("There is no stash at position {index}"))?;
        picked.push((index, found.oid.clone(), found.message.clone()));
    }
    if picked.is_empty() {
        return Err("No stashes were picked".to_string());
    }
    // Highest index first: that is the oldest, and it goes on first.
    picked.sort_by_key(|(index, _, _)| std::cmp::Reverse(*index));

    let mut run = StashRun {
        applied: Vec::new(),
        stopped: None,
        conflicted: Vec::new(),
    };

    for (_, oid, message) in picked {
        let out = git_cmd::run(&root, &["stash", "apply", &oid])?;
        if !out.ok {
            let reason = if out.stderr.trim().is_empty() {
                out.stdout.trim().to_string()
            } else {
                out.stderr.trim().to_string()
            };
            // One that stopped is on the undo stack too: the run is over, and
            // this is the entry whose mess is in the tree.
            if has_unmerged(&root) {
                remember_applied(&root, &oid, true);
                record_applied(state, &oid, true);
            }
            run.stopped = Some(StashStop { message, reason });
            run.conflicted = unmerged_paths(&root);
            break;
        }
        run.applied.push(message);
        // Each one steps back on its own, newest first — the same order they
        // went on, reversed. A dropped stash has nowhere to go back to, so a
        // pop of several records nothing.
        if drop_after {
            // Re-resolved from the commit id: every drop renumbers the rest.
            if let Some(at) = journal::stash_index(state, &oid) {
                git_cmd::run_checked(&root, &["stash", "drop", &format!("stash@{{{at}}}")])?;
            }
        } else {
            record_applied(state, &oid, false);
        }
    }

    Ok(run)
}

/// The files a stopped apply left with both sides in them.
fn unmerged_paths(root: &std::path::Path) -> Vec<String> {
    let Ok(out) = git_cmd::run(root, &["diff", "--name-only", "--diff-filter=U"]) else {
        return Vec::new();
    };
    if !out.ok {
        return Vec::new();
    }
    out.stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Turns a stash into a branch, which is the safe way out of a stash that no
/// longer applies cleanly to the current branch.
pub fn stash_branch(state: &AppState, index: usize, name: &str) -> Result<String, String> {
    let root = state.path()?;
    let selector = format!("stash@{{{index}}}");
    // A branch fetched as `origin/-f` is real, and without this a name
    // beginning with `-` is parsed as a flag rather than the branch to create.
    git_cmd::run_checked(
        &root,
        &["stash", "branch", "--end-of-options", name, &selector],
    )?;
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
/// Sorted, so a fingerprint taken now and one taken later line up, and so the
/// files go back in the same order every time.
fn stash_touched_paths(root: &Path, selector: &str) -> Result<BTreeSet<String>, String> {
    let raw = git_cmd::run_checked(
        root,
        &[
            "stash",
            "show",
            "--include-untracked",
            "--name-only",
            selector,
        ],
    )?;
    Ok(raw
        .lines()
        .map(str::to_string)
        .filter(|l| !l.is_empty())
        .collect())
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
        &[
            "log",
            "--format=%H%x00%h%x00%s%x00%an%x00%at",
            &format!("{oid}..HEAD"),
        ],
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
    Ok(
        git_cmd::run_checked(&root, &["log", "-1", "--format=%B", oid])?
            .trim_end()
            .to_string(),
    )
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
        (
            "open",
            vec!["-R".into(), target.to_string_lossy().into_owned()],
        )
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

/// The lines of a hunk the user picked out, named by their line numbers.
///
/// Numbers rather than positions in the list: the window's model of a hunk and
/// the text `git diff` prints are built by two different pieces of code, and a
/// line's number is the one thing both of them agree on. A `+` line is named by
/// where it lands, a `-` line by where it was.
#[derive(serde::Deserialize, Clone, Debug, Default)]
pub struct Lines {
    pub added: Vec<u32>,
    pub removed: Vec<u32>,
}

impl Lines {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
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
    lines: Option<Lines>,
) -> Result<String, String> {
    let root = state.path()?;

    // Unstaging reads the index-versus-HEAD diff; the other two read the
    // working tree against the index.
    let pathspec = format!(":(literal){path}");
    let diff = if action == HunkAction::Unstage {
        git_cmd::run_checked(
            &root,
            &[
                "diff",
                "--cached",
                "--no-color",
                "--unified=3",
                "--",
                &pathspec,
            ],
        )?
    } else {
        git_cmd::run_checked(
            &root,
            &["diff", "--no-color", "--unified=3", "--", &pathspec],
        )?
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

    // Discarding and unstaging are applied backwards, which swaps what an
    // unpicked line has to become — see `partial_hunk_patch`.
    let reverse = action != HunkAction::Stage;
    let picked = lines.filter(|lines| !lines.is_empty());
    let count = picked
        .as_ref()
        .map(|lines| lines.added.len() + lines.removed.len());
    let patch = match &picked {
        Some(lines) => partial_hunk_patch(&diff, hunk_index, lines, reverse)?,
        None => single_hunk_patch(&diff, hunk_index)?,
    };

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

    let what = match count {
        Some(1) => "one line".to_string(),
        Some(many) => format!("{many} lines"),
        None => "one hunk".to_string(),
    };
    Ok(match action {
        HunkAction::Stage => format!("Staged {what} of {path}"),
        HunkAction::Unstage => format!("Unstaged {what} of {path}"),
        HunkAction::Discard => format!("Discarded {what} of {path}"),
    })
}

/// Splits a diff into its header and its hunks, as text.
fn split_hunks(diff: &str) -> (Vec<&str>, Vec<Vec<&str>>) {
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
    (header, hunks)
}

/// The two starting line numbers out of an `@@ -a,b +c,d @@` header.
fn header_starts(header: &str) -> Option<(u32, u32)> {
    let rest = header.strip_prefix("@@")?;
    let inner = rest.split("@@").next()?;
    let mut old = None;
    let mut new = None;
    for part in inner.split_whitespace() {
        let (sign, digits) = part.split_at(1);
        let first = digits.split(',').next()?;
        let value: u32 = first.parse().ok()?;
        match sign {
            "-" => old = Some(value),
            "+" => new = Some(value),
            _ => {}
        }
    }
    Some((old?, new?))
}

/// Rebuilds one hunk holding only the lines that were picked.
///
/// This is what `git add -p` does when you answer its questions one line at a
/// time. A line that was not picked has to become something the patch can still
/// apply around:
///
/// - forwards, an unpicked `+` never happened, so it goes; an unpicked `-` is
///   still in the file, so it becomes context.
/// - backwards — unstaging, or discarding — the patch is applied in reverse, so
///   the two swap: an unpicked `+` becomes context and an unpicked `-` goes.
///
/// The counts in the `@@` header are then whatever survived, which is why they
/// are recomputed rather than carried over.
fn partial_hunk_patch(
    diff: &str,
    hunk_index: usize,
    lines: &Lines,
    reverse: bool,
) -> Result<String, String> {
    let (header, hunks) = split_hunks(diff);
    let hunk = hunks.get(hunk_index).ok_or_else(|| missing_hunk(&hunks))?;
    let head = hunk.first().ok_or("That hunk is empty")?;
    let (old_start, new_start) =
        header_starts(head).ok_or_else(|| format!("Could not read the hunk header: {head}"))?;

    let mut body: Vec<String> = Vec::new();
    let mut old_at = old_start;
    let mut new_at = new_start;
    let mut old_count = 0u32;
    let mut new_count = 0u32;
    // Whether the line just written was kept, so the "no newline" remark that
    // belongs to it can follow it or go with it.
    let mut kept_last = true;

    for line in hunk.iter().skip(1) {
        let mut chars = line.chars();
        match chars.next() {
            Some('+') => {
                let picked = lines.added.contains(&new_at);
                new_at += 1;
                if picked {
                    body.push((*line).to_string());
                    new_count += 1;
                    kept_last = true;
                } else if reverse {
                    // Still in the index; it has to be there for the reverse
                    // patch to line up, as context.
                    body.push(format!(" {}", chars.as_str()));
                    old_count += 1;
                    new_count += 1;
                    kept_last = true;
                } else {
                    kept_last = false;
                }
            }
            Some('-') => {
                let picked = lines.removed.contains(&old_at);
                old_at += 1;
                if picked {
                    body.push((*line).to_string());
                    old_count += 1;
                    kept_last = true;
                } else if reverse {
                    kept_last = false;
                } else {
                    // Not being taken out after all, so it stays as context.
                    body.push(format!(" {}", chars.as_str()));
                    old_count += 1;
                    new_count += 1;
                    kept_last = true;
                }
            }
            Some('\\') => {
                // "No newline at end of file" belongs to the line above it.
                if kept_last {
                    body.push((*line).to_string());
                }
            }
            // Context, and the empty line git writes for a blank context line.
            _ => {
                body.push((*line).to_string());
                old_at += 1;
                new_at += 1;
                old_count += 1;
                new_count += 1;
                kept_last = true;
            }
        }
    }

    if old_count == new_count && !body.iter().any(|line| line.starts_with(['+', '-'])) {
        return Err("None of those lines are changes".to_string());
    }

    let mut patch = header.join("\n");
    patch.push('\n');
    patch.push_str(&format!(
        "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
    ));
    patch.push_str(&body.join("\n"));
    // `git apply` insists on a trailing newline.
    patch.push('\n');
    Ok(patch)
}

fn missing_hunk(hunks: &[Vec<&str>]) -> String {
    format!(
        "That hunk is no longer there — the file has {} {}",
        hunks.len(),
        if hunks.len() == 1 { "hunk" } else { "hunks" }
    )
}

/// Rebuilds a patch containing the file header and exactly one of its hunks.
fn single_hunk_patch(diff: &str, hunk_index: usize) -> Result<String, String> {
    let (header, hunks) = split_hunks(diff);
    let hunk = hunks.get(hunk_index).ok_or_else(|| missing_hunk(&hunks))?;

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

    /// A file with one hunk holding two added lines and one removed one.
    const HUNK_DIFF: &str = concat!(
        "diff --git a/a.txt b/a.txt\n",
        "index 111..222 100644\n",
        "--- a/a.txt\n",
        "+++ b/a.txt\n",
        "@@ -1,4 +1,5 @@\n",
        " one\n",
        "-two\n",
        "+TWO\n",
        "+two and a half\n",
        " three\n",
        " four\n"
    );

    fn body(patch: &str) -> Vec<&str> {
        patch
            .lines()
            .skip_while(|line| !line.starts_with("@@"))
            .collect()
    }

    #[test]
    fn staging_one_added_line_leaves_the_rest_as_they_were() {
        let lines = Lines {
            added: vec![2],
            removed: vec![],
        };
        let patch = partial_hunk_patch(HUNK_DIFF, 0, &lines, false).unwrap();
        assert_eq!(
            body(&patch),
            vec![
                "@@ -1,4 +1,5 @@",
                " one",
                // Not picked, and still in the file, so it becomes context.
                " two",
                "+TWO",
                " three",
                " four",
            ]
        );
    }

    #[test]
    fn staging_only_the_removal_drops_the_additions() {
        let lines = Lines {
            added: vec![],
            removed: vec![2],
        };
        let patch = partial_hunk_patch(HUNK_DIFF, 0, &lines, false).unwrap();
        assert_eq!(
            body(&patch),
            vec!["@@ -1,4 +1,3 @@", " one", "-two", " three", " four"]
        );
    }

    #[test]
    fn unstaging_keeps_what_it_is_not_taking_back() {
        // Applied in reverse, so an unpicked `+` has to stay as context and an
        // unpicked `-` goes: the mirror of the forward case.
        let lines = Lines {
            added: vec![3],
            removed: vec![],
        };
        let patch = partial_hunk_patch(HUNK_DIFF, 0, &lines, true).unwrap();
        assert_eq!(
            body(&patch),
            vec![
                "@@ -1,4 +1,5 @@",
                " one",
                " TWO",
                "+two and a half",
                " three",
                " four",
            ]
        );
    }

    #[test]
    fn the_counts_in_the_header_are_what_survived() {
        let all = Lines {
            added: vec![2, 3],
            removed: vec![2],
        };
        let patch = partial_hunk_patch(HUNK_DIFF, 0, &all, false).unwrap();
        // Everything picked is the whole hunk again.
        assert!(patch.contains("@@ -1,4 +1,5 @@"));
        assert_eq!(
            body(&patch),
            body(&single_hunk_patch(HUNK_DIFF, 0).unwrap())
        );
    }

    #[test]
    fn picking_nothing_that_is_a_change_is_refused() {
        let lines = Lines {
            added: vec![99],
            removed: vec![99],
        };
        assert!(partial_hunk_patch(HUNK_DIFF, 0, &lines, false).is_err());
    }

    #[test]
    fn a_no_newline_remark_goes_with_the_line_it_belongs_to() {
        const TAIL: &str = concat!(
            "--- a/a.txt\n",
            "+++ b/a.txt\n",
            "@@ -1,2 +1,2 @@\n",
            " one\n",
            "-two\n",
            "\\ No newline at end of file\n",
            "+two\n"
        );
        // The removal is not picked, so going forwards it becomes context and
        // the remark that belongs to it stays with it.
        let kept = partial_hunk_patch(
            TAIL,
            0,
            &Lines {
                added: vec![2],
                removed: vec![],
            },
            false,
        )
        .unwrap();
        assert!(kept.contains("\\ No newline at end of file"));

        // Reversing drops that line, and the remark has nothing left to
        // belong to.
        let dropped = partial_hunk_patch(
            TAIL,
            0,
            &Lines {
                added: vec![],
                removed: vec![],
            },
            true,
        );
        assert!(dropped.is_err() || !dropped.unwrap().contains("No newline"));
    }

    #[test]
    fn reads_the_starts_out_of_a_header() {
        assert_eq!(header_starts("@@ -1,4 +1,5 @@"), Some((1, 1)));
        assert_eq!(header_starts("@@ -12 +34 @@ fn thing()"), Some((12, 34)));
        assert_eq!(header_starts("not a header"), None);
    }

    /// A line as git writes it into `.git/logs/refs/stash`.
    const ENTRY: &str = "0000000000000000000000000000000000000000 abc123 A <a@b.c> 1700000000 +0100\tOn main: the old name";

    #[test]
    fn renaming_keeps_the_branch_the_stash_was_made_on() {
        let out = renamed_entry(ENTRY, "abc123", "the new name").expect("renamed");
        assert!(out.ends_with("\tOn main: the new name"));
        // Everything before the message is the reflog's own bookkeeping and is
        // left exactly as it was.
        assert_eq!(out.split('\t').next(), ENTRY.split('\t').next());
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
        assert_eq!(
            stashed_on("stash@{0}: On main: gitnoob auto-stash: pulling"),
            None
        );
        assert_eq!(
            stashed_on("stash@{0}: WIP on main: 1234567 something"),
            None
        );
    }
}
