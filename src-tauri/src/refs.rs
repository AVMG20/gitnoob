use std::collections::HashMap;

use git2::{BranchType, Repository, StatusOptions};
use serde::Serialize;

use crate::git_cmd;
use crate::journal::{self, Mode};
use crate::state::AppState;
use crate::work;

#[derive(Serialize)]
pub struct RepoInfo {
    pub path: String,
    pub name: String,
    pub head: String,
    pub detached: bool,
    pub state: String,
    /// Who a commit made here would be authored by: this repository's effective
    /// `user.name`, which is local config if it has one and global otherwise.
    /// Empty when git has no name to use, which is worth showing as such.
    pub author: String,
}

#[derive(Serialize)]
pub struct LocalBranch {
    pub name: String,
    pub oid: String,
    pub is_head: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Serialize)]
pub struct RemoteBranch {
    pub name: String,
    pub remote: String,
    pub oid: String,
}

#[derive(Serialize)]
pub struct Tag {
    pub name: String,
    pub oid: String,
}

#[derive(Serialize)]
pub struct Stash {
    pub index: usize,
    pub message: String,
}

#[derive(Serialize)]
pub struct RefTree {
    pub locals: Vec<LocalBranch>,
    pub remotes: Vec<RemoteBranch>,
    pub tags: Vec<Tag>,
    pub stashes: Vec<Stash>,
}

/// One entry in the working-tree status list.
#[derive(Serialize)]
pub struct StatusEntry {
    pub path: String,
    /// One of: added, modified, deleted, renamed, typechange, untracked.
    pub kind: String,
}

#[derive(Serialize)]
pub struct WorkingStatus {
    pub staged: Vec<StatusEntry>,
    pub unstaged: Vec<StatusEntry>,
    pub conflicted: Vec<String>,
}

pub fn describe(state: &AppState) -> Result<RepoInfo, String> {
    let repo = state.repo()?;
    let path = state.path()?;

    let (head, detached) = match repo.head() {
        Ok(h) => {
            if repo.head_detached().unwrap_or(false) {
                let short = h
                    .target()
                    .map(|o| o.to_string()[..7].to_string())
                    .unwrap_or_default();
                (short, true)
            } else {
                (h.shorthand().unwrap_or("HEAD").to_string(), false)
            }
        }
        // An empty repository has a HEAD that points nowhere yet.
        Err(_) => ("(no commits yet)".to_string(), false),
    };

    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());

    // `snapshot` resolves the whole chain — local, global, system — the same way
    // a commit would.
    let author = repo
        .config()
        .and_then(|mut c| c.snapshot())
        .and_then(|c| c.get_string("user.name"))
        .unwrap_or_default();

    Ok(RepoInfo {
        path: path.to_string_lossy().into_owned(),
        name,
        head,
        detached,
        state: format!("{:?}", repo.state()),
        author,
    })
}

