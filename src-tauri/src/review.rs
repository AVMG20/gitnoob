//! Checking out somebody else's review.
//!
//! A review opened from a branch in this repository is an ordinary checkout.
//! A review opened from a fork is not: its branch is in a repository this
//! clone has never spoken to, so there is no `origin/their-branch` to track
//! and `git checkout their-branch` fails with "pathspec did not match". That
//! error is the whole reason this module exists — the answer is to add the
//! fork as a remote, fetch the one branch, and check that out, which is what a
//! person would type by hand.

use serde::{Deserialize, Serialize};

use crate::config::ForgeKind;
use crate::git_cmd;
use crate::refs;
use crate::remote;
use crate::state::AppState;

/// A review, as much of it as checking one out needs.
#[derive(Deserialize)]
pub struct ReviewTarget {
    pub number: i64,
    /// The branch name inside the repository it lives in, without any owner
    /// prefix: `fix-typo`, not `them:fix-typo`.
    pub branch: String,
    /// The tip the forge last reported. Used only when the branch itself can
    /// no longer be fetched.
    pub head_sha: String,
    /// Where the branch lives, when the forge still has it.
    pub source: Option<Source>,
}

#[derive(Deserialize)]
pub struct Source {
    pub owner: String,
    pub ssh_url: String,
    pub https_url: String,
    /// False for a branch in the repository being reviewed, which needs no
    /// remote adding.
    pub is_fork: bool,
}

/// Checks out a review, doing whatever it takes to have the commits first.
pub fn checkout(state: &AppState, review: ReviewTarget) -> Result<String, String> {
    let path = state.path()?;

    let fork = match &review.source {
        Some(source) if source.is_fork => source,
        // The branch is in this repository. It may still be a branch this
        // clone has not fetched since it was opened, so a miss is worth one
        // fetch before giving up.
        _ => {
            if !known_locally(state, &review.branch) {
                let _ = remote::fetch(state, remote::primary(state).as_deref());
            }
            // The message is the whole answer here: a review checkout of a
            // diverged branch has no banner to hand the question to, so
            // whatever the checkout said about the state is what gets shown.
            return refs::checkout(state, &review.branch).map(|outcome| outcome.message);
        }
    };

    // The fork is gone but the forge kept the review: the commits are still in
    // the upstream repository, under a ref only the forge writes.
    if fork.ssh_url.is_empty() && fork.https_url.is_empty() {
        return from_forge_ref(state, &review);
    }

    let url = pick_url(state, fork);
    let name = remote_named(state, &fork.owner, &url)?;
    git_cmd::run_checked(&path, &["fetch", &name, &review.branch])?;

    let branch = &review.branch;
    let tracking = format!("{name}/{branch}");
    let local = local_name(state, &name, branch);
    if refs::has_local_branch(state, &local) {
        return refs::checkout(state, &local).map(|outcome| outcome.message);
    }
    refs::checkout_tracking(state, &local, &tracking)
}

/// Whether the name already means something here, so a fetch can be skipped
/// when it would tell us nothing new.
fn known_locally(state: &AppState, branch: &str) -> bool {
    refs::has_local_branch(state, branch) || refs::has_remote_branch(state, branch)
}

/// ssh or https, matching how this clone already talks to its own remote. A
/// clone made over https has no key to offer a fork, and one made over ssh
/// would be asked for a password by an https URL.
fn pick_url(state: &AppState, fork: &Source) -> String {
    let over_ssh = remote::primary(state)
        .and_then(|name| remote::remote_url(state, &name).ok())
        .map(|url| url.starts_with("git@") || url.starts_with("ssh://"))
        .unwrap_or(false);
    let (first, second) = if over_ssh {
        (&fork.ssh_url, &fork.https_url)
    } else {
        (&fork.https_url, &fork.ssh_url)
    };
    if first.is_empty() {
        second.clone()
    } else {
        first.clone()
    }
}

