use git2::{BranchType, Sort};
use serde::Serialize;

use crate::git_cmd::{self, CmdOutput};
use crate::journal::{self, Mode};
use crate::state::AppState;
use crate::work;

#[derive(Serialize)]
pub struct CommitSummary {
    pub oid: String,
    pub short: String,
    pub summary: String,
    pub author: String,
    pub time: i64,
}

/// Everything the UI needs to decide whether a push is safe, and to explain the
/// consequences if it is not.
#[derive(Serialize)]
pub struct PushPreview {
    pub branch: String,
    pub remote: String,
    pub upstream: Option<String>,
    /// Where the upstream stood when this was read. A force push pins its
    /// lease to it, so what the user was shown is exactly what the push is
    /// allowed to replace — not whatever a fetch in the meantime moved it to.
    pub upstream_oid: Option<String>,
    /// True when the branch has no upstream yet and the push will create one.
    pub new_upstream: bool,
    pub ahead: usize,
    pub behind: usize,
    /// A plain push is rejected when the branch has diverged from its upstream.
    pub force_needed: bool,
    /// Commits that exist only on the remote. A force push discards these — this
    /// is the list to put in front of the user before they confirm.
    pub will_orphan: Vec<CommitSummary>,
    pub will_push: Vec<CommitSummary>,
}

#[derive(Serialize, Debug)]
pub struct MergeOutcome {
    pub ok: bool,
    pub message: String,
    pub conflicts: Vec<String>,
}

pub fn fetch(state: &AppState, remote: Option<&str>) -> Result<CmdOutput, String> {
    let path = state.path()?;
    let mut args = vec!["fetch", "--prune"];
    match remote {
        Some(r) => args.push(r),
        None => args.push("--all"),
    }
    git_cmd::run(&path, &args)
}

/// Which way a pull joins the two histories, as the flag `git pull` takes.
///
/// `None` leaves the choice to the user's own `pull.rebase`, the way a typed
/// `git pull` would — except that a bare `git pull` on a diverged branch stops
/// with "Need to specify how to reconcile divergent branches" when nothing is
/// set, so the answer is always spelled out on the command line.
fn reconcile_flag(state: &AppState, rebase: Option<bool>) -> String {
    let configured = match rebase {
        Some(true) => return "--rebase".to_string(),
        Some(false) => return "--no-rebase".to_string(),
        None => state
            .path()
            .ok()
            .and_then(|root| git_cmd::run_checked(&root, &["config", "--get", "pull.rebase"]).ok())
            .map(|value| value.trim().to_ascii_lowercase())
            .unwrap_or_default(),
    };
    match configured.as_str() {
        // `interactive` would open an editor there is no terminal for; the
        // plain rebase is what it means once nobody is there to edit the plan.
        "true" | "interactive" => "--rebase".to_string(),
        "merges" => "--rebase=merges".to_string(),
        _ => "--no-rebase".to_string(),
    }
}

/// Pulls the checked out branch.
///
/// Uncommitted work rides along on git's own `--autostash`: set down before
/// the pull and put back after it — and, when the pull stops on a conflict,
/// put back by whichever of abort, continue or commit finishes it. Nothing is
/// left sitting in the stash for the user to remember.
pub fn pull(state: &AppState, rebase: Option<bool>) -> Result<MergeOutcome, String> {
    let path = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    let flag = reconcile_flag(state, rebase);

    let out = git_cmd::run(
        &path,
        &[
            "-c",
            "merge.conflictStyle=diff3",
            "pull",
            "--autostash",
            &flag,
        ],
    )?;
    let moved = out.ok;
    let outcome = settled(state, out);

    if moved {
        journal::record(
            state,
            "pull",
            "Pull".to_string(),
            branch,
            before,
            journal::head_oid(state),
            Mode::Hard,
            true,
        );
    }
    Ok(outcome)
}

