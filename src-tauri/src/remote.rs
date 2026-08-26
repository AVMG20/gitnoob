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

#[derive(Serialize)]
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

/// Pulls, stashing uncommitted work first so the pull is not refused, then
/// putting it back.
pub fn pull(state: &AppState, rebase: bool) -> Result<CmdOutput, String> {
    let path = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    let held = work::stash_before(state, "pulling")?;
    // Always say which reconciliation is meant. A bare `git pull` on a diverged
    // branch stops with "Need to specify how to reconcile divergent branches"
    // unless the user has set `pull.rebase` — and the caller has already asked
    // them, so there is nothing left to be undecided about.
    let mut args = vec!["pull"];
    args.push(if rebase { "--rebase" } else { "--no-rebase" });
    let mut output = git_cmd::run(&path, &args)?;

    match work::restore_after(state, held) {
        Ok(Some(note)) => output.stdout = format!("{}\n{note}", output.stdout.trim()),
        // The pull itself worked; say so, and say what happened to the stash.
        Err(error) => output.stderr = format!("{}\n{error}", output.stderr.trim()),
        Ok(None) => {}
    }

    if output.ok {
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
    Ok(output)
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
/// in the working tree — so that case is reported rather than guessed at.
pub fn pull_branch(state: &AppState, branch: &str, rebase: bool) -> Result<CmdOutput, String> {
    // The branch you are on is an ordinary pull, stash dance and all.
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
    let mut out = git_cmd::run(&path, &["fetch", &remote, &refspec])?;

    // It has commits of its own, so the update has to be a real merge, and a
    // merge needs the branch in the working tree. Do the whole dance out of
    // sight instead of handing the problem back.
    if !out.ok && out.stderr.contains("non-fast-forward") {
        return pull_by_visiting(state, branch, rebase);
    }

    if out.ok {
        let after = branch_oid(state, branch);
        if before == after {
            out.stdout = format!("{branch} was already up to date");
        } else {
            out.stdout = format!("{branch} brought up to date with {remote}/{theirs}");
        }
    }
    Ok(out)
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

/// Pulls a branch that has diverged, by going there and coming back.
///
/// Stash, switch, pull, switch back, unstash. Every step is undone on the way
/// out, including on failure: if the pull conflicts, the merge is abandoned
/// before returning, so the repository is left exactly as it was found and the
/// user is told the branch needs their attention rather than discovering
/// themselves standing on it mid-merge.
///
/// The one thing this cannot hide is `auto_stash` being switched off with a
/// dirty tree, because then the switch itself is refused — and quietly
/// overriding a setting is worse than saying so.
fn pull_by_visiting(state: &AppState, branch: &str, rebase: bool) -> Result<CmdOutput, String> {
    let path = state.path()?;
    let original = journal::current_branch(state)
        .ok_or_else(|| "HEAD is detached; check out a branch first".to_string())?;

    let held = work::stash_before(state, &format!("pulling {branch}"))?;

    let switched = git_cmd::run(&path, &["checkout", branch, "--"])?;
    if !switched.ok {
        let _ = work::restore_after(state, held);
        let mut out = switched;
        out.stderr = format!(
            "{}\n\nCould not step onto {branch} to update it. Commit or stash your changes, or \
             turn auto-stash on in settings.",
            out.stderr.trim()
        );
        return Ok(out);
    }

    // See `pull`: name the reconciliation rather than leaving it to config.
    let mut args = vec!["pull"];
    args.push(if rebase { "--rebase" } else { "--no-rebase" });
    let mut out = git_cmd::run(&path, &args)?;

    if !out.ok {
        // Leave nothing half-finished behind us. Only one of these applies; the
        // other is a no-op that reports there is nothing to abort.
        let _ = git_cmd::run(&path, &["merge", "--abort"]);
        let _ = git_cmd::run(&path, &["rebase", "--abort"]);
        out.stderr = format!(
            "{}\n\n{branch} was left as it was: updating it needs a merge that does not apply \
             cleanly. Check it out to work through it.",
            out.stderr.trim()
        );
    } else {
        out.stdout = format!("{branch} brought up to date");
    }

    // Home again, whichever way the pull went.
    let back = git_cmd::run(&path, &["checkout", &original, "--"])?;
    if !back.ok {
        out.stderr = format!(
            "{}\n\nCould not return to {original} — you are on {branch}.",
            out.stderr.trim()
        );
        out.ok = false;
        return Ok(out);
    }

    match work::restore_after(state, held) {
        Ok(Some(note)) => out.stdout = format!("{}\n{note}", out.stdout.trim()),
        Err(error) => {
            out.stderr = format!("{}\n{error}", out.stderr.trim());
            out.ok = false;
        }
        Ok(None) => {}
    }
    Ok(out)
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
/// last fetch, which is precisely the case where a plain force would silently
/// destroy someone else's commits.
pub fn push(
    state: &AppState,
    remote: &str,
    branch: &str,
    force: bool,
    set_upstream: bool,
) -> Result<CmdOutput, String> {
    let path = state.path()?;
    let mut args = vec!["push"];
    if force {
        args.push("--force-with-lease");
    }
    if set_upstream {
        args.push("--set-upstream");
    }
    args.push(remote);
    args.push(branch);
    git_cmd::run(&path, &args)
}

pub fn merge(state: &AppState, branch: &str, no_ff: bool) -> Result<MergeOutcome, String> {
    let path = state.path()?;
    let branch_name = branch.to_string();
    // diff3 keeps the merge base in the conflict markers, which the conflict
    // resolver shows as its third pane.
    let mut args = vec!["-c", "merge.conflictStyle=diff3", "merge"];
    if no_ff {
        args.push("--no-ff");
    }
    args.push(branch);

    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    let out = git_cmd::run(&path, &args)?;
    let conflicts = crate::conflict::list(state).unwrap_or_default();

    if out.ok && conflicts.is_empty() {
        journal::record(
            state,
            "merge",
            format!("Merge {branch_label}", branch_label = branch_name),
            branch,
            before,
            journal::head_oid(state),
            Mode::Hard,
            true,
        );
    }
    let message = if out.stderr.trim().is_empty() {
        out.stdout.trim().to_string()
    } else {
        format!("{}\n{}", out.stdout.trim(), out.stderr.trim())
            .trim()
            .to_string()
    };

    Ok(MergeOutcome {
        ok: out.ok && conflicts.is_empty(),
        message,
        conflicts,
    })
}

pub fn abort_merge(state: &AppState) -> Result<String, String> {
    let path = state.path()?;
    git_cmd::run_checked(&path, &["merge", "--abort"])
}

/// Merges one branch into another, whichever one is checked out.
///
/// Git only ever merges into the branch you are standing on, which is why
/// merging two other branches normally means checking one out, merging, and
/// remembering to come back. Dropping one branch onto another says what is
/// wanted plainly enough that none of that should be the user's problem.
///
/// Three cases, cheapest first: the target is already checked out and this is
/// an ordinary merge; the target is merely behind, so its ref is moved with no
/// working tree involved at all; or the two have diverged, and the merge has to
/// be made somewhere — so it is made on the target and we come home after.
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

    merge_by_visiting(state, source, target, no_ff)
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

/// Merges into a branch by going there and coming back.
///
/// Conflicts are the one thing that cannot be hidden: an unfinished merge lives
/// in the working tree, so leaving would abandon it. In that case we stay on the
/// target and say so — the resolver opens on a branch the user is actually
/// standing on, which is the only state where finishing the merge makes sense.
fn merge_by_visiting(
    state: &AppState,
    source: &str,
    target: &str,
    no_ff: bool,
) -> Result<MergeOutcome, String> {
    let path = state.path()?;
    let original = journal::current_branch(state)
        .ok_or_else(|| "HEAD is detached; check out a branch first".to_string())?;

    let held = work::stash_before(state, &format!("merging {source} into {target}"))?;

    let switched = git_cmd::run(&path, &["checkout", target, "--"])?;
    if !switched.ok {
        let _ = work::restore_after(state, held);
        return Ok(MergeOutcome {
            ok: false,
            message: format!(
                "{}\n\nCould not step onto {target} to merge into it. Commit or stash your \
                 changes, or turn auto-stash on in settings.",
                switched.stderr.trim()
            ),
            conflicts: Vec::new(),
        });
    }

    let mut outcome = merge(state, source, no_ff)?;

    if !outcome.conflicts.is_empty() {
        outcome.message = format!(
            "{}\n\nYou are on {target} with the merge half-done. Resolve it here, then switch \
             back to {original}.",
            outcome.message.trim()
        );
        // The stash belongs to the branch it was taken from, and putting it
        // back on top of a conflicted tree would tangle the two.
        if held.stashed {
            outcome.message = format!(
                "{}\nYour open changes are still stashed.",
                outcome.message.trim()
            );
        }
        return Ok(outcome);
    }

    let back = git_cmd::run(&path, &["checkout", &original, "--"])?;
    if !back.ok {
        outcome.ok = false;
        outcome.message = format!(
            "{}\n\nCould not return to {original} — you are on {target}.",
            outcome.message.trim()
        );
        return Ok(outcome);
    }

    if outcome.ok {
        outcome.message = format!("{source} merged into {target}");
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

/// Rebases a branch onto another without asking the user to stand on it first.
///
/// `git rebase <onto> <branch>` checks the branch out itself, so the trip is
/// git's rather than ours; all this adds is the way home and the stash around
/// it. As with a merge, a conflict keeps us there, because that is where the
/// rebase has to be finished.
pub fn rebase_branch(
    state: &AppState,
    branch: &str,
    onto: &str,
) -> Result<MergeOutcome, String> {
    let path = state.path()?;
    let original = journal::current_branch(state)
        .ok_or_else(|| "HEAD is detached; check out a branch first".to_string())?;
    let held = work::stash_before(state, &format!("rebasing {branch} onto {onto}"))?;

    let before = branch_oid(state, branch);
    let out = git_cmd::run(
        &path,
        &["-c", "merge.conflictStyle=diff3", "rebase", onto, branch],
    )?;
    let conflicts = crate::conflict::list(state).unwrap_or_default();

    let mut outcome = MergeOutcome {
        ok: out.ok && conflicts.is_empty(),
        message: format!("{}\n{}", out.stdout.trim(), out.stderr.trim())
            .trim()
            .to_string(),
        conflicts,
    };

    if !outcome.conflicts.is_empty() {
        outcome.message = format!(
            "{}\n\nYou are on {branch} with the rebase half-done. Resolve it here, then switch \
             back to {original}.",
            outcome.message.trim()
        );
        return Ok(outcome);
    }

    if outcome.ok {
        journal::record(
            state,
            "rebase",
            format!("Rebase {branch} onto {onto}"),
            Some(branch.to_string()),
            before,
            branch_oid(state, branch),
            Mode::Hard,
            true,
        );
        outcome.message = format!("{branch} rebased onto {onto}");
    }

    if original != branch {
        let back = git_cmd::run(&path, &["checkout", &original, "--"])?;
        if !back.ok {
            outcome.ok = false;
            outcome.message = format!(
                "{}\n\nCould not return to {original} — you are on {branch}.",
                outcome.message.trim()
            );
            return Ok(outcome);
        }
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

/// Commits in `head` that are not in `base` — the `base..head` range.
fn range(state: &AppState, base: &str, head: &str, limit: usize) -> Result<Vec<CommitSummary>, String> {
    let repo = state.repo()?;
    let base = git2::Oid::from_str(base).map_err(|_| "Bad revision".to_string())?;
    let head = git2::Oid::from_str(head).map_err(|_| "Bad revision".to_string())?;

    let mut walk = repo.revwalk().map_err(err)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME).map_err(err)?;
    walk.push(head).map_err(err)?;
    walk.hide(base).map_err(err)?;
    take(&repo, walk, limit)
}

fn range_from(state: &AppState, head: &str, limit: usize) -> Result<Vec<CommitSummary>, String> {
    let repo = state.repo()?;
    let head = git2::Oid::from_str(head).map_err(|_| "Bad revision".to_string())?;
    let mut walk = repo.revwalk().map_err(err)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME).map_err(err)?;
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

/// Replays the current branch on top of another.
///
/// Like a merge, this stashes uncommitted work first: a rebase refuses to start
/// with a dirty tree, and asking the user to tidy up by hand is the sort of
/// thing this application exists to avoid.
pub fn rebase(state: &AppState, onto: &str) -> Result<MergeOutcome, String> {
    let path = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    let held = work::stash_before(state, &format!("rebasing onto {onto}"))?;
    let out = git_cmd::run(&path, &["rebase", onto])?;
    let conflicts = crate::conflict::list(state).unwrap_or_default();

    // Only put the changes back once the rebase has finished; mid-rebase the
    // working tree belongs to git.
    let mut notes = Vec::new();
    if conflicts.is_empty() {
        match work::restore_after(state, held) {
            Ok(Some(note)) => notes.push(note),
            Err(error) => notes.push(error),
            Ok(None) => {}
        }
    } else if held.stashed {
        notes.push(
            "Your uncommitted changes are in the stash until the rebase finishes".to_string(),
        );
    }

    if out.ok && conflicts.is_empty() {
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

    let mut message = [out.stdout.trim(), out.stderr.trim()]
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    for note in notes {
        message = format!("{}\n{note}", message.trim());
    }

    Ok(MergeOutcome {
        ok: out.ok && conflicts.is_empty(),
        message: message.trim().to_string(),
        conflicts,
    })
}

pub fn abort_rebase(state: &AppState) -> Result<String, String> {
    let path = state.path()?;
    git_cmd::run_checked(&path, &["rebase", "--abort"])
}

/// Continues a rebase after its conflicts have been resolved and staged.
pub fn continue_rebase(state: &AppState) -> Result<MergeOutcome, String> {
    let path = state.path()?;
    let out = git_cmd::run(&path, &["-c", "core.editor=true", "rebase", "--continue"])?;
    let conflicts = crate::conflict::list(state).unwrap_or_default();
    Ok(MergeOutcome {
        ok: out.ok && conflicts.is_empty(),
        message: [out.stdout.trim(), out.stderr.trim()]
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
        conflicts,
    })
}

/// Whether git is part-way through something the user has to finish.
#[derive(Serialize)]
pub struct InProgress {
    pub merging: bool,
    pub rebasing: bool,
    pub cherry_picking: bool,
    pub reverting: bool,
    /// Files are conflicted but git is running nothing, which is what a `stash
    /// pop` leaves behind. There is no merge to abort in this state.
    pub restoring: bool,
}

pub fn in_progress(state: &AppState) -> Result<InProgress, String> {
    let root = state.path()?;
    let git_dir = root.join(".git");
    // Worktrees and submodules keep a file here instead of a directory.
    let git_dir = if git_dir.is_file() {
        std::fs::read_to_string(&git_dir)
            .ok()
            .and_then(|text| {
                text.strip_prefix("gitdir:")
                    .map(|p| std::path::PathBuf::from(p.trim()))
            })
            .unwrap_or(git_dir)
    } else {
        git_dir
    };

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
        restoring: !running && has_unmerged(&root),
    })
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
    git_cmd::run(&path, &["push", remote, "--delete", &format!("refs/tags/{tag}")])
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
        .and_then(|branch| repo.branch_upstream_remote(&format!("refs/heads/{branch}")).ok())
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
    git_cmd::run_checked(&path, &["remote", "get-url", remote])
        .map(|url| url.trim().to_string())
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
    Ok(format!("Renamed remote {from} to {to}; branches tracking {from}/ now track {to}/"))
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
