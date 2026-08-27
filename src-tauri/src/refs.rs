use std::collections::HashMap;

use git2::{BranchType, Repository, StatusOptions};
use serde::Serialize;

use crate::git_cmd;
use crate::remote;
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
    /// The address those commits would carry, which is what a picture for the
    /// person is looked up by.
    pub author_email: String,
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
    /// The commit the tag names — for an annotated tag, what the tag object
    /// points at rather than the tag object itself. The graph has rows for
    /// commits and nothing else, so this is the id that can be looked up.
    pub oid: String,
    /// An annotated tag is an object in its own right, with a message, a
    /// tagger and a date of its own. A lightweight one is just a name.
    pub annotated: bool,
    /// The first line of the tag's own message, when it has one.
    pub message: Option<String>,
    /// Seconds since the epoch: the tagger's time when there is one, the
    /// commit's otherwise, so both kinds sort against each other.
    pub when: i64,
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
    let settings = repo.config().and_then(|mut c| c.snapshot());
    let field = |key: &str| {
        settings
            .as_ref()
            .ok()
            .and_then(|c| c.get_string(key).ok())
            .unwrap_or_default()
    };

    Ok(RepoInfo {
        path: path.to_string_lossy().into_owned(),
        name,
        head,
        detached,
        state: format!("{:?}", repo.state()),
        author: field("user.name"),
        author_email: field("user.email"),
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
    if let Ok(refs) = repo.references_glob("refs/tags/*") {
        for r in refs.flatten() {
            let Some(name) = r.shorthand() else { continue };
            // Peeling is the whole point: `git tag -a` writes a tag object,
            // and its id is not any commit's id. Anything that then treats it
            // as one — a graph chip, a click that means "show me this" — is
            // hung on an id that does not exist in the history.
            let Ok(commit) = r.peel_to_commit() else { continue };
            let annotated = r.target().and_then(|oid| repo.find_tag(oid).ok());
            let when = annotated
                .as_ref()
                .and_then(|t| t.tagger().map(|who| who.when().seconds()))
                .unwrap_or_else(|| commit.time().seconds());
            let message = annotated.as_ref().and_then(|t| {
                t.message()
                    .and_then(|m| m.trim().lines().next())
                    .filter(|line| !line.is_empty())
                    .map(|line| line.to_string())
            });
            tags.push(Tag {
                name: name.to_string(),
                oid: commit.id().to_string(),
                annotated: annotated.is_some(),
                message,
                when,
            });
        }
    }
    // Newest first, not alphabetical. Version tags are the common case and
    // sorting them by name puts v0.10.0 above v0.9.0, which is the wrong
    // release; by date the list reads as the release history it is.
    tags.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| a.name.cmp(&b.name)));

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
                // As above: an annotated tag's own id decorates nothing,
                // because no row in the graph carries it.
                let commit = r.peel_to_commit().map(|c| c.id()).unwrap_or(oid);
                push(commit, "tag", name.to_string(), false);
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
/// Where git refuses over uncommitted work, it is set down and picked back up
/// on the other side, so switching branches mid-change does not need the user
/// to tidy up by hand first.
pub fn checkout(state: &AppState, name: &str) -> Result<String, String> {
    // Decide the argument list before touching the working tree; the repo handle
    // must not be alive while git runs.
    let args: Vec<String> = {
        let repo = state.repo()?;
        // Every form ends in `--`. Without it git falls back to reading the
        // name as a path, and `git checkout notes.txt` on a name that is not a
        // ref throws away the uncommitted changes in that file without a word.
        // With it, a name that is not a ref is an error, which is the truth.
        if repo.find_branch(name, BranchType::Local).is_ok() {
            vec!["checkout".into(), name.into(), "--".into()]
        } else if repo.find_branch(name, BranchType::Remote).is_ok() {
            let local = name.split_once('/').map(|(_, n)| n).unwrap_or(name);
            vec![
                "checkout".into(),
                "-b".into(),
                local.into(),
                "--track".into(),
                name.into(),
                "--".into(),
            ]
        } else {
            // Could be a tag or a raw revision; let git decide and report.
            vec!["checkout".into(), name.into(), "--".into()]
        }
    };

    switch(state, name, &args)
}