/// What a merge-like command left behind: whether it went through, what it
/// said, and which files it stopped on.
///
/// The conflicts are read from the index rather than from the exit code. A
/// command can exit 0 and still leave conflicted files — an autostash that
/// would not go back on cleanly is the usual case — and those are conflicts the
/// resolver has to open on all the same.
fn settled(state: &AppState, out: CmdOutput) -> MergeOutcome {
    let conflicts = crate::conflict::list(state).unwrap_or_default();
    MergeOutcome {
        ok: out.ok && conflicts.is_empty(),
        message: said(&out),
        conflicts,
    }
}

/// Everything git printed, stdout first, with nothing empty in between.
fn said(out: &CmdOutput) -> String {
    [out.stdout.trim(), out.stderr.trim()]
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Brings a branch up to date with its upstream, whether or not it is the one
/// checked out.
///
/// For a branch you are not on, this needs no working tree at all: `git fetch
/// <remote> <theirs>:<ours>` moves the local ref straight to what the remote
/// has. Open changes are beside the point because nothing touches them, and
/// there is no checkout, no stash, and no way to end up somewhere unexpected.
///
/// That only works while the update is a fast-forward. A branch that has moved
/// on locally needs its commits replayed, and replaying them means having them
/// in the working tree — so that case is a [`visit`].
pub fn pull_branch(
    state: &AppState,
    branch: &str,
    rebase: Option<bool>,
) -> Result<MergeOutcome, String> {
    // The branch you are on is an ordinary pull.
    if journal::current_branch(state).as_deref() == Some(branch) {
        return pull(state, rebase);
    }

    let path = state.path()?;
    let (remote, theirs) = {
        let repo = state.repo()?;
        let local = repo
            .find_branch(branch, BranchType::Local)
            .map_err(|_| format!("No local branch named {branch}"))?;
        let upstream = local
            .upstream()
            .map_err(|_| format!("{branch} is not tracking anything to pull from"))?;
        let full = upstream
            .name()
            .ok()
            .flatten()
            .ok_or_else(|| format!("Could not read what {branch} tracks"))?
            .to_string();
        // `origin/feature/x` is remote `origin`, branch `feature/x`.
        let (remote, theirs) = full
            .split_once('/')
            .ok_or_else(|| format!("{full} does not name a remote branch"))?;
        (remote.to_string(), theirs.to_string())
    };

    let before = branch_oid(state, branch);

    let refspec = format!("{theirs}:{branch}");
    let out = git_cmd::run(&path, &["fetch", &remote, &refspec])?;

    // It has commits of its own, so the update has to be a real merge, and a
    // merge needs the branch in the working tree.
    if !out.ok && out.stderr.contains("non-fast-forward") {
        return visit(
            state,
            branch,
            &format!("pulling {branch}"),
            &format!("{branch} brought up to date with {remote}/{theirs}"),
            |state| pull(state, rebase),
        );
    }

    let message = if !out.ok {
        said(&out)
    } else if before == branch_oid(state, branch) {
        format!("{branch} was already up to date")
    } else {
        format!("{branch} brought up to date with {remote}/{theirs}")
    };
    Ok(MergeOutcome {
        ok: out.ok,
        message,
        conflicts: Vec::new(),
    })
}

/// Where a local branch points, as an owned string.
///
/// The `Branch` borrows the `Repository`, so the handle has to outlive the
/// lookup; taking the id out here keeps that confined to one place.
fn branch_oid(state: &AppState, branch: &str) -> Option<String> {
    let repo = state.repo().ok()?;
    let found = repo.find_branch(branch, BranchType::Local).ok()?;
    found.get().target().map(|oid| oid.to_string())
}

/// Does something standing on another branch, and comes home after.
///
/// Git merges, rebases and pulls into the branch you are on, which is why
/// "merge this into that" normally means checking out, acting, and remembering
/// to come back. This is that trip made out of sight: open work is set down,
/// `target` is checked out, `act` runs on it, and the way home is the reverse.
/// `done` is what to say when it all worked.
///
/// Conflicts are the one thing that does not travel. A half-done merge lives in
/// the working tree, and leaving the user standing in one on a branch they did
/// not ask to be on, with their own changes hidden in a stash, is exactly the
/// surprise this application exists to avoid. So the operation is abandoned,
/// the trip is undone, and the message says what to do instead: check the
/// branch out and do it there, where the resolver can open on it. Nothing is
/// changed and nothing is lost — the same repository as before, one sentence
/// wiser.
fn visit(
    state: &AppState,
    target: &str,
    reason: &str,
    done: &str,
    act: impl FnOnce(&AppState) -> Result<MergeOutcome, String>,
) -> Result<MergeOutcome, String> {
    let path = state.path()?;
    let original = journal::current_branch(state)
        .ok_or_else(|| "HEAD is detached; check out a branch first".to_string())?;

    let held = work::stash_before(state, reason)?;

    let switched = git_cmd::run(&path, &["switch", "--", target])?;
    if !switched.ok {
        let mut message = format!(
            "{}\n\nCould not step onto {target}.",
            switched.stderr.trim()
        );
        if let Err(note) = work::restore_after(state, held) {
            message = format!("{message}\n{note}");
        }
        return Ok(MergeOutcome {
            ok: false,
            message,
            conflicts: Vec::new(),
        });
    }

    let mut outcome = act(state)?;

    if !outcome.conflicts.is_empty() {
        // Only one of these applies; the other reports nothing to abort.
        let _ = git_cmd::run(&path, &["merge", "--abort"]);
        let _ = git_cmd::run(&path, &["rebase", "--abort"]);
        outcome.message = format!(
            "{} would conflict in {}. {target} was left as it was: check it out and try again \
             there to resolve them.",
            capitalised(reason),
            outcome.conflicts.join(", ")
        );
        outcome.conflicts.clear();
        outcome.ok = false;
    } else if outcome.ok {
        outcome.message = done.to_string();
    }

    // Home again, whichever way it went.
    let back = git_cmd::run(&path, &["switch", "--", &original])?;
    if !back.ok {
        outcome.ok = false;
        outcome.message = format!(
            "{}\n\nCould not return to {original} — you are on {target}.{}",
            outcome.message.trim(),
            if held.stashed {
                " Your open changes are in the stash."
            } else {
                ""
            }
        );
        return Ok(outcome);
    }

    match work::restore_after(state, held) {
        Ok(Some(note)) => outcome.message = format!("{}\n{note}", outcome.message.trim()),
        Err(error) => {
            outcome.ok = false;
            outcome.message = format!("{}\n{error}", outcome.message.trim());
        }
        Ok(None) => {}
    }
    Ok(outcome)
}

/// "merging a into b" as the start of a sentence.
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub fn push_preview(
    state: &AppState,
    branch: Option<&str>,
    fetch_first: bool,
) -> Result<PushPreview, String> {
    if fetch_first {
        // Divergence can only be judged against a current view of the remote, so
        // offer to refresh it before answering.
        let _ = fetch(state, None)?;
    }

    let repo = state.repo()?;
    let branch_name = match branch {
        Some(b) => b.to_string(),
        None => repo
            .head()
            .ok()
            .filter(|_| !repo.head_detached().unwrap_or(false))
            .and_then(|h| h.shorthand().map(|s| s.to_string()))
            .ok_or_else(|| "HEAD is detached; check out a branch to push".to_string())?,
    };

    let local = repo
        .find_branch(&branch_name, BranchType::Local)
        .map_err(|_| format!("No local branch named {branch_name}"))?;
    let local_oid = local
        .get()
        .target()
        .ok_or_else(|| format!("Branch {branch_name} has no commit"))?;

    let upstream = local.upstream().ok();
    let upstream_name = upstream
        .as_ref()
        .and_then(|u| u.name().ok().flatten().map(|s| s.to_string()));
    let upstream_oid = upstream.as_ref().and_then(|u| u.get().target());

    // The remote to push to: the upstream's remote, else the first configured
    // one, else the conventional name.
    let remote = upstream_name
        .as_ref()
        .and_then(|n| n.split_once('/').map(|(r, _)| r.to_string()))
        .or_else(|| {
            repo.remotes()
                .ok()
                .and_then(|list| list.get(0).map(|s| s.to_string()))
        })
        .unwrap_or_else(|| "origin".to_string());

    let (ahead, behind) = match upstream_oid {
        Some(up) => repo.graph_ahead_behind(local_oid, up).map_err(err)?,
        None => (0, 0),
    };

    // `will_push` is local-only history; `will_orphan` is remote-only history.
    let will_push = match upstream_oid {
        Some(up) => range(state, &format!("{up}"), &format!("{local_oid}"), 50)?,
        None => range_from(state, &format!("{local_oid}"), 50)?,
    };
    let will_orphan = match upstream_oid {
        Some(up) if behind > 0 => range(state, &format!("{local_oid}"), &format!("{up}"), 50)?,
        _ => Vec::new(),
    };

    Ok(PushPreview {
        branch: branch_name,
        remote,
        upstream: upstream_name,
        upstream_oid: upstream_oid.map(|oid| oid.to_string()),
        new_upstream: upstream_oid.is_none(),
        ahead,
        behind,
        force_needed: behind > 0,
        will_orphan,
        will_push,
    })
}

/// Pushes `branch`.
///
/// When `force` is set the push uses `--force-with-lease`, never a bare
/// `--force`: the lease makes the remote reject the push if it moved since the
/// last look, which is precisely the case where a plain force would silently
/// destroy someone else's commits.
///
/// `lease` is what that look saw — the upstream commit the preview showed the
/// user — and the lease is pinned to it. Without it git leases against the
/// remote-tracking ref, which the fetch on a timer moves under your feet: a
/// fetch landing between the preview and the click would quietly widen what
/// the push is allowed to throw away.
pub fn push(
    state: &AppState,
    remote: &str,
    branch: &str,
    force: bool,
    set_upstream: bool,
    lease: Option<&str>,
) -> Result<CmdOutput, String> {
    let path = state.path()?;
    let with_lease = match lease {
        Some(expected) => format!("--force-with-lease=refs/heads/{branch}:{expected}"),
        None => "--force-with-lease".to_string(),
    };
    let mut args = vec!["push"];
    if force {
        args.push(&with_lease);
    }
    if set_upstream {
        args.push("--set-upstream");
    }
    args.push(remote);
    args.push(branch);
    git_cmd::run(&path, &args)
}

/// Merges a branch into the checked out one, carrying open work on git's own
/// `--autostash` the way [`pull`] does.
pub fn merge(state: &AppState, branch: &str, no_ff: bool) -> Result<MergeOutcome, String> {
    let path = state.path()?;
    // diff3 keeps the merge base in the conflict markers, which the conflict
    // resolver shows as its third pane.
    let mut args = vec!["-c", "merge.conflictStyle=diff3", "merge", "--autostash"];
    if no_ff {
        args.push("--no-ff");
    }
    args.push(branch);

    let before = journal::head_oid(state);
    let on = journal::current_branch(state);
    let out = git_cmd::run(&path, &args)?;
    let moved = out.ok;
    let outcome = settled(state, out);

    if moved {
        journal::record(
            state,
            "merge",
            format!("Merge {branch}"),
            on,
            before,
            journal::head_oid(state),
            Mode::Hard,
            true,
        );
    }
    Ok(outcome)
}

pub fn abort_merge(state: &AppState) -> Result<String, String> {
    let path = state.path()?;
    git_cmd::run_checked(&path, &["merge", "--abort"])
}

/// Merges one branch into another, whichever one is checked out.
///
/// Three cases, cheapest first: the target is already checked out and this is
/// an ordinary merge; the target is merely behind, so its ref is moved with no
/// working tree involved at all; or the two have diverged, and the merge has to
/// be made somewhere — so it is made on the target, on a [`visit`].
pub fn merge_into(
    state: &AppState,
    source: &str,
    target: &str,
    no_ff: bool,
) -> Result<MergeOutcome, String> {
    if journal::current_branch(state).as_deref() == Some(target) {
        return merge(state, source, no_ff);
    }

    if !no_ff && can_fast_forward(state, target, source)? {
        return fast_forward_ref(state, source, target);
    }

    visit(
        state,
        target,
        &format!("merging {source} into {target}"),
        &format!("{source} merged into {target}"),
        |state| merge(state, source, no_ff),
    )
}

/// Moves a branch that is strictly behind straight to where the other one is.
///
/// `git fetch . <source>:<target>` updates the ref and nothing else: the
/// working tree is not touched, so open changes are beside the point and there
/// is no way to end up somewhere unexpected. Git refuses the refspec unless it
/// really is a fast-forward, which makes this safe even if the ancestry check
/// above were wrong.
fn fast_forward_ref(state: &AppState, source: &str, target: &str) -> Result<MergeOutcome, String> {
    let path = state.path()?;
    let before = branch_oid(state, target);
    let out = git_cmd::run(&path, &["fetch", ".", &format!("{source}:{target}")])?;
    let after = branch_oid(state, target);

    if out.ok {
        journal::record(
            state,
            "merge",
            format!("Fast-forward {target} to {source}"),
            Some(target.to_string()),
            before,
            after,
            Mode::Hard,
            true,
        );
    }

    Ok(MergeOutcome {
        ok: out.ok,
        message: if out.ok {
            format!("{target} fast-forwarded to {source}")
        } else {
            out.stderr.trim().to_string()
        },
        conflicts: Vec::new(),
    })
}

/// Rebases a branch onto another without asking the user to stand on it first:
/// a [`visit`] to the branch, with the rebase made there.
pub fn rebase_branch(state: &AppState, branch: &str, onto: &str) -> Result<MergeOutcome, String> {
    if journal::current_branch(state).as_deref() == Some(branch) {
        return rebase(state, onto);
    }
    visit(
        state,
        branch,
        &format!("rebasing {branch} onto {onto}"),
        &format!("{branch} rebased onto {onto}"),
        |state| rebase(state, onto),
    )
}

/// Commits in `head` that are not in `base` — the `base..head` range.
fn range(
    state: &AppState,
    base: &str,
    head: &str,
    limit: usize,
) -> Result<Vec<CommitSummary>, String> {
    let repo = state.repo()?;
    let base = git2::Oid::from_str(base).map_err(|_| "Bad revision".to_string())?;
    let head = git2::Oid::from_str(head).map_err(|_| "Bad revision".to_string())?;

    let mut walk = repo.revwalk().map_err(err)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)
        .map_err(err)?;
    walk.push(head).map_err(err)?;
    walk.hide(base).map_err(err)?;
    take(&repo, walk, limit)
}

