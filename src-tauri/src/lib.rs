pub mod ai;
pub mod avatar;
pub mod blame;
pub mod config;
pub mod conflict;
pub mod create;
pub mod diff;
pub mod forge;
pub mod git_cmd;
pub mod graph;
pub mod journal;
pub mod lfs;
pub mod rebase;
pub mod refs;
pub mod remote;
pub mod review;
pub mod sign;
pub mod ssh;
pub mod submodule;
pub mod state;
pub mod watch;
pub mod work;
pub mod worktree;

use std::path::PathBuf;

use state::AppState;
use tauri::{Emitter, Manager, State};

// --- repository -------------------------------------------------------------

/// Opens a repository and, unless told otherwise, records it in the active
/// profile's tab strip.
///
/// `record: false` is for stepping into a submodule: it is a repository of its
/// own and everything below has to point at it, but it is not a project the
/// user opened and it should not turn into a tab of its own or into a recent.
#[tauri::command]
async fn open_repo(
    path: String,
    record: Option<bool>,
    app: tauri::AppHandle,
    watching: State<'_, watch::Slot>,
    state: State<'_, AppState>,
) -> Result<refs::RepoInfo, String> {
    let root = state::discover_workdir(&PathBuf::from(&path))?;
    let recorded = root.to_string_lossy().into_owned();
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| recorded.clone());

    state.set_path(root);
    if record.unwrap_or(true) {
        state.update_config(|config| {
            if let Some(profile) = config.active_mut() {
                if !profile.projects.iter().any(|p| p.path == recorded) {
                    profile.projects.push(config::Project {
                        path: recorded.clone(),
                        name: name.clone(),
                    });
                }
                config::remember_recent(profile, &recorded, &name);
                profile.active_project = Some(recorded.clone());
            }
        })?;
    }

    // Watch this one instead of whichever was open before. Assigning replaces
    // the old watch, and dropping it stops its thread.
    *watching.lock().unwrap() = watch::start(app, PathBuf::from(&recorded));

    refs::describe(&state)
}

/// The repository named on the command line, if any: `gitnoob /path/to/repo`.
#[tauri::command]
fn startup_repo() -> Option<String> {
    std::env::args()
        .skip(1)
        // Skip anything that looks like a flag; only a bare path names a repo.
        .find(|arg| !arg.starts_with('-'))
}

/// Clones a repository into a folder named after it.
///
/// A clone is the slowest thing the app does, so it runs on the blocking pool
/// rather than the thread that would otherwise be drawing the window. Being a
/// git CLI run, the profile's key and the machine's credential helper are
/// already in force.
#[tauri::command]
async fn clone_repo(url: String, parent: String) -> Result<create::NewRepo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let parent = PathBuf::from(&parent);
        if !parent.is_dir() {
            return Err(format!("{} is not a folder", parent.display()));
        }
        create::clone(&url, &parent)
    })
    .await
    .map_err(|e| format!("The clone did not finish: {e}"))?
}

/// Creates a new repository with a first commit and a starter `.gitignore`,
/// committing as the active profile where one has an identity.
#[tauri::command]
async fn init_repo(
    name: String,
    parent: String,
    state: State<'_, AppState>,
) -> Result<create::NewRepo, String> {
    // Read here rather than inside the closure: `State` cannot cross into the
    // blocking pool, and the config is small.
    let identity = state.config().active().and_then(|profile| {
        let name = profile.git_name.clone().filter(|s| !s.trim().is_empty());
        let email = profile.git_email.clone().filter(|s| !s.trim().is_empty());
        Some((name?, email?))
    });
    tauri::async_runtime::spawn_blocking(move || {
        let parent = PathBuf::from(&parent);
        if !parent.is_dir() {
            return Err(format!("{} is not a folder", parent.display()));
        }
        create::init(&parent, &name, identity)
    })
    .await
    .map_err(|e| format!("Creating the repository did not finish: {e}"))?
}

#[tauri::command]
fn repo_info(state: State<'_, AppState>) -> Result<refs::RepoInfo, String> {
    refs::describe(&state)
}

#[tauri::command]
async fn ref_tree(state: State<'_, AppState>) -> Result<refs::RefTree, String> {
    refs::tree(&state)
}

#[tauri::command]
async fn working_status(state: State<'_, AppState>) -> Result<refs::WorkingStatus, String> {
    refs::status(&state)
}

// --- history ----------------------------------------------------------------