/// Creates a local branch following a remote one and switches to it.
///
/// `checkout` does this for a name it recognises as remote-tracking; this is
/// for the caller that has just fetched the remote branch and knows what the
/// local one should be called, which is not always the last path segment.
pub fn checkout_tracking(state: &AppState, local: &str, tracking: &str) -> Result<String, String> {
    let args = vec![
        "checkout".to_string(),
        "-b".to_string(),
        local.to_string(),
        "--track".to_string(),
        tracking.to_string(),
        "--".to_string(),
    ];
    switch(state, local, &args)
}

/// Creates a local branch at a revision and switches to it, for commits that
/// arrived without a branch to hang them on.
pub fn checkout_at(state: &AppState, local: &str, revision: &str) -> Result<String, String> {
    let args = vec![
        "checkout".to_string(),
        "-b".to_string(),
        local.to_string(),
        revision.to_string(),
        "--".to_string(),
    ];
    switch(state, local, &args)
}

/// Runs a prepared checkout, bringing the uncommitted work along if it has to.
fn switch(state: &AppState, name: &str, args: &[String]) -> Result<String, String> {
    let path = state.path()?;
    let borrowed: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    // Try the switch as it is first. Git carries uncommitted edits across a
    // branch change whenever they do not collide with what that change touches,
    // and that is the common case — stashing every time churns the working tree,
    // loses the staged/unstaged split, and risks a conflicted pop for nothing.
    let out = match git_cmd::run_checked(&path, &borrowed) {
        Ok(message) => message,
        Err(error) if !refused_over_local_changes(&error) => return Err(error),
        // The edits are in the way of the switch. Try to bring them along, and
        // say what is in the way only if that could not be done.
        Err(error) => carry(state, name, &borrowed, &error)?,
    };

    // Not recorded in the history. Undo is for the things you did not mean to
    // do — a commit, an amend, a reset, a pull that rewound the branch — and
    // switching branches is neither hard to spot nor hard to reverse. It only
    // filled the list, pushing the steps worth undoing off the end of it.
    Ok(out)
}