fn range_from(state: &AppState, head: &str, limit: usize) -> Result<Vec<CommitSummary>, String> {
    let repo = state.repo()?;
    let head = git2::Oid::from_str(head).map_err(|_| "Bad revision".to_string())?;
    let mut walk = repo.revwalk().map_err(err)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)
        .map_err(err)?;
    walk.push(head).map_err(err)?;
    take(&repo, walk, limit)
}

fn take(
    repo: &git2::Repository,
    walk: git2::Revwalk,
    limit: usize,
) -> Result<Vec<CommitSummary>, String> {
    let mut out = Vec::new();
    for oid in walk.take(limit) {
        let oid = oid.map_err(err)?;
        let commit = repo.find_commit(oid).map_err(err)?;
        out.push(CommitSummary {
            oid: oid.to_string(),
            short: oid.to_string()[..7].to_string(),
            summary: commit.summary().unwrap_or("").to_string(),
            author: commit.author().name().unwrap_or("").to_string(),
            time: commit.time().seconds(),
        });
    }
    Ok(out)
}

fn err(e: git2::Error) -> String {
    e.message().to_string()
}

/// Replays the current branch on top of another, carrying open work on git's
/// own `--autostash` the way [`pull`] does: a rebase refuses to start with a
/// dirty tree, and asking the user to tidy up by hand is the sort of thing this
/// application exists to avoid.
pub fn rebase(state: &AppState, onto: &str) -> Result<MergeOutcome, String> {
    let path = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    let out = git_cmd::run(
        &path,
        &[
            "-c",
            "merge.conflictStyle=diff3",
            "rebase",
            "--autostash",
            onto,
        ],
    )?;
    let moved = out.ok;
    let outcome = settled(state, out);

    if moved {
        journal::record(
            state,
            "rebase",
            format!("Rebase onto {onto}"),
            branch,
            before,
            journal::head_oid(state),
            Mode::Hard,
            true,
        );
    }
    Ok(outcome)
}

