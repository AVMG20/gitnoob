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
    stage_all_expecting(state, None)
}

/// Stages everything, after checking the index against what the caller saw.
///
/// `Some(false)` is "stage everything and commit" from a list that showed
/// nothing staged. What is staged by the time it runs — a `git add -p` in a
/// terminal a moment ago — was picked by hand, and sweeping the rest in on
/// top of it would commit it with everything else, unseen. So that refuses.
pub fn stage_all_expecting(
    state: &AppState,
    expect_staged: Option<bool>,
) -> Result<String, String> {
    let root = state.path()?;
    if expect_staged == Some(false) && index_differs_from_head(&root)? {
        return Err(
            "Something was staged since the list was read, so nothing more was staged. Look at \
             what is staged and commit again."
                .to_string(),
        );
    }
    git_cmd::run_checked(&root, &["add", "--all"])
}

/// Whether anything is staged: the index differs from HEAD's tree, or holds
/// anything at all on a branch with no commits yet.
fn index_differs_from_head(root: &Path) -> Result<bool, String> {
    let repo = git2::Repository::open(root).map_err(|e| e.message().to_string())?;
    let tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
    let index = repo.index().map_err(|e| e.message().to_string())?;
    let diff = repo
        .diff_tree_to_index(tree.as_ref(), Some(&index), None)
        .map_err(|e| e.message().to_string())?;
    Ok(diff.deltas().len() > 0)
}

pub fn unstage(state: &AppState, paths: &[String]) -> Result<String, String> {
    let root = state.path()?;
    // Same as staging, the other way round: taking back only the name the row
    // carries leaves the file both moved and deleted in the index.
    let both = with_both_halves(state, paths, Side::Staged);
    let pathspecs = literal_pathspecs(&both);
    // Before the first commit there is no HEAD for `restore --staged` to take
    // the index back to, and it fails with "could not resolve HEAD". Taking a
    // file out of the index is all unstaging means there, and `rm --cached`
    // does exactly that and nothing to the file on disk. `-f` because rm
    // otherwise refuses a file whose staged copy differs from the one on disk
    // — which is the ordinary state of a file edited after it was staged, and
    // the copy on disk is the one that stays.
    let mut args = if unborn(state) {
        vec!["rm", "--cached", "-r", "-f", "--quiet", "--"]
    } else {
        vec!["restore", "--staged", "--"]
    };
    args.extend(pathspecs.iter().map(String::as_str));
    git_cmd::run_checked(&root, &args)
}