/// Switches with the uncommitted work set down and picked back up again.
///
/// Git refuses a switch whenever a file you have changed differs between the
/// two branches, which is a stricter test than whether the change would
/// actually collide: nine times in ten the edit is nowhere near what the
/// branches disagree about, and a stash, a switch and a three-way apply carry
/// it across without a murmur. This is that, tried in the background.
///
/// What makes it safe to try is that it puts everything back when it does not
/// work. The stash is applied rather than popped, so the work is still in it
/// while the apply is being judged; a conflict means the working tree is reset,
/// the old branch is checked out, the stash goes back on it, and the user is
/// told what git said in the first place. Nobody is left standing in a
/// half-applied stash on a branch they did not ask to be on.
fn carry(state: &AppState, name: &str, args: &[&str], refusal: &str) -> Result<String, String> {
    // The setting says "stash and restore around branch switches and pulls".
    // Off means the user wants to be told, not helped.
    if !state.config().global.auto_stash {
        return Err(in_the_way(name, refusal));
    }
    let root = state.path()?;
    let previous = crate::journal::current_branch(state);
    // Where to put HEAD back if this does not work out. A detached HEAD has no
    // branch to name, so the commit itself is the way back.
    let was = previous
        .clone()
        .or_else(|| crate::journal::head_oid(state));
    let message = match &previous {
        Some(branch) => format!("{} on {branch}: switching to {name}", work::AUTO_STASH),
        None => format!("{}: switching to {name}", work::AUTO_STASH),
    };

    // Untracked files too: git refuses over those as well, and a switch that
    // left them behind would be a switch that lost them.
    git_cmd::run_checked(
        &root,
        &["stash", "push", "--include-untracked", "-m", &message],
    )
    .map_err(|_| in_the_way(name, refusal))?;

    // Which stash this is, so nothing else that lands on top of it can be
    // dropped by mistake later.
    let held = git_cmd::run_checked(&root, &["rev-parse", "stash@{0}"])
        .map(|out| out.trim().to_string())
        .unwrap_or_default();

    // The switch itself, now that nothing is in its way.
    if git_cmd::run_checked(&root, args).is_err() {
        let _ = git_cmd::run_checked(&root, &["stash", "pop", "--index"]);
        return Err(in_the_way(name, refusal));
    }

    // `apply`, not `pop`: the work stays in the stash until it is clear that it
    // landed. `--index` keeps what was staged staged.
    let landed = git_cmd::run_checked(&root, &["stash", "apply", "--index"]).is_ok()
        // Only the staged/unstaged split could not be restored, which is not
        // worth refusing the switch over. The tree is cleaned first: a failed
        // apply can leave part of itself behind.
        || (git_cmd::run_checked(&root, &["reset", "--hard", "HEAD"]).is_ok()
            && git_cmd::run_checked(&root, &["stash", "apply"]).is_ok());

    if landed {
        drop_stash(state, &held);
        return Ok(format!("Switched to {name}, bringing your changes with you"));
    }

    // It would have conflicted. Put everything back exactly as it was: the
    // tree, then the branch, then the work on top of it.
    let _ = git_cmd::run_checked(&root, &["reset", "--hard", "HEAD"]);
    if let Some(back) = &was {
        let _ = git_cmd::run_checked(&root, &["checkout", back, "--"]);
    }
    if git_cmd::run_checked(&root, &["stash", "pop", "--index"]).is_err()
        && git_cmd::run_checked(&root, &["stash", "pop"]).is_err()
    {
        return Err(format!(
            "Switching to {name} would have conflicted with your changes, and putting them back \
             afterwards did not work either. They are safe in the stash, at the top of the list."
        ));
    }
    Err(in_the_way(name, refusal))
}

/// Removes the stash this made, wherever it has ended up in the list.
///
/// It has to go: a switch that carried the work across leaves a stash holding a
/// copy of changes that are now in the working tree, and a morning of hopping
/// between branches would leave a list of them. It is found by its commit id
/// rather than dropped off the top, because anything could have added a stash
/// in between — the user, another window, a terminal — and dropping the top of
/// the list without looking is how a tool eats work nobody asked it to touch.
fn drop_stash(state: &AppState, held: &str) {
    if held.is_empty() {
        return;
    }
    let Ok(root) = state.path() else { return };
    let Some(at) = crate::journal::stash_index(state, held) else {
        return;
    };
    let _ = git_cmd::run_checked(&root, &["stash", "drop", &format!("stash@{{{at}}}")]);
}

/// Whether a name is already a branch here.
pub fn has_local_branch(state: &AppState, name: &str) -> bool {
    state
        .repo()
        .map(|repo| repo.find_branch(name, BranchType::Local).is_ok())
        .unwrap_or(false)
}

/// Whether any remote this clone knows carries a branch by this name.
pub fn has_remote_branch(state: &AppState, name: &str) -> bool {
    let Ok(repo) = state.repo() else {
        return false;
    };
    let Ok(list) = repo.branches(Some(BranchType::Remote)) else {
        return false;
    };
    let mut found = false;
    for (branch, _) in list.flatten() {
        let matches = branch
            .name()
            .ok()
            .flatten()
            .and_then(|full| full.split_once('/').map(|(_, rest)| rest == name))
            .unwrap_or(false);
        if matches {
            found = true;
            break;
        }
    }
    found
}

/// What a local branch tracks, as `remote/branch`.
pub fn upstream_of(state: &AppState, name: &str) -> Option<String> {
    let repo = state.repo().ok()?;
    let branch = repo.find_branch(name, BranchType::Local).ok()?;
    let upstream = branch.upstream().ok()?;
    upstream.name().ok().flatten().map(|s| s.to_string())
}

/// Whether a revision is already in this clone's object store.
pub fn has_commit(state: &AppState, revision: &str) -> bool {
    state
        .repo()
        .map(|repo| repo.revparse_single(revision).is_ok())
        .unwrap_or(false)
}