/// The remote to fetch the fork from: the one already pointing at it if there
/// is one, otherwise a new one named after whoever owns it.
fn remote_named(state: &AppState, owner: &str, url: &str) -> Result<String, String> {
    let existing = remote::remotes(state)?;
    for name in &existing {
        if let Ok(configured) = remote::remote_url(state, name) {
            if same_repository(&configured, url) {
                return Ok(name.clone());
            }
        }
    }

    // Remote names share their rules with branch names, and forge owners can
    // hold characters that break both.
    let cleaned: String = owner
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let base = cleaned.trim_matches(['-', '.']).to_string();
    let base = if base.is_empty() {
        "fork".to_string()
    } else {
        base
    };

    let mut name = base.clone();
    let mut suffix = 2;
    while existing.contains(&name) {
        name = format!("{base}-{suffix}");
        suffix += 1;
    }
    remote::remote_add(state, &name, url)?;
    // Remembered, so a fetch that later fails on it can offer to take it off.
    let _ = remote::mark_review_remote(state, &name);
    Ok(name)
}

/// Whether two addresses are the same repository written differently: with or
/// without `.git`, over ssh or https, with or without a trailing slash.
fn same_repository(one: &str, two: &str) -> bool {
    fn shape(url: &str) -> String {
        let url = url.trim().trim_end_matches('/');
        let url = url.strip_suffix(".git").unwrap_or(url);
        let url = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
            .or_else(|| url.strip_prefix("ssh://git@"))
            .or_else(|| url.strip_prefix("git@"))
            .unwrap_or(url);
        // `git@host:owner/name` and `https://host/owner/name` differ by one
        // character once the scheme is off.
        url.replacen(':', "/", 1).to_lowercase()
    }
    !one.trim().is_empty() && shape(one) == shape(two)
}

/// What to call the branch here. The branch's own name when it is free — that
/// is what everyone calls it — and prefixed with whose it is when it is not,
/// so a fork's `main` never lands on top of yours.
fn local_name(state: &AppState, remote_name: &str, branch: &str) -> String {
    let tracking = format!("{remote_name}/{branch}");
    // Free, or already this very review from an earlier checkout: either way
    // the branch's own name is the one to use.
    let is_the_review =
        |name: &str| refs::upstream_of(state, name).as_deref() == Some(tracking.as_str());
    if !refs::has_local_branch(state, branch) || is_the_review(branch) {
        return branch.to_string();
    }

    // Taken by something of ours. Say whose the other one is rather than
    // landing a fork's `main` on top of the `main` here.
    let leaf = branch.rsplit('/').next().unwrap_or(branch);
    let owned = format!("{remote_name}-{leaf}");
    let mut name = owned.clone();
    let mut suffix = 2;
    while refs::has_local_branch(state, &name) && !is_the_review(&name) {
        name = format!("{owned}-{suffix}");
        suffix += 1;
    }
    name
}

/// The last resort: the copy of the review the forge itself keeps in the
/// upstream repository, which outlives the fork it came from.
fn from_forge_ref(state: &AppState, review: &ReviewTarget) -> Result<String, String> {
    let path = state.path()?;
    let remote_name = remote::primary(state)
        .ok_or_else(|| "This repository has no remote to fetch the review from".to_string())?;
    let kind = state.config().active().map(|profile| profile.forge);
    let number = review.number;
    let specs: Vec<String> = match kind {
        Some(ForgeKind::GitLab) => vec![format!("refs/merge-requests/{number}/head")],
        Some(ForgeKind::GitHub) => vec![format!("refs/pull/{number}/head")],
        // No profile to ask: try the one, then the other.
        _ => vec![
            format!("refs/pull/{number}/head"),
            format!("refs/merge-requests/{number}/head"),
        ],
    };

    let mut last = String::new();
    for spec in &specs {
        match git_cmd::run_checked(&path, &["fetch", &remote_name, spec]) {
            Ok(_) => {
                let local = free_name(state, &format!("review-{number}"));
                // FETCH_HEAD rather than the reported sha: the fetch just put
                // it there, and the forge's sha can be a merge commit the
                // fetch did not bring.
                return refs::checkout_at(state, &local, "FETCH_HEAD");
            }
            Err(error) => last = error,
        }
    }

    if !review.head_sha.is_empty() && refs::has_commit(state, &review.head_sha) {
        let local = free_name(state, &format!("review-{number}"));
        return refs::checkout_at(state, &local, &review.head_sha);
    }
    Err(format!(
        "The branch this review came from has been deleted, and the review could not be fetched from {remote_name}: {last}"
    ))
}