/// Whether HEAD names a branch that has no commits yet.
pub(crate) fn unborn(state: &AppState) -> bool {
    let Ok(repo) = state.repo() else {
        return false;
    };
    let unborn = matches!(repo.head(), Err(e) if e.code() == git2::ErrorCode::UnbornBranch);
    unborn
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
    // The full status pays for rename detection across the whole tree; a path
    // that is plainly an edit — the common case by far — is answered by one
    // cheap lookup instead. Only a path that reads as new to its side can be
    // the arriving half of a move, because that is what the half looks like
    // until the scan pairs it with its departure.
    if !could_be_moved(state, paths, side) {
        return Vec::new();
    }
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

/// Whether any of `paths` could be half of a move, without a tree-wide scan.
fn could_be_moved(state: &AppState, paths: &[String], side: Side) -> bool {
    let Ok(repo) = state.repo() else {
        // Not knowing is not a reason to skip the careful path.
        return true;
    };
    paths
        .iter()
        .any(|path| match repo.status_file(std::path::Path::new(path)) {
            Ok(flags) => match side {
                Side::Staged => {
                    flags.intersects(git2::Status::INDEX_NEW | git2::Status::INDEX_RENAMED)
                }
                Side::Unstaged => flags.intersects(git2::Status::WT_NEW | git2::Status::WT_RENAMED),
            },
            Err(_) => true,
        })
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
    let label = match aimed.as_slice() {
        [one] => format!("Discard changes to {one}"),
        many => format!("Discard changes to {} files", many.len()),
    };
    throw_away_saved(state, &root, &aimed, "discard", label, || {
        let pathspecs = literal_pathspecs(&aimed);
        let mut args = vec!["restore", "--worktree", "--"];
        args.extend(pathspecs.iter().map(String::as_str));
        git_cmd::run_checked(&root, &args)
    })
}

/// Throws files away with what they held saved first, and puts the throwing
/// away on the undo history.
///
/// Every file in `paths` is written into the object database before `act`
/// runs, and looked at again after. Whatever changed between the two is the
/// step: undoing it writes the saved bytes back, as long as nothing has
/// touched those files since. Recorded even when `act` fails, since a command
/// that stopped half-way may still have thrown some of them away.
fn throw_away_saved<T>(
    state: &AppState,
    root: &Path,
    paths: &[String],
    kind: &str,
    label: String,
    act: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let before = crate::recover::look(root, paths, true)?;
    let said = act();
    if let Ok(after) = crate::recover::look(root, paths, true) {
        let mut entry = journal::Entry::new(kind, label, Mode::Files, true);
        entry.files = crate::recover::changes(before, after);
        journal::record_entry_at(state, root, entry);
    }
    said
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
    // A folder full of untracked files is one row; every file in it is saved.
    let files = crate::recover::untracked_under(&root, paths)?;
    let label = match files.as_slice() {
        [one] => format!("Delete {one}"),
        many => format!("Delete {} untracked files", many.len()),
    };
    throw_away_saved(state, &root, &files, "delete-untracked", label, || {
        let pathspecs = literal_pathspecs(paths);
        let mut args = vec!["clean", "-f", "-d", "--"];
        args.extend(pathspecs.iter().map(String::as_str));
        git_cmd::run_checked(&root, &args)
    })?;
    Ok(format!(
        "Deleted {} {}",
        paths.len(),
        if paths.len() == 1 { "file" } else { "files" }
    ))
}

pub fn commit(state: &AppState, message: &str, amend: bool) -> Result<String, String> {
    commit_with(state, &state.path()?, message, amend, false)
}

/// A commit, with `--no-verify` when the user asked for the hooks to be
/// skipped. Only ever on an explicit click: a hook is the repository's own
/// rule, and stepping round it is not something to do on anyone's behalf.
fn commit_with(
    state: &AppState,
    root: &Path,
    message: &str,
    amend: bool,
    no_verify: bool,
) -> Result<String, String> {
    let root = root.to_path_buf();
    if message.trim().is_empty() {
        return Err("A commit needs a message".to_string());
    }
    // Everything below reads `root`, never the state's path again: a hook can
    // take long enough for the window to have moved to another repository,
    // and this commit's undo step and its trip home belong to this one.
    let before = journal::head_at(&root);
    let branch = journal::branch_at(&root);

    // Read now: whether this commit is the one that concludes a paused visit's
    // merge depends on what git was in the middle of before it.
    let leg = crate::remote::trip_at(&root);
    // The same for the history: a commit that finishes a merge is the end of
    // that merge, and a commit in the middle of a rebase is part of it.
    let running = journal::running_at(&root);
    let waiting = journal::pending_at(&root);

    // A commit that concludes a merge, pick or revert is the moment git
    // records how its conflicts were resolved, so rerere has to be on for it
    // as it was for the command that stopped.
    let concluding = crate::remote::git_dir(&root).is_ok_and(|dir| {
        ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"]
            .iter()
            .any(|name| dir.join(name).exists())
    });
    let rerere = if concluding {
        crate::rebase::rerere_flags(state)
    } else {
        Vec::new()
    };
    let mut args: Vec<&str> = rerere.iter().map(String::as_str).collect();
    args.extend(["commit", "-m", message]);
    if amend {
        args.push("--amend");
    }
    if no_verify {
        args.push("--no-verify");
    }
    let output = git_cmd::run_checked(&root, &args)?;

    let summary = message.lines().next().unwrap_or("").trim().to_string();
    match running {
        // A merge finished by hand is still a merge. Recorded as a commit, its
        // undo was a soft reset that left the whole merged branch staged as
        // if it were your own work — and the next commit squashed it in and
        // lost the second parent. As a merge it is undone the way any merge
        // is: back to the commit before it, other open work kept.
        Some("merge") if !amend => match waiting {
            Some(waiting) => journal::finish_pending_at(state, &root, Some(waiting)),
            None => {
                let mut entry =
                    journal::Entry::new("merge", format!("Merge: {summary}"), Mode::Hard, true);
                entry.branch = branch;
                entry.before = before;
                entry.after = journal::head_at(&root);
                journal::record_entry_at(state, &root, entry);
            }
        },
        // The rebase records itself, whole, when it finishes.
        Some("rebase") => {}
        // Undo is a soft reset back to `before`: for an amend that is the
        // commit that was replaced, which still exists as an object.
        _ => {
            let mut entry = journal::Entry::new(
                if amend { "amend" } else { "commit" },
                format!("{}: {summary}", if amend { "Amend" } else { "Commit" }),
                Mode::Soft,
                false,
            );
            entry.branch = branch;
            entry.before = before;
            entry.after = journal::head_at(&root);
            journal::record_entry_at(state, &root, entry);
        }
    }
    // The commit that concludes a merge made on another branch's behalf is
    // also the end of that trip.
    Ok(crate::remote::and_home_at(&root, leg, output))
}

/// How a commit went, told in enough detail to offer a way on when it did not.
#[derive(Serialize, Debug)]
pub struct CommitAttempt {
    pub ok: bool,
    /// Git's answer on success; everything git and the hooks printed otherwise.
    pub output: String,
    /// Set when the commit was stopped and a hook is what stopped it.
    pub hook: Option<HookStop>,
}

#[derive(Serialize, Debug, Default)]
pub struct HookStop {
    /// The hooks this repository runs on a commit, by name.
    pub hooks: Vec<String>,
    /// Staged files the hook rewrote on disk. Staging them again picks up
    /// exactly what the hook did, because nothing else was unstaged in them.
    pub changed: Vec<String>,
    /// Staged files the hook rewrote that already had unstaged edits of the
    /// user's own. Staging those would sweep the edits into the commit too,
    /// so they are named and left alone.
    pub mixed: Vec<String>,
}

/// Commits, and when that fails, says whether a hook stopped it and what the
/// hook did to the files on its way out.
///
/// Formatters run from a pre-commit hook rewrite the files they object to and
/// then fail, leaving the fixed copy on disk and the unfixed one staged. The
/// way on is to stage what the hook wrote — but only where that is all there
/// is to stage, which is why every staged file is fingerprinted before the
/// commit runs and compared after.
pub fn commit_attempt(
    state: &AppState,
    message: &str,
    amend: bool,
    no_verify: bool,
) -> Result<CommitAttempt, String> {
    let root = state.path()?;
    let status = crate::refs::status(state)?;
    let staged: Vec<String> = status
        .staged
        .iter()
        .filter(|entry| entry.kind != "deleted")
        .map(|entry| entry.path.clone())
        .collect();
    let dirty: BTreeSet<&str> = status.unstaged.iter().map(|e| e.path.as_str()).collect();
    let before: Vec<Option<git2::Oid>> = staged.iter().map(|p| disk_oid(&root, p)).collect();

    match commit_with(state, &root, message, amend, no_verify) {
        Ok(output) => Ok(CommitAttempt {
            ok: true,
            output,
            hook: None,
        }),
        Err(output) => {
            let hooks = commit_hooks(&root, no_verify);
            let hook = (!hooks.is_empty() && !git_said_no(&output)).then(|| {
                let mut stop = HookStop {
                    hooks,
                    ..HookStop::default()
                };
                for (path, was) in staged.iter().zip(before) {
                    if disk_oid(&root, path) == was {
                        continue;
                    }
                    if dirty.contains(path.as_str()) {
                        stop.mixed.push(path.clone());
                    } else {
                        stop.changed.push(path.clone());
                    }
                }
                stop
            });
            Ok(CommitAttempt {
                ok: false,
                output,
                hook,
            })
        }
    }
}

/// The blob a file on disk would be, or `None` when there is no file.
fn disk_oid(root: &Path, path: &str) -> Option<git2::Oid> {
    let full = root.join(path);
    if !full.is_file() {
        return None;
    }
    git2::Oid::hash_file(git2::ObjectType::Blob, full).ok()
}

/// The hooks a `git commit` here would run, by name.
///
/// Asked through `--git-path`, which follows `core.hooksPath` (husky and its
/// kind live in `.husky`) and a linked worktree's shared hooks alike.
/// `--no-verify` skips two of the three; `prepare-commit-msg` runs anyway.
fn commit_hooks(root: &Path, no_verify: bool) -> Vec<String> {
    let names: &[&str] = if no_verify {
        &["prepare-commit-msg"]
    } else {
        &["pre-commit", "prepare-commit-msg", "commit-msg"]
    };
    names
        .iter()
        .filter(|name| {
            let Ok(found) =
                git_cmd::run_checked(root, &["rev-parse", "--git-path", &format!("hooks/{name}")])
            else {
                return false;
            };
            runnable(&root.join(found.trim()))
        })
        .map(|name| name.to_string())
        .collect()
}

/// Whether git would run this file as a hook. Git skips one that is not
/// executable, with a hint, rather than failing on it.
fn runnable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// Failures git reports itself, which no hook is to blame for even when the
/// repository has hooks.
fn git_said_no(output: &str) -> bool {
    const OWN: &[&str] = &[
        "nothing to commit",
        "nothing added to commit",
        "no changes added to commit",
        "Please tell me who you are",
        "unable to auto-detect email",
        "gpg failed to sign",
        "failed to write commit object",
        "index.lock",
        "You have unmerged paths",
        "Committing is not possible because you have unmerged files",
        // The app's own refusal, before git is even asked.
        "A commit needs a message",
    ];
    OWN.iter().any(|own| output.contains(own))
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
    // Read before it goes: the line it has in the list is what putting it back
    // has to recreate.
    let message = journal::stash_message_at(&root, &oid);
    let held = before_stash(&root, &oid);
    let said = git_cmd::run_checked(
        &root,
        &[
            DIFF3[0],
            DIFF3[1],
            "stash",
            "pop",
            &format!("stash@{{{at}}}"),
        ],
    );
    remember_applied(&root, &oid, said.is_err());
    if said.is_err() && has_unmerged(&root) {
        record_applied(state, &root, &oid, true, held);
    } else if said.is_ok() {
        record_popped(state, &root, &oid, message, held);
    }
    said
}

/// What the paths a stash touches hold just before it goes on: the index
/// entry and the file, each saved so it can be written back.
///
/// Undoing an apply used to put every one of those paths back to what HEAD
/// has, index and all. A stash goes on over staged work in a file — git
/// merges into it — and that undo threw the staged work away with the stash.
/// With this kept, the undo puts back exactly what was there.
pub struct BeforeStash {
    staged: Vec<journal::Staged>,
    files: Vec<(String, Option<journal::Saved>)>,
}

/// Reads [`BeforeStash`] for `oid`, or `None` when it cannot be read — the undo then
/// falls back to the branch's copy, as it always did.
pub fn before_stash(root: &Path, oid: &str) -> Option<BeforeStash> {
    let touched = stash_touched_paths(root, oid).ok()?;
    let repo = git2::Repository::open(root).ok()?;
    let index = repo.index().ok()?;
    let staged = touched
        .iter()
        .map(|path| journal::Staged {
            path: path.clone(),
            index: index
                .get_path(Path::new(path), 0)
                .map(|found| journal::IndexSaved {
                    mode: format!("{:o}", found.mode),
                    blob: found.id.to_string(),
                }),
        })
        .collect();
    let paths: Vec<String> = touched.into_iter().collect();
    let files = crate::recover::look(root, &paths, true).ok()?;
    Some(BeforeStash { staged, files })
}

/// Writes what [`before_stash`] read onto the step, with what the apply left beside
/// it so the undo can tell whether the files were worked on since.
pub fn keep_before(root: &Path, held: Option<BeforeStash>, entry: &mut journal::Entry) {
    entry.staged = Vec::new();
    entry.files = Vec::new();
    let Some(held) = held else { return };
    let paths: Vec<String> = held.files.iter().map(|(path, _)| path.clone()).collect();
    let Ok(after) = crate::recover::look(root, &paths, false) else {
        return;
    };
    let mut after: HashMap<String, Option<journal::Saved>> = after.into_iter().collect();
    entry.files = held
        .files
        .into_iter()
        .map(|(path, before)| journal::FileSave {
            after: after.remove(&path).unwrap_or(None),
            path,
            before,
        })
        .collect();
    entry.staged = held.staged;
}

/// Puts a pop that went through on the undo stack.
///
/// The entry is off the list, so undoing it is two things: the files go back
/// to what the branch has, as for an apply, and the stash goes back on the
/// list. The fingerprint is what stops the first half from taking anything
/// done to those files since.
fn record_popped(
    state: &AppState,
    root: &Path,
    oid: &str,
    message: Option<String>,
    held: Option<BeforeStash>,
) {
    let label = message
        .as_deref()
        .map(|said| split_subject(said).1)
        .unwrap_or_else(|| "a stash".to_string());
    let mut entry = journal::Entry::new("stash-pop", format!("Pop: {label}"), Mode::Pop, true);
    entry.branch = journal::branch_at(root);
    entry.before = applied_fingerprint(root, oid);
    entry.after = Some(oid.to_string());
    entry.message = message;
    entry.head = journal::head_at(root);
    keep_before(root, held, &mut entry);
    journal::record_entry_at(state, root, entry);
}

/// What the files a stash touched look like now, for an apply or pop that has
/// just gone on. See [`fingerprint`].
pub fn applied_fingerprint(root: &Path, oid: &str) -> Option<String> {
    let touched = stash_touched_paths(root, oid).ok()?;
    fingerprint(root, &touched)
}

/// Refuses if any file a stash touched has changed since `taken` was read.
pub fn applied_unchanged(root: &Path, oid: &str, taken: Option<&str>) -> Result<(), String> {
    let Some(taken) = taken else { return Ok(()) };
    let touched = stash_touched_paths(root, oid)?;
    match moved_since(root, &touched, taken) {
        Some(path) => Err(format!(
            "{path} has been worked on since the stash went on, so taking it back off would \
             throw that away. Discard it yourself if that is what you meant."
        )),
        None => Ok(()),
    }
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
fn record_applied(
    state: &AppState,
    root: &Path,
    oid: &str,
    conflicted: bool,
    held: Option<BeforeStash>,
) {
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
            let touched = stash_touched_paths(root, oid).ok()?;
            fingerprint(root, &touched)
        })
        .flatten();
    let mut entry =
        journal::Entry::new("stash-apply", format!("Apply: {label}"), Mode::Apply, true);
    entry.branch = journal::branch_at(root);
    entry.before = taken;
    entry.after = Some(oid.to_string());
    keep_before(root, held, &mut entry);
    journal::record_entry_at(state, root, entry);
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
    /// Whether somebody gave it its message. A stash taken without one is
    /// called after the commit it was taken on, and one this app took on its
    /// own is called after the reason; neither is a message worth reusing.
    pub named: bool,
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
        let named = subject.starts_with("On ") && !is_auto_stash(&subject);

        out.push(StashEntry {
            index,
            oid,
            message,
            branch,
            time,
            files,
            named,
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
    let held = before_stash(&root, &oid);
    let said = git_cmd::run_checked(&root, &[DIFF3[0], DIFF3[1], "stash", "apply", &oid]);
    remember_applied(&root, &oid, said.is_err());
    let conflicted = said.is_err() && has_unmerged(&root);
    // An apply that git refused outright changed nothing, and there is
    // nothing to step back out of.
    if said.is_ok() || conflicted {
        record_applied(state, &root, &oid, conflicted, held);
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
    // out of whatever state the resolving got to. What the paths held before
    // it went on is on that apply's step in the history, when it is there.
    let stacks = state.journal_at(&root, |journal| journal.stacks());
    let step = stacks.undo.into_iter().find(|entry| {
        matches!(entry.mode, Mode::Apply | Mode::Pop) && entry.after.as_deref() == Some(&oid)
    });
    let (staged, files) = step
        .map(|entry| (entry.staged, entry.files))
        .unwrap_or_default();
    undo_applied_at(&root, &oid, None, &staged, &files)
}

/// Takes a named stash back off the tree. See [`undo_stash_apply`].
///
/// `taken` is the fingerprint from when the apply landed, for one that landed
/// cleanly; `None` for one that stopped on a conflict, which is always backed
/// out of. See [`record_applied`].
pub fn undo_applied(state: &AppState, oid: &str, taken: Option<&str>) -> Result<String, String> {
    undo_applied_at(&state.path()?, oid, taken, &[], &[])
}

/// [`undo_applied`] in a work tree read once by the caller.
///
/// `staged` and `files` are what [`before_stash`] read before the stash went on, when
/// the step has them: those are put back exactly. A step without them, from
/// an older version, falls back to the branch's copy of each path.
pub fn undo_applied_at(
    root: &Path,
    oid: &str,
    taken: Option<&str>,
    staged: &[journal::Staged],
    files: &[journal::FileSave],
) -> Result<String, String> {
    let root = root.to_path_buf();

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

    if !staged.is_empty() {
        let put_back = put_back_held(&root, taken.is_some(), staged, files)?;
        if let Some(marker) = applied_marker(&root) {
            let _ = std::fs::remove_file(marker);
        }
        return Ok(format!(
            "Put {put_back} {} back — the stash is still on the list",
            if put_back == 1 { "file" } else { "files" }
        ));
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

/// Puts back what the index and the files held before a stash went on.
///
/// With `check`, for an apply that landed cleanly, every file has to read
/// exactly as the apply left it first; one that stopped is git's mess and is
/// backed out of whatever the resolving got to. The index goes first, entry by
/// entry — a conflicted path's three stages are cleared and the old entry put
/// in their place — and then the files are written.
fn put_back_held(
    root: &Path,
    check: bool,
    staged: &[journal::Staged],
    files: &[journal::FileSave],
) -> Result<usize, String> {
    if check {
        crate::recover::check_files(root, files, true)?;
    }
    let width = staged
        .iter()
        .find_map(|one| one.index.as_ref().map(|found| found.blob.len()))
        .unwrap_or(40);
    let none = "0".repeat(width);
    let mut input = String::new();
    for one in staged {
        // Mode 0 takes the path out of the index, every stage of it.
        input.push_str(&format!("0 {none}\t{}\0", one.path));
        if let Some(found) = &one.index {
            input.push_str(&format!("{} {}\t{}\0", found.mode, found.blob, one.path));
        }
    }
    let said = git_cmd::run_with_input(root, &["update-index", "-z", "--index-info"], &input)?;
    if !said.ok {
        return Err(format!(
            "Could not put the index back: {}",
            said.stderr.trim()
        ));
    }
    let count = crate::recover::write_files(root, files, true)?;
    // The entries were written without the files' stat data; a refresh
    // keeps every one of them from reading as modified.
    let _ = git_cmd::run(root, &["update-index", "-q", "--refresh"]);
    Ok(count.max(staged.len()))
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
    let line = journal::stash_message_at(&root, &oid);
    git_cmd::run_checked(&root, &["stash", "drop", &format!("stash@{{{at}}}")])?;

    // The commit outlives the entry: nothing points at it any more, but git
    // keeps it until a garbage collection, and `stash store` puts it back on
    // the list exactly as it read.
    let label = line
        .as_deref()
        .map(|said| split_subject(said).1)
        .unwrap_or_else(|| name.clone());
    let mut entry = journal::Entry::new(
        "stash-drop",
        format!("Drop stash: {label}"),
        Mode::Drop,
        false,
    );
    entry.after = Some(oid);
    entry.message = line;
    journal::record_entry_at(state, &root, entry);
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
        let held = before_stash(&root, &oid);
        let out = git_cmd::run(&root, &[DIFF3[0], DIFF3[1], "stash", "apply", &oid])?;
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
                record_applied(state, &root, &oid, true, held);
            }
            run.stopped = Some(StashStop { message, reason });
            run.conflicted = unmerged_paths(&root);
            break;
        }
        run.applied.push(message);
        // Each one steps back on its own, newest first — the same order they
        // went on, reversed. A popped one is put back on the list by its undo.
        if drop_after {
            let line = journal::stash_message_at(&root, &oid);
            // Re-resolved from the commit id: every drop renumbers the rest.
            if let Some(at) = journal::stash_index(state, &oid) {
                git_cmd::run_checked(&root, &["stash", "drop", &format!("stash@{{{at}}}")])?;
                record_popped(state, &root, &oid, line, held);
            }
        } else {
            record_applied(state, &root, &oid, false, held);
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
///
/// `--index` puts back what was staged as staged, so a pull does not quietly
/// unstage everything on the way through. When the index cannot be restored
/// git gives up before touching the working tree — "conflicts in index. Try
/// without --index." — and that one case is retried plain: the split is lost,
/// the work is not, and the switch is not worth refusing over it.
pub fn restore_after(state: &AppState, held: Held) -> Result<Option<String>, String> {
    if !held.stashed {
        return Ok(None);
    }
    let root = state.path()?;
    match pop_top(&root) {
        Ok(_) => Ok(Some("Local changes were stashed and put back".to_string())),
        Err(error) => Err(format!(
            "Your changes are safe in the stash, but putting them back hit a problem: {error}"
        )),
    }
}

/// Whether a `stash list` line is a stash made to carry work across an
/// operation: one of this app's own, or the one git's `--autostash` leaves on
/// the list when it could not put the work back.
pub fn is_auto_stash(line: &str) -> bool {
    line.contains(AUTO_STASH) || line.trim().ends_with(": autostash")
}

/// Whether the top stash is such a carried-work stash.
///
/// `undo_restore` puts that stash back, so without it on top there is nothing
/// to undo and offering the button only leads to a refusal.
pub fn auto_stash_on_top(root: &Path) -> bool {
    git_cmd::run_checked(root, &["stash", "list", "-1"])
        .map(|out| is_auto_stash(&out))
        .unwrap_or(false)
}

/// Pops the top stash, staged files staged where that can be done.
///
/// `--index` gives up before touching the working tree when the index cannot
/// be restored — "conflicts in index. Try without --index." — and that one
/// case is retried plain: the split is lost, the work is not.
fn pop_top(root: &Path) -> Result<String, String> {
    pop_stash(root, "stash@{0}")
}

/// Pops one stash by its `stash@{n}` selector, the same way as [`pop_top`].
pub fn pop_stash(root: &Path, selector: &str) -> Result<String, String> {
    git_cmd::run_checked(
        root,
        &[DIFF3[0], DIFF3[1], "stash", "pop", "--index", selector],
    )
    .or_else(|error| {
        if has_unmerged(root) {
            Err(error)
        } else {
            git_cmd::run_checked(root, &[DIFF3[0], DIFF3[1], "stash", "pop", selector])
        }
    })
}

/// Puts the merge base into the markers of a stash that will not go back on
/// cleanly, so the resolver can show it as a third pane the way it does for a
/// merge.
const DIFF3: [&str; 2] = ["-c", "merge.conflictStyle=diff3"];

/// Every path a stash entry touches, tracked or not.
///
/// Read from the stash commit itself: its tree against the commit it was
/// taken on, with no rename detection — a move is a deletion and an addition,
/// and both halves have to go back — plus everything in the third,
/// untracked-files parent an auto-stash always has. `git stash show
/// --name-only` used to be asked instead, and it quotes a name with anything
/// outside ASCII in it, which then matched no file, and folds a move into
/// one line naming only where it went.
/// Sorted, so a fingerprint taken now and one taken later line up, and so the
/// files go back in the same order every time.
fn stash_touched_paths(root: &Path, selector: &str) -> Result<BTreeSet<String>, String> {
    let repo = git2::Repository::open(root).map_err(|e| e.message().to_string())?;
    let stash = repo
        .revparse_single(selector)
        .and_then(|found| found.peel_to_commit())
        .map_err(|_| format!("There is no stash {selector}"))?;
    let base = stash.parent(0).map_err(|e| e.message().to_string())?;
    let base_tree = base.tree().map_err(|e| e.message().to_string())?;
    let tree = stash.tree().map_err(|e| e.message().to_string())?;
    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&tree), None)
        .map_err(|e| e.message().to_string())?;
    let mut paths = BTreeSet::new();
    for delta in diff.deltas() {
        for file in [delta.old_file(), delta.new_file()] {
            if file.id().is_zero() {
                continue;
            }
            if let Some(path) = file.path() {
                paths.insert(path.to_string_lossy().into_owned());
            }
        }
    }
    if let Ok(untracked) = stash.parent(2) {
        let tree = untracked.tree().map_err(|e| e.message().to_string())?;
        tree.walk(git2::TreeWalkMode::PreOrder, |dir, item| {
            if item.kind() != Some(git2::ObjectType::Tree) {
                if let Some(name) = item.name() {
                    paths.insert(format!("{dir}{name}"));
                }
            }
            git2::TreeWalkResult::Ok
        })
        .map_err(|e| e.message().to_string())?;
    }
    Ok(paths)
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
///
/// Where "back" is depends on what moved. After a switch it is the branch the
/// work was taken from, named in the stash. After a pull, merge or rebase the
/// branch is the same one but its commit has moved on, and the work can never
/// fit on the new one — git's own `--autostash` leaves exactly this shape when
/// it could not put the work back — so the step that moved it is taken back
/// too, off the history like any other undo, and the work goes on where it
/// came from.
pub fn undo_restore(state: &AppState) -> Result<String, String> {
    let root = state.path()?;
    let list = git_cmd::run_checked(&root, &["stash", "list"])?;
    let top = list.lines().next().unwrap_or_default().to_string();
    if !is_auto_stash(&top) {
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

    enum Back {
        Branch(String),
        Step { before: String, label: String },
        Here,
    }
    let home = stashed_on(&top).map(str::to_string);
    let here = crate::journal::current_branch(state);
    let back = match (&home, &here) {
        (Some(home), Some(here)) if home != here => Back::Branch(home.clone()),
        _ => match crate::journal::take_step_standing_here(state) {
            Some((before, label)) => Back::Step { before, label },
            // Not on record — a pull or rebase from before a restart, or one
            // made in a terminal. The branch's own log still says what moved
            // it last and from where.
            None => match here
                .as_deref()
                .and_then(|branch| moved_by_last(&root, branch))
            {
                Some((before, label)) => Back::Step { before, label },
                None => Back::Here,
            },
        },
    };

    // Clears the half-applied pop: conflicted files, and whatever else came
    // out of the stash cleanly. The stash still holds all of it.
    git_cmd::run_checked(&root, &["reset", "--hard", "HEAD"])?;
    match &back {
        Back::Branch(home) => {
            git_cmd::run_checked(&root, &["switch", "--", home])?;
        }
        Back::Step { before, .. } => {
            git_cmd::run_checked(&root, &["reset", "--hard", before])?;
        }
        Back::Here => {
            // Nothing says where the work came from, and popping it onto the
            // same commit again would only stop on the same conflict. The
            // tree is clean and the work stays where it is safe: in the stash.
            if let Ok(held) = git_cmd::run_checked(&root, &["rev-parse", "stash@{0}"]) {
                clean_stash_leftovers(&root, held.trim());
            }
            return Ok(
                "The conflicted files are cleared, and your changes are still in the stash at \
                 the top of the list — putting them back on this commit would stop on the same \
                 conflict. Apply it once the branch is where the work belongs."
                    .to_string(),
            );
        }
    }

    pop_top(&root)?;
    Ok(match back {
        Back::Branch(branch) => format!("Back on {branch}, with your changes"),
        Back::Step { label, .. } => format!("Undid: {label}, and your changes are back"),
        Back::Here => unreachable!("answered above"),
    })
}

/// Where a branch was before the pull, merge or rebase that moved it last,
/// read from the branch's own reflog, when that move is what it stands on.
fn moved_by_last(root: &Path, branch: &str) -> Option<(String, String)> {
    let repo = git2::Repository::open(root).ok()?;
    let head = repo.head().ok()?.target()?;
    let reflog = repo.reflog(&format!("refs/heads/{branch}")).ok()?;
    let last = reflog.get(0)?;
    let said = last.message().unwrap_or("");
    let moved = ["pull", "merge", "rebase"]
        .iter()
        .any(|verb| said.starts_with(verb));
    if !moved || last.id_new() != head || last.id_old().is_zero() {
        return None;
    }
    let label = said.split(':').next().unwrap_or(said).trim().to_string();
    Some((last.id_old().to_string(), label))
}

/// Removes the files a half-applied stash wrote that `reset --hard` leaves:
/// the untracked ones it carries in its third parent, named literally. They
/// are copies of what the stash still holds, and left in place they make the
/// next apply refuse over files that "already exist".
fn clean_stash_leftovers(root: &Path, held: &str) {
    let Ok(listed) = git_cmd::run_checked(
        root,
        &["ls-tree", "-r", "--name-only", &format!("{held}^3")],
    ) else {
        return;
    };
    let specs: Vec<String> = listed
        .lines()
        .filter(|line| !line.is_empty())
        .map(|path| format!(":(literal){path}"))
        .collect();
    if specs.is_empty() {
        return;
    }
    let mut args = vec!["clean", "-f", "-d", "--"];
    args.extend(specs.iter().map(String::as_str));
    let _ = git_cmd::run_checked(root, &args);
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
    /// Move the branch and the files it changes, keeping uncommitted work.
    /// Git refuses, and nothing moves, when a file with uncommitted changes is
    /// one the move would have to change. The reset the reflog view offers.
    Keep,
}

impl ResetMode {
    fn flag(&self) -> &'static str {
        match self {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
            ResetMode::Keep => "--keep",
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
    /// Files a hard reset would write over. Everything staged counts — a file
    /// added to the index but not to a commit is taken away with the rest —
    /// but untracked files do not: `git reset --hard` leaves them where they
    /// are, so counting them warned about work that was never at risk.
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
        unstaged_files: status
            .unstaged
            .iter()
            .filter(|entry| entry.kind != "untracked")
            .count(),
    })
}

/// Moves the current branch to a commit.
pub fn reset(state: &AppState, oid: &str, mode: ResetMode) -> Result<String, String> {
    let root = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    git_cmd::run_checked(&root, &["reset", mode.flag(), oid, "--"])?;

    let short: String = oid.chars().take(7).collect();
    journal::record(
        state,
        "reset",
        format!("Reset to {short}"),
        branch,
        before,
        journal::head_oid(state),
        // Undoing a hard reset has to put the working tree back too.
        if matches!(mode, ResetMode::Hard | ResetMode::Keep) {
            Mode::Hard
        } else {
            Mode::Soft
        },
        matches!(mode, ResetMode::Hard | ResetMode::Keep),
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

    // A run of picks that is still part-way through has to be finished or
    // abandoned first; git would say "already in progress" in fewer words.
    if let Some((kind, _)) = sequence(&crate::remote::git_dir(&root)?) {
        return Err(format!(
            "A {kind} of several commits is still part-way through. Continue or abort it first."
        ));
    }

    // diff3 so the resolver has the base to show, rerere so a conflict that
    // was resolved before is answered the same way.
    let flags = crate::rebase::conflict_flags(state);
    let mut args: Vec<&str> = flags.iter().map(String::as_str).collect();
    args.push("cherry-pick");
    if options.no_commit {
        args.push("--no-commit");
    }
    if options.record_origin {
        args.push("-x");
    }
    // A merge commit has no single change of its own; git has to be told which
    // parent counts as the line it was merged into, and refuses without. The
    // first parent — the branch the merge landed on — is what picking a merge
    // means nine times in ten. `-m` applies to the whole list, and git refuses
    // it on an ordinary commit, so a list has to be one kind or the other.
    let merges = ordered.iter().filter(|oid| is_merge(&root, oid)).count();
    if merges == ordered.len() {
        args.extend(["-m", "1"]);
    } else if merges > 0 {
        return Err("Cherry-pick merge commits separately from ordinary ones".to_string());
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

/// Whether a commit has more than one parent.
fn is_merge(root: &Path, oid: &str) -> bool {
    git_cmd::run_checked(root, &["rev-list", "--parents", "-n", "1", oid])
        .map(|line| line.split_whitespace().count() > 2)
        .unwrap_or(false)
}

/// Sorts commits oldest first, by asking git who descends from whom.
///
/// It used to be one `rev-list --topo-order --no-walk`, which reads well and
/// does something else: `--no-walk` orders by commit *date*, and two commits
/// made in the same second have no order by date at all — the answer depended
/// on the order the selection was clicked in. Ancestry is the thing actually
/// meant, so ancestry is what is asked: each commit goes in front of the first
/// one that descends from it, and commits with no line between them — picks
/// off two different branches — keep the order they were given in.
pub(crate) fn in_history_order(state: &AppState, oids: &[String]) -> Result<Vec<String>, String> {
    if oids.len() < 2 {
        return Ok(oids.to_vec());
    }
    let root = state.path()?;
    let descends = |old: &str, new: &str| {
        git_cmd::run(&root, &["merge-base", "--is-ancestor", old, new])
            .map(|out| out.ok)
            .unwrap_or(false)
    };

    let mut ordered: Vec<String> = Vec::with_capacity(oids.len());
    for oid in oids {
        let at = ordered
            .iter()
            .position(|placed| descends(oid, placed))
            .unwrap_or(ordered.len());
        ordered.insert(at, oid.clone());
    }
    Ok(ordered)
}

/// Adds a commit that undoes an earlier one, leaving history intact.
pub fn revert(state: &AppState, oid: &str) -> Result<String, String> {
    let root = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    let flags = crate::rebase::conflict_flags(state);
    let mut args: Vec<&str> = flags.iter().map(String::as_str).collect();
    args.extend(["revert", "--no-edit", oid]);
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

// --- a run of picks or reverts that stopped ----------------------------------
//
// `git cherry-pick a b c` that stops on `b` leaves two things: the conflict
// (`CHERRY_PICK_HEAD`), and the list of what is still to come in
// `.git/sequencer`. Committing the fix clears the first and not the second, so
// `c` was silently never applied, and the next pick of several failed with
// "a cherry-pick is already in progress". Only `--continue` works the list, so
// that is what the way on is, with `--skip` and `--abort` beside it.

/// The run git's sequencer is part-way through: `"cherry-pick"` or `"revert"`,
/// and how many commits its list still holds. `None` when there is no list.
pub fn sequence(git_dir: &Path) -> Option<(String, usize)> {
    let todo = std::fs::read_to_string(git_dir.join("sequencer").join("todo")).ok()?;
    let steps: Vec<&str> = todo
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let kind = match steps.first()?.split_whitespace().next()? {
        "revert" | "r" => "revert",
        _ => "cherry-pick",
    };
    Some((kind.to_string(), steps.len()))
}

/// What a continue, skip or abort of a run left behind.
#[derive(Serialize, Debug)]
pub struct SequenceOutcome {
    /// The run is over, one way or the other.
    pub finished: bool,
    pub message: String,
    /// Files it stopped on next, if it stopped again.
    pub conflicts: Vec<String>,
}

/// Which run is under way, from the sequencer or from the one commit git
/// stopped on when there is no list.
fn running_sequence(git_dir: &Path) -> Option<&'static str> {
    if let Some((kind, _)) = sequence(git_dir) {
        return Some(if kind == "revert" {
            "revert"
        } else {
            "cherry-pick"
        });
    }
    if git_dir.join("CHERRY_PICK_HEAD").exists() {
        return Some("cherry-pick");
    }
    if git_dir.join("REVERT_HEAD").exists() {
        return Some("revert");
    }
    None
}

/// Carries on with the run: commits the stopped commit if that has not been
/// done by hand, then applies the rest of the list.
pub fn sequence_continue(state: &AppState) -> Result<SequenceOutcome, String> {
    sequence_step(state, "--continue")
}

/// Leaves the stopped commit out and applies the rest.
pub fn sequence_skip(state: &AppState) -> Result<SequenceOutcome, String> {
    sequence_step(state, "--skip")
}

fn sequence_step(state: &AppState, how: &str) -> Result<SequenceOutcome, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let kind = running_sequence(&git_dir)
        .ok_or_else(|| "No cherry-pick or revert is part-way through".to_string())?;
    if how == "--continue" && has_unmerged(&root) {
        return Err("Some files are still conflicted. Resolve them first.".to_string());
    }

    // Where the run started, for the undo step once it is over. The sequencer
    // writes it down; a single stopped commit has no list and starts here.
    let started = std::fs::read_to_string(git_dir.join("sequencer").join("head"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|oid| !oid.is_empty())
        .or_else(|| journal::head_oid(state));
    let branch = journal::current_branch(state);
    let leg = crate::remote::trip(state);

    let flags = crate::rebase::conflict_flags(state);
    let mut args: Vec<&str> = flags.iter().map(String::as_str).collect();
    args.extend([kind, how]);
    // Continuing commits the stopped one with the message git prepared, the
    // way saving the editor unchanged would.
    let out = git_cmd::run_with_env(&root, &args, &[("GIT_EDITOR", "true")])?;

    let conflicts = crate::conflict::list(state).unwrap_or_default();
    if running_sequence(&git_dir).is_some() {
        if !conflicts.is_empty() {
            return Ok(SequenceOutcome {
                finished: false,
                message: format!(
                    "The {kind} stopped again, with conflicts in {}",
                    conflicts.join(", ")
                ),
                conflicts,
            });
        }
        if !out.ok {
            return Err(git_words(&out));
        }
        return Ok(SequenceOutcome {
            finished: false,
            message: format!("The {kind} stopped part-way"),
            conflicts,
        });
    }
    if !out.ok {
        return Err(git_words(&out));
    }

    journal::record(
        state,
        kind,
        if kind == "revert" {
            "Revert commits".to_string()
        } else {
            "Cherry-pick commits".to_string()
        },
        branch,
        started,
        journal::head_oid(state),
        Mode::Hard,
        true,
    );
    let done = if kind == "revert" {
        "Revert finished"
    } else {
        "Cherry-pick finished"
    };
    Ok(SequenceOutcome {
        finished: true,
        message: crate::remote::and_home(state, leg, done.to_string()),
        conflicts,
    })
}

/// Abandons the run and puts the branch back where it started.
///
/// Git only rewinds while HEAD is where the run left it. A commit made by
/// hand since — the stopped one, committed from the box — makes it stop the
/// run and leave the branch alone rather than throw that commit away, and
/// that is said rather than passed off as a full abort.
pub fn sequence_abort(state: &AppState) -> Result<SequenceOutcome, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let kind = running_sequence(&git_dir)
        .ok_or_else(|| "No cherry-pick or revert is part-way through".to_string())?;
    let started = std::fs::read_to_string(git_dir.join("sequencer").join("head"))
        .ok()
        .map(|text| text.trim().to_string());
    let leg = crate::remote::trip(state);
    let out = git_cmd::run(&root, &[kind, "--abort"])?;
    if !out.ok {
        return Err(git_words(&out));
    }
    let now = journal::head_oid(state);
    let message = match (&started, &now) {
        (Some(was), Some(now)) if was != now => format!(
            "The {kind} was stopped, but git left the branch where it is: it moved since the run \
             started (a commit made by hand), and rewinding would throw that away. Undo or \
             reset to {} to go back all the way.",
            &was[..was.len().min(7)]
        ),
        _ => format!("{} abandoned; the branch is as it was", capitalise(kind)),
    };
    Ok(SequenceOutcome {
        finished: true,
        message: crate::remote::and_home(state, leg, message),
        conflicts: Vec::new(),
    })
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn git_words(out: &git_cmd::CmdOutput) -> String {
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
    let mut args = plain_diff();
    if action == HunkAction::Unstage {
        args.push("--cached");
    }
    args.extend(["--", pathspec.as_str()]);
    let diff = git_cmd::run_checked(&root, &args)?;

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

    let what = match count {
        Some(1) => "one line".to_string(),
        Some(many) => format!("{many} lines"),
        None => "one hunk".to_string(),
    };
    let out = if action == HunkAction::Discard {
        // The one hunk action that loses anything: what those lines said is
        // in no commit and no index, so the file is saved before they go.
        throw_away_saved(
            state,
            &root,
            &[path.to_string()],
            "discard",
            format!("Discard {what} of {path}"),
            || git_cmd::run_with_input(&root, args, &patch),
        )?
    } else {
        git_cmd::run_with_input(&root, args, &patch)?
    };
    if !out.ok {
        let detail = if out.stderr.trim().is_empty() {
            out.stdout
        } else {
            out.stderr
        };
        return Err(format!("git could not apply that hunk: {}", detail.trim()));
    }

    Ok(match action {
        HunkAction::Stage => format!("Staged {what} of {path}"),
        HunkAction::Unstage => format!("Unstaged {what} of {path}"),
        HunkAction::Discard => format!("Discarded {what} of {path}"),
    })
}

/// The start of a `git diff` that prints the patch the window is showing.
///
/// The hunks on screen come from libgit2 (`diff.rs`: Myers, the indent
/// heuristic, three lines of context, hunks never merged), and the buttons
/// name a hunk by its position. The patch handed to `git apply` comes from the
/// CLI, which reads the user's config: `diff.algorithm=histogram` cuts the file
/// into different hunks, `diff.interHunkContext` merges neighbours, so "the
/// second hunk" is a different change — and `diff.external`, `color.diff=always`
/// or `diff.noprefix` give `git apply` something it cannot read at all. Every
/// one of those is pinned here, as a `-c` (harmless on a git too old to know
/// the setting) and, where there is one, as a flag as well.
pub fn plain_diff() -> Vec<&'static str> {
    vec![
        "-c",
        "diff.noprefix=false",
        "-c",
        "diff.mnemonicPrefix=false",
        "-c",
        "diff.relative=false",
        "-c",
        "diff.suppressBlankEmpty=false",
        "-c",
        "diff.srcPrefix=a/",
        "-c",
        "diff.dstPrefix=b/",
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--diff-algorithm=myers",
        "--indent-heuristic",
        "--unified=3",
        "--inter-hunk-context=0",
    ]
}

/// The lines of a diff with whatever ends them other than `\n` left on.
///
/// `str::lines` takes a `\r` off along with the `\n`, and a file checked in
/// with CRLF endings has a `\r` on every line of its patch. Joined back up with
/// bare `\n`s the patch no longer matches the file, and `git apply` says it
/// does not apply.
fn patch_lines(diff: &str) -> impl Iterator<Item = &str> {
    diff.split_inclusive('\n')
        .map(|line| line.strip_suffix('\n').unwrap_or(line))
}

/// Splits a diff into its header and its hunks, as text.
fn split_hunks(diff: &str) -> (Vec<&str>, Vec<Vec<&str>>) {
    let mut header: Vec<&str> = Vec::new();
    let mut hunks: Vec<Vec<&str>> = Vec::new();
    for line in patch_lines(diff) {
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
///
/// A file whose last line has no newline needs one more step. Git writes that
/// line as `-x`, a `\ No newline at end of file` remark, and `+x` again with
/// the newline on if anything was added after it. Leave the `-x` unpicked
/// going forwards and it is context still missing its newline, with picked
/// lines after it: `git apply` glued them onto it, `a\nb` and a picked `+c`
/// staged `a\nbc`. So the pair is brought in by itself — the line is taken
/// out without its newline and put back with one. Backwards the same line,
/// picked, is restored with lines kept after it, so it is restored with its
/// newline: the remark goes.
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
    // The line just written, when it was a `-` line: where it is in `body`,
    // and whether it was picked or turned into context.
    let mut last_minus: Option<(usize, bool)> = None;
    // The old side's last line, without its newline, kept as context going
    // forwards: where it is in `body`.
    let mut bare_context: Option<usize> = None;
    // The old side's last line, picked going backwards: where its remark is.
    let mut bare_removed: Option<usize> = None;

    for line in hunk.iter().skip(1) {
        let mut chars = line.chars();
        match chars.next() {
            Some('+') => {
                let picked = lines.added.contains(&new_at);
                new_at += 1;
                last_minus = None;
                if picked != reverse {
                    if let Some(at) = bare_context.take().filter(|_| !reverse) {
                        // Forwards: take the bare line out and put it back
                        // with its newline, so what follows starts a line of
                        // its own. The counts are what the context had.
                        let text = body[at][1..].to_string();
                        body[at] = format!("-{text}");
                        body.push(format!("+{text}"));
                    }
                    if let Some(at) = bare_removed.take().filter(|_| reverse) {
                        // Backwards, a line left in as context: the restored
                        // line has a line after it now.
                        body.remove(at);
                    }
                }
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
                    last_minus = Some((body.len() - 1, true));
                } else if reverse {
                    kept_last = false;
                    last_minus = None;
                } else {
                    // Not being taken out after all, so it stays as context.
                    body.push(format!(" {}", chars.as_str()));
                    old_count += 1;
                    new_count += 1;
                    kept_last = true;
                    last_minus = Some((body.len() - 1, false));
                }
            }
            Some('\\') => {
                // "No newline at end of file" belongs to the line above it.
                if kept_last {
                    body.push((*line).to_string());
                    match last_minus.take() {
                        Some((at, false)) => bare_context = Some(at),
                        Some((_, true)) => bare_removed = Some(body.len() - 1),
                        None => {}
                    }
                }
            }
            // Context, and the empty line git writes for a blank context line.
            _ => {
                last_minus = None;
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

    // A side that was empty — a file being created, or one being deleted — is
    // numbered from 0. Once some of its lines survive as context it is not
    // empty any more, and git counts a non-empty side from 1.
    let old_start = if old_count > 0 {
        old_start.max(1)
    } else {
        old_start
    };
    let new_start = if new_count > 0 {
        new_start.max(1)
    } else {
        new_start
    };
    let header = reshaped_header(&header, old_count > 0, new_count > 0);

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

/// The file header for a patch that only takes part of a creation or a
/// deletion.
///
/// Staged for the first time, a file's patch says "new file" and starts from
/// `/dev/null`. Taking back some of its lines — unstaging them, or unstaging
/// on a branch with no commits, where every staged file is new — leaves the
/// rest as context, and a creation with context is something `git apply`
/// refuses: the file it says is new would already have to hold those lines.
/// So when lines survive on the side the header calls empty, the header is
/// rewritten into an ordinary edit of the file under its own name. Deletion
/// is the same the other way round.
fn reshaped_header(header: &[&str], old_kept: bool, new_kept: bool) -> Vec<String> {
    let created = header.contains(&"--- /dev/null");
    let deleted = header.contains(&"+++ /dev/null");
    let fix_old = created && old_kept;
    let fix_new = deleted && new_kept;
    if !fix_old && !fix_new {
        return header.iter().map(|line| line.to_string()).collect();
    }
    // The other side names the file, with git's own quoting if it has any.
    let named = |prefix: &str, from: char, to: char| -> Option<String> {
        let line = header.iter().find(|line| line.starts_with(prefix))?;
        let rest = &line[prefix.len()..];
        let renamed = if let Some(tail) = rest.strip_prefix('"') {
            format!("\"{to}{}", tail.strip_prefix(from).unwrap_or(tail))
        } else {
            format!("{to}{}", rest.strip_prefix(from).unwrap_or(rest))
        };
        Some(renamed)
    };
    let old_name = fix_old.then(|| named("+++ ", 'b', 'a')).flatten();
    let new_name = fix_new.then(|| named("--- ", 'a', 'b')).flatten();
    header
        .iter()
        .filter(|line| {
            // The mode belongs to the creation or deletion that is no longer
            // what this patch is, and the index line names a blob on the
            // empty side that does not exist.
            !(fix_old && line.starts_with("new file mode")
                || fix_new && line.starts_with("deleted file mode")
                || line.starts_with("index "))
        })
        .map(|line| match (*line, &old_name, &new_name) {
            ("--- /dev/null", Some(name), _) => format!("--- {name}"),
            ("+++ /dev/null", _, Some(name)) => format!("+++ {name}"),
            _ => line.to_string(),
        })
        .collect()
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