/// Whether git turned a checkout down because uncommitted work was in the way,
/// as opposed to failing for some other reason. Only the first is worth
/// stashing and retrying for.
fn refused_over_local_changes(error: &str) -> bool {
    error.contains("would be overwritten by checkout")
        || error.contains("Please commit your changes or stash them")
        || error.contains("would be overwritten by merge")
}

/// The refusal, in terms of the files rather than of git's plumbing.
///
/// Git names the files it would have overwritten in the middle of a paragraph
/// about merging and stashing. The files are the useful part — they are what
/// has to be dealt with — so they are counted, listed, and followed by the
/// three things that deal with them.
fn in_the_way(name: &str, error: &str) -> String {
    let files: Vec<&str> = error
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("error:")
                && !line.starts_with("Please")
                && !line.starts_with("Aborting")
                && !line.starts_with("fatal:")
                && !line.starts_with("warning:")
        })
        .collect();

    if files.is_empty() {
        return format!(
            "Cannot switch to {name}: your open changes are in the way. Commit, stash, or \
             discard them first."
        );
    }

    let count = files.len();
    let listed = if count > 6 {
        format!("{}\n…and {} more", files[..6].join("\n"), count - 6)
    } else {
        files.join("\n")
    };
    format!(
        "Cannot switch to {name}: {count} {} would be overwritten.\n{listed}\nCommit, stash, or \
         discard your changes first.",
        if count == 1 { "file" } else { "files" }
    )
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
    // See `checkout`: the trailing `--` stops a start point that is not a ref
    // from being read as a path.
    args.push("--");
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

/// A copy of the branch on a remote, and what deleting it there would cost.
#[derive(Serialize)]
pub struct RemoteCopy {
    /// Full remote-tracking name, e.g. `origin/feature`.
    pub name: String,
    pub remote: String,
    /// Commits on the remote copy that the branch you are on cannot reach.
    /// These are the ones a remote delete strands, and no reflog here brings
    /// them back for anyone else — this is the number that matters most.
    pub unmerged: usize,
}

/// The branch a repository is organised around.
///
/// "Is this branch safe to delete?" is really "does the line everything ends up
/// on already hold this work?", and the branch you happen to be standing on is
/// not that line. Neither is a branch like `staging` that holds every commit
/// today and gets reset tomorrow — which is exactly the case that read as safe
/// when the answer was worked out from whatever could reach the tip.
#[derive(Serialize, Clone)]
pub struct Trunk {
    /// The ref the comparison uses: a local branch where there is one, else a
    /// remote-tracking copy. `None` when the repository has neither.
    pub name: Option<String>,
    /// True when this repository was told which branch it is, rather than the
    /// usual names being tried in turn.
    pub chosen: bool,
}

/// Where the choice is kept: the repository's own git config, so it survives a
/// profile switch, travels with the clone, and can be read and changed with
/// `git config gitnoob.trunk` by somebody who would rather do that.
const TRUNK_KEY: &str = "gitnoob.trunk";

/// Whether a ref exists here, by either of the names a branch answers to.
fn ref_exists(repo: &Repository, name: &str) -> bool {
    repo.find_branch(name, BranchType::Local).is_ok()
        || repo.find_branch(name, BranchType::Remote).is_ok()
}

pub fn trunk(state: &AppState) -> Trunk {
    let Ok(repo) = state.repo() else {
        return Trunk { name: None, chosen: false };
    };

    // What this repository was told, when it still names something real. A
    // branch that has since been deleted falls back rather than measuring
    // everything against nothing.
    if let Ok(path) = state.path() {
        if let Ok(raw) = git_cmd::run_checked(&path, &["config", "--get", TRUNK_KEY]) {
            let chosen = raw.trim();
            if !chosen.is_empty() && ref_exists(&repo, chosen) {
                return Trunk { name: Some(chosen.to_string()), chosen: true };
            }
        }
    }

    // The usual names, local first: a clone that has never checked the default
    // branch out still has `origin/main` to measure against.
    for name in ["main", "master", "origin/main", "origin/master"] {
        if ref_exists(&repo, name) {
            return Trunk { name: Some(name.to_string()), chosen: false };
        }
    }
    Trunk { name: None, chosen: false }
}