fn free_name(state: &AppState, base: &str) -> String {
    let mut name = base.to_string();
    let mut suffix = 2;
    while refs::has_local_branch(state, &name) {
        name = format!("{base}-{suffix}");
        suffix += 1;
    }
    name
}

// --- after a merge ------------------------------------------------------------
//
// A review merged on the forge leaves the clone standing on a branch whose work
// now lives on the trunk, with a trunk that is behind. What anybody types next is
// the same every time: fetch, switch to the trunk, pull it, delete the branch.
// The first three cannot lose anything and happen by themselves; deleting is
// offered and never done here.

/// What tidying up after a merge did, and what it leaves to the user.
#[derive(Serialize, Debug, Default)]
pub struct AfterMerge {
    /// What happened, in a sentence or two.
    pub message: String,
    /// The trunk, when the switch to it happened.
    pub switched_to: Option<String>,
    /// The local branch that carried the merged work, when it can be deleted
    /// without losing anything that was not in the review.
    pub branch: Option<String>,
    /// Where that branch pointed, so a deletion can be undone.
    pub sha: Option<String>,
    /// Commits on the local branch that the review did not have. When there
    /// are any, the branch is not offered for deletion.
    pub extra: usize,
}

/// Fetches, steps off the merged branch onto the trunk and fast-forwards it.
///
/// `merged` is the review's source branch as the forge names it; the local
/// branch is the one tracking it, or the one of that name. `head` is the
/// review's head commit, which counts as published even once the forge has
/// deleted the branch it was on. `switch` is the setting: off, and only the
/// fetch happens.
///
/// `from` is where the review's branch lived, when the forge said. A review
/// from a fork is somebody else's branch: one of mine that happens to share
/// its name — `feature`, or `main` — is no branch of the review's, and is
/// neither switched off nor offered for deletion. Nor is the trunk, ever.
///
/// The repository is read once, at the start. The fetch takes as long as the
/// network does, and the switch after it used to read the path again — by
/// which time the window could be showing another repository, which then got
/// checked out onto its trunk.
pub fn after_merge(
    state: &AppState,
    merged: &str,
    head: Option<&str>,
    remote: Option<&str>,
    switch: bool,
    from: Option<&ReviewFrom>,
) -> Result<AfterMerge, String> {
    // Every helper below asks the state for its path; this copy keeps the one
    // read now, whatever the window does in the meantime.
    let state = &state.pinned();
    let path = state.path()?;
    let local = crate::forge::Local::read(state)?;
    let fork = from.is_some_and(|from| from.fork);
    let trunk = trunk_here(state);
    // The upstream's own project has to be the review's, when that is known:
    // a branch of the same name on another remote is another branch.
    // GitLab names no repository for a review from the project itself, and
    // the window passes that on as an empty name: unknown, not another one.
    let repo = from
        .and_then(|from| from.repo.as_deref())
        .filter(|repo| !repo.is_empty());
    let same_project = |remote_name: &str| match repo {
        Some(repo) => local
            .slug_of(remote_name)
            .is_some_and(|slug| format!("{}/{}", slug.owner, slug.name).eq_ignore_ascii_case(repo)),
        None => remote.map_or(true, |wanted| wanted == remote_name),
    };
    let tracks_it = |name: &str| {
        local
            .upstreams
            .get(name)
            .map(|(remote_name, branch)| branch == merged && same_project(remote_name))
            .unwrap_or(false)
    };
    let carrying = if fork {
        None
    } else {
        match &local.head {
            Some(head)
                if tracks_it(head) || (head == merged && !local.upstreams.contains_key(head)) =>
            {
                Some(head.clone())
            }
            _ => {
                let mut names: Vec<&String> =
                    local.upstreams.keys().filter(|n| tracks_it(n)).collect();
                names.sort();
                names.first().map(|name| name.to_string()).or_else(|| {
                    (refs::has_local_branch(state, merged) && !local.upstreams.contains_key(merged))
                        .then(|| merged.to_string())
                })
            }
        }
    }
    .filter(|name| Some(name) != trunk.as_ref());

    // Counted before the fetch prunes the branch the forge just deleted: after
    // that, a squash-merged branch's commits are on no remote at all.
    let extra = match &carrying {
        Some(name) => {
            let tip = format!("refs/heads/{name}");
            let mut args = vec!["rev-list", "--count", tip.as_str(), "--not", "--remotes"];
            let head = head.filter(|sha| refs::has_commit(state, sha));
            if let Some(sha) = head {
                args.push(sha);
            }
            git_cmd::run_checked(&path, &args)
                .ok()
                .and_then(|out| out.trim().parse::<usize>().ok())
                .unwrap_or(0)
        }
        None => 0,
    };
    let sha = carrying
        .as_ref()
        .and_then(|name| {
            git_cmd::run_checked(
                &path,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{name}"),
                ],
            )
            .ok()
        })
        .map(|out| out.trim().to_string());

    let mut said: Vec<String> = Vec::new();
    if let Ok(out) = remote::fetch(state, remote) {
        if !out.ok {
            said.push(
                "The fetch afterwards did not work, so the branches here may be behind".to_string(),
            );
        }
    }

    let mut switched_to = None;
    let on_it = carrying.is_some() && carrying == local.head;
    if switch && on_it {
        let dirty = git_cmd::run_checked(&path, &["status", "--porcelain", "--untracked-files=no"])
            .map(|out| !out.trim().is_empty())
            .unwrap_or(true);
        match (dirty, trunk.clone()) {
            (true, Some(trunk)) => said.push(format!(
                "You have changes here, so I stayed on {}. Commit or stash them, then switch to {trunk} and pull.",
                local.head.clone().unwrap_or_default()
            )),
            (_, None) => said.push("There is no main branch to switch to".to_string()),
            (false, Some(trunk)) if Some(&trunk) == local.head.as_ref() => {}
            (false, Some(trunk)) => {
                let behind_note = fast_forward(state, &trunk);
                match refs::checkout(state, &trunk) {
                    Ok(_) => {
                        said.push(match behind_note {
                            Some(note) => format!("Switched to {trunk}; {note}"),
                            None => format!("Switched to {trunk}, which is up to date"),
                        });
                        switched_to = Some(trunk);
                    }
                    Err(error) => said.push(format!("Could not switch to {trunk}: {error}")),
                }
            }
        }
    }

    let kept = carrying.clone().unwrap_or_else(|| merged.to_string());
    let offered = carrying.filter(|_| extra == 0);
    if extra > 0 {
        said.push(format!(
            "{kept} has {extra} commit{} that {} not on the forge, so it is kept",
            if extra == 1 { "" } else { "s" },
            if extra == 1 { "is" } else { "are" }
        ));
    }
    Ok(AfterMerge {
        message: said.join(". "),
        switched_to,
        branch: offered,
        sha,
        extra,
    })
}