pub fn abort_rebase(state: &AppState) -> Result<String, String> {
    let path = state.path()?;
    git_cmd::run_checked(&path, &["rebase", "--abort"])
}

/// Continues a rebase after its conflicts have been resolved and staged.
pub fn continue_rebase(state: &AppState) -> Result<MergeOutcome, String> {
    let path = state.path()?;
    let out = git_cmd::run(&path, &["-c", "core.editor=true", "rebase", "--continue"])?;
    Ok(settled(state, out))
}

/// Whether git is part-way through something the user has to finish.
#[derive(Serialize)]
pub struct InProgress {
    pub merging: bool,
    pub rebasing: bool,
    pub cherry_picking: bool,
    pub reverting: bool,
    /// A switch, pull, merge or rebase put the work back and it did not fit:
    /// files are conflicted, git is running nothing, and the auto-stash it
    /// made is still on the list. Only then is there a restore to undo —
    /// conflicts left by a stash applied by hand look identical to git and are
    /// not undoable.
    pub restoring: bool,
    /// The stash a conflicted apply or pop came from, while its mess is still
    /// in the tree. Set only while the apply can still be taken back off.
    pub applied_stash: Option<String>,
    /// The message git has already written for what it is part-way through.
    ///
    /// A merge names itself — "Merge branch 'x' into 'y'" — and git keeps that
    /// sentence in `MERGE_MSG` from the moment the merge starts, conflicts or
    /// not. Nobody wants to retype it, and a commit box that insists on a
    /// message it is already holding is a box asking a question it knows the
    /// answer to.
    pub prepared: Option<String>,
}