/// Names the branch this repository is organised around, or forgets the name
/// when given nothing, which puts the usual guesses back.
pub fn set_trunk(state: &AppState, name: Option<&str>) -> Result<String, String> {
    let path = state.path()?;
    let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
        // `--unset` on a key that is not there exits 5; there is nothing wrong
        // with clearing a choice that was never made.
        let _ = git_cmd::run(&path, &["config", "--local", "--unset", TRUNK_KEY]);
        return Ok("Main branch back to whichever of main and master exists".to_string());
    };
    let repo = state.repo()?;
    if !ref_exists(&repo, name) {
        return Err(format!("No branch named {name}"));
    }
    git_cmd::run_checked(&path, &["config", "--local", TRUNK_KEY, name])?;
    Ok(format!("{name} is now this repository's main branch"))
}

/// What deleting a branch would cost, so the question can be asked properly.
#[derive(Serialize)]
pub struct BranchDeletion {
    pub name: String,
    /// Checked out. Git refuses to delete this one, and so do we.
    pub is_head: bool,
    /// Reachable from HEAD, so the branch you are on already holds every commit.
    pub merged: bool,
    /// The branch HEAD is on, so the answer can say what "merged" was measured
    /// against. `None` on a detached HEAD, where there is no branch to name.
    pub head: Option<String>,
    /// Other local branches that can also reach the tip. A branch merged into
    /// `develop` while you stand on `main` is not lost either, and saying
    /// "reachable from nothing" about it would be a lie. It is not a promise
    /// of safety either: `staging` holds everything until the day it is reset.
    pub also_on: Vec<String>,
    /// The branch everything below was measured against — the repository's
    /// trunk, or HEAD when it has no trunk to speak of.
    pub against: Option<String>,
    /// The trunk already holds every commit on this branch. This, rather than
    /// `merged`, is the question worth answering: whether the work has landed
    /// where work lands.
    pub trunk_holds: bool,
    /// Commits on this branch that `against` cannot reach: what a local delete
    /// would actually cost, when it costs anything.
    pub only_here: usize,
    pub upstream: Option<String>,
    /// Commits on this branch that its upstream does not have.
    pub unpushed: usize,
    /// The copy on the remote this branch belongs to — the only remote a
    /// delete is ever offered for.
    pub remote: Option<RemoteCopy>,
    /// Copies on any other remote: forks, mirrors, a colleague's clone. Named
    /// so nothing comes as a surprise, never deleted from here.
    pub other_remotes: Vec<String>,
}