/// Where a merged review's branch lived, as the forge said.
#[derive(serde::Deserialize, Debug, Clone, Default)]
pub struct ReviewFrom {
    /// It came from a fork: somebody else's repository.
    #[serde(default)]
    pub fork: bool,
    /// `owner/name` of the repository the branch was in.
    #[serde(default)]
    pub repo: Option<String>,
}

/// The trunk as a local branch name: `main` even when only `origin/main`
/// exists yet, which checking out creates.
fn trunk_here(state: &AppState) -> Option<String> {
    let name = refs::trunk(state).name?;
    if refs::has_local_branch(state, &name) {
        return Some(name);
    }
    let remotes = remote::remotes(state).unwrap_or_default();
    for remote in remotes {
        if let Some(rest) = name.strip_prefix(&format!("{remote}/")) {
            return Some(rest.to_string());
        }
    }
    Some(name)
}

/// Moves a local branch that is not checked out up to what it tracks, when
/// that is a fast-forward and nothing else. Says what it did, or why not.
fn fast_forward(state: &AppState, branch: &str) -> Option<String> {
    let path = state.path().ok()?;
    if !refs::has_local_branch(state, branch) {
        return None;
    }
    let upstream = refs::upstream_of(state, branch)?;
    let theirs = format!("refs/remotes/{upstream}");
    let ours = format!("refs/heads/{branch}");
    let before = git_cmd::run_checked(&path, &["rev-parse", "--verify", "--quiet", &ours]).ok()?;
    // A fetch from this repository into itself: git refuses anything but a
    // fast-forward without a `+`, and refuses a branch checked out in another
    // worktree, which is exactly the care this needs.
    let out = git_cmd::run(
        &path,
        &["fetch", "--no-tags", ".", &format!("{theirs}:{ours}")],
    )
    .ok()?;
    if !out.ok {
        return Some(format!(
            "it has commits {upstream} does not, so it was not fast-forwarded"
        ));
    }
    let after = git_cmd::run_checked(&path, &["rev-parse", "--verify", "--quiet", &ours]).ok()?;
    (before.trim() != after.trim()).then(|| format!("fast-forwarded to {upstream}"))
}