/// Where git keeps its own state for this working tree.
///
/// A worktree and a submodule have a `.git` file naming the real directory
/// rather than a directory of their own, and everything that reads git's
/// in-progress state has to follow it.
pub fn git_dir(root: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let found = root.join(".git");
    if !found.is_file() {
        return Ok(found);
    }
    let text = std::fs::read_to_string(&found)
        .map_err(|e| format!("Could not read {}: {e}", found.display()))?;
    Ok(text
        .strip_prefix("gitdir:")
        .map(|path| {
            let path = std::path::PathBuf::from(path.trim());
            // It is allowed to be relative to the working tree.
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        })
        .unwrap_or(found))
}

pub fn in_progress(state: &AppState) -> Result<InProgress, String> {
    let root = state.path()?;
    let git_dir = git_dir(&root)?;

    let merging = git_dir.join("MERGE_HEAD").exists();
    let rebasing = git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists();
    let cherry_picking = git_dir.join("CHERRY_PICK_HEAD").exists();
    let reverting = git_dir.join("REVERT_HEAD").exists();
    let running = merging || rebasing || cherry_picking || reverting;

    Ok(InProgress {
        merging,
        rebasing,
        cherry_picking,
        reverting,
        restoring: !running && has_unmerged(&root) && work::auto_stash_on_top(&root),
        applied_stash: (!running).then(|| work::applied_stash(&root)).flatten(),
        prepared: prepared_message(&git_dir),
    })
}