#[tauri::command]
async fn commit_graph(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<graph::GraphPage, String> {
    let fallback = state.config().global.graph_page_size;
    graph::build(&state, limit.unwrap_or(fallback))
}

#[tauri::command]
async fn commit_detail(
    oid: String,
    state: State<'_, AppState>,
) -> Result<diff::CommitDetail, String> {
    diff::commit_detail(&state, &oid)
}

#[tauri::command]
async fn commit_file_diff(
    oid: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<diff::FileDiff, String> {
    diff::commit_file_diff(&state, &oid, &path)
}

#[tauri::command]
async fn working_file_diff(
    path: String,
    side: diff::Side,
    state: State<'_, AppState>,
) -> Result<diff::FileDiff, String> {
    diff::working_file_diff(&state, &path, side)
}

/// The whole file, for the view that shows the changes in place rather than
/// pulled out into hunks.
#[tauri::command]
async fn file_text(
    path: String,
    commit: Option<String>,
    side: Option<diff::Side>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    diff::file_text(
        &state,
        &path,
        commit.as_deref(),
        matches!(side, Some(diff::Side::Staged)),
    )
}

// --- branches ---------------------------------------------------------------

#[tauri::command]
async fn checkout(
    name: String,
    state: State<'_, AppState>,
) -> Result<refs::CheckoutOutcome, String> {
    refs::checkout(&state, &name)
}

#[tauri::command]
async fn create_branch(
    name: String,
    start: Option<String>,
    checkout: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    refs::create_branch(&state, &name, start.as_deref(), checkout.unwrap_or(true))
}

/// How far back a commit sits in the graph's walk, so a page big enough to
/// hold its row can be asked for.
#[tauri::command]
async fn commit_depth(oid: String, state: State<'_, AppState>) -> Result<Option<usize>, String> {
    graph::depth(&state, &oid)
}

/// Checks out the branch a review was opened from, adding the fork it lives
/// in as a remote when that is what standing on those commits takes.
#[tauri::command]
async fn checkout_review(
    review: review::ReviewTarget,
    state: State<'_, AppState>,
) -> Result<String, String> {
    review::checkout(&state, review)
}

/// The branch this repository is organised around, which is what "has this
/// work landed?" is really asking about.
#[tauri::command]
fn trunk_branch(state: State<'_, AppState>) -> refs::Trunk {
    refs::trunk(&state)
}

/// Names that branch, or forgets the name when given nothing.
#[tauri::command]
async fn set_trunk_branch(
    name: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    refs::set_trunk(&state, name.as_deref())
}

/// What deleting a branch would cost, read before the question is asked.
#[tauri::command]
async fn delete_branch_preview(
    name: String,
    state: State<'_, AppState>,
) -> Result<refs::BranchDeletion, String> {
    refs::deletion_preview(&state, &name)
}

#[tauri::command]
async fn delete_branch(
    name: String,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    refs::delete_branch(&state, &name, force.unwrap_or(false))
}

#[tauri::command]
async fn rename_branch(
    from: String,
    to: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    refs::rename_branch(&state, &from, &to)
}

#[tauri::command]
fn set_upstream(
    branch: String,
    upstream: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    refs::set_upstream(&state, &branch, &upstream)
}

#[tauri::command]
fn unset_upstream(branch: String, state: State<'_, AppState>) -> Result<String, String> {
    refs::unset_upstream(&state, &branch)
}

/// Local branches whose upstream is gone, i.e. safe to tidy away.
#[tauri::command]
fn stale_branches(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    refs::stale_branches(&state)
}

// --- worktrees ---------------------------------------------------------------

/// Every folder this repository is checked out into.
#[tauri::command]
async fn worktree_list(state: State<'_, AppState>) -> Result<Vec<worktree::Worktree>, String> {
    worktree::list(&state)
}

/// Checks a branch out into a new folder, creating it from a remote-tracking
/// ref when `track` names one.
#[tauri::command]
async fn worktree_add(
    path: String,
    branch: String,
    track: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    worktree::add(&state, &path, &branch, track.as_deref())
}

/// Removes a worktree; `force` throws its uncommitted work away too.
#[tauri::command]
async fn worktree_remove(
    path: String,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    worktree::remove(&state, &path, force.unwrap_or(false))
}

#[tauri::command]
fn add_to_gitignore(pattern: String, state: State<'_, AppState>) -> Result<String, String> {
    refs::add_to_gitignore(&state, &pattern)
}

// --- git lfs -----------------------------------------------------------------

/// Whether this repository uses LFS, and whether the tool for it is here.
#[tauri::command]
fn lfs_status(state: State<'_, AppState>) -> Result<lfs::Status, String> {
    lfs::status(&state)
}

/// Fetches the real contents of one LFS file, or of every one of them.
#[tauri::command]
async fn lfs_pull(path: Option<String>, state: State<'_, AppState>) -> Result<String, String> {
    lfs::pull(&state, path.as_deref())
}

// --- interactive rebase -------------------------------------------------------

/// The commits between a chosen one and HEAD, oldest first, for the plan.
#[tauri::command]
async fn rebase_plan(
    onto: String,
    state: State<'_, AppState>,
) -> Result<Vec<rebase::Candidate>, String> {
    rebase::plan(&state, &onto)
}

/// Runs the plan the window built.
#[tauri::command]
async fn rebase_start(
    onto: String,
    steps: Vec<rebase::Step>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    rebase::start(&state, &onto, steps)
}

/// Where a rebase has got to, or nothing when none is running.
#[tauri::command]
async fn rebase_progress(state: State<'_, AppState>) -> Result<Option<rebase::Progress>, String> {
    rebase::progress(&state)
}

#[tauri::command]
async fn rebase_continue(state: State<'_, AppState>) -> Result<String, String> {
    rebase::resume(&state)
}

#[tauri::command]
async fn rebase_skip(state: State<'_, AppState>) -> Result<String, String> {
    rebase::skip(&state)
}

#[tauri::command]
async fn rebase_abort(state: State<'_, AppState>) -> Result<String, String> {
    rebase::abort(&state)
}

/// Gives the commit a stopped rebase is sitting on a new message, and goes on.
#[tauri::command]
async fn rebase_reword(message: String, state: State<'_, AppState>) -> Result<String, String> {
    rebase::reword(&state, &message)
}

// --- blame and file history --------------------------------------------------

/// Who last touched each line of a file, as runs of consecutive lines.
#[tauri::command]
async fn blame_file(
    path: String,
    commit: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<blame::BlameRun>, String> {
    blame::of(&state, &path, commit.as_deref())
}

/// Every commit that touched a file, newest first, across renames.
#[tauri::command]
async fn file_history(
    path: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<blame::FileCommit>, String> {
    blame::history(&state, &path, limit.unwrap_or(200))
}

// --- signatures --------------------------------------------------------------

/// What git makes of the signature on every commit of the current page.
///
/// Asked for only while the setting is on: it runs the machine's gpg or
/// ssh-keygen once per commit, which is not a thing to do on every refresh
/// without being told to.
#[tauri::command]
async fn signature_marks(
    limit: usize,
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, sign::Mark>, String> {
    if !state.config().global.verify_signatures {
        return Ok(std::collections::HashMap::new());
    }
    sign::marks(&state, limit)
}

/// Everything git will say about one commit's signature.
#[tauri::command]
async fn commit_signature(
    oid: String,
    state: State<'_, AppState>,
) -> Result<sign::Signature, String> {
    sign::of(&state, &oid)
}

/// Whether a commit made in this repository right now would be signed.
#[tauri::command]
fn signing_setup(state: State<'_, AppState>) -> Result<sign::Setup, String> {
    sign::setup(&state)
}

// --- submodules --------------------------------------------------------------

/// Every repository kept inside this one.
#[tauri::command]
async fn submodule_list(state: State<'_, AppState>) -> Result<Vec<submodule::Submodule>, String> {
    submodule::list(&state)
}

/// Clones what is missing and moves each one onto the commit this repository
/// records. Without a path, all of them.
#[tauri::command]
async fn submodule_update(
    path: Option<String>,
    recursive: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    submodule::update(&state, path.as_deref(), recursive.unwrap_or(false))
}

/// Copies the URLs in `.gitmodules` over the ones each was cloned with.
#[tauri::command]
async fn submodule_sync(
    path: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    submodule::sync(&state, path.as_deref())
}

/// Adds a repository as a submodule, cloning it into `path`.
#[tauri::command]
async fn submodule_add(
    url: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    submodule::add(&state, &url, &path)
}

/// Empties a submodule's folder while leaving it declared.
#[tauri::command]
async fn submodule_deinit(
    path: String,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    submodule::deinit(&state, &path, force.unwrap_or(false))
}

/// Takes a submodule out of the working tree, the index and `.gitmodules`.
#[tauri::command]
async fn submodule_remove(
    path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    submodule::remove(&state, &path)
}

/// A git command typed into the log's prompt, run in the open repository.
#[tauri::command]
async fn run_git(
    args: Vec<String>,
    state: State<'_, AppState>,
) -> Result<git_cmd::CmdOutput, String> {
    let cwd = state.path()?;
    tauri::async_runtime::spawn_blocking(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        git_cmd::run_typed(&cwd, &args)
    })
    .await
    .map_err(|e| format!("git did not finish: {e}"))?
}

#[tauri::command]
fn remotes(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    remote::remotes(&state)
}

// --- managing the remotes themselves ----------------------------------------

/// The address a remote fetches from, shown when it is about to be edited.
#[tauri::command]
fn remote_url(remote: String, state: State<'_, AppState>) -> Result<String, String> {
    remote::remote_url(&state, &remote)
}

#[tauri::command]
async fn remote_add(
    name: String,
    url: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    remote::remote_add(&state, &name, &url)
}

#[tauri::command]
async fn remote_set_url(
    name: String,
    url: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    remote::remote_set_url(&state, &name, &url)
}

#[tauri::command]
async fn remote_rename(
    from: String,
    to: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    remote::remote_rename(&state, &from, &to)
}

#[tauri::command]
async fn remote_remove(name: String, state: State<'_, AppState>) -> Result<String, String> {
    remote::remote_remove(&state, &name)
}

#[tauri::command]
fn can_fast_forward(
    branch: String,
    onto: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    remote::can_fast_forward(&state, &branch, &onto)
}

/// How two branches stand to each other, so a menu can offer only the moves
/// that would actually do something.
#[tauri::command]
fn branch_relation(
    source: String,
    target: String,
    state: State<'_, AppState>,
) -> Result<remote::BranchRelation, String> {
    remote::relation(&state, &source, &target)
}

#[tauri::command]
async fn delete_remote_branch(
    remote_name: String,
    branch: String,
    state: State<'_, AppState>,
) -> Result<git_cmd::CmdOutput, String> {
    remote::delete_remote_branch(&state, &remote_name, &branch)
}

#[tauri::command]
async fn push_tag(
    remote_name: String,
    tag: String,
    state: State<'_, AppState>,
) -> Result<git_cmd::CmdOutput, String> {
    remote::push_tag(&state, &remote_name, &tag)
}

#[tauri::command]
async fn delete_remote_tag(
    remote_name: String,
    tag: String,
    state: State<'_, AppState>,
) -> Result<git_cmd::CmdOutput, String> {
    remote::delete_remote_tag(&state, &remote_name, &tag)
}

#[tauri::command]
async fn commit_patch(oid: String, state: State<'_, AppState>) -> Result<String, String> {
    work::commit_patch(&state, &oid)
}

#[tauri::command]
fn reveal(path: String, state: State<'_, AppState>) -> Result<(), String> {
    work::reveal(&state, &path)
}

// --- working tree -----------------------------------------------------------

#[tauri::command]
async fn stage(paths: Vec<String>, state: State<'_, AppState>) -> Result<String, String> {
    work::stage(&state, &paths)
}

#[tauri::command]
async fn stage_all(state: State<'_, AppState>) -> Result<String, String> {
    work::stage_all(&state)
}

#[tauri::command]
async fn unstage(paths: Vec<String>, state: State<'_, AppState>) -> Result<String, String> {
    work::unstage(&state, &paths)
}

#[tauri::command]
async fn discard(paths: Vec<String>, state: State<'_, AppState>) -> Result<String, String> {
    work::discard(&state, &paths)
}

/// Stage, unstage or discard a single hunk of a file.
#[tauri::command]
async fn apply_hunk(
    path: String,
    hunk_index: usize,
    action: work::HunkAction,
    lines: Option<work::Lines>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::apply_hunk(&state, &path, hunk_index, action, lines)
}

#[tauri::command]
async fn commit(
    message: String,
    amend: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::commit(&state, &message, amend.unwrap_or(false))
}

#[tauri::command]
fn amend_draft(state: State<'_, AppState>) -> Result<work::AmendDraft, String> {
    work::amend_draft(&state)
}

#[tauri::command]
fn reword_check(oid: String, state: State<'_, AppState>) -> Result<work::RewordCheck, String> {
    work::reword_check(&state, &oid)
}

#[tauri::command]
async fn reword(
    oid: String,
    message: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::reword(&state, &oid, &message)
}

#[tauri::command]
async fn stash_push(
    message: Option<String>,
    include_untracked: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::stash_push(
        &state,
        message.as_deref(),
        include_untracked.unwrap_or(true),
    )
}

#[tauri::command]
async fn stash_pop(index: usize, state: State<'_, AppState>) -> Result<String, String> {
    work::stash_pop(&state, index)
}

#[tauri::command]
async fn stash_list(state: State<'_, AppState>) -> Result<Vec<work::StashEntry>, String> {
    work::stash_list(&state)
}

#[tauri::command]
async fn stash_apply(index: usize, state: State<'_, AppState>) -> Result<String, String> {
    work::stash_apply(&state, index)
}

/// Applies several stashes in one go, oldest first, dropping each that goes on
/// cleanly when `drop_after` says to.
#[tauri::command]
async fn stash_apply_many(
    indexes: Vec<usize>,
    drop_after: Option<bool>,
    state: State<'_, AppState>,
) -> Result<work::StashRun, String> {
    work::stash_apply_many(&state, indexes, drop_after.unwrap_or(false))
}

#[tauri::command]
async fn stash_drop(index: usize, state: State<'_, AppState>) -> Result<String, String> {
    work::stash_drop(&state, index)
}

/// Gives a stash a new description, leaving it where it is in the list.
#[tauri::command]
async fn stash_rename(
    index: usize,
    message: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::stash_rename(&state, index, &message)
}

/// Deletes files git is not tracking.
#[tauri::command]
async fn delete_untracked(
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::delete_untracked(&state, &paths)
}

#[tauri::command]
async fn stash_branch(
    index: usize,
    name: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::stash_branch(&state, index, &name)
}

/// The commit a stash points at, so its diff can be shown like any commit's.
#[tauri::command]
fn stash_oid(index: usize, state: State<'_, AppState>) -> Result<String, String> {
    work::stash_oid(&state, index)
}

// --- moving a branch and replaying commits ----------------------------------

#[tauri::command]
async fn reset_preview(
    oid: String,
    state: State<'_, AppState>,
) -> Result<work::ResetPreview, String> {
    work::reset_preview(&state, &oid)
}

#[tauri::command]
async fn reset(
    oid: String,
    mode: work::ResetMode,
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::reset(&state, &oid, mode)
}

#[tauri::command]
async fn cherry_pick(
    oids: Vec<String>,
    options: Option<work::CherryPickOptions>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::cherry_pick(&state, &oids, options.unwrap_or_default())
}

#[tauri::command]
async fn revert(oid: String, state: State<'_, AppState>) -> Result<String, String> {
    work::revert(&state, &oid)
}

#[tauri::command]
fn create_tag(
    name: String,
    oid: String,
    message: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::create_tag(&state, &name, &oid, message.as_deref())
}

#[tauri::command]
fn delete_tag(name: String, state: State<'_, AppState>) -> Result<String, String> {
    work::delete_tag(&state, &name)
}

#[tauri::command]
fn commit_message_text(oid: String, state: State<'_, AppState>) -> Result<String, String> {
    work::commit_message_text(&state, &oid)
}

// --- undo and redo ----------------------------------------------------------

#[tauri::command]
fn history(state: State<'_, AppState>) -> journal::Stacks {
    journal::stacks(&state)
}

#[tauri::command]
async fn undo(state: State<'_, AppState>) -> Result<String, String> {
    journal::undo(&state)
}

#[tauri::command]
async fn redo(state: State<'_, AppState>) -> Result<String, String> {
    journal::redo(&state)
}

// --- remotes ----------------------------------------------------------------

#[tauri::command]
async fn fetch(
    remote: Option<String>,
    state: State<'_, AppState>,
) -> Result<git_cmd::CmdOutput, String> {
    remote::fetch(&state, remote.as_deref())
}

#[tauri::command]
async fn pull(
    rebase: Option<bool>,
    state: State<'_, AppState>,
) -> Result<git_cmd::CmdOutput, String> {
    remote::pull(&state, rebase.unwrap_or(false))
}

/// Brings any branch up to date, checked out or not.
#[tauri::command]
async fn pull_branch(
    branch: String,
    rebase: Option<bool>,
    state: State<'_, AppState>,
) -> Result<git_cmd::CmdOutput, String> {
    remote::pull_branch(&state, &branch, rebase.unwrap_or(false))
}

#[tauri::command]
async fn push_preview(
    branch: Option<String>,
    fetch_first: Option<bool>,
    state: State<'_, AppState>,
) -> Result<remote::PushPreview, String> {
    remote::push_preview(&state, branch.as_deref(), fetch_first.unwrap_or(false))
}

#[tauri::command]
async fn push(
    remote_name: String,
    branch: String,
    force: Option<bool>,
    set_upstream: Option<bool>,
    state: State<'_, AppState>,
) -> Result<git_cmd::CmdOutput, String> {
    remote::push(
        &state,
        &remote_name,
        &branch,
        force.unwrap_or(false),
        set_upstream.unwrap_or(false),
    )
}

#[tauri::command]
async fn merge(
    branch: String,
    no_ff: Option<bool>,
    state: State<'_, AppState>,
) -> Result<remote::MergeOutcome, String> {
    remote::merge(&state, &branch, no_ff.unwrap_or(false))
}

#[tauri::command]
async fn abort_merge(state: State<'_, AppState>) -> Result<String, String> {
    remote::abort_merge(&state)
}

#[tauri::command]
async fn rebase(onto: String, state: State<'_, AppState>) -> Result<remote::MergeOutcome, String> {
    remote::rebase(&state, &onto)
}

/// Merges one branch into another, whichever one happens to be checked out.
#[tauri::command]
async fn merge_into(
    source: String,
    target: String,
    no_ff: Option<bool>,
    state: State<'_, AppState>,
) -> Result<remote::MergeOutcome, String> {
    remote::merge_into(&state, &source, &target, no_ff.unwrap_or(false))
}

/// Rebases a branch onto another without the user standing on it first.
#[tauri::command]
async fn rebase_branch(
    branch: String,
    onto: String,
    state: State<'_, AppState>,
) -> Result<remote::MergeOutcome, String> {
    remote::rebase_branch(&state, &branch, &onto)
}

#[tauri::command]
async fn abort_rebase(state: State<'_, AppState>) -> Result<String, String> {
    remote::abort_rebase(&state)
}

#[tauri::command]
async fn continue_rebase(state: State<'_, AppState>) -> Result<remote::MergeOutcome, String> {
    remote::continue_rebase(&state)
}

/// Reports a merge, rebase, cherry-pick or revert that git is part-way through.
#[tauri::command]
fn in_progress(state: State<'_, AppState>) -> Result<remote::InProgress, String> {
    remote::in_progress(&state)
}

/// Puts the tree back the way it was before an auto-stash refused to go back
/// on, returning to the branch the work was taken from.
#[tauri::command]
async fn undo_restore(state: State<'_, AppState>) -> Result<String, String> {
    work::undo_restore(&state)
}

// --- conflicts --------------------------------------------------------------

#[tauri::command]
fn conflict_list(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    conflict::list(&state)
}

#[tauri::command]
async fn conflict_read(
    path: String,
    state: State<'_, AppState>,
) -> Result<conflict::ConflictFile, String> {
    conflict::read(&state, &path)
}

#[tauri::command]
async fn conflict_preview(
    path: String,
    choices: Vec<conflict::Resolution>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    conflict::preview(&state, &path, &choices)
}

#[tauri::command]
async fn conflict_resolve(
    path: String,
    choices: Vec<conflict::Resolution>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    conflict::resolve(&state, &path, &choices)
}

/// Takes one side in every conflicted file at once.
#[tauri::command]
async fn conflict_resolve_all(side: String, state: State<'_, AppState>) -> Result<String, String> {
    conflict::resolve_all(&state, &side)
}

/// Stages every conflicted file as it stands, if none of them has markers left.
#[tauri::command]
async fn conflict_stage_all(state: State<'_, AppState>) -> Result<String, String> {
    conflict::stage_all(&state)
}

/// Which conflicted files still have git's markers in them.
#[tauri::command]
async fn conflict_marked(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    conflict::marked(&state)
}

/// Ends a conflict by staging the file exactly as it stands on disk.
#[tauri::command]
async fn conflict_resolve_as_is(
    path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    conflict::resolve_as_is(&state, &path)
}

#[tauri::command]
async fn conflict_resolve_whole(
    path: String,
    side: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    conflict::resolve_whole(&state, &path, &side)
}

// --- configuration, profiles and projects -----------------------------------

#[tauri::command]
fn config_get(state: State<'_, AppState>) -> config::Config {
    state.config()
}

#[tauri::command]
fn config_set_global(
    global: config::Global,
    state: State<'_, AppState>,
) -> Result<config::Config, String> {
    state.update_config(|config| config.global = global)?;
    Ok(state.config())
}

/// Creates the profile when its id is unknown, updates it otherwise.
#[tauri::command]
fn profile_save(
    profile: config::Profile,
    state: State<'_, AppState>,
) -> Result<config::Config, String> {
    state.update_config(|config| {
        match config.profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => {
                // Open projects belong to the profile, not to the settings form.
                let projects = existing.projects.clone();
                let active = existing.active_project.clone();
                *existing = profile.clone();
                existing.projects = projects;
                existing.active_project = active;
            }
            None => config.profiles.push(profile.clone()),
        }
        if config.active_profile.is_none() {
            config.active_profile = Some(profile.id.clone());
        }
    })?;
    // The key may have been what changed.
    ssh::apply(&state.config());
    Ok(state.config())
}

#[tauri::command]
fn profile_delete(id: String, state: State<'_, AppState>) -> Result<config::Config, String> {
    state.update_config(|config| {
        // Never leave the app with no profile to work in.
        if config.profiles.len() > 1 {
            config.profiles.retain(|p| p.id != id);
            if config.active_profile.as_deref() == Some(id.as_str()) {
                config.active_profile = config.profiles.first().map(|p| p.id.clone());
            }
        }
    })?;
    let _ = config::secret_set(&config::forge_key(&id), "");
    ssh::apply(&state.config());
    Ok(state.config())
}

/// Switches profile, which also swaps the tab strip.
#[tauri::command]
fn profile_activate(id: String, state: State<'_, AppState>) -> Result<config::Config, String> {
    state.update_config(|config| {
        if config.profiles.iter().any(|p| p.id == id) {
            config.active_profile = Some(id.clone());
        }
    })?;
    // The repository that was open belongs to the previous profile, and so does
    // the key every git command should now be using.
    state.clear_path();
    ssh::apply(&state.config());
    Ok(state.config())
}

#[tauri::command]
fn project_close(path: String, state: State<'_, AppState>) -> Result<config::Config, String> {
    state.update_config(|config| {
        if let Some(profile) = config.active_mut() {
            profile.projects.retain(|p| p.path != path);
            if profile.active_project.as_deref() == Some(path.as_str()) {
                profile.active_project = profile.projects.last().map(|p| p.path.clone());
            }
        }
    })?;
    let open = state.path().ok().map(|p| p.to_string_lossy().into_owned());
    if open.as_deref() == Some(path.as_str()) {
        state.clear_path();
    }
    Ok(state.config())
}

/// Reorders the tab strip after a drag.
#[tauri::command]
fn project_reorder(
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<config::Config, String> {
    state.update_config(|config| {
        if let Some(profile) = config.active_mut() {
            let mut ordered: Vec<config::Project> = Vec::with_capacity(profile.projects.len());
            for path in &paths {
                if let Some(found) = profile.projects.iter().find(|p| &p.path == path) {
                    ordered.push(found.clone());
                }
            }
            // Anything the caller did not mention keeps its place at the end.
            for project in &profile.projects {
                if !ordered.iter().any(|p| p.path == project.path) {
                    ordered.push(project.clone());
                }
            }
            profile.projects = ordered;
        }
    })?;
    Ok(state.config())
}

/// Takes a repository out of the profile's recents. The folder is untouched.
#[tauri::command]
fn project_forget(path: String, state: State<'_, AppState>) -> Result<config::Config, String> {
    state.update_config(|config| {
        if let Some(profile) = config.active_mut() {
            profile.recents.retain(|one| one.path != path);
        }
    })?;
    Ok(state.config())
}

/// Writes the active profile's identity into this repository's local config.
///
/// A profile is a person, and choosing one is the whole statement, so this runs
/// by itself whenever a repository is opened. `Ok(None)` means there was
/// nothing to do — the profile carries no identity, or the repository already
/// commits as that person — and nothing is said in that case.
#[tauri::command]
fn apply_identity(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let root = state.path()?;
    let config = state.config();
    let Some(profile) = config.active() else {
        return Ok(None);
    };
    let mut said: Vec<String> = Vec::new();

    if let (Some(name), Some(email)) = (
        profile.git_name.clone().filter(|s| !s.trim().is_empty()),
        profile.git_email.clone().filter(|s| !s.trim().is_empty()),
    ) {
        if configured(&root, "user.name").as_deref() != Some(name.as_str())
            || configured(&root, "user.email").as_deref() != Some(email.as_str())
        {
            git_cmd::run_checked(&root, &["config", "--local", "user.name", &name])?;
            git_cmd::run_checked(&root, &["config", "--local", "user.email", &email])?;
            said.push(format!("Committing here as {name} <{email}>"));
        }
    }

    // Signing is applied separately from the identity, because a profile may
    // well set one and not the other. Each setting the profile has no opinion
    // about is left exactly as the machine had it: a `None` here means "say
    // nothing", which is not the same as "off".
    let mut signing = Vec::new();
    if let Some(key) = profile.signing_key.clone().filter(|s| !s.trim().is_empty()) {
        signing.push(("user.signingkey", key));
    }
    if let Some(format) = profile
        .signing_format
        .clone()
        .filter(|s| !s.trim().is_empty())
    {
        signing.push(("gpg.format", format));
    }
    if let Some(on) = profile.sign_commits {
        signing.push(("commit.gpgsign", on.to_string()));
    }
    if let Some(on) = profile.sign_tags {
        signing.push(("tag.gpgsign", on.to_string()));
    }

    let mut signing_changed = false;
    for (key, value) in signing {
        if configured(&root, key).as_deref() == Some(value.as_str()) {
            continue;
        }
        git_cmd::run_checked(&root, &["config", "--local", key, &value])?;
        signing_changed = true;
    }
    if signing_changed {
        said.push(match profile.sign_commits {
            Some(false) => "Commits here are not signed".to_string(),
            _ => "Signing set up for this repository".to_string(),
        });
    }

    Ok((!said.is_empty()).then(|| said.join(". ")))
}

/// What git would use for a setting in this repository, local or inherited.
fn configured(root: &std::path::Path, key: &str) -> Option<String> {
    let out = git_cmd::run(root, &["config", "--get", key]).ok()?;
    out.ok
        .then(|| out.stdout.trim().to_string())
        .filter(|value| !value.is_empty())
}

// --- ssh --------------------------------------------------------------------

/// The key pairs in `~/.ssh`, so a profile can be pointed at one without the
/// user having to type a path.
#[tauri::command]
fn ssh_keys() -> Vec<ssh::SshKey> {
    ssh::list_keys()
}

/// Tries the profile's key against its forge and reports who the forge thinks
/// you are — the quickest way to catch a work key aimed at a personal account.
#[tauri::command]
async fn ssh_test(
    host: Option<String>,
    key: Option<String>,
    state: State<'_, AppState>,
) -> Result<ssh::SshTest, String> {
    let config = state.config();
    let profile = config.active();
    let host = host
        .filter(|h| !h.trim().is_empty())
        .or_else(|| profile.map(|p| p.host.clone()))
        .unwrap_or_default();
    let key = key.or_else(|| profile.and_then(|p| p.ssh_key.clone()));

    // ssh can sit for as long as ConnectTimeout allows, so keep it off the
    // thread that draws the window.
    tauri::async_runtime::spawn_blocking(move || ssh::test(&host, key.as_deref()))
        .await
        .map_err(|e| format!("The connection test did not finish: {e}"))?
}

#[tauri::command]
fn secret_set(key: String, value: String) -> Result<(), String> {
    config::secret_set(&key, &value)
}

/// Reports whether a secret exists, never what it is.
#[tauri::command]
fn secret_status(key: String) -> bool {
    config::secret_get(&key).is_some()
}

#[tauri::command]
fn forge_secret_key(state: State<'_, AppState>) -> Option<String> {
    state.active_profile_id().map(|id| config::forge_key(&id))
}

// --- forge ------------------------------------------------------------------

/// Where a token for this forge is created, so the settings form can link
/// straight there instead of describing the click path.
#[tauri::command]
fn forge_token_url(kind: config::ForgeKind, host: String) -> Option<String> {
    forge::token_url(kind, &host)
}

#[tauri::command]
fn forge_status(state: State<'_, AppState>) -> forge::ForgeStatus {
    forge::status(&state)
}

/// The account the active profile's token belongs to, with their picture.
#[tauri::command]
async fn forge_me(state: State<'_, AppState>) -> Result<forge::ForgeUser, String> {
    forge::me(&state).await
}

#[tauri::command]
async fn forge_check(state: State<'_, AppState>) -> Result<String, String> {
    forge::check(&state).await
}

#[tauri::command]
async fn forge_reviews(state: State<'_, AppState>) -> Result<Vec<forge::Review>, String> {
    forge::reviews(&state).await
}

/// Everything one review says about itself, asked for when it is opened rather
/// than on every refresh of the list.
#[tauri::command]
async fn forge_review_detail(
    number: i64,
    state: State<'_, AppState>,
) -> Result<forge::ReviewDetail, String> {
    forge::review_detail(&state, number).await
}

/// The repositories the active profile's token can see, so cloning is picking
/// one from a list rather than pasting an address.
#[tauri::command]
async fn forge_repos(state: State<'_, AppState>) -> Result<Vec<forge::ForgeRepo>, String> {
    forge::repos(&state).await
}

/// Everyone the project's review can be assigned to or reviewed by.
#[tauri::command]
async fn forge_members(state: State<'_, AppState>) -> Result<Vec<forge::Member>, String> {
    forge::members(&state).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn forge_create_review(
    source: Option<String>,
    target: String,
    title: String,
    body: String,
    draft: Option<bool>,
    assignees: Option<Vec<forge::Member>>,
    reviewers: Option<Vec<forge::Member>>,
    state: State<'_, AppState>,
) -> Result<forge::Review, String> {
    forge::create_review(
        &state,
        source,
        target,
        title,
        body,
        draft.unwrap_or(false),
        assignees.unwrap_or_default(),
        reviewers.unwrap_or_default(),
    )
    .await
}

/// The forge's own new-review page, with the form already filled in.
#[tauri::command]
fn forge_compare_url(
    source: String,
    target: String,
    title: String,
    body: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    forge::compare_url(&state, &source, &target, &title, &body)
}

/// The picture for each profile that has an account, keyed by profile id.
#[tauri::command]
async fn forge_faces(
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, String>, String> {
    Ok(forge::faces(&state).await)
}

// --- review conversation -----------------------------------------------------

/// Everything said under one review: conversation comments and diff threads.
#[tauri::command]
async fn forge_review_comments(
    number: i64,
    state: State<'_, AppState>,
) -> Result<Vec<forge::ReviewComment>, String> {
    forge::review_comments(&state, number).await
}

/// Every file a review changes across all of its commits, patches included.
#[tauri::command]
async fn forge_review_files(
    number: i64,
    state: State<'_, AppState>,
) -> Result<Vec<forge::ReviewFileChange>, String> {
    forge::review_files(&state, number).await
}

/// The commits a review's source branch puts ahead of its target.
#[tauri::command]
async fn forge_review_commits(
    number: i64,
    state: State<'_, AppState>,
) -> Result<Vec<forge::ReviewCommit>, String> {
    forge::review_commits(&state, number).await
}

/// Leaves one comment on the conversation itself.
#[tauri::command]
async fn forge_post_comment(
    number: i64,
    body: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    forge::post_comment(&state, number, &body).await
}

/// Answers one comment already on the record.
#[tauri::command]
async fn forge_reply_comment(
    number: i64,
    parent_id: i64,
    body: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    forge::reply_comment(&state, number, parent_id, &body).await
}

/// Starts a thread on one line of one file's diff.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn forge_add_diff_comment(
    number: i64,
    head_sha: String,
    base_sha: String,
    start_sha: String,
    path: String,
    line: i64,
    side: String,
    body: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    forge::add_diff_comment(
        &state, number, &head_sha, &base_sha, &start_sha, &path, line, &side, &body,
    )
    .await
}

/// Hands down a verdict: approve, request changes or plain comment.
#[tauri::command]
async fn forge_submit_review(
    number: i64,
    event: String,
    body: String,
    comments: Option<Vec<forge::PendingComment>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    forge::submit_review(&state, number, &event, &body, comments.unwrap_or_default()).await
}

/// Merges the review as it stands.
#[tauri::command]
async fn forge_merge_review(
    number: i64,
    squash: Option<bool>,
    delete_branch: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    forge::merge_review(
        &state,
        number,
        squash.unwrap_or(false),
        delete_branch.unwrap_or(false),
    )
    .await
}

/// Closes or reopens a review.
#[tauri::command]
async fn forge_set_review_state(
    number: i64,
    action: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    forge::set_review_state(&state, number, &action).await
}

/// Whether the review can land: its checks, its verdicts, its conflicts.
#[tauri::command]
async fn forge_review_status(
    number: i64,
    state: State<'_, AppState>,
) -> Result<forge::ReviewStatus, String> {
    forge::review_status(&state, number).await
}

/// Marks one diff thread settled, or unsettles it again.
#[tauri::command]
async fn forge_resolve_thread(
    number: i64,
    thread: String,
    resolved: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    forge::resolve_thread(&state, number, &thread, resolved).await
}

/// Sets who owns the review and who is being asked to look at it.
#[tauri::command]
async fn forge_set_review_people(
    number: i64,
    assignees: Vec<forge::Member>,
    reviewers: Vec<forge::Member>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    forge::set_review_people(&state, number, assignees, reviewers).await
}

/// Every label this project has.
#[tauri::command]
async fn forge_project_labels(state: State<'_, AppState>) -> Result<Vec<forge::Label>, String> {
    forge::project_labels(&state).await
}

/// Sets the review's labels to exactly these.
#[tauri::command]
async fn forge_set_labels(
    number: i64,
    labels: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    forge::set_labels(&state, number, labels).await
}

/// Rewrites the review's title and description.
#[tauri::command]
async fn forge_update_review(
    number: i64,
    title: String,
    body: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    forge::update_review(&state, number, &title, &body).await
}

/// Marks the review ready to be read, or puts it back to a draft.
#[tauri::command]
async fn forge_set_draft(
    number: i64,
    draft: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    forge::set_draft(&state, number, draft).await
}

/// One file as it stands at the review's head, for the whole-file view.
#[tauri::command]
async fn forge_review_file_text(
    number: i64,
    path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    forge::review_file_text(&state, number, &path).await
}

// --- authors ----------------------------------------------------------------

/// The picture for one commit author, or `None` when there is none to find.
#[tauri::command]
async fn avatar(email: String, state: State<'_, AppState>) -> Result<Option<String>, String> {
    avatar::find(&state, &email).await
}

// --- AI ---------------------------------------------------------------------

#[tauri::command]
fn ai_status(state: State<'_, AppState>) -> ai::AiStatus {
    ai::status(&state)
}

#[tauri::command]
async fn ai_models(
    refresh: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<ai::Model>, String> {
    ai::models(&state, refresh.unwrap_or(false)).await
}

#[tauri::command]
async fn ai_commit_message(state: State<'_, AppState>) -> Result<ai::CommitMessage, String> {
    ai::commit_message(&state).await
}

/// A commit message for a commit that already exists, from its own diff.
#[tauri::command]
async fn ai_commit_message_for(
    oid: String,
    state: State<'_, AppState>,
) -> Result<ai::CommitMessage, String> {
    ai::commit_message_for(&state, &oid).await
}

/// The title and description of a review, written from the branch's commits.
#[tauri::command]
async fn ai_review_message(
    source: String,
    target: String,
    state: State<'_, AppState>,
) -> Result<ai::CommitMessage, String> {
    ai::review_message(&state, source, target).await
}

#[tauri::command]
async fn ai_resolve_conflict(
    path: String,
    index: usize,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    ai::resolve_conflict(&state, path, index).await
}

// --- misc -------------------------------------------------------------------

/// Hands a URL to the desktop, for opening a pull request in the browser.
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    // Only ever open web links; never a local path or a shell command.
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("Only http and https links can be opened".to_string());
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open {url}: {e}"))
}

/**
 * Takes the animation off the scroll wheel.
 *
 * WebKitGTK smooths a wheel event into an animation and goes on running it
 * after the wheel has stopped, so a flick keeps travelling for a moment and the
 * window reads as lagging behind the hand — most of all in the commit list and
 * the diff, which are the two things in this app anybody scrolls. Chromium,
 * which is what every other desktop git client is built on, stops when you do,
 * and that is the feel being matched.
 *
 * There is no Tauri setting for it, so it is asked of the webview directly.
 * Failing to reach it is not worth a word to the user: the app works, the
 * scrolling is merely the one WebKit chose.
 */
/// Opens the window at the size it was left at.
///
/// Read through `sane`, which clamps it to something a person can still get
/// hold of and to something the screen can actually show — a size saved on a
/// large monitor and reopened on a laptop is the everyday way to end up with a
/// window bigger than the desktop and a title bar out of reach.
fn restore_size(window: &tauri::WebviewWindow, saved: Option<config::WindowSize>) {
    let Some(saved) = saved else { return };
    // What the screen can show, in the same units the window is sized in.
    let screen = window.current_monitor().ok().flatten().map(|monitor| {
        let size = monitor.size().to_logical::<f64>(monitor.scale_factor());
        (size.width, size.height)
    });
    let Some(size) = saved.sane(screen) else {
        return;
    };

    if saved.maximized {
        let _ = window.maximize();
        return;
    }
    let _ = window.set_size(tauri::LogicalSize::new(size.width, size.height));
    // Sizing a window leaves it where it was, which for a window that has grown
    // can be half off the screen.
    let _ = window.center();
}

/// Remembers the size the window is left at.
///
/// Written when the window is resized rather than when the app closes: a client
/// that is killed, crashes, or is quit by the machine shutting down would
/// otherwise never save anything, and this is a setting nobody would think to
/// check had been saved. Resizing sends an event per frame, so it is written
/// only once the size has held still for a moment.
fn remember_size(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let handle = app.clone();
    let pending: std::sync::Arc<std::sync::atomic::AtomicU64> =
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    window.on_window_event(move |event| {
        let tauri::WindowEvent::Resized(_) = event else {
            return;
        };
        // Each resize event cancels the last: only the one that is still the
        // newest a second later writes anything.
        let ticket = pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let handle = handle.clone();
        let pending = pending.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(700));
            if pending.load(std::sync::atomic::Ordering::SeqCst) != ticket {
                return;
            }
            let Some(window) = handle.get_webview_window("main") else {
                return;
            };
            // A minimised window reports a size of nothing at all, which is not
            // a size to come back to.
            if window.is_minimized().unwrap_or(false) {
                return;
            }
            let maximized = window.is_maximized().unwrap_or(false);
            let Ok(size) = window.inner_size() else {
                return;
            };
            let scale = window.scale_factor().unwrap_or(1.0);
            let logical = size.to_logical::<f64>(scale);
            let size = config::WindowSize {
                width: logical.width,
                height: logical.height,
                maximized,
            };
            // Saved as given; the guards are on the way back out, where they
            // can also take the screen it is being opened on into account.
            if size.sane(None).is_some() {
                let state = handle.state::<AppState>();
                let _ = state.update_config(|config| config.global.window = Some(size));
            }
        });
    });
}

#[cfg(target_os = "linux")]
fn snap_scrolling(window: &tauri::WebviewWindow) {
    use webkit2gtk::{SettingsExt, WebViewExt};

    let _ = window.with_webview(|webview| {
        if let Some(settings) = WebViewExt::settings(&webview.inner()) {
            settings.set_enable_smooth_scrolling(false);
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            config::use_dir(&dir);
            let state = AppState::new(dir);
            // Before any git command runs, so the very first fetch already uses
            // the right key.
            ssh::apply(&state.config());
            // Every git command the app runs goes to the window, so the log
            // reads as the session the user would have typed themselves.
            let reporting = app.handle().clone();
            git_cmd::report_to(move |command| {
                let _ = reporting.emit("git-command", command);
            });
            let saved = state.config().global.window;
            app.manage(state);
            if let Some(window) = app.get_webview_window("main") {
                restore_size(&window, saved);
            }
            remember_size(app.handle());
            // Holds the watch for whichever repository is open; empty until one
            // is.
            app.manage(watch::Slot::default());
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                snap_scrolling(&window);
            }
            // The inspector is only compiled into debug builds, and having it
            // open from the start is the only way to see a failure that happens
            // before the page can report anything itself.
            #[cfg(debug_assertions)]
            if std::env::var("GITUI_DEVTOOLS").is_ok() {
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }
            Ok(())
        })
        .invoke_handler(aimed(tauri::generate_handler![
            open_repo,
            startup_repo,
            clone_repo,
            init_repo,
            repo_info,
            ref_tree,
            working_status,
            commit_graph,
            commit_detail,
            commit_file_diff,
            working_file_diff,
            file_text,
            checkout,
            create_branch,
            checkout_review,
            commit_depth,
            delete_branch,
            trunk_branch,
            set_trunk_branch,
            delete_branch_preview,
            rename_branch,
            set_upstream,
            unset_upstream,
            stale_branches,
            worktree_list,
            worktree_add,
            worktree_remove,
            add_to_gitignore,
            run_git,
            lfs_status,
            lfs_pull,
            rebase_plan,
            rebase_start,
            rebase_progress,
            rebase_continue,
            rebase_skip,
            rebase_abort,
            rebase_reword,
            blame_file,
            file_history,
            signature_marks,
            commit_signature,
            signing_setup,
            submodule_list,
            submodule_update,
            submodule_sync,
            submodule_add,
            submodule_deinit,
            submodule_remove,
            remotes,
            remote_url,
            remote_add,
            remote_set_url,
            remote_rename,
            remote_remove,
            can_fast_forward,
            branch_relation,
            delete_remote_branch,
            push_tag,
            delete_remote_tag,
            commit_patch,
            reveal,
            stage,
            stage_all,
            unstage,
            discard,
            apply_hunk,
            commit,
            amend_draft,
            reword_check,
            reword,
            stash_push,
            stash_pop,
            stash_list,
            stash_apply,
            stash_apply_many,
            stash_drop,
            stash_rename,
            delete_untracked,
            stash_branch,
            stash_oid,
            reset_preview,
            reset,
            cherry_pick,
            revert,
            create_tag,
            delete_tag,
            commit_message_text,
            history,
            undo,
            redo,
            fetch,
            pull,
            pull_branch,
            push_preview,
            push,
            merge,
            merge_into,
            rebase_branch,
            abort_merge,
            rebase,
            abort_rebase,
            continue_rebase,
            in_progress,
            undo_restore,
            conflict_list,
            conflict_read,
            conflict_preview,
            conflict_resolve,
            conflict_resolve_whole,
            conflict_resolve_as_is,
            conflict_resolve_all,
            conflict_stage_all,
            conflict_marked,
            config_get,
            config_set_global,
            profile_save,
            profile_delete,
            profile_activate,
            project_close,
            project_reorder,
            project_forget,
            apply_identity,
            ssh_keys,
            ssh_test,
            secret_set,
            secret_status,
            forge_secret_key,
            forge_token_url,
            forge_status,
            forge_me,
            forge_check,
            forge_reviews,
            forge_review_detail,
            forge_repos,
            forge_members,
            forge_create_review,
            forge_compare_url,
            forge_faces,
            forge_review_comments,
            forge_review_files,
            forge_review_commits,
            forge_post_comment,
            forge_reply_comment,
            forge_add_diff_comment,
            forge_submit_review,
            forge_merge_review,
            forge_set_review_state,
            forge_review_status,
            forge_resolve_thread,
            forge_set_review_people,
            forge_project_labels,
            forge_set_labels,
            forge_update_review,
            forge_set_draft,
            forge_review_file_text,
            avatar,
            ai_status,
            ai_models,
            ai_commit_message,
            ai_commit_message_for,
            ai_review_message,
            ai_resolve_conflict,
            open_external,
        ]))
        .run(tauri::generate_context!())
        .expect("error while running gitnoob");
}

/// Points the open repository at whichever one the caller meant, per call.
///
/// The window has project tabs; underneath, one path is open at a time. That
/// was a race: a fetch on a large repository takes seconds, and a tab switched
/// while it ran moved the path out from under everything that had not read it
/// yet — so the second half of an operation could act on a different repository
/// from the first, with nothing on screen to say so.
///
/// Every call from the window now carries `__repo`, the repository the window
/// believes it is asking about, and it is applied here — after the message has
/// arrived and before the command runs. Since the git work itself is
/// synchronous, a command reads the path before anything else can dispatch, and
/// what it reads is what its own caller meant rather than whatever was open by
/// the time it got there.
///
/// Commands with nothing to do with a repository — settings, profiles, the
/// model list — carry it too and are unaffected by it.
fn aimed<R: tauri::Runtime>(
    handler: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        if let tauri::ipc::InvokeBody::Json(payload) = invoke.message.payload() {
            if let Some(repo) = payload.get("__repo").and_then(|value| value.as_str()) {
                if !repo.is_empty() {
                    if let Some(state) = invoke.message.state_ref().try_get::<AppState>() {
                        state.set_path(PathBuf::from(repo));
                    }
                }
            }
        }
        handler(invoke)
    }
}
