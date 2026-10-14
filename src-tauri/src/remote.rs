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
    /// Every commit a force push would drop has a copy in the branch being
    /// pushed — it was rebased, squashed or amended here. Pulling now would
    /// bring the old copies back next to the new ones, so forcing is the
    /// answer to lead with.
    pub rewritten: bool,
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
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
    let path = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    let flag = reconcile_flag(state, rebase);

    // diff3 for the resolver's base pane, and rerere so a conflict resolved
    // once is answered the same way the next time it comes up.
    let flags = crate::rebase::conflict_flags(state);
    let mut args: Vec<&str> = flags.iter().map(String::as_str).collect();
    args.extend(["pull", "--autostash", &flag]);
    let out = git_cmd::run(&path, &args)?;
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
    } else {
        // Stopped on conflicts: recorded by whatever finishes it.
        journal::begin_pending(state, "pull", "Pull".to_string(), branch, before);
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
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
    // The branch you are on is an ordinary pull.
    if journal::current_branch(state).as_deref() == Some(branch) {
        return pull(state, rebase);
    }

    let path = state.path()?;
    let (remote, theirs, tracking) = {
        let repo = state.repo()?;
        let local = repo
            .find_branch(branch, BranchType::Local)
            .map_err(|_| format!("No local branch named {branch}"))?;
        let upstream = local
            .upstream()
            .map_err(|_| format!("{branch} is not tracking anything to pull from"))?;
        let tracking = upstream
            .get()
            .name()
            .ok_or_else(|| format!("Could not read what {branch} tracks"))?
            .to_string();
        // Read from the config, so a remote with a slash in its name and a
        // branch called something else over there both come out right.
        let (remote, theirs) = crate::refs::configured_upstream(&repo, branch)
            .ok_or_else(|| format!("Could not read what {branch} tracks"))?;
        (remote, theirs, tracking)
    };
    let shown = tracking
        .strip_prefix("refs/remotes/")
        .unwrap_or(&tracking)
        .to_string();

    // A local upstream (`branch.x.remote = .`) has nothing to fetch.
    if remote != "." {
        let out = git_cmd::run(&path, &["fetch", &remote, "--", &theirs])?;
        if !out.ok {
            return Ok(MergeOutcome {
                ok: false,
                message: said(&out),
                conflicts: Vec::new(),
            });
        }
    }

    // Measured after the fetch, against what the remote has now. A branch
    // with nothing to take is left exactly where it is — the old way tried
    // to move it backwards, was refused as "non-fast-forward", and went the
    // long way round through a stash, a checkout and a pull to find that out.
    let (ahead, behind) = {
        let repo = state.repo()?;
        let mine = repo
            .find_branch(branch, BranchType::Local)
            .ok()
            .and_then(|b| b.get().target());
        let theirs_oid = repo.find_reference(&tracking).ok().and_then(|r| r.target());
        match (mine, theirs_oid) {
            (Some(mine), Some(theirs_oid)) => {
                repo.graph_ahead_behind(mine, theirs_oid).map_err(err)?
            }
            _ => return Err(format!("Could not read where {branch} or {shown} points")),
        }
    };

    if behind == 0 {
        return Ok(MergeOutcome {
            ok: true,
            message: match ahead {
                0 => format!("{branch} was already up to date"),
                n => format!(
                    "{branch} is {n} {} ahead of {shown}, with nothing to pull",
                    if n == 1 { "commit" } else { "commits" }
                ),
            },
            conflicts: Vec::new(),
        });
    }

    // It has commits of its own, so the update has to be a real merge, and a
    // merge needs the branch in the working tree.
    if ahead > 0 {
        return visit(
            state,
            branch,
            &format!("pulling {branch}"),
            &format!("{branch} brought up to date with {shown}"),
            |state| pull(state, rebase),
        );
    }

    // Strictly behind: the ref moves and nothing else does. Git refuses the
    // refspec unless it is a fast-forward, and refuses a branch checked out
    // in another worktree, so neither can be got wrong here. Not written into
    // the undo history, the same as before: undo steps the branch you are on,
    // and this one is not.
    let out = git_cmd::run(
        &path,
        &[
            "fetch",
            ".",
            "--",
            &format!("{tracking}:refs/heads/{branch}"),
        ],
    )?;
    Ok(MergeOutcome {
        ok: out.ok,
        message: if out.ok {
            format!("{branch} brought up to date with {shown}")
        } else {
            said(&out)
        },
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
/// Conflicts pause the trip rather than end it. A half-done merge can only be
/// finished in the working tree, so that is where you are left: on `target`,
/// with the resolver open on what clashed, and your own open work waiting in
/// the stash. The way home is written down (see [`come_home`]), and finishing
/// or aborting the merge or rebase takes it — back to where you started, with
/// your changes put back.
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
    let stash = held.stashed.then(|| journal::stash_ref(&path)).flatten();

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

    // An error is still a reason to go home: stopping here would strand the
    // user on `target` with their work in the stash and nobody saying so.
    let mut outcome = act(state).unwrap_or_else(|message| MergeOutcome {
        ok: false,
        message,
        conflicts: Vec::new(),
    });

    // Only a merge or rebase git is part-way through can be finished later.
    // Conflicts with nothing running fall through to the way home, which says
    // plainly where it stopped.
    let paused = git_dir(&path).ok().and_then(|dir| underway(&dir));
    if let (false, Some(underway)) = (outcome.conflicts.is_empty(), paused) {
        let trip = Trip {
            original: original.clone(),
            target: target.to_string(),
            stash,
            underway,
        };
        outcome.message = match write_trip(&path, &trip) {
            Ok(()) => format!(
                "{} stopped on conflicts in {}. You are on {target} to resolve them; finishing \
                 or aborting takes you back to {original}{}.",
                capitalised(reason),
                outcome.conflicts.join(", "),
                if held.stashed {
                    " and puts your open changes back"
                } else {
                    ""
                }
            ),
            Err(error) => format!(
                "{} stopped on conflicts in {}. You are on {target} to resolve them, and will \
                 have to go back to {original} yourself: {error}{}",
                capitalised(reason),
                outcome.conflicts.join(", "),
                if held.stashed {
                    " Your open changes are in the stash."
                } else {
                    ""
                }
            ),
        };
        return Ok(outcome);
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

/// Where a paused [`visit`] writes its way home, inside the git directory so
/// it goes wherever the repository goes and survives a restart.
const TRIP: &str = "gitnoob-visit";

/// A [`visit`] that stopped on conflicts: the branch it left, the one it is
/// standing on, the stash its open work went into, and which merge or rebase it
/// is waiting on.
pub struct Trip {
    original: String,
    target: String,
    /// The stash commit, so the one put back is this trip's own and not
    /// whatever happens to be on top of the list by then.
    stash: Option<String>,
    /// What [`underway`] said when the trip was written. The trip belongs to
    /// that merge or rebase and no other: one finished or abandoned in a
    /// terminal leaves it with nothing to wait for, and a later merge on the
    /// same branch is not the way home.
    underway: String,
}

/// The merge or rebase git is part-way through, as something that tells one
/// from another: the commit being merged, or where a rebase started and what
/// it is going onto. Read from git's own files, so it is the same on every
/// step of a rebase and gone the moment it ends.
fn underway(git_dir: &std::path::Path) -> Option<String> {
    let read = |name: &str| {
        std::fs::read_to_string(git_dir.join(name))
            .ok()
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|text| !text.is_empty())
    };
    if let Some(head) = read("MERGE_HEAD") {
        return Some(format!("merge {head}"));
    }
    ["rebase-merge", "rebase-apply"].iter().find_map(|dir| {
        let started = read(&format!("{dir}/orig-head"))?;
        let onto = read(&format!("{dir}/onto")).unwrap_or_default();
        Some(format!("rebase {started} {onto}"))
    })
}

fn write_trip(root: &std::path::Path, trip: &Trip) -> Result<(), String> {
    let file = git_dir(root)?.join(TRIP);
    let text = format!(
        "{}\n{}\n{}\n{}\n",
        trip.original,
        trip.target,
        trip.stash.as_deref().unwrap_or(""),
        trip.underway
    );
    std::fs::write(&file, text).map_err(|e| format!("Could not write {}: {e}", file.display()))
}

fn read_trip(git_dir: &std::path::Path) -> Option<Trip> {
    let text = std::fs::read_to_string(git_dir.join(TRIP)).ok()?;
    let mut lines = text.lines();
    Some(Trip {
        original: lines.next()?.to_string(),
        target: lines.next()?.to_string(),
        stash: Some(lines.next()?.trim().to_string()).filter(|oid| !oid.is_empty()),
        underway: lines.next()?.trim().to_string(),
    })
}

/// Where a paused [`visit`] stands, read before something that may end it.
pub enum Leg {
    /// No trip, or one waiting on something other than what git is doing now.
    None,
    /// The trip whose merge or rebase git is part-way through right now.
    Underway(Trip),
    /// A trip with nothing left to wait for: its merge or rebase was finished
    /// or abandoned outside the app.
    Stale(Trip),
}

/// The paused [`visit`], if there is one, and whether it is still waiting.
///
/// Read *before* a commit, continue or abort, and handed to [`come_home`]
/// after: whether that step is what ends the trip depends on what git was in
/// the middle of when it started, and that is gone by the time it is over.
pub fn trip(state: &AppState) -> Leg {
    let Ok(root) = state.path() else {
        return Leg::None;
    };
    trip_at(&root)
}

/// [`trip`] in a work tree read once by the caller.
pub fn trip_at(root: &std::path::Path) -> Leg {
    let Ok(git_dir) = git_dir(root) else {
        return Leg::None;
    };
    let Some(trip) = read_trip(&git_dir) else {
        return Leg::None;
    };
    match underway(&git_dir) {
        Some(now) if now == trip.underway => Leg::Underway(trip),
        Some(_) => Leg::None,
        None => Leg::Stale(trip),
    }
}

/// The branch a paused [`visit`] goes back to once its merge or rebase is done.
pub fn returns_to(state: &AppState) -> Option<String> {
    returns_to_at(&state.path().ok()?)
}

fn returns_to_at(root: &std::path::Path) -> Option<String> {
    match trip_at(root) {
        Leg::Underway(trip) => Some(trip.original),
        _ => None,
    }
}

/// Finishes a paused [`visit`], once there is nothing left to finish.
///
/// Called after everything that can end a merge or a rebase — a commit, a
/// continue, a skip, an abort — with what [`trip`] said before it ran. While
/// git is still part-way through, the trip waits. Once it is done, you are
/// taken back to the branch you started on and your open work comes out of the
/// stash. Answers with what to tell the user, or nothing when there was no
/// trip to finish.
///
/// A trip whose merge ended somewhere else, or one you have wandered off on
/// your own in the meantime, is simply forgotten: being moved somewhere then
/// would be the surprise. The stash is left for you to pick up, and you are
/// told where it is.
pub fn come_home(state: &AppState, leg: Leg) -> Option<String> {
    come_home_at(&state.path().ok()?, leg)
}

/// [`come_home`] in a work tree read once by the caller, before whatever ran
/// in between: a commit with a slow hook outlasts a tab switch.
pub fn come_home_at(root: &std::path::Path, leg: Leg) -> Option<String> {
    let root = root.to_path_buf();
    let (trip, stale) = match leg {
        Leg::None => return None,
        Leg::Underway(trip) => (trip, false),
        Leg::Stale(trip) => (trip, true),
    };
    let progress = in_progress_at(&root).ok()?;
    if progress.merging
        || progress.rebasing
        || progress.cherry_picking
        || progress.reverting
        || has_unmerged(&root)
    {
        // Still going: either the very same merge or rebase, which the trip
        // keeps waiting on, or a stale trip that can wait for another time.
        return None;
    }
    let _ = std::fs::remove_file(git_dir(&root).ok()?.join(TRIP));

    let stash_at = trip
        .stash
        .as_deref()
        .and_then(|oid| journal::stash_index_at(&root, oid));
    let left_in_stash = format!("Your open changes from {} are in the stash.", trip.original);
    if stale || journal::branch_at(&root).as_deref() != Some(trip.target.as_str()) {
        return stash_at.map(|_| left_in_stash);
    }

    let back = match git_cmd::run(&root, &["switch", "--", &trip.original]) {
        Ok(back) => back,
        Err(error) => return Some(format!("Could not return to {}: {error}", trip.original)),
    };
    if !back.ok {
        let mut message = format!(
            "Could not return to {}: {}",
            trip.original,
            back.stderr.trim()
        );
        if stash_at.is_some() {
            message = format!("{message}\n{left_in_stash}");
        }
        return Some(message);
    }

    let note = match (trip.stash.is_some(), stash_at) {
        (false, _) => return Some(format!("Back on {}", trip.original)),
        // Popped or dropped by hand while the trip was paused.
        (true, None) => format!(
            "The stash holding your open changes from {} is no longer on the list.",
            trip.original
        ),
        (true, Some(at)) => match work::pop_stash(&root, &format!("stash@{{{at}}}")) {
            Ok(_) => "Local changes were stashed and put back".to_string(),
            Err(error) => format!(
                "Your changes are safe in the stash, but putting them back hit a problem: {error}"
            ),
        },
    };
    Some(format!("Back on {}. {note}", trip.original))
}

/// `text`, with whatever [`come_home`] had to say underneath it.
pub fn and_home(state: &AppState, leg: Leg, text: String) -> String {
    match come_home(state, leg) {
        Some(note) => format!("{}\n{note}", text.trim()),
        None => text,
    }
}

/// [`and_home`] in a work tree read once by the caller.
pub fn and_home_at(root: &std::path::Path, leg: Leg, text: String) -> String {
    match come_home_at(root, leg) {
        Some(note) => format!("{}\n{note}", text.trim()),
        None => text,
    }
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

    // The remote to push to: the upstream's remote, else the first configured
    // one, else the conventional name.
    let remote = crate::refs::configured_upstream(&repo, &branch_name)
        .map(|(remote, _)| remote)
        .filter(|remote| remote != ".")
        .or_else(|| {
            repo.remotes()
                .ok()
                .and_then(|list| list.get(0).map(|s| s.to_string()))
        })
        .unwrap_or_else(|| "origin".to_string());

    // Measured against where the push actually lands, which is not always the
    // upstream: a branch that follows `origin/main` but is not `main` is
    // pushed under its own name (see [`push_target`]), so `main` is neither
    // what it would overwrite nor what a lease should be pinned to.
    let (dest, _) = push_target(&repo, &remote, &branch_name);
    let upstream_name = Some(format!("{remote}/{dest}"));
    let upstream_oid = repo
        .find_reference(&format!("refs/remotes/{remote}/{dest}"))
        .ok()
        .and_then(|r| r.target());
    let upstream_name = upstream_name.filter(|_| upstream_oid.is_some());

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
    let rewritten = match upstream_oid {
        Some(up) if behind > 0 && ahead > 0 => {
            crate::graph::carried(&repo, up, local_oid).is_some()
        }
        _ => false,
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
        rewritten,
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
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
    let path = state.path()?;
    let (dest, retarget) = push_target(&state.repo()?, remote, branch);
    let dest = format!("refs/heads/{dest}");
    let with_lease = match lease {
        Some(expected) => format!("--force-with-lease={dest}:{expected}"),
        None => "--force-with-lease".to_string(),
    };
    let refspec = format!("refs/heads/{branch}:{dest}");
    let mut args = vec!["push"];
    if force {
        args.push(&with_lease);
    }
    // A branch that follows nothing yet — or something other than where it
    // is going — is told to follow what it is pushed to, whether or not the
    // caller remembered to ask: a push that leaves the branch without an
    // upstream is a branch whose counts and Pull say nothing afterwards.
    if set_upstream || retarget {
        args.push("--set-upstream");
    }
    args.push(remote);
    args.push(&refspec);
    git_cmd::run(&path, &args)
}

/// Where a push of `branch` to `remote` lands, as a branch name on that
/// remote, and whether the branch has to be told to follow it afterwards.
///
/// The same name as here, the way git's own `push.default=simple` does it.
/// The one exception is a branch checked out from a review on a fork, which
/// gitnoob names `alice-main` locally while it follows `main` on the remote
/// it added for alice: pushing that one has to update the review, not start a
/// new branch beside it, so it goes to the branch it follows.
///
/// Following the upstream's name everywhere would be wrong in a way that
/// costs other people's work. A branch started from `origin/main` — which
/// earlier versions of this app did without `--no-track`, as does
/// `git checkout -b feature origin/main` in a terminal — follows `main`, and a
/// push of it would land on `main`, with a force push replacing it. Such a
/// branch goes to its own name instead and is set to follow that, which is
/// what its first push would have done if it had been started cleanly.
pub fn push_target(repo: &git2::Repository, remote: &str, branch: &str) -> (String, bool) {
    match crate::refs::configured_upstream(repo, branch) {
        Some((tracked, merge)) if tracked == remote && merge == branch => (merge, false),
        Some((tracked, merge)) if tracked == remote && is_review_remote(repo, remote) => {
            (merge, false)
        }
        _ => (branch.to_string(), true),
    }
}

/// Whether gitnoob added this remote to check out a review from a fork.
fn is_review_remote(repo: &git2::Repository, remote: &str) -> bool {
    repo.config()
        .and_then(|config| config.get_bool(&format!("remote.{remote}.{REVIEW_REMOTE}")))
        .unwrap_or(false)
}

/// Merges a branch into the checked out one, carrying open work on git's own
/// `--autostash` the way [`pull`] does.
pub fn merge(state: &AppState, branch: &str, no_ff: bool) -> Result<MergeOutcome, String> {
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
    let path = state.path()?;
    // diff3 keeps the merge base in the conflict markers, which the conflict
    // resolver shows as its third pane; rerere answers a conflict that was
    // resolved before the way it was resolved then.
    let flags = crate::rebase::conflict_flags(state);
    let mut args: Vec<&str> = flags.iter().map(String::as_str).collect();
    args.extend(["merge", "--autostash"]);
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
    } else {
        // Stopped on conflicts: recorded by whatever finishes it.
        journal::begin_pending(state, "merge", format!("Merge {branch}"), on, before);
    }
    Ok(outcome)
}

pub fn abort_merge(state: &AppState) -> Result<String, String> {
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
    let path = state.path()?;
    let leg = trip(state);
    let waiting = journal::pending(state);
    let out = git_cmd::run_checked(&path, &["merge", "--abort"])?;
    journal::finish_pending(state, waiting);
    Ok(and_home(state, leg, out))
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
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
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
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
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
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
    let path = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    // Branches stacked on this one ride along (`--update-refs`), so a stack
    // of reviews is rebased in one go rather than left behind on the old
    // base. Which ones is written down first: the rebase can stop, and the
    // continue that finishes it is what gets to say which of them moved.
    //
    // Git's `--update-refs` cannot be told to leave a branch out, and it moves
    // every one on the replayed commits: a backup made at the tip, a
    // colleague's branch checked out to look at. When one of those is there,
    // the stack is left where it is too rather than take them along.
    let (mut carried, held) = crate::rebase::carried_and_held(state, Some(onto));
    let update_refs = crate::rebase::update_refs_on(state) && held.is_empty();
    let mut left = None;
    if !update_refs && !carried.is_empty() {
        let names: Vec<&str> = carried.iter().map(|(name, _)| name.as_str()).collect();
        left = Some(format!(
            "Left {} where {} — moving {} would have moved {} as well.",
            names.join(", "),
            if names.len() == 1 {
                "it was"
            } else {
                "they were"
            },
            if names.len() == 1 { "it" } else { "them" },
            held.join(", ")
        ));
        carried.clear();
    }
    let flags = crate::rebase::conflict_flags(state);
    let mut args: Vec<&str> = flags.iter().map(String::as_str).collect();
    args.extend(["rebase", "--autostash"]);
    if update_refs {
        args.push("--update-refs");
    } else if !held.is_empty() {
        // Off whatever `rebase.updateRefs` says, for the same reason. Only
        // reached on a git that knows the flag: `held` is empty otherwise.
        args.push("--no-update-refs");
    }
    args.push(onto);
    let dir = git_dir(&path)?;
    crate::rebase::remember_carried(&dir, &carried);

    let out = git_cmd::run(&path, &args)?;
    let moved = out.ok;
    let mut outcome = settled(state, out);
    if underway(&dir).is_none() {
        outcome.message = crate::rebase::with_carried(state, &dir, outcome.message);
    }
    if let Some(left) = left {
        outcome.message = format!("{}\n{left}", outcome.message.trim_end());
    }

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
    } else {
        // Stopped on conflicts: recorded by whatever finishes it.
        journal::begin_pending(
            state,
            "rebase",
            format!("Rebase onto {onto}"),
            branch,
            before,
        );
    }
    Ok(outcome)
}

pub fn abort_rebase(state: &AppState) -> Result<String, String> {
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
    let path = state.path()?;
    let leg = trip(state);
    let waiting = journal::pending(state);
    crate::rewrite::clear_for_abort(state);
    let out = git_cmd::run_checked(&path, &["rebase", "--abort"])?;
    journal::finish_pending(state, waiting);
    // Nothing was carried: git moves the stacked branches only at the end.
    if let Ok(dir) = git_dir(&path) {
        crate::rebase::remember_carried(&dir, &[]);
        crate::rebase::clear_marks(&dir);
    }
    Ok(and_home(state, leg, out))
}

/// Continues a rebase after its conflicts have been resolved and staged.
pub fn continue_rebase(state: &AppState) -> Result<MergeOutcome, String> {
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
    let path = state.path()?;
    let leg = trip(state);
    let waiting = journal::pending(state);
    let flags = crate::rebase::conflict_flags(state);
    let mut args: Vec<&str> = flags.iter().map(String::as_str).collect();
    args.extend(["-c", "core.editor=true", "rebase", "--continue"]);
    let out = git_cmd::run(&path, &args)?;
    journal::finish_pending(state, waiting);
    let mut outcome = settled(state, out);
    if let Ok(dir) = git_dir(&path) {
        if underway(&dir).is_none() {
            outcome.message = crate::rebase::with_carried(state, &dir, outcome.message);
        }
    }
    if outcome.ok {
        outcome.message = and_home(state, leg, outcome.message);
    }
    Ok(outcome)
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
    /// The branch a merge or rebase started from another branch goes back to
    /// once it is finished or aborted.
    pub returns_to: Option<String>,
    /// A cherry-pick or revert of several commits that is part-way through:
    /// `"cherry-pick"` or `"revert"`, so the way on is named rightly. Set
    /// while git's sequencer has a list, which outlives the conflict itself —
    /// committing the stopped commit by hand leaves the rest still to apply.
    pub sequencer: Option<String>,
    /// How many commits that list still has to apply, the stopped one included.
    pub sequence_left: usize,
    /// Something that names the merge, rebase, pick or stash apply under way,
    /// the same on every look for as long as it lasts. The resolver files the
    /// choices it remembers under it, and drops them once it changes.
    pub operation: Option<String>,
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
    in_progress_at(&state.path()?)
}

/// [`in_progress`] in a work tree read once by the caller.
pub fn in_progress_at(root: &std::path::Path) -> Result<InProgress, String> {
    let root = root.to_path_buf();
    let git_dir = git_dir(&root)?;

    let merging = git_dir.join("MERGE_HEAD").exists();
    let rebasing = git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists();
    let (sequencer, sequence_left) = match work::sequence(&git_dir) {
        Some((kind, left)) => (Some(kind), left),
        None => (None, 0),
    };
    let cherry_picking =
        git_dir.join("CHERRY_PICK_HEAD").exists() || sequencer.as_deref() == Some("cherry-pick");
    let reverting = git_dir.join("REVERT_HEAD").exists() || sequencer.as_deref() == Some("revert");
    let running = merging || rebasing || cherry_picking || reverting;
    let applied_stash = (!running).then(|| work::applied_stash(&root)).flatten();

    Ok(InProgress {
        merging,
        rebasing,
        cherry_picking,
        reverting,
        restoring: !running && has_unmerged(&root) && work::auto_stash_on_top(&root),
        operation: operation_of(&git_dir, applied_stash.as_deref()),
        applied_stash,
        prepared: prepared_message(&git_dir),
        returns_to: returns_to_at(&root),
        sequencer,
        sequence_left,
    })
}

/// A name for what git is part-way through, stable for as long as it lasts.
///
/// Built from git's own files: the commit being merged, where a rebase
/// started, where a run of picks started, the stash being applied. Two
/// different operations practically never share one.
fn operation_of(git_dir: &std::path::Path, applied_stash: Option<&str>) -> Option<String> {
    if let Some(found) = underway(git_dir) {
        return Some(found);
    }
    let read = |name: &str| {
        std::fs::read_to_string(git_dir.join(name))
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    };
    if let Some(head) = read("sequencer/head") {
        return Some(format!("sequence {head}"));
    }
    if let Some(oid) = read("CHERRY_PICK_HEAD") {
        return Some(format!("cherry-pick {oid}"));
    }
    if let Some(oid) = read("REVERT_HEAD") {
        return Some(format!("revert {oid}"));
    }
    applied_stash.map(|oid| format!("stash {oid}"))
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
    // Spelled out, like the tag delete below: a tag with the branch's name
    // would otherwise make `--delete <name>` ambiguous, or take the tag.
    git_cmd::run(
        &path,
        &["push", remote, "--delete", &format!("refs/heads/{branch}")],
    )
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
///
/// A remote gitnoob added to check out a review is never it either, even
/// while a branch from that review is checked out and tracks it: the fetch on
/// a timer would otherwise ask only the fork, and the trunk this repository
/// is measured against would quietly stop moving.
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
        .and_then(|buf| buf.as_str().map(|s| s.to_string()))
        .filter(|remote| !is_review_remote(&repo, remote));
    if tracked.is_some() {
        return tracked;
    }

    let list = repo.remotes().ok()?;
    let names: Vec<String> = list.iter().flatten().map(|s| s.to_string()).collect();
    let own: Vec<&String> = names
        .iter()
        .filter(|name| !is_review_remote(&repo, name))
        .collect();
    own.iter()
        .find(|name| name.as_str() == "origin")
        .or_else(|| own.first())
        .map(|name| name.to_string())
        .or_else(|| names.first().cloned())
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
    /// Whether merging `source` into `target` would go through on its own:
    /// worked out in memory, before anything is dropped. `None` when it could
    /// not be asked — a git older than 2.38, or nothing to merge.
    pub clean: Option<bool>,
    /// The files that merge would stop on, when it would.
    pub conflicts: Vec<String>,
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
    drop(repo);
    // Only worth asking when there is something to bring over and the two have
    // both moved: a fast-forward cannot conflict.
    let trial = if ahead > 0 && behind > 0 {
        trial_merge(state, source_oid, target_oid)
    } else {
        None
    };
    Ok(BranchRelation {
        ahead,
        behind,
        clean: match (&trial, ahead > 0 && behind == 0) {
            (_, true) => Some(true),
            (Some(conflicts), _) => Some(conflicts.is_empty()),
            (None, _) => None,
        },
        conflicts: trial.unwrap_or_default(),
    })
}

/// Merges two commits in memory and says which files would conflict, without
/// touching the index, the working tree or any ref.
///
/// `git merge-tree --write-tree` (git 2.38 and later) does the whole merge in
/// the object store: it writes the result as a tree and stops there. Exit 0 is
/// clean, 1 is conflicted with the files listed. Anything else — an older git
/// that does not know the flag, histories with nothing in common — is "do not
/// know", and the menu says nothing rather than guess.
fn trial_merge(state: &AppState, source: git2::Oid, target: git2::Oid) -> Option<Vec<String>> {
    let path = state.path().ok()?;
    let out = git_cmd::run(
        &path,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            "-z",
            &target.to_string(),
            &source.to_string(),
        ],
    )
    .ok()?;
    match out.code {
        0 => Some(Vec::new()),
        1 => Some(conflicted_names(&out.stdout)),
        _ => None,
    }
}

/// The file names `merge-tree -z --name-only` lists after the tree it wrote.
fn conflicted_names(stdout: &str) -> Vec<String> {
    let mut names: Vec<String> = stdout
        .split('\0')
        .map(|part| part.trim_matches('\n'))
        .filter(|part| !part.is_empty())
        // The first entry is the id of the tree the merge wrote.
        .skip(1)
        .map(String::from)
        .collect();
    names.dedup();
    names
}

/// The local trunk moved up to where its remote has it, when that is safe.
#[derive(Serialize, Debug)]
pub struct TrunkMove {
    pub branch: String,
    pub upstream: String,
    pub commits: usize,
}

/// After a fetch, brings the local trunk up to its upstream — but only by
/// fast-forward, and only when nobody is standing on it.
///
/// A local `main` nobody has pulled in weeks is what every "is it merged?" and
/// every "branch from main" was quietly measured against. Moving it is safe
/// exactly when it has nothing of its own (a fast-forward cannot lose a
/// commit) and no worktree has it checked out (moving a checked-out branch
/// under its files would leave them looking like changes). Either test
/// failing leaves it alone, and so does git: `fetch .` refuses a refspec that
/// is not a fast-forward and a branch checked out anywhere.
///
/// Not written into the undo history. It is housekeeping the user did not ask
/// for in the moment, and putting it on top of the stack would make ⌘Z undo
/// it instead of whatever they did last.
pub fn keep_trunk_current(state: &AppState) -> Result<Option<TrunkMove>, String> {
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
    if !state.config().global.keep_trunk_current {
        return Ok(None);
    }
    let path = state.path()?;
    let (branch, tracking, shown, commits) = {
        let repo = state.repo()?;
        let Some(trunk) = crate::refs::trunk_of(&repo) else {
            return Ok(None);
        };
        let Ok(local) = repo.find_branch(&trunk, BranchType::Local) else {
            return Ok(None);
        };
        let Ok(upstream) = local.upstream() else {
            return Ok(None);
        };
        let (Some(mine), Some(theirs)) = (local.get().target(), upstream.get().target()) else {
            return Ok(None);
        };
        let Some(tracking) = upstream.get().name().map(String::from) else {
            return Ok(None);
        };
        let (ahead, behind) = repo.graph_ahead_behind(mine, theirs).map_err(err)?;
        if ahead > 0 || behind == 0 {
            return Ok(None);
        }
        let shown = tracking
            .strip_prefix("refs/remotes/")
            .unwrap_or(&tracking)
            .to_string();
        (trunk, tracking, shown, behind)
    };
    let checked_out = crate::worktree::list(state)?
        .iter()
        .any(|tree| tree.branch.as_deref() == Some(branch.as_str()));
    // A trunk being rebased in some worktree is listed there as a detached
    // HEAD, so the test above does not see it, and git refuses the move
    // below. That is the right answer, but not a failure worth a line in the
    // log on every fetch until the rebase ends.
    if checked_out || being_rebased(&state.repo()?, &branch) {
        return Ok(None);
    }
    let out = git_cmd::run(
        &path,
        &[
            "fetch",
            ".",
            "--",
            &format!("{tracking}:refs/heads/{branch}"),
        ],
    )?;
    if !out.ok {
        return Err(said(&out));
    }
    Ok(Some(TrunkMove {
        branch,
        upstream: shown,
        commits,
    }))
}

/// Whether a rebase of `branch` is under way in any worktree of the
/// repository: git writes the branch it will come back to into
/// `rebase-merge/head-name` (or `rebase-apply/head-name`) of that worktree's
/// own git directory.
fn being_rebased(repo: &git2::Repository, branch: &str) -> bool {
    let common = repo.commondir().to_path_buf();
    let mut dirs = vec![common.clone()];
    if let Ok(trees) = std::fs::read_dir(common.join("worktrees")) {
        dirs.extend(trees.flatten().map(|tree| tree.path()));
    }
    let wanted = format!("refs/heads/{branch}");
    dirs.iter().any(|dir| {
        ["rebase-merge", "rebase-apply"].iter().any(|sub| {
            std::fs::read_to_string(dir.join(sub).join("head-name"))
                .map(|name| name.trim() == wanted)
                .unwrap_or(false)
        })
    })
}

/// Fetches the repository's own remote and nothing else.
///
/// For the fetch on a timer. `fetch --all` asks every remote, and one that is
/// gone — a fork added to look at a review, deleted since — made every
/// fetch fail, every ten minutes, with a notice about a remote the user had
/// forgotten existed. The branches being worked on live on this one.
pub fn fetch_primary(state: &AppState) -> Result<CmdOutput, String> {
    let primary = primary(state);
    fetch(state, primary.as_deref())
}

/// Brings a branch up to date with the trunk as the remote has it: fetches,
/// then rebases onto it or merges it in.
///
/// `how` is `rebase`, `merge`, or `auto` (the setting's default): a branch
/// nobody else has seen is rebased, because that keeps its history a straight
/// line and costs nobody anything; one that is already on its remote gets a
/// merge, because rebasing it would mean a force push over a branch somebody
/// may have pulled. Both go the usual way — open work carried on autostash, a
/// branch not checked out visited and come back from, conflicts left for the
/// resolver with the way home written down.
pub fn update_from_trunk(
    state: &AppState,
    branch: &str,
    how: Option<&str>,
) -> Result<MergeOutcome, String> {
    // Read once: see [`AppState::pinned`].
    let pinned = state.pinned();
    let state = &pinned;
    let target = crate::refs::trunk_target(state)
        .ok_or_else(|| "There is no main branch here to update from".to_string())?;
    let (trunk, remote) = {
        let repo = state.repo()?;
        let trunk = crate::refs::trunk_of(&repo);
        let remote = crate::refs::remote_part(&repo, &target)
            .filter(|_| repo.find_branch(&target, BranchType::Remote).is_ok());
        (trunk, remote)
    };
    if trunk.as_deref() == Some(branch) || target == branch {
        return Err(format!(
            "{branch} is the main branch itself — pull it instead"
        ));
    }

    let mut note = String::new();
    if let Some((remote, theirs)) = &remote {
        let path = state.path()?;
        let out = git_cmd::run(&path, &["fetch", remote, "--", theirs])?;
        if !out.ok {
            note = format!(
                "\nCould not fetch {remote}, so this used what was already here: {}",
                out.stderr.trim()
            );
        }
    }

    let relation = relation(state, &target, branch)?;
    if relation.ahead == 0 {
        return Ok(MergeOutcome {
            ok: true,
            message: format!("{branch} already has everything on {target}{note}"),
            conflicts: Vec::new(),
        });
    }

    let configured = state.config().global.update_from_trunk;
    let how = how.unwrap_or(configured.as_str());
    let pushed = crate::refs::upstream_of(state, branch).is_some();
    let rebase = match how {
        "rebase" => true,
        "merge" => false,
        _ => !pushed,
    };
    let mut outcome = if rebase {
        rebase_branch(state, branch, &target)?
    } else {
        merge_into(state, &target, branch, false)?
    };
    if outcome.ok {
        outcome.message = if rebase {
            format!("{branch} rebased onto {target}")
        } else {
            format!("{target} merged into {branch}")
        };
    }
    outcome.message = format!("{}{note}", outcome.message);
    Ok(outcome)
}

/// The config key that marks a remote as one gitnoob added to check out a
/// review from a fork, so it can be offered for removal once it stops
/// answering. Removing the remote removes the key with it.
const REVIEW_REMOTE: &str = "gitnoob-review";

/// Marks a remote as added for a review.
pub fn mark_review_remote(state: &AppState, name: &str) -> Result<(), String> {
    let path = state.path()?;
    git_cmd::run_checked(
        &path,
        &["config", &format!("remote.{name}.{REVIEW_REMOTE}"), "true"],
    )
    .map(|_| ())
}

/// The remotes gitnoob added for reviews and that are still configured.
pub fn review_remotes(state: &AppState) -> Vec<String> {
    let Ok(repo) = state.repo() else {
        return Vec::new();
    };
    let Ok(config) = repo.config().and_then(|mut c| c.snapshot()) else {
        return Vec::new();
    };
    let suffix = format!(".{REVIEW_REMOTE}");
    let mut found = Vec::new();
    if let Ok(entries) = config.entries(Some(r"^remote\..*\.gitnoob-review$")) {
        let mut entries = entries;
        while let Some(Ok(entry)) = entries.next() {
            if let Some(name) = entry
                .name()
                .and_then(|key| key.strip_prefix("remote."))
                .and_then(|rest| rest.strip_suffix(&suffix))
            {
                found.push(name.to_string());
            }
        }
    }
    found.sort();
    found.dedup();
    found
}