pub fn tree(state: &AppState) -> Result<RefTree, String> {
    let mut repo = state.repo()?;

    let head_name = repo
        .head()
        .ok()
        .filter(|_| !repo.head_detached().unwrap_or(false))
        .and_then(|h| h.shorthand().map(|s| s.to_string()));

    let mut locals = Vec::new();
    for branch in repo.branches(Some(BranchType::Local)).map_err(err)? {
        let (branch, _) = branch.map_err(err)?;
        let name = match branch.name().map_err(err)? {
            Some(n) => n.to_string(),
            None => continue,
        };
        let oid = match branch.get().target() {
            Some(o) => o,
            None => continue,
        };

        // Ahead/behind is what drives the "safe to push?" question later on.
        let (upstream, ahead, behind) = match branch.upstream() {
            Ok(up) => {
                let up_name = up.name().ok().flatten().map(|s| s.to_string());
                let counts = up
                    .get()
                    .target()
                    .and_then(|up_oid| repo.graph_ahead_behind(oid, up_oid).ok())
                    .unwrap_or((0, 0));
                (up_name, counts.0, counts.1)
            }
            Err(_) => (None, 0, 0),
        };

        locals.push(LocalBranch {
            is_head: Some(&name) == head_name.as_ref(),
            name,
            oid: oid.to_string(),
            upstream,
            ahead,
            behind,
        });
    }
    locals.sort_by(|a, b| a.name.cmp(&b.name));

    let mut remotes = Vec::new();
    for branch in repo.branches(Some(BranchType::Remote)).map_err(err)? {
        let (branch, _) = branch.map_err(err)?;
        let full = match branch.name().map_err(err)? {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip the symbolic `origin/HEAD`; it is a pointer, not a branch.
        if full.ends_with("/HEAD") {
            continue;
        }
        let oid = match branch.get().target() {
            Some(o) => o.to_string(),
            None => continue,
        };
        let (remote, name) = match full.split_once('/') {
            Some((r, n)) => (r.to_string(), n.to_string()),
            None => (String::new(), full.clone()),
        };
        remotes.push(RemoteBranch { name, remote, oid });
    }
    remotes.sort_by(|a, b| (&a.remote, &a.name).cmp(&(&b.remote, &b.name)));

    let mut tags = Vec::new();
    repo.tag_foreach(|oid, name| {
        let name = String::from_utf8_lossy(name)
            .trim_start_matches("refs/tags/")
            .to_string();
        tags.push(Tag {
            name,
            oid: oid.to_string(),
        });
        true
    })
    .map_err(err)?;
    tags.sort_by(|a, b| a.name.cmp(&b.name));

    let mut stashes = Vec::new();
    repo.stash_foreach(|index, message, _| {
        stashes.push(Stash {
            index,
            message: message.to_string(),
        });
        true
    })
    .map_err(err)?;

    Ok(RefTree {
        locals,
        remotes,
        tags,
        stashes,
    })
}

/// One ref decorating a commit in the graph.
pub struct Decoration {
    pub kind: String,
    pub name: String,
    /// True for the one local branch that is checked out. The graph draws it
    /// differently, because "which of these am I standing on" is the question
    /// the decorations are there to answer.
    pub head: bool,
}

/// Maps every commit that carries a ref to its labels, for graph decoration.
pub fn labels_by_oid(repo: &Repository) -> HashMap<String, Vec<Decoration>> {
    let mut map: HashMap<String, Vec<Decoration>> = HashMap::new();
    let detached = repo.head_detached().unwrap_or(false);
    let current = if detached {
        None
    } else {
        repo.head()
            .ok()
            .and_then(|h| h.shorthand().map(|s| s.to_string()))
    };

    let mut push = |oid: git2::Oid, kind: &str, name: String, head: bool| {
        map.entry(oid.to_string()).or_default().push(Decoration {
            kind: kind.to_string(),
            name,
            head,
        });
    };

    if let Ok(refs) = repo.references() {
        for r in refs.flatten() {
            let Some(oid) = r.target() else { continue };
            let Some(name) = r.shorthand() else { continue };
            if r.is_branch() {
                let is_head = current.as_deref() == Some(name);
                push(oid, "local", name.to_string(), is_head);
            } else if r.is_remote() {
                if name.ends_with("/HEAD") {
                    continue;
                }
                push(oid, "remote", name.to_string(), false);
            } else if r.is_tag() {
                push(oid, "tag", name.to_string(), false);
            }
        }
    }
    // A detached HEAD deserves its own marker; otherwise HEAD is implied by the
    // branch label already collected above.
    if detached {
        if let Some(oid) = repo.head().ok().and_then(|h| h.target()) {
            push(oid, "head", "HEAD".to_string(), true);
        }
    }
    map
}

pub fn status(state: &AppState) -> Result<WorkingStatus, String> {
    let repo = state.repo()?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);

    let statuses = repo.statuses(Some(&mut opts)).map_err(err)?;
    let mut out = WorkingStatus {
        staged: Vec::new(),
        unstaged: Vec::new(),
        conflicted: Vec::new(),
    };

    for entry in statuses.iter() {
        let path = entry.path().unwrap_or("").to_string();
        if path.is_empty() {
            continue;
        }
        let s = entry.status();

        if s.is_conflicted() {
            out.conflicted.push(path);
            continue;
        }
        if let Some(kind) = staged_kind(s) {
            out.staged.push(StatusEntry {
                path: path.clone(),
                kind,
            });
        }
        if let Some(kind) = unstaged_kind(s) {
            out.unstaged.push(StatusEntry { path, kind });
        }
    }
    Ok(out)
}

fn staged_kind(s: git2::Status) -> Option<String> {
    let kind = if s.is_index_new() {
        "added"
    } else if s.is_index_modified() {
        "modified"
    } else if s.is_index_deleted() {
        "deleted"
    } else if s.is_index_renamed() {
        "renamed"
    } else if s.is_index_typechange() {
        "typechange"
    } else {
        return None;
    };
    Some(kind.to_string())
}

fn unstaged_kind(s: git2::Status) -> Option<String> {
    let kind = if s.is_wt_new() {
        "untracked"
    } else if s.is_wt_modified() {
        "modified"
    } else if s.is_wt_deleted() {
        "deleted"
    } else if s.is_wt_renamed() {
        "renamed"
    } else if s.is_wt_typechange() {
        "typechange"
    } else {
        return None;
    };
    Some(kind.to_string())
}