/// The message git wrote for the merge or squash it is part-way through.
///
/// Comment lines go: git strips them itself at commit time, and the list of
/// conflicted files it puts under `# Conflicts:` is a note to the person doing
/// the merge, not part of what they are committing.
fn prepared_message(git_dir: &std::path::Path) -> Option<String> {
    for name in ["MERGE_MSG", "SQUASH_MSG"] {
        let Ok(text) = std::fs::read_to_string(git_dir.join(name)) else {
            continue;
        };
        let kept: Vec<&str> = text.lines().filter(|line| !line.starts_with('#')).collect();
        let message = kept.join("\n").trim().to_string();
        if !message.is_empty() {
            return Some(message);
        }
    }
    None
}

/// True when the index holds unmerged entries, whatever put them there.
fn has_unmerged(root: &std::path::Path) -> bool {
    git_cmd::run_checked(root, &["ls-files", "--unmerged"])
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false)
}

/// Deletes a branch on the remote.
pub fn delete_remote_branch(
    state: &AppState,
    remote: &str,
    branch: &str,
) -> Result<CmdOutput, String> {
    let path = state.path()?;
    git_cmd::run(&path, &["push", remote, "--delete", branch])
}

pub fn push_tag(state: &AppState, remote: &str, tag: &str) -> Result<CmdOutput, String> {
    let path = state.path()?;
    git_cmd::run(&path, &["push", remote, tag])
}

