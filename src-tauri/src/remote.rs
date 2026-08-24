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
    let mut args = vec!["pull"];
    if rebase {
        args.push("--rebase");
    }
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

    Ok(InProgress {
        merging: git_dir.join("MERGE_HEAD").exists(),
        rebasing: git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists(),
        cherry_picking: git_dir.join("CHERRY_PICK_HEAD").exists(),
        reverting: git_dir.join("REVERT_HEAD").exists(),
    })
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

/// The remotes configured for this repository.
pub fn remotes(state: &AppState) -> Result<Vec<String>, String> {
    let repo = state.repo()?;
    let list = repo.remotes().map_err(|e| e.message().to_string())?;
    Ok(list.iter().flatten().map(|s| s.to_string()).collect())
}

/// Whether a fast-forward alone would bring `branch` up to `onto`.
///
/// Knowing this lets the drag-and-drop menu offer the cheap answer first
/// instead of making a merge commit nobody wanted.
pub fn can_fast_forward(state: &AppState, branch: &str, onto: &str) -> Result<bool, String> {
    let path = state.path()?;
    Ok(git_cmd::run_checked(&path, &["merge-base", "--is-ancestor", branch, onto]).is_ok())
}