/// Checks out an existing local branch, or a remote branch by creating a local
/// tracking branch for it.
///
/// Uncommitted work is stashed first and put back afterwards, so switching
/// branches mid-change does not need the user to tidy up by hand.
pub fn checkout(state: &AppState, name: &str) -> Result<String, String> {
    let path = state.path()?;
    let previous = journal::current_branch(state);

    // Decide the argument list before touching the working tree; the repo handle
    // must not be alive while git runs.
    let args: Vec<String> = {
        let repo = state.repo()?;
        if repo.find_branch(name, BranchType::Local).is_ok() {
            vec!["checkout".into(), name.into()]
        } else if repo.find_branch(name, BranchType::Remote).is_ok() {
            let local = name.split_once('/').map(|(_, n)| n).unwrap_or(name);
            vec![
                "checkout".into(),
                "-b".into(),
                local.into(),
                "--track".into(),
                name.into(),
            ]
        } else {
            // Could be a tag or a raw revision; let git decide and report.
            vec!["checkout".into(), name.into()]
        }
    };

    let borrowed: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    // Try the switch as it is first. Git carries uncommitted edits across a
    // branch change whenever they do not collide with what that change touches,
    // and that is the common case — stashing every time churns the working tree,
    // loses the staged/unstaged split, and risks a conflicted pop for nothing.
    let plain = git_cmd::run_checked(&path, &borrowed);
    let (out, restored) = match plain {
        Ok(message) => (message, None),
        Err(error) if !refused_over_local_changes(&error) => return Err(error),
        Err(error) => {
            // It collided. Now the stash is worth it.
            let held = work::stash_before(state, &format!("switching to {name}"))?;
            if !held.stashed {
                // Auto-stash is off, or there was nothing to stash; either way
                // retrying would fail the same way. Report what git said.
                return Err(error);
            }
            match git_cmd::run_checked(&path, &borrowed) {
                Ok(message) => (message, work::restore_after(state, held)?),
                Err(again) => {
                    // Put the changes back before reporting, so a failed switch
                    // leaves the working tree as it was.
                    let _ = work::restore_after(state, held);
                    return Err(again);
                }
            }
        }
    };

    let landed = journal::current_branch(state);
    if let (Some(from), Some(to)) = (previous.clone(), landed.clone()) {
        if from != to {
            journal::record(
                state,
                "checkout",
                format!("Switch to {to}"),
                None,
                Some(from),
                Some(to),
                Mode::Checkout,
                false,
            );
        }
    }

    let mut message = out;
    if let Some(note) = restored {
        message = format!("{}\n{note}", message.trim());
    }
    Ok(message)
}

/// Whether git turned a checkout down because uncommitted work was in the way,
/// as opposed to failing for some other reason. Only the first is worth
/// stashing and retrying for.
fn refused_over_local_changes(error: &str) -> bool {
    error.contains("would be overwritten by checkout")
        || error.contains("Please commit your changes or stash them")
        || error.contains("would be overwritten by merge")
}

pub fn create_branch(state: &AppState, name: &str, start: Option<&str>, checkout: bool) -> Result<String, String> {
    let path = state.path()?;
    let mut args: Vec<&str> = if checkout {
        vec!["checkout", "-b", name]
    } else {
        vec!["branch", name]
    };
    if let Some(start) = start {
        args.push(start);
    }
    // `git checkout -b` says "Switched to a new branch" on stderr, so its stdout
    // is empty on success. Say it ourselves rather than hand back nothing.
    git_cmd::run_checked(&path, &args)?;
    Ok(match (checkout, start) {
        (true, Some(start)) => format!("Created {name} from {start} and checked it out"),
        (true, None) => format!("Created {name} and checked it out"),
        (false, Some(start)) => format!("Created {name} from {start}"),
        (false, None) => format!("Created {name}"),
    })
}

/// What deleting a branch would cost, so the question can be asked properly.
#[derive(Serialize)]
pub struct BranchDeletion {
    pub name: String,
    /// Checked out. Git refuses to delete this one, and so do we.
    pub is_head: bool,
    /// Reachable from HEAD, so nothing is lost by deleting it.
    pub merged: bool,
    pub upstream: Option<String>,
    /// Commits on this branch that its upstream does not have. These are what a
    /// delete actually costs, when it costs anything.
    pub unpushed: usize,
    /// Remote branches of the same name, e.g. `origin/feature`. Their presence
    /// is what turns one question into two.
    pub remotes: Vec<String>,
}