pub fn delete_remote_tag(state: &AppState, remote: &str, tag: &str) -> Result<CmdOutput, String> {
    let path = state.path()?;
    git_cmd::run(
        &path,
        &["push", remote, "--delete", &format!("refs/tags/{tag}")],
    )
}

/// The remote this repository is really about.
///
/// The one the current branch tracks, else the conventional `origin`, else
/// whichever is configured first. A fork or a mirror added alongside it is
/// never it, which is what keeps a delete from reaching somebody else's copy.
pub fn primary(state: &AppState) -> Option<String> {
    let repo = state.repo().ok()?;
    let tracked = repo
        .head()
        .ok()
        .filter(|head| head.is_branch())
        .and_then(|head| head.shorthand().map(|s| s.to_string()))
        .and_then(|branch| {
            repo.branch_upstream_remote(&format!("refs/heads/{branch}"))
                .ok()
        })
        .and_then(|buf| buf.as_str().map(|s| s.to_string()));
    if tracked.is_some() {
        return tracked;
    }

    let list = repo.remotes().ok()?;
    let names: Vec<String> = list.iter().flatten().map(|s| s.to_string()).collect();
    names
        .iter()
        .find(|name| *name == "origin")
        .or_else(|| names.first())
        .cloned()
}

/// The remotes configured for this repository.
pub fn remotes(state: &AppState) -> Result<Vec<String>, String> {
    let repo = state.repo()?;
    let list = repo.remotes().map_err(|e| e.message().to_string())?;
    Ok(list.iter().flatten().map(|s| s.to_string()).collect())
}

// --- managing the remotes themselves ----------------------------------------
//
// `remotes` above only ever listed them, so a repository cloned over https
// could not be moved to ssh — or a second remote added — without dropping to
// the command line. All four are plain `git remote` subcommands, run through
// the CLI wrapper so they turn up in the activity log like any other change.

/// The address a remote fetches from, shown when it is about to be edited.
pub fn remote_url(state: &AppState, remote: &str) -> Result<String, String> {
    let path = state.path()?;
    git_cmd::run_checked(&path, &["remote", "get-url", remote]).map(|url| url.trim().to_string())
}