pub fn deletion_preview(state: &AppState, name: &str) -> Result<BranchDeletion, String> {
    let path = state.path()?;
    let repo = state.repo()?;
    let branch = repo
        .find_branch(name, BranchType::Local)
        .map_err(|_| format!("No local branch named {name}"))?;
    let oid = branch
        .get()
        .target()
        .ok_or_else(|| format!("Branch {name} has no commit"))?;

    let head_ref = repo.head().ok();
    let head_oid = head_ref.as_ref().and_then(|h| h.target());
    let detached = repo.head_detached().unwrap_or(false);
    let head = head_ref
        .as_ref()
        .filter(|_| !detached)
        .and_then(|h| h.shorthand().map(String::from));

    // Merged means HEAD can already reach it, which is the question `git
    // branch -d` asks and so decides whether deleting needs forcing.
    let merged = match head_oid {
        Some(head_oid) => head_oid == oid || repo.graph_descendant_of(head_oid, oid).unwrap_or(false),
        None => false,
    };

    // Whether the work has landed is a question about the trunk, not about
    // wherever you happen to be standing. Falling back to HEAD only when the
    // repository has no trunk at all keeps an answer for the odd clone that
    // has neither main nor master.
    let trunk = trunk(state);
    let against_oid = trunk
        .name
        .as_deref()
        .and_then(|name| {
            repo.find_branch(name, BranchType::Local)
                .or_else(|_| repo.find_branch(name, BranchType::Remote))
                .ok()
        })
        .and_then(|found| found.get().target())
        .or(head_oid);
    let against = trunk.name.clone().or_else(|| head.clone());
    let trunk_holds = match against_oid {
        Some(base) => base == oid || repo.graph_descendant_of(base, oid).unwrap_or(false),
        None => false,
    };

    // The other local branches holding this history. `git branch --contains`
    // answers in one walk what a descendant check per branch would pay for
    // several times over, and it is the same question git itself asks.
    let also_on = git_cmd::run_checked(
        &path,
        &["branch", "--contains", &oid.to_string(), "--format=%(refname:short)"],
    )
    .map(|raw| {
        raw.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            // A detached HEAD is listed as "(HEAD detached at abc1234)", which
            // is not a branch anybody can be told to look at.
            .filter(|line| !line.starts_with('('))
            // Not the branch itself, and not the trunk — the trunk is the
            // answer above, and repeating it here says nothing new. The branch
            // you are standing on stays in: now that landing is judged against
            // the trunk, HEAD holding the work is the same weak evidence as any
            // other branch holding it, and leaving it out of the list would
            // hide the very branch the reader is looking at.
            .filter(|line| *line != name && Some(*line) != trunk.name.as_deref())
            .map(String::from)
            .collect()
    })
    .unwrap_or_default();

    // What this branch holds that the trunk does not. `trunk_holds` is the same
    // question asked as a yes or no; this is the number to put in the warning.
    let only_here = match against_oid {
        Some(base) => repo
            .graph_ahead_behind(oid, base)
            .map(|(ahead, _)| ahead)
            .unwrap_or(0),
        None => 0,
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

    // The remote this branch belongs to: the one it tracks, else the one the
    // repository is really about. Copies on any other remote are somebody
    // else's to delete.
    let home = upstream
        .as_ref()
        .and_then(|full| full.split_once('/').map(|(remote, _)| remote.to_string()))
        .or_else(|| remote::primary(state));

    let mut remote = None;
    let mut other_remotes = Vec::new();
    if let Ok(list) = repo.branches(Some(BranchType::Remote)) {
        for entry in list.flatten() {
            let (remote_branch, _) = entry;
            let Some(full) = remote_branch.name().ok().flatten() else {
                continue;
            };
            if full.ends_with("/HEAD") {
                continue;
            }
            let Some((host, rest)) = full.split_once('/') else {
                continue;
            };
            if rest != name {
                continue;
            }
            if Some(host) == home.as_deref() {
                // What the remote holds that the trunk cannot reach: the same
                // yardstick the local half uses, so the two halves of the
                // answer agree with each other.
                let base = against_oid.unwrap_or(oid);
                let unmerged = remote_branch
                    .get()
                    .target()
                    .and_then(|remote_oid| repo.graph_ahead_behind(base, remote_oid).ok())
                    .map(|(_, behind)| behind)
                    .unwrap_or(0);
                remote = Some(RemoteCopy {
                    name: full.to_string(),
                    remote: host.to_string(),
                    unmerged,
                });
            } else {
                other_remotes.push(full.to_string());
            }
        }
    }
    other_remotes.sort();

    Ok(BranchDeletion {
        name: name.to_string(),
        // By name, not by commit: another branch can sit on the same commit as
        // HEAD without being the one checked out.
        is_head: !detached && head.as_deref() == Some(name),
        merged,
        head,
        also_on,
        against,
        trunk_holds,
        only_here,
        upstream,
        unpushed,
        remote,
        other_remotes,
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
    // On a case-insensitive filesystem (the default on macOS and Windows) git
    // refuses a rename that changes only the letter case of a branch name: its
    // "already exists" check finds the very branch being renamed. Going through
    // a temporary name sidesteps it. HEAD follows the branch through both hops,
    // so the checked-out branch ends up on the new name either way.
    if from != to && from.eq_ignore_ascii_case(to) {
        let repo = state.repo()?;
        let mut suffix = 0;
        let temp = loop {
            let candidate = if suffix == 0 {
                format!("renaming-{to}")
            } else {
                format!("renaming-{to}-{suffix}")
            };
            if repo.find_branch(&candidate, BranchType::Local).is_err() {
                break candidate;
            }
            suffix += 1;
        };
        git_cmd::run_checked(&root, &["branch", "-m", from, &temp])?;
        git_cmd::run_checked(&root, &["branch", "-m", &temp, to])?;
    } else {
        git_cmd::run_checked(&root, &["branch", "-m", from, to])?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A repository of its own, with somewhere to push to, removed when the
    /// test ends.
    struct Sandbox {
        root: PathBuf,
        work: PathBuf,
        state: AppState,
    }

    impl Sandbox {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "gitnoob-refs-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();

            let server = root.join("server.git");
            run(&root, &["init", "--quiet", "--bare", "server.git"]);
            let work = root.join("work");
            std::fs::create_dir_all(&work).unwrap();
            run(&work, &["init", "--quiet", "--initial-branch=main"]);
            run(&work, &["config", "user.email", "test@example.com"]);
            run(&work, &["config", "user.name", "Test"]);
            run(&work, &["remote", "add", "origin", server.to_str().unwrap()]);

            let state = AppState::new(root.join("config"));
            state.set_path(work.clone());
            let sandbox = Sandbox { root, work, state };
            sandbox.commit("first");
            run(&sandbox.work, &["push", "--quiet", "-u", "origin", "main"]);
            sandbox
        }

        fn git(&self, args: &[&str]) -> String {
            run(&self.work, args)
        }

        /// A commit on whatever is checked out, with content nobody else will
        /// write, so no two commits share an oid.
        fn commit(&self, message: &str) -> String {
            let file = self.work.join("log.txt");
            let mut text = std::fs::read_to_string(&file).unwrap_or_default();
            text.push_str(message);
            text.push('\n');
            std::fs::write(&file, text).unwrap();
            self.git(&["add", "-A"]);
            self.git(&["commit", "--quiet", "-m", message]);
            self.git(&["rev-parse", "HEAD"]).trim().to_string()
        }

        /// A second clone, for the commits somebody else pushes.
        fn elsewhere(&self, branch: &str, message: &str) {
            let other = self.root.join("other");
            if !other.exists() {
                run(
                    &self.root,
                    &["clone", "--quiet", self.root.join("server.git").to_str().unwrap(), "other"],
                );
                run(&other, &["config", "user.email", "them@example.com"]);
                run(&other, &["config", "user.name", "Them"]);
            }
            run(&other, &["checkout", "--quiet", branch]);
            let file = other.join("log.txt");
            let mut text = std::fs::read_to_string(&file).unwrap_or_default();
            text.push_str(message);
            text.push('\n');
            std::fs::write(&file, text).unwrap();
            run(&other, &["add", "-A"]);
            run(&other, &["commit", "--quiet", "-m", message]);
            run(&other, &["push", "--quiet", "origin", branch]);
            self.git(&["fetch", "--quiet", "origin"]);
        }

        fn preview(&self, name: &str) -> BranchDeletion {
            deletion_preview(&self.state, name).unwrap()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn run(dir: &Path, args: &[&str]) -> String {
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

    #[test]
    fn a_merged_branch_is_merged_however_far_ahead_of_its_upstream_it_is() {
        let sandbox = Sandbox::new("merged-unpushed");
        sandbox.git(&["checkout", "--quiet", "-b", "feature"]);
        sandbox.git(&["push", "--quiet", "-u", "origin", "feature"]);
        sandbox.commit("work nobody has pushed");
        sandbox.git(&["checkout", "--quiet", "main"]);
        sandbox.git(&["merge", "--quiet", "--no-ff", "-m", "merge", "feature"]);

        let preview = sandbox.preview("feature");
        // The commit is on main. That it never reached origin/feature costs
        // nothing, and the old wording called it lost.
        assert!(preview.merged);
        assert_eq!(preview.unpushed, 1);
        assert_eq!(preview.only_here, 0, "main holds the commit");
        assert_eq!(preview.head.as_deref(), Some("main"));
    }

    #[test]
    fn an_unmerged_branch_that_is_fully_pushed_still_names_its_upstream() {
        let sandbox = Sandbox::new("unmerged-pushed");
        sandbox.git(&["checkout", "--quiet", "-b", "feature"]);
        sandbox.commit("work");
        sandbox.git(&["push", "--quiet", "-u", "origin", "feature"]);
        sandbox.git(&["checkout", "--quiet", "main"]);

        let preview = sandbox.preview("feature");
        assert!(!preview.merged);
        // Nothing is ahead of the upstream, so the remote holds every commit —
        // the opposite of "no upstream holding a copy".
        assert_eq!(preview.unpushed, 0);
        assert_eq!(preview.only_here, 1);
        assert_eq!(preview.upstream.as_deref(), Some("origin/feature"));
        assert_eq!(preview.remote.as_ref().map(|r| r.unmerged), Some(1));
    }

    #[test]
    fn commits_only_on_the_remote_are_counted_even_when_the_local_branch_is_merged() {
        let sandbox = Sandbox::new("remote-ahead");
        sandbox.git(&["checkout", "--quiet", "-b", "feature"]);
        sandbox.commit("work");
        sandbox.git(&["push", "--quiet", "-u", "origin", "feature"]);
        sandbox.git(&["checkout", "--quiet", "main"]);
        sandbox.git(&["merge", "--quiet", "--no-ff", "-m", "merge", "feature"]);
        // Somebody pushes to the branch after it was merged here.
        sandbox.elsewhere("feature", "their later work");

        let preview = sandbox.preview("feature");
        assert!(preview.merged);
        assert_eq!(preview.unpushed, 0);
        // Deleting on the remote would strand this one, and the local half of
        // the answer says nothing about it.
        let remote = preview.remote.expect("origin has a copy");
        assert_eq!(remote.name, "origin/feature");
        assert_eq!(remote.unmerged, 1);
    }

    #[test]
    fn a_branch_merged_into_another_local_branch_says_which_one() {
        let sandbox = Sandbox::new("also-on");
        sandbox.git(&["checkout", "--quiet", "-b", "feature"]);
        sandbox.commit("work");
        sandbox.git(&["checkout", "--quiet", "-b", "develop"]);
        sandbox.git(&["checkout", "--quiet", "main"]);

        let preview = sandbox.preview("feature");
        assert!(!preview.merged, "main cannot reach it");
        assert_eq!(preview.only_here, 1);
        assert_eq!(preview.also_on, vec!["develop".to_string()]);
    }

    #[test]
    fn copies_on_other_remotes_are_named_but_not_the_one_offered() {
        let sandbox = Sandbox::new("fork");
        let fork = sandbox.root.join("fork.git");
        run(&sandbox.root, &["init", "--quiet", "--bare", "fork.git"]);
        sandbox.git(&["remote", "add", "fork", fork.to_str().unwrap()]);
        sandbox.git(&["checkout", "--quiet", "-b", "feature"]);
        sandbox.commit("work");
        sandbox.git(&["push", "--quiet", "-u", "origin", "feature"]);
        sandbox.git(&["push", "--quiet", "fork", "feature"]);
        sandbox.git(&["fetch", "--quiet", "fork"]);
        sandbox.git(&["checkout", "--quiet", "main"]);

        let preview = sandbox.preview("feature");
        assert_eq!(
            preview.remote.as_ref().map(|r| r.remote.as_str()),
            Some("origin"),
            "the branch's own remote, never the fork"
        );
        assert_eq!(preview.other_remotes, vec!["fork/feature".to_string()]);
    }

    #[test]
    fn the_checked_out_branch_is_marked_as_such() {
        let sandbox = Sandbox::new("head");
        sandbox.git(&["checkout", "--quiet", "-b", "feature"]);

        assert!(sandbox.preview("feature").is_head);
        assert!(!sandbox.preview("main").is_head);
    }
}