/// Deletes the branch a merge left behind, if it is still where it was when
/// the deletion was offered. A branch that moved since has work the offer did
/// not know about.
pub fn delete_merged(state: &AppState, name: &str, sha: &str) -> Result<String, String> {
    // Read once, as for `after_merge`.
    let state = &state.pinned();
    let path = state.path()?;
    let now = git_cmd::run_checked(
        &path,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ],
    )
    .map(|out| out.trim().to_string())
    .map_err(|_| format!("There is no branch called {name} any more"))?;
    if now != sha {
        return Err(format!(
            "{name} has moved since the merge, so it was not deleted"
        ));
    }
    if crate::journal::current_branch(state).as_deref() == Some(name) {
        return Err(format!("{name} is checked out, so it was not deleted"));
    }
    refs::delete_branch(state, name, true)?;
    Ok(format!(
        "Deleted {name} (it was at {})",
        &sha[..sha.len().min(7)]
    ))
}

/// One commit a new review would carry, for writing its description.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct DraftCommit {
    pub subject: String,
    pub body: String,
}

/// The commits on `source` that `target` does not have, oldest first, for
/// filling in a new review. The target as the forge has it where a remote
/// copy exists, for the same reason the AI description reads it that way.
pub fn branch_commits(
    state: &AppState,
    source: &str,
    target: &str,
) -> Result<Vec<DraftCommit>, String> {
    let root = state.path()?;
    let preferred = crate::forge::known_target(state).map(|chosen| chosen.remote);
    let target = crate::ai::resolve_target(&root, target, preferred.as_deref())?;
    let source = crate::ai::resolve_ref(&root, source)?;
    let repo = state.repo()?;
    let find = |name: &str| {
        repo.revparse_single(name)
            .and_then(|object| object.peel_to_commit())
            .map(|commit| commit.id())
            .map_err(|e| e.message().to_string())
    };
    let mut walk = repo.revwalk().map_err(|e| e.message().to_string())?;
    walk.push(find(&source)?)
        .map_err(|e| e.message().to_string())?;
    walk.hide(find(&target)?)
        .map_err(|e| e.message().to_string())?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::REVERSE)
        .map_err(|e| e.message().to_string())?;
    let mut out = Vec::new();
    for oid in walk.flatten().take(100) {
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        if commit.parent_count() > 1 {
            continue;
        }
        let message = commit.message().unwrap_or("").to_string();
        let (subject, body) = message.split_once('\n').unwrap_or((&message, ""));
        out.push(DraftCommit {
            subject: subject.trim().to_string(),
            body: body.trim().to_string(),
        });
    }
    Ok(out)
}