/// A remote name git will accept: it becomes a section header in the config
/// and half of every `remote/branch` ref, so the same characters that break a
/// branch break it too.
fn valid_remote_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains(['/', ' ', '~', '^', ':', '?', '*', '[', '\\'])
    {
        return Err(format!("\"{name}\" cannot be used as a remote name"));
    }
    Ok(())
}

pub fn remote_add(state: &AppState, name: &str, url: &str) -> Result<String, String> {
    valid_remote_name(name)?;
    let url = url.trim();
    if url.is_empty() {
        return Err("Give the remote an address to fetch from".to_string());
    }
    let path = state.path()?;
    git_cmd::run_checked(&path, &["remote", "add", name, url])?;
    Ok(format!("Added remote {name}"))
}

pub fn remote_set_url(state: &AppState, name: &str, url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Give the remote an address to fetch from".to_string());
    }
    let path = state.path()?;
    git_cmd::run_checked(&path, &["remote", "set-url", name, url])?;
    Ok(format!("Now fetching {name} from {url}"))
}

pub fn remote_rename(state: &AppState, from: &str, to: &str) -> Result<String, String> {
    valid_remote_name(to)?;
    let path = state.path()?;
    git_cmd::run_checked(&path, &["remote", "rename", from, to])?;
    // git moves the remote-tracking branches with the name; the local branches
    // still point at the old ones, which is worth saying rather than leaving
    // to be discovered as upstreams that suddenly do not exist.
    Ok(format!(
        "Renamed remote {from} to {to}; branches tracking {from}/ now track {to}/"
    ))
}

/// Removes a remote and its remote-tracking branches. Nothing local is touched
/// — not the branches, not their upstream settings — so this is undoable by
/// adding the remote back.
pub fn remote_remove(state: &AppState, name: &str) -> Result<String, String> {
    let path = state.path()?;
    git_cmd::run_checked(&path, &["remote", "remove", name])?;
    Ok(format!("Removed remote {name}"))
}

/// Whether a fast-forward alone would bring `branch` up to `onto`.
///
/// Knowing this lets the drag-and-drop menu offer the cheap answer first
/// instead of making a merge commit nobody wanted.
pub fn can_fast_forward(state: &AppState, branch: &str, onto: &str) -> Result<bool, String> {
    let path = state.path()?;
    Ok(git_cmd::run_checked(&path, &["merge-base", "--is-ancestor", branch, onto]).is_ok())
}

/// How two branches stand to each other.
///
/// Merge, fast-forward and rebase are not always all possible, and offering one
/// that would do nothing is worse than not offering it: the user picks it,
/// something happens or does not, and they are left guessing which.
#[derive(serde::Serialize, Debug)]
pub struct BranchRelation {
    /// Commits on `source` that `target` does not have.
    pub ahead: usize,
    /// Commits on `target` that `source` does not have.
    pub behind: usize,
}

impl BranchRelation {
    /// `source` is already contained in `target`; there is nothing to bring over.
    pub fn merged(&self) -> bool {
        self.ahead == 0
    }
    /// `target` has nothing of its own, so it can simply be moved forward.
    pub fn fast_forward(&self) -> bool {
        self.ahead > 0 && self.behind == 0
    }
}

pub fn relation(state: &AppState, source: &str, target: &str) -> Result<BranchRelation, String> {
    let repo = state.repo()?;
    let resolve = |name: &str| -> Result<git2::Oid, String> {
        repo.revparse_single(name)
            .map_err(|_| format!("No branch or commit named {name}"))?
            .peel_to_commit()
            .map(|c| c.id())
            .map_err(|e| e.message().to_string())
    };
    let (source_oid, target_oid) = (resolve(source)?, resolve(target)?);
    let (ahead, behind) = repo
        .graph_ahead_behind(source_oid, target_oid)
        .map_err(|e| e.message().to_string())?;
    Ok(BranchRelation { ahead, behind })
}