pub fn deletion_preview(state: &AppState, name: &str) -> Result<BranchDeletion, String> {
    let repo = state.repo()?;
    let branch = repo
        .find_branch(name, BranchType::Local)
        .map_err(|_| format!("No local branch named {name}"))?;
    let oid = branch
        .get()
        .target()
        .ok_or_else(|| format!("Branch {name} has no commit"))?;

    let head = repo.head().ok().and_then(|h| h.target());
    // Merged means HEAD can already reach it: deleting the label loses nothing.
    let merged = match head {
        Some(head) => head == oid || repo.graph_descendant_of(head, oid).unwrap_or(false),
        None => false,
    };

    let upstream_ref = branch.upstream().ok();
    let upstream = upstream_ref
        .as_ref()
        .and_then(|u| u.name().ok().flatten().map(|s| s.to_string()));
    let unpushed = upstream_ref
        .as_ref()
        .and_then(|u| u.get().target())
        .and_then(|up| repo.graph_ahead_behind(oid, up).ok())
        .map(|(ahead, _)| ahead)
        .unwrap_or(0);

    // Any remote carrying this branch name, not only the tracked one: a branch
    // pushed to two remotes has two copies to think about.
    let mut remotes = Vec::new();
    if let Ok(list) = repo.branches(Some(BranchType::Remote)) {
        for entry in list.flatten() {
            let (remote_branch, _) = entry;
            let Some(full) = remote_branch.name().ok().flatten() else {
                continue;
            };
            if full.ends_with("/HEAD") {
                continue;
            }
            if full.splitn(2, '/').nth(1) == Some(name) {
                remotes.push(full.to_string());
            }
        }
    }
    remotes.sort();

    Ok(BranchDeletion {
        name: name.to_string(),
        // By name, not by commit: another branch can sit on the same commit as
        // HEAD without being the one checked out.
        is_head: !repo.head_detached().unwrap_or(false)
            && repo.head().ok().and_then(|h| h.shorthand().map(String::from)) == Some(name.to_string()),
        merged,
        upstream,
        unpushed,
        remotes,
    })
}

pub fn delete_branch(state: &AppState, name: &str, force: bool) -> Result<String, String> {
    let path = state.path()?;
    let flag = if force { "-D" } else { "-d" };
    git_cmd::run_checked(&path, &["branch", flag, name])
}

fn err(e: git2::Error) -> String {
    e.message().to_string()
}

pub fn rename_branch(state: &AppState, from: &str, to: &str) -> Result<String, String> {
    let root = state.path()?;
    git_cmd::run_checked(&root, &["branch", "-m", from, to])?;
    Ok(format!("Renamed {from} to {to}"))
}

/// Points a local branch at a remote-tracking branch.
pub fn set_upstream(state: &AppState, branch: &str, upstream: &str) -> Result<String, String> {
    let root = state.path()?;
    git_cmd::run_checked(
        &root,
        &["branch", &format!("--set-upstream-to={upstream}"), branch],
    )?;
    Ok(format!("{branch} now tracks {upstream}"))
}

pub fn unset_upstream(state: &AppState, branch: &str) -> Result<String, String> {
    let root = state.path()?;
    git_cmd::run_checked(&root, &["branch", "--unset-upstream", branch])?;
    Ok(format!("{branch} no longer tracks anything"))
}

/// Local branches whose upstream has disappeared from the remote.
///
/// After a fetch with prune this is the list of branches whose work has almost
/// certainly been merged and deleted server-side — the tidy-up nobody
/// remembers to do.
pub fn stale_branches(state: &AppState) -> Result<Vec<String>, String> {
    let root = state.path()?;
    let raw = git_cmd::run_checked(
        &root,
        &["branch", "--format=%(refname:short)%00%(upstream:track)"],
    )?;
    Ok(raw
        .lines()
        .filter_map(|line| {
            let (name, track) = line.split_once('\0')?;
            // git spells a vanished upstream "[gone]".
            track.contains("gone").then(|| name.to_string())
        })
        .collect())
}

/// Adds a pattern to the repository's `.gitignore`, creating it if needed.
pub fn add_to_gitignore(state: &AppState, pattern: &str) -> Result<String, String> {
    let root = state.path()?;
    let file = root.join(".gitignore");
    let mut text = std::fs::read_to_string(&file).unwrap_or_default();

    if text.lines().any(|line| line.trim() == pattern.trim()) {
        return Ok(format!("{pattern} is already ignored"));
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(pattern.trim());
    text.push('\n');
    std::fs::write(&file, text).map_err(|e| format!("Could not write .gitignore: {e}"))?;
    Ok(format!("Added {pattern} to .gitignore"))
}