/// Puts a deleted branch back where it was.
pub fn restore_branch(state: &AppState, name: &str, sha: &str) -> Result<String, String> {
    if refs::has_local_branch(state, name) {
        return Err(format!("There is a branch called {name} already"));
    }
    refs::create_branch(state, name, Some(sha), false)?;
    Ok(format!("Restored {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// A clone of a project, with somebody else's fork of it next door: the
    /// shape a cross-repository review arrives in.
    struct Sandbox {
        root: PathBuf,
        work: PathBuf,
        fork: PathBuf,
        state: AppState,
    }

    impl Sandbox {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "gitnoob-review-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();

            // The project everyone is reviewing.
            git(&root, &["init", "--quiet", "--bare", "upstream.git"]);
            let seed = root.join("seed");
            std::fs::create_dir_all(&seed).unwrap();
            git(&seed, &["init", "--quiet", "--initial-branch=main"]);
            git(&seed, &["config", "user.email", "us@example.com"]);
            git(&seed, &["config", "user.name", "Us"]);
            std::fs::write(seed.join("readme.md"), "start\n").unwrap();
            git(&seed, &["add", "-A"]);
            git(&seed, &["commit", "--quiet", "-m", "start"]);
            git(
                &seed,
                &[
                    "remote",
                    "add",
                    "origin",
                    root.join("upstream.git").to_str().unwrap(),
                ],
            );
            git(&seed, &["push", "--quiet", "-u", "origin", "main"]);

            // Their fork of it, with the branch the review was opened from.
            git(
                &root,
                &[
                    "clone",
                    "--quiet",
                    "--bare",
                    root.join("upstream.git").to_str().unwrap(),
                    "fork.git",
                ],
            );
            let theirs = root.join("theirs");
            git(
                &root,
                &[
                    "clone",
                    "--quiet",
                    root.join("fork.git").to_str().unwrap(),
                    "theirs",
                ],
            );
            git(&theirs, &["config", "user.email", "them@example.com"]);
            git(&theirs, &["config", "user.name", "Them"]);
            git(&theirs, &["checkout", "--quiet", "-b", "fix-typo"]);
            std::fs::write(theirs.join("readme.md"), "start, fixed\n").unwrap();
            git(&theirs, &["add", "-A"]);
            git(&theirs, &["commit", "--quiet", "-m", "fix the typo"]);
            git(&theirs, &["push", "--quiet", "-u", "origin", "fix-typo"]);

            // Our own clone, which has never heard of the fork.
            let work = root.join("work");
            git(
                &root,
                &[
                    "clone",
                    "--quiet",
                    root.join("upstream.git").to_str().unwrap(),
                    "work",
                ],
            );
            git(&work, &["config", "user.email", "us@example.com"]);
            git(&work, &["config", "user.name", "Us"]);

            let state = AppState::new(root.join("config"));
            state.set_path(work.clone());
            Sandbox {
                fork: root.join("fork.git"),
                root,
                work,
                state,
            }
        }

        /// A review from the fork, the way the forge would describe it.
        fn review_from_fork(&self, branch: &str) -> ReviewTarget {
            ReviewTarget {
                number: 7,
                branch: branch.to_string(),
                head_sha: String::new(),
                source: Some(Source {
                    owner: "them".to_string(),
                    ssh_url: String::new(),
                    https_url: self.fork.to_str().unwrap().to_string(),
                    is_fork: true,
                }),
            }
        }

        fn head(&self) -> String {
            git(&self.work, &["rev-parse", "--abbrev-ref", "HEAD"])
                .trim()
                .to_string()
        }

        fn remotes(&self) -> Vec<String> {
            git(&self.work, &["remote"])
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_review_from_a_fork_adds_the_remote_it_needs_and_checks_the_branch_out() {
        let sandbox = Sandbox::new("fork");
        // The failure this replaces: no remote carries the branch, so a plain
        // checkout by name has nothing to find.
        assert!(refs::checkout(&sandbox.state, "fix-typo").is_err());

        checkout(&sandbox.state, sandbox.review_from_fork("fix-typo")).unwrap();

        assert_eq!(sandbox.head(), "fix-typo");
        assert!(sandbox.remotes().contains(&"them".to_string()));
        assert_eq!(
            refs::upstream_of(&sandbox.state, "fix-typo").as_deref(),
            Some("them/fix-typo"),
            "pushing back to the review should reach the fork, not our own remote"
        );
        // Read back without the line ending: a Windows checkout writes CRLF
        // where the fixture wrote LF, and which one landed is not what this is
        // about.
        assert_eq!(
            std::fs::read_to_string(sandbox.work.join("readme.md"))
                .unwrap()
                .replace("\r\n", "\n"),
            "start, fixed\n"
        );
    }

    #[test]
    fn checking_the_same_review_out_twice_reuses_the_remote_and_the_branch() {
        let sandbox = Sandbox::new("twice");
        checkout(&sandbox.state, sandbox.review_from_fork("fix-typo")).unwrap();
        git(&sandbox.work, &["checkout", "--quiet", "main"]);
        checkout(&sandbox.state, sandbox.review_from_fork("fix-typo")).unwrap();

        assert_eq!(sandbox.head(), "fix-typo");
        let named: Vec<String> = sandbox
            .remotes()
            .into_iter()
            .filter(|r| r.starts_with("them"))
            .collect();
        assert_eq!(
            named,
            vec!["them".to_string()],
            "no second remote for the same fork"
        );
    }

    #[test]
    fn a_fork_branch_that_clashes_with_one_of_ours_is_kept_apart() {
        let sandbox = Sandbox::new("clash");
        // A branch of our own already called what the review calls its own.
        git(&sandbox.work, &["checkout", "--quiet", "-b", "fix-typo"]);
        std::fs::write(sandbox.work.join("ours.md"), "ours\n").unwrap();
        git(&sandbox.work, &["add", "-A"]);
        git(&sandbox.work, &["commit", "--quiet", "-m", "our own work"]);
        git(&sandbox.work, &["checkout", "--quiet", "main"]);

        checkout(&sandbox.state, sandbox.review_from_fork("fix-typo")).unwrap();

        assert_eq!(sandbox.head(), "them-fix-typo");
        // Ours is untouched, which is the point of not reusing the name.
        assert!(refs::has_local_branch(&sandbox.state, "fix-typo"));
        assert!(!sandbox.work.join("ours.md").exists());
    }

    #[test]
    fn a_review_from_this_repository_is_an_ordinary_checkout() {
        let sandbox = Sandbox::new("same-repo");
        // Pushed to the project itself rather than to a fork.
        let theirs = sandbox.root.join("theirs");
        git(
            &theirs,
            &[
                "remote",
                "add",
                "upstream",
                sandbox.root.join("upstream.git").to_str().unwrap(),
            ],
        );
        git(&theirs, &["push", "--quiet", "upstream", "fix-typo"]);

        let review = ReviewTarget {
            number: 8,
            branch: "fix-typo".to_string(),
            head_sha: String::new(),
            source: Some(Source {
                owner: "us".to_string(),
                ssh_url: String::new(),
                https_url: sandbox
                    .root
                    .join("upstream.git")
                    .to_str()
                    .unwrap()
                    .to_string(),
                is_fork: false,
            }),
        };
        // Not fetched yet: the checkout has to go and get it.
        checkout(&sandbox.state, review).unwrap();

        assert_eq!(sandbox.head(), "fix-typo");
        assert_eq!(
            sandbox.remotes(),
            vec!["origin".to_string()],
            "no remote added for our own branch"
        );
    }

    #[test]
    fn the_same_repository_written_differently_is_recognised() {
        assert!(same_repository(
            "git@github.com:them/app.git",
            "https://github.com/them/app"
        ));
        assert!(same_repository(
            "https://github.com/Them/App.git",
            "https://github.com/them/app/"
        ));
        assert!(same_repository(
            "ssh://git@gitlab.com/group/sub/app.git",
            "https://gitlab.com/group/sub/app"
        ));
    }

    #[test]
    fn different_repositories_are_not() {
        assert!(!same_repository(
            "git@github.com:them/app.git",
            "git@github.com:us/app.git"
        ));
        assert!(!same_repository(
            "git@github.com:them/app.git",
            "git@gitlab.com:them/app.git"
        ));
        // An unset remote must not match everything.
        assert!(!same_repository("", ""));
    }
}
