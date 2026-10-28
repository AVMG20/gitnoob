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
pub mod guard;
pub mod home;
pub mod identity;
pub mod journal;
pub mod lfs;
pub mod rebase;
pub mod recover;
pub mod refs;
pub mod remote;
pub mod review;
pub mod rewrite;
pub mod sign;
pub mod ssh;
pub mod state;
pub mod submodule;
pub mod watch;
pub mod work;
pub mod worktree;

use std::path::PathBuf;

use state::AppState;
use tauri::{Emitter, Manager, State};

/// The state a command works with: the app's own, with the path pinned to the
/// repository the call was addressed to, as it stood when the call arrived.
///
/// The path is read from the call's own `__repo`, not from the shared path
/// [`aimed`] sets. Tauri builds an async command's arguments inside the task
/// it hands to the runtime, so this runs whenever that task is first polled —
/// with every worker busy on a slow git run, that is after the window's next
/// calls have arrived, and the shared path by then may be another tab's. From
/// here on nothing another tab does moves it: every `state.path()` inside the
/// command, however late, is the repository its caller meant. The settings and
/// the undo histories are shared with the app's state, so nothing else about a
/// command changes.
pub struct Aimed(AppState);

/// The state a call addressed with `payload` works with: pinned to its
/// `__repo` when it names one, and to the shared path otherwise.
fn aim(state: &AppState, payload: &tauri::ipc::InvokeBody) -> AppState {
    let named = match payload {
        tauri::ipc::InvokeBody::Json(payload) => payload
            .get("__repo")
            .and_then(|value| value.as_str())
            .filter(|repo| !repo.is_empty())
            .map(PathBuf::from),
        _ => None,
    };
    match named {
        Some(path) => state.pinned_to(path),
        None => state.pinned(),
    }
}

impl std::ops::Deref for Aimed {
    type Target = AppState;
    fn deref(&self) -> &AppState {
        &self.0
    }
}

impl<'de, R: tauri::Runtime> tauri::ipc::CommandArg<'de, R> for Aimed {
    fn from_command(
        command: tauri::ipc::CommandItem<'de, R>,
    ) -> Result<Self, tauri::ipc::InvokeError> {
        let state = command
            .message
            .webview_ref()
            .try_state::<AppState>()
            .ok_or_else(|| tauri::ipc::InvokeError::from("The app state is not set up"))?;
        Ok(Aimed(aim(&state, command.message.payload())))
    }
}

// --- repository -------------------------------------------------------------

/// Opens a repository and, unless told otherwise, records it in the active
/// profile's tab strip.
///
/// `record: false` is for stepping into a submodule: it is a repository of its
/// own and everything below has to point at it, but it is not a project the
/// user opened and it should not turn into a tab of its own or into a recent.
///
/// `replace` names the tab this one takes the place of, for a worktree opened
/// in the tab it was clicked from rather than as a tab of its own.
#[tauri::command]
async fn open_repo(
    path: String,
    record: Option<bool>,
    replace: Option<String>,
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
                config::place_tab(profile, &recorded, &name, replace.as_deref());
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

/// Everything the home tab draws: the projects, and a year of your commits.
///
/// One command rather than one per repository: it is a handful of `git log`
/// runs either way, and doing them together keeps the page from drawing itself
/// forty times as they land.
#[tauri::command]
async fn home_summary(state: Aimed) -> Result<home::HomeSummary, String> {
    home::summary(&state)
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
async fn init_repo(name: String, parent: String, state: Aimed) -> Result<create::NewRepo, String> {
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
fn repo_info(state: Aimed) -> Result<refs::RepoInfo, String> {
    refs::describe(&state)
}

#[tauri::command]
async fn ref_tree(state: Aimed) -> Result<refs::RefTree, String> {
    refs::tree(&state)
}

#[tauri::command]
async fn working_status(state: Aimed) -> Result<refs::WorkingStatus, String> {
    refs::status_counted(&state)
}

// --- history ----------------------------------------------------------------

#[tauri::command]
async fn commit_graph(
    limit: Option<usize>,
    solo: Option<Vec<String>>,
    state: Aimed,
) -> Result<graph::GraphPage, String> {
    let fallback = state.config().global.graph_page_size;
    graph::soloed(&state, limit.unwrap_or(fallback), &solo.unwrap_or_default())
}

#[tauri::command]
async fn commit_detail(oid: String, state: Aimed) -> Result<diff::CommitDetail, String> {
    diff::commit_detail(&state, &oid)
}

#[tauri::command]
async fn commit_file_diff(
    oid: String,
    path: String,
    state: Aimed,
) -> Result<diff::FileDiff, String> {
    diff::commit_file_diff(&state, &oid, &path)
}

#[tauri::command]
async fn working_file_diff(
    path: String,
    side: diff::Side,
    state: Aimed,
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
    state: Aimed,
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
async fn checkout(name: String, state: Aimed) -> Result<refs::CheckoutOutcome, String> {
    refs::checkout(&state, &name)
}

#[tauri::command]
async fn create_branch(
    name: String,
    start: Option<String>,
    checkout: Option<bool>,
    state: Aimed,
) -> Result<String, String> {
    refs::create_branch(&state, &name, start.as_deref(), checkout.unwrap_or(true))
}

/// How far back a commit sits in the graph's walk, so a page big enough to
/// hold its row can be asked for.
#[tauri::command]
async fn commit_depth(
    oid: String,
    solo: Option<Vec<String>>,
    state: Aimed,
) -> Result<Option<usize>, String> {
    graph::depth(&state, &oid, &solo.unwrap_or_default())
}

/// Checks out the branch a review was opened from, adding the fork it lives
/// in as a remote when that is what standing on those commits takes.
#[tauri::command]
async fn checkout_review(review: review::ReviewTarget, state: Aimed) -> Result<String, String> {
    review::checkout(&state, review)
}

/// The branch this repository is organised around, which is what "has this
/// work landed?" is really asking about.
#[tauri::command]
fn trunk_branch(state: Aimed) -> refs::Trunk {
    refs::trunk(&state)
}

/// Names that branch, or forgets the name when given nothing.
#[tauri::command]
async fn set_trunk_branch(name: Option<String>, state: Aimed) -> Result<String, String> {
    refs::set_trunk(&state, name.as_deref())
}

/// What deleting a branch would cost, read before the question is asked.
#[tauri::command]
async fn delete_branch_preview(name: String, state: Aimed) -> Result<refs::BranchDeletion, String> {
    refs::deletion_preview(&state, &name)
}

#[tauri::command]
async fn delete_branch(name: String, force: Option<bool>, state: Aimed) -> Result<String, String> {
    refs::delete_branch(&state, &name, force.unwrap_or(false))
}

#[tauri::command]
async fn rename_branch(from: String, to: String, state: Aimed) -> Result<String, String> {
    refs::rename_branch(&state, &from, &to)
}

#[tauri::command]
fn set_upstream(branch: String, upstream: String, state: Aimed) -> Result<String, String> {
    refs::set_upstream(&state, &branch, &upstream)
}

#[tauri::command]
fn unset_upstream(branch: String, state: Aimed) -> Result<String, String> {
    refs::unset_upstream(&state, &branch)
}

/// Local branches whose upstream is gone, i.e. safe to tidy away.
#[tauri::command]
fn stale_branches(state: Aimed) -> Result<Vec<String>, String> {
    refs::stale_branches(&state)
}

// --- worktrees ---------------------------------------------------------------

/// Every folder this repository is checked out into.
#[tauri::command]
async fn worktree_list(state: Aimed) -> Result<Vec<worktree::Worktree>, String> {
    worktree::list(&state)
}

/// Checks a branch out into a new folder, creating it from a remote-tracking
/// ref when `track` names one.
#[tauri::command]
async fn worktree_add(
    path: String,
    branch: String,
    track: Option<String>,
    state: Aimed,
) -> Result<String, String> {
    worktree::add(&state, &path, &branch, track.as_deref())
}

/// Removes a worktree; `force` throws its uncommitted work away too.
#[tauri::command]
async fn worktree_remove(
    path: String,
    force: Option<bool>,
    state: Aimed,
) -> Result<String, String> {
    worktree::remove(&state, &path, force.unwrap_or(false))
}

#[tauri::command]
fn add_to_gitignore(pattern: String, state: Aimed) -> Result<String, String> {
    refs::add_to_gitignore(&state, &pattern)
}

// --- git lfs -----------------------------------------------------------------

/// Whether this repository uses LFS, and whether the tool for it is here.
#[tauri::command]
fn lfs_status(state: Aimed) -> Result<lfs::Status, String> {
    lfs::status(&state)
}

/// Fetches the real contents of one LFS file, or of every one of them.
#[tauri::command]
async fn lfs_pull(path: Option<String>, state: Aimed) -> Result<String, String> {
    lfs::pull(&state, path.as_deref())
}

// --- interactive rebase -------------------------------------------------------

/// The commits between a chosen one and HEAD, oldest first, for the plan.
#[tauri::command]
async fn rebase_plan(onto: String, state: Aimed) -> Result<Vec<rebase::Candidate>, String> {
    rebase::plan(&state, &onto)
}

/// Runs the plan the window built.
#[tauri::command]
async fn rebase_start(
    onto: String,
    steps: Vec<rebase::Step>,
    state: Aimed,
) -> Result<String, String> {
    rebase::start(&state, &onto, steps)
}

/// Where a rebase has got to, or nothing when none is running.
#[tauri::command]
async fn rebase_progress(state: Aimed) -> Result<Option<rebase::Progress>, String> {
    rebase::progress(&state)
}

#[tauri::command]
async fn rebase_continue(state: Aimed) -> Result<String, String> {
    rebase::resume(&state)
}

#[tauri::command]
async fn rebase_skip(state: Aimed) -> Result<String, String> {
    rebase::skip(&state)
}

#[tauri::command]
async fn rebase_abort(state: Aimed) -> Result<String, String> {
    rebase::abort(&state)
}

/// What a squash of the chosen commits would fold, and what would stop it.
#[tauri::command]
async fn squash_preview(oids: Vec<String>, state: Aimed) -> Result<rebase::SquashPreview, String> {
    rebase::squash_preview(&state, &oids)
}

/// Folds a run of commits into one carrying the message the window collected.
#[tauri::command]
async fn squash(oids: Vec<String>, message: String, state: Aimed) -> Result<String, String> {
    rebase::squash(&state, &oids, &message)
}

/// Gives the commit a stopped rebase is sitting on a new message, and goes on.
#[tauri::command]
async fn rebase_reword(message: String, state: Aimed) -> Result<String, String> {
    rebase::reword(&state, &message)
}

// --- blame and file history --------------------------------------------------

/// Who last touched each line of a file, as runs of consecutive lines.
#[tauri::command]
async fn blame_file(
    path: String,
    commit: Option<String>,
    state: Aimed,
) -> Result<Vec<blame::BlameRun>, String> {
    blame::of(&state, &path, commit.as_deref())
}

/// Every commit that touched a file, newest first, across renames.
#[tauri::command]
async fn file_history(
    path: String,
    limit: Option<usize>,
    state: Aimed,
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
    state: Aimed,
) -> Result<std::collections::HashMap<String, sign::Mark>, String> {
    if !state.config().global.verify_signatures {
        return Ok(std::collections::HashMap::new());
    }
    sign::marks(&state, limit)
}

/// Everything git will say about one commit's signature.
#[tauri::command]
async fn commit_signature(oid: String, state: Aimed) -> Result<sign::Signature, String> {
    sign::of(&state, &oid)
}

/// Whether a commit made in this repository right now would be signed.
#[tauri::command]
fn signing_setup(state: Aimed) -> Result<sign::Setup, String> {
    sign::setup(&state)
}

// --- submodules --------------------------------------------------------------

/// Every repository kept inside this one.
#[tauri::command]
async fn submodule_list(state: Aimed) -> Result<Vec<submodule::Submodule>, String> {
    submodule::list(&state)
}

/// Clones what is missing and moves each one onto the commit this repository
/// records. Without a path, all of them.
#[tauri::command]
async fn submodule_update(
    path: Option<String>,
    recursive: Option<bool>,
    state: Aimed,
) -> Result<String, String> {
    submodule::update(&state, path.as_deref(), recursive.unwrap_or(false))
}

/// Copies the URLs in `.gitmodules` over the ones each was cloned with.
#[tauri::command]
async fn submodule_sync(path: Option<String>, state: Aimed) -> Result<String, String> {
    submodule::sync(&state, path.as_deref())
}

/// Adds a repository as a submodule, cloning it into `path`.
#[tauri::command]
async fn submodule_add(url: String, path: String, state: Aimed) -> Result<String, String> {
    submodule::add(&state, &url, &path)
}

/// Empties a submodule's folder while leaving it declared.
#[tauri::command]
async fn submodule_deinit(
    path: String,
    force: Option<bool>,
    state: Aimed,
) -> Result<String, String> {
    submodule::deinit(&state, &path, force.unwrap_or(false))
}

/// Takes a submodule out of the working tree, the index and `.gitmodules`.
#[tauri::command]
async fn submodule_remove(path: String, state: Aimed) -> Result<String, String> {
    submodule::remove(&state, &path)
}

/// A git command typed into the log's prompt, run in the open repository.
#[tauri::command]
async fn run_git(args: Vec<String>, state: Aimed) -> Result<git_cmd::CmdOutput, String> {
    let cwd = state.path()?;
    tauri::async_runtime::spawn_blocking(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        git_cmd::run_typed(&cwd, &args)
    })
    .await
    .map_err(|e| format!("git did not finish: {e}"))?
}

/// The remote a new branch should go to: the one the checked out branch
/// tracks, else `origin`, else the first configured. A fork added to look at a
/// review is never it.
#[tauri::command]
fn primary_remote(state: Aimed) -> Option<String> {
    remote::primary(&state)
}

#[tauri::command]
fn remotes(state: Aimed) -> Result<Vec<String>, String> {
    remote::remotes(&state)
}

// --- managing the remotes themselves ----------------------------------------

/// The address a remote fetches from, shown when it is about to be edited.
#[tauri::command]
fn remote_url(remote: String, state: Aimed) -> Result<String, String> {
    remote::remote_url(&state, &remote)
}

#[tauri::command]
async fn remote_add(name: String, url: String, state: Aimed) -> Result<String, String> {
    remote::remote_add(&state, &name, &url)
}

#[tauri::command]
async fn remote_set_url(name: String, url: String, state: Aimed) -> Result<String, String> {
    remote::remote_set_url(&state, &name, &url)
}

#[tauri::command]
async fn remote_rename(from: String, to: String, state: Aimed) -> Result<String, String> {
    remote::remote_rename(&state, &from, &to)
}

#[tauri::command]
async fn remote_remove(name: String, state: Aimed) -> Result<String, String> {
    remote::remote_remove(&state, &name)
}

#[tauri::command]
fn can_fast_forward(branch: String, onto: String, state: Aimed) -> Result<bool, String> {
    remote::can_fast_forward(&state, &branch, &onto)
}

/// How two branches stand to each other, so a menu can offer only the moves
/// that would actually do something.
#[tauri::command]
fn branch_relation(
    source: String,
    target: String,
    state: Aimed,
) -> Result<remote::BranchRelation, String> {
    remote::relation(&state, &source, &target)
}

#[tauri::command]
async fn delete_remote_branch(
    remote_name: String,
    branch: String,
    state: Aimed,
) -> Result<git_cmd::CmdOutput, String> {
    remote::delete_remote_branch(&state, &remote_name, &branch)
}

#[tauri::command]
async fn push_tag(
    remote_name: String,
    tag: String,
    state: Aimed,
) -> Result<git_cmd::CmdOutput, String> {
    remote::push_tag(&state, &remote_name, &tag)
}

#[tauri::command]
async fn delete_remote_tag(
    remote_name: String,
    tag: String,
    state: Aimed,
) -> Result<git_cmd::CmdOutput, String> {
    remote::delete_remote_tag(&state, &remote_name, &tag)
}

#[tauri::command]
async fn commit_patch(oid: String, state: Aimed) -> Result<String, String> {
    work::commit_patch(&state, &oid)
}

#[tauri::command]
fn reveal(path: String, state: Aimed) -> Result<(), String> {
    work::reveal(&state, &path)
}

// --- working tree -----------------------------------------------------------

#[tauri::command]
async fn stage(paths: Vec<String>, state: Aimed) -> Result<String, String> {
    work::stage(&state, &paths)
}

#[tauri::command]
async fn stage_all(expect_staged: Option<bool>, state: Aimed) -> Result<String, String> {
    work::stage_all_expecting(&state, expect_staged)
}

#[tauri::command]
async fn unstage(paths: Vec<String>, state: Aimed) -> Result<String, String> {
    work::unstage(&state, &paths)
}

#[tauri::command]
async fn discard(paths: Vec<String>, state: Aimed) -> Result<String, String> {
    work::discard(&state, &paths)
}

/// Stage, unstage or discard a single hunk of a file.
#[tauri::command]
async fn apply_hunk(
    path: String,
    hunk_index: usize,
    action: work::HunkAction,
    lines: Option<work::Lines>,
    state: Aimed,
) -> Result<String, String> {
    work::apply_hunk(&state, &path, hunk_index, action, lines)
}

#[tauri::command]
async fn commit(message: String, amend: Option<bool>, state: Aimed) -> Result<String, String> {
    work::commit(&state, &message, amend.unwrap_or(false))
}

/// Commits, and when a hook stops it, says so with what the hook changed.
#[tauri::command]
async fn commit_attempt(
    message: String,
    amend: Option<bool>,
    no_verify: Option<bool>,
    state: Aimed,
) -> Result<work::CommitAttempt, String> {
    work::commit_attempt(
        &state,
        &message,
        amend.unwrap_or(false),
        no_verify.unwrap_or(false),
    )
}

/// What in the staged diff is worth a second look before it is committed.
#[tauri::command]
async fn commit_guard(debug: Option<bool>, state: Aimed) -> Result<Vec<guard::Finding>, String> {
    guard::scan(&state, debug.unwrap_or(false))
}

#[tauri::command]
fn amend_draft(state: Aimed) -> Result<work::AmendDraft, String> {
    work::amend_draft(&state)
}

#[tauri::command]
fn reword_check(oid: String, state: Aimed) -> Result<work::RewordCheck, String> {
    work::reword_check(&state, &oid)
}

#[tauri::command]
async fn reword(oid: String, message: String, state: Aimed) -> Result<String, String> {
    work::reword(&state, &oid, &message)
}

#[tauri::command]
async fn stash_push(
    message: Option<String>,
    include_untracked: Option<bool>,
    state: Aimed,
) -> Result<String, String> {
    work::stash_push(
        &state,
        message.as_deref(),
        include_untracked.unwrap_or(true),
    )
}

#[tauri::command]
async fn stash_pop(index: usize, state: Aimed) -> Result<String, String> {
    work::stash_pop(&state, index)
}

#[tauri::command]
async fn stash_list(state: Aimed) -> Result<Vec<work::StashEntry>, String> {
    work::stash_list(&state)
}

#[tauri::command]
async fn stash_apply(index: usize, state: Aimed) -> Result<String, String> {
    work::stash_apply(&state, index)
}

/// Applies several stashes in one go, oldest first, dropping each that goes on
/// cleanly when `drop_after` says to.
#[tauri::command]
async fn stash_apply_many(
    indexes: Vec<usize>,
    drop_after: Option<bool>,
    state: Aimed,
) -> Result<work::StashRun, String> {
    work::stash_apply_many(&state, indexes, drop_after.unwrap_or(false))
}

#[tauri::command]
async fn stash_drop(index: usize, state: Aimed) -> Result<String, String> {
    work::stash_drop(&state, index)
}

/// Gives a stash a new description, leaving it where it is in the list.
#[tauri::command]
async fn stash_rename(index: usize, message: String, state: Aimed) -> Result<String, String> {
    work::stash_rename(&state, index, &message)
}

/// Deletes files git is not tracking.
#[tauri::command]
async fn delete_untracked(paths: Vec<String>, state: Aimed) -> Result<String, String> {
    work::delete_untracked(&state, &paths)
}

#[tauri::command]
async fn stash_branch(index: usize, name: String, state: Aimed) -> Result<String, String> {
    work::stash_branch(&state, index, &name)
}

/// The commit a stash points at, so its diff can be shown like any commit's.
#[tauri::command]
fn stash_oid(index: usize, state: Aimed) -> Result<String, String> {
    work::stash_oid(&state, index)
}

// --- moving a branch and replaying commits ----------------------------------

#[tauri::command]
async fn reset_preview(oid: String, state: Aimed) -> Result<work::ResetPreview, String> {
    work::reset_preview(&state, &oid)
}

#[tauri::command]
async fn reset(oid: String, mode: work::ResetMode, state: Aimed) -> Result<String, String> {
    work::reset(&state, &oid, mode)
}

#[tauri::command]
async fn cherry_pick(
    oids: Vec<String>,
    options: Option<work::CherryPickOptions>,
    state: Aimed,
) -> Result<String, String> {
    work::cherry_pick(&state, &oids, options.unwrap_or_default())
}

#[tauri::command]
async fn revert(oid: String, state: Aimed) -> Result<String, String> {
    work::revert(&state, &oid)
}

#[tauri::command]
fn create_tag(
    name: String,
    oid: String,
    message: Option<String>,
    state: Aimed,
) -> Result<String, String> {
    work::create_tag(&state, &name, &oid, message.as_deref())
}

#[tauri::command]
fn delete_tag(name: String, state: Aimed) -> Result<String, String> {
    work::delete_tag(&state, &name)
}

#[tauri::command]
fn commit_message_text(oid: String, state: Aimed) -> Result<String, String> {
    work::commit_message_text(&state, &oid)
}

// --- undo and redo ----------------------------------------------------------

#[tauri::command]
fn history(state: Aimed) -> journal::Stacks {
    journal::stacks(&state)
}

#[tauri::command]
async fn undo(state: Aimed) -> Result<String, String> {
    journal::undo(&state)
}

#[tauri::command]
async fn redo(state: Aimed) -> Result<String, String> {
    journal::redo(&state)
}

/// Undoes one step by id: a notice's Undo, which goes through the history so
/// the two cannot disagree about what is left to undo.
#[tauri::command]
async fn undo_step(id: usize, state: Aimed) -> Result<String, String> {
    journal::undo_step(&state, id)
}

// --- remotes ----------------------------------------------------------------

#[tauri::command]
async fn fetch(remote: Option<String>, state: Aimed) -> Result<git_cmd::CmdOutput, String> {
    remote::fetch(&state, remote.as_deref())
}

#[tauri::command]
async fn pull(rebase: Option<bool>, state: Aimed) -> Result<remote::MergeOutcome, String> {
    remote::pull(&state, rebase)
}

/// Brings any branch up to date, checked out or not.
#[tauri::command]
async fn pull_branch(
    branch: String,
    rebase: Option<bool>,
    state: Aimed,
) -> Result<remote::MergeOutcome, String> {
    remote::pull_branch(&state, &branch, rebase)
}

#[tauri::command]
async fn push_preview(
    branch: Option<String>,
    fetch_first: Option<bool>,
    state: Aimed,
) -> Result<remote::PushPreview, String> {
    remote::push_preview(&state, branch.as_deref(), fetch_first.unwrap_or(false))
}

#[tauri::command]
async fn push(
    remote_name: String,
    branch: String,
    force: Option<bool>,
    set_upstream: Option<bool>,
    lease: Option<String>,
    state: Aimed,
) -> Result<git_cmd::CmdOutput, String> {
    remote::push(
        &state,
        &remote_name,
        &branch,
        force.unwrap_or(false),
        set_upstream.unwrap_or(false),
        lease.as_deref(),
    )
}

#[tauri::command]
async fn merge(
    branch: String,
    no_ff: Option<bool>,
    state: Aimed,
) -> Result<remote::MergeOutcome, String> {
    remote::merge(&state, &branch, no_ff.unwrap_or(false))
}

#[tauri::command]
async fn abort_merge(state: Aimed) -> Result<String, String> {
    remote::abort_merge(&state)
}

#[tauri::command]
async fn rebase(onto: String, state: Aimed) -> Result<remote::MergeOutcome, String> {
    remote::rebase(&state, &onto)
}

/// Merges one branch into another, whichever one happens to be checked out.
#[tauri::command]
async fn merge_into(
    source: String,
    target: String,
    no_ff: Option<bool>,
    state: Aimed,
) -> Result<remote::MergeOutcome, String> {
    remote::merge_into(&state, &source, &target, no_ff.unwrap_or(false))
}

/// Rebases a branch onto another without the user standing on it first.
#[tauri::command]
async fn rebase_branch(
    branch: String,
    onto: String,
    state: Aimed,
) -> Result<remote::MergeOutcome, String> {
    remote::rebase_branch(&state, &branch, &onto)
}

#[tauri::command]
async fn abort_rebase(state: Aimed) -> Result<String, String> {
    remote::abort_rebase(&state)
}

#[tauri::command]
async fn continue_rebase(state: Aimed) -> Result<remote::MergeOutcome, String> {
    remote::continue_rebase(&state)
}

/// Reports a merge, rebase, cherry-pick or revert that git is part-way through.
#[tauri::command]
fn in_progress(state: Aimed) -> Result<remote::InProgress, String> {
    remote::in_progress(&state)
}

/// Puts the tree back the way it was before an auto-stash refused to go back
/// on, returning to the branch the work was taken from.
#[tauri::command]
async fn undo_restore(state: Aimed) -> Result<String, String> {
    work::undo_restore(&state)
}

// --- conflicts --------------------------------------------------------------

#[tauri::command]
fn conflict_list(state: Aimed) -> Result<Vec<String>, String> {
    conflict::list(&state)
}

#[tauri::command]
async fn conflict_read(path: String, state: Aimed) -> Result<conflict::ConflictFile, String> {
    conflict::read(&state, &path)
}

#[tauri::command]
async fn conflict_preview(
    path: String,
    choices: Vec<conflict::Resolution>,
    state: Aimed,
) -> Result<String, String> {
    conflict::preview(&state, &path, &choices)
}

#[tauri::command]
async fn conflict_resolve(
    path: String,
    choices: Vec<conflict::Resolution>,
    state: Aimed,
) -> Result<String, String> {
    conflict::resolve(&state, &path, &choices)
}

/// Takes one side in every conflicted file at once.
#[tauri::command]
async fn conflict_resolve_all(side: String, state: Aimed) -> Result<String, String> {
    conflict::resolve_all(&state, &side)
}

/// Ends the named conflicts by taking the files back to what the branch has.
#[tauri::command]
async fn conflict_discard(paths: Vec<String>, state: Aimed) -> Result<String, String> {
    conflict::discard(&state, &paths)
}

/// Stages every conflicted file as it stands, if none of them has markers left.
#[tauri::command]
async fn conflict_stage_all(state: Aimed) -> Result<String, String> {
    conflict::stage_all(&state)
}

/// Which conflicted files still have git's markers in them.
#[tauri::command]
async fn conflict_marked(state: Aimed) -> Result<Vec<String>, String> {
    conflict::marked(&state)
}

/// Ends a conflict by staging the file exactly as it stands on disk.
#[tauri::command]
async fn conflict_resolve_as_is(path: String, state: Aimed) -> Result<String, String> {
    conflict::resolve_as_is(&state, &path)
}

#[tauri::command]
async fn conflict_resolve_whole(
    path: String,
    side: String,
    state: Aimed,
) -> Result<String, String> {
    conflict::resolve_whole(&state, &path, &side)
}

// --- configuration, profiles and projects -----------------------------------

#[tauri::command]
fn config_get(state: Aimed) -> config::Config {
    state.config()
}

#[tauri::command]
fn config_set_global(global: config::Global, state: Aimed) -> Result<config::Config, String> {
    state.update_config(|config| config.global = global)?;
    Ok(state.config())
}

/// Creates the profile when its id is unknown, updates it otherwise.
#[tauri::command]
fn profile_save(profile: config::Profile, state: Aimed) -> Result<config::Config, String> {
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
fn profile_delete(id: String, state: Aimed) -> Result<config::Config, String> {
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
fn project_reorder(paths: Vec<String>, state: Aimed) -> Result<config::Config, String> {
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
fn project_forget(path: String, state: Aimed) -> Result<config::Config, String> {
    state.update_config(|config| {
        if let Some(profile) = config.active_mut() {
            profile.recents.retain(|one| one.path != path);
        }
    })?;
    Ok(state.config())
}

/// Writes the profile's identity into this repository's local config.
///
/// A profile is a person, and choosing one is the whole statement, so this runs
/// by itself whenever a repository is opened. `Ok(None)` means there was
/// nothing to do and nothing is said. It no longer replaces an address the
/// repository already commits with, or writes one into a project on another
/// forge, unless `force` says the user agreed: see `identity`.
#[tauri::command]
fn apply_identity(force: Option<bool>, state: Aimed) -> Result<Option<String>, String> {
    identity::apply(&state, force.unwrap_or(false))
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
    state: Aimed,
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
fn forge_secret_key(state: Aimed) -> Option<String> {
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
async fn forge_status(state: Aimed) -> Result<forge::ForgeStatus, String> {
    Ok(forge::status(&state).await)
}

/// The account the active profile's token belongs to, with their picture.
#[tauri::command]
async fn forge_me(state: Aimed) -> Result<forge::ForgeUser, String> {
    forge::me(&state).await
}

#[tauri::command]
async fn forge_check(state: Aimed) -> Result<String, String> {
    forge::check(&state).await
}

#[tauri::command]
async fn forge_reviews(state: Aimed) -> Result<Vec<forge::Review>, String> {
    forge::reviews(&state).await
}

/// Open reviews the forge finds for the sidebar filter, for the ones beyond
/// the first fifty the list holds.
#[tauri::command]
async fn forge_search_reviews(query: String, state: Aimed) -> Result<Vec<forge::Review>, String> {
    forge::search_reviews(&state, &query).await
}

/// Everything one review says about itself, asked for when it is opened rather
/// than on every refresh of the list.
#[tauri::command]
async fn forge_review_detail(number: i64, state: Aimed) -> Result<forge::ReviewDetail, String> {
    forge::review_detail(&state, number).await
}

/// The repositories the active profile's token can see, so cloning is picking
/// one from a list rather than pasting an address.
#[tauri::command]
async fn forge_repos(state: Aimed) -> Result<Vec<forge::ForgeRepo>, String> {
    forge::repos(&state).await
}

/// Everyone the project's review can be assigned to or reviewed by.
#[tauri::command]
async fn forge_members(state: Aimed) -> Result<Vec<forge::Member>, String> {
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
    state: Aimed,
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
async fn forge_compare_url(
    source: String,
    target: String,
    title: String,
    body: String,
    state: Aimed,
) -> Result<String, String> {
    forge::compare_url(&state, &source, &target, &title, &body).await
}

/// The picture for each profile that has an account, keyed by profile id.
#[tauri::command]
async fn forge_faces(state: Aimed) -> Result<std::collections::HashMap<String, String>, String> {
    Ok(forge::faces(&state).await)
}

// --- review conversation -----------------------------------------------------

/// Everything said under one review: conversation comments and diff threads.
#[tauri::command]
async fn forge_review_comments(
    number: i64,
    state: Aimed,
) -> Result<Vec<forge::ReviewComment>, String> {
    forge::review_comments(&state, number).await
}

/// Every file a review changes across all of its commits, patches included.
#[tauri::command]
async fn forge_review_files(
    number: i64,
    state: Aimed,
) -> Result<Vec<forge::ReviewFileChange>, String> {
    forge::review_files(&state, number).await
}

/// The commits a review's source branch puts ahead of its target.
#[tauri::command]
async fn forge_review_commits(
    number: i64,
    state: Aimed,
) -> Result<Vec<forge::ReviewCommit>, String> {
    forge::review_commits(&state, number).await
}

/// Leaves one comment on the conversation itself.
#[tauri::command]
async fn forge_post_comment(number: i64, body: String, state: Aimed) -> Result<(), String> {
    forge::post_comment(&state, number, &body).await
}

/// Answers one comment already on the record.
#[tauri::command]
async fn forge_reply_comment(
    number: i64,
    parent_id: i64,
    body: String,
    state: Aimed,
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
    state: Aimed,
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
    state: Aimed,
) -> Result<(), String> {
    forge::submit_review(&state, number, &event, &body, comments.unwrap_or_default()).await
}

/// Merges the review as it stands.
#[tauri::command]
async fn forge_merge_review(
    number: i64,
    squash: Option<bool>,
    method: Option<String>,
    delete_branch: Option<bool>,
    sha: Option<String>,
    state: Aimed,
) -> Result<String, String> {
    // `squash` is the older way of asking, kept so a caller that only knows
    // it still merges the way it meant.
    let method = method.unwrap_or_else(|| {
        if squash.unwrap_or(false) {
            "squash".to_string()
        } else {
            "merge".to_string()
        }
    });
    forge::merge_review(&state, number, &method, delete_branch.unwrap_or(false), sha).await
}

/// Closes or reopens a review.
#[tauri::command]
async fn forge_set_review_state(number: i64, action: String, state: Aimed) -> Result<(), String> {
    forge::set_review_state(&state, number, &action).await
}

/// Whether the review can land: its checks, its verdicts, its conflicts.
#[tauri::command]
async fn forge_review_status(number: i64, state: Aimed) -> Result<forge::ReviewStatus, String> {
    forge::review_status(&state, number).await
}

/// Marks one diff thread settled, or unsettles it again.
#[tauri::command]
async fn forge_resolve_thread(
    number: i64,
    thread: String,
    resolved: bool,
    state: Aimed,
) -> Result<(), String> {
    forge::resolve_thread(&state, number, &thread, resolved).await
}

/// Takes back one remark under a review.
#[tauri::command]
async fn forge_delete_comment(
    number: i64,
    id: i64,
    kind: String,
    state: Aimed,
) -> Result<(), String> {
    forge::delete_comment(&state, number, id, &kind).await
}

/// Sets who owns the review and who is being asked to look at it.
#[tauri::command]
async fn forge_set_review_people(
    number: i64,
    assignees: Vec<forge::Member>,
    reviewers: Vec<forge::Member>,
    state: Aimed,
) -> Result<(), String> {
    forge::set_review_people(&state, number, assignees, reviewers).await
}

/// Every label this project has.
#[tauri::command]
async fn forge_project_labels(state: Aimed) -> Result<Vec<forge::Label>, String> {
    forge::project_labels(&state).await
}

/// Sets the review's labels to exactly these.
#[tauri::command]
async fn forge_set_labels(number: i64, labels: Vec<String>, state: Aimed) -> Result<(), String> {
    forge::set_labels(&state, number, labels).await
}

/// Rewrites the review's title and description.
#[tauri::command]
async fn forge_update_review(
    number: i64,
    title: String,
    body: String,
    state: Aimed,
) -> Result<(), String> {
    forge::update_review(&state, number, &title, &body).await
}

/// Marks the review ready to be read, or puts it back to a draft.
#[tauri::command]
async fn forge_set_draft(number: i64, draft: bool, state: Aimed) -> Result<(), String> {
    forge::set_draft(&state, number, draft).await
}

/// One file as it stands at the review's head, for the whole-file view.
#[tauri::command]
async fn forge_review_file_text(number: i64, path: String, state: Aimed) -> Result<String, String> {
    forge::review_file_text(&state, number, &path).await
}

/// The open repository's forge project: default branch, merge methods, parent.
#[tauri::command]
async fn forge_repo_info(state: Aimed) -> Result<forge::RepoFacts, String> {
    forge::repo_info(&state).await
}

/// Whether the forge protects a branch, of the project reviews live in or of
/// the project `remote` points at. Asked once a session per branch.
#[tauri::command]
async fn forge_is_protected(
    branch: String,
    remote: Option<String>,
    state: Aimed,
) -> Result<bool, String> {
    forge::is_protected(&state, &branch, remote.as_deref()).await
}

/// Asks somebody who has reviewed already to look again.
#[tauri::command]
async fn forge_rerequest_review(number: i64, login: String, state: Aimed) -> Result<(), String> {
    forge::rerequest_review(&state, number, &login).await
}

/// What every account with a token says is waiting on you.
#[tauri::command]
async fn forge_inbox(state: Aimed) -> Result<forge::Inbox, String> {
    Ok(forge::inbox(&state).await)
}

/// Tidies up after a review merged from here: fetch, step off the merged
/// branch onto the trunk and fast-forward it. Offers the branch for deletion
/// rather than deleting it.
#[tauri::command]
async fn review_after_merge(
    branch: String,
    head: Option<String>,
    remote: Option<String>,
    from: Option<review::ReviewFrom>,
    state: Aimed,
) -> Result<review::AfterMerge, String> {
    let switch = state.config().global.post_merge_switch;
    review::after_merge(
        &state,
        &branch,
        head.as_deref(),
        remote.as_deref(),
        switch,
        from.as_ref(),
    )
}

/// Deletes the branch a merge left behind, if it has not moved since.
#[tauri::command]
async fn review_delete_merged(name: String, sha: String, state: Aimed) -> Result<String, String> {
    review::delete_merged(&state, &name, &sha)
}

/// Puts a branch deleted after a merge back where it was.
#[tauri::command]
async fn review_restore_branch(name: String, sha: String, state: Aimed) -> Result<String, String> {
    review::restore_branch(&state, &name, &sha)
}

/// The commits a new review from `source` into `target` would carry.
#[tauri::command]
async fn review_branch_commits(
    source: String,
    target: String,
    state: Aimed,
) -> Result<Vec<review::DraftCommit>, String> {
    review::branch_commits(&state, &source, &target)
}

/// Whether the open repository's remote fits the profile it answers to.
#[tauri::command]
fn profile_match(state: Aimed) -> forge::ProfileMatch {
    forge::profile_match(&state)
}

/// Makes a repository answer to a profile other than the active one, or, with
/// no id, to the active one again.
#[tauri::command]
fn repo_profile_set(
    path: String,
    id: Option<String>,
    state: Aimed,
) -> Result<config::Config, String> {
    state.update_config(|config| match id {
        Some(id) if config.profiles.iter().any(|p| p.id == id) => {
            config.repo_profiles.insert(path.clone(), id);
        }
        _ => {
            config.repo_profiles.remove(&path);
        }
    })?;
    // The key git pushes with follows the profile.
    ssh::apply(&state.config());
    Ok(state.config())
}

/// What the open repository commits as, and what its profile would make it,
/// so the window can ask before an address is replaced.
#[tauri::command]
fn identity_check(state: Aimed) -> Result<identity::IdentityCheck, String> {
    identity::check(&state)
}

// --- authors ----------------------------------------------------------------

/// The picture for one commit author, or `None` when there is none to find.
#[tauri::command]
async fn avatar(email: String, state: Aimed) -> Result<Option<String>, String> {
    avatar::find(&state, &email).await
}

// --- AI ---------------------------------------------------------------------

#[tauri::command]
fn ai_status(state: Aimed) -> ai::AiStatus {
    ai::status(&state)
}

#[tauri::command]
async fn ai_models(refresh: Option<bool>, state: Aimed) -> Result<Vec<ai::Model>, String> {
    ai::models(&state, refresh.unwrap_or(false)).await
}

#[tauri::command]
async fn ai_commit_message(
    amend: Option<bool>,
    draft: Option<String>,
    state: Aimed,
) -> Result<ai::CommitMessage, String> {
    ai::commit_message(&state, amend.unwrap_or(false), draft).await
}

/// A commit message for a commit that already exists, from its own diff.
#[tauri::command]
async fn ai_commit_message_for(oid: String, state: Aimed) -> Result<ai::CommitMessage, String> {
    ai::commit_message_for(&state, &oid).await
}

/// One message for the commits a squash would fold, from their folded diff.
#[tauri::command]
async fn ai_squash_message(oids: Vec<String>, state: Aimed) -> Result<ai::CommitMessage, String> {
    ai::squash_message(&state, &oids).await
}

/// The title and description of a review, written from the branch's commits.
#[tauri::command]
async fn ai_review_message(
    source: String,
    target: String,
    state: Aimed,
) -> Result<ai::CommitMessage, String> {
    ai::review_message(&state, source, target).await
}

#[tauri::command]
async fn ai_resolve_conflict(
    path: String,
    index: usize,
    state: Aimed,
) -> Result<Vec<String>, String> {
    ai::resolve_conflict(&state, path, index).await
}

// --- runs of picks, reopened conflicts, rewrites from one commit -------------

/// Carries on with a cherry-pick or revert of several commits that stopped.
#[tauri::command]
async fn sequence_continue(state: Aimed) -> Result<work::SequenceOutcome, String> {
    work::sequence_continue(&state)
}

/// Leaves out the commit a run of picks stopped on and applies the rest.
#[tauri::command]
async fn sequence_skip(state: Aimed) -> Result<work::SequenceOutcome, String> {
    work::sequence_skip(&state)
}

/// Abandons a run of picks and puts the branch back where it started.
#[tauri::command]
async fn sequence_abort(state: Aimed) -> Result<work::SequenceOutcome, String> {
    work::sequence_abort(&state)
}

/// The files resolved so far in what is under way, which can be reopened.
#[tauri::command]
async fn conflict_resolved(state: Aimed) -> Result<Vec<String>, String> {
    conflict::resolved(&state)
}

/// Puts the conflict markers back into a file that was resolved.
#[tauri::command]
async fn conflict_reopen(path: String, state: Aimed) -> Result<String, String> {
    conflict::reopen(&state, &path)
}

/// Whether this repository has git remember conflict resolutions.
#[tauri::command]
fn rerere_enabled(state: Aimed) -> bool {
    rebase::rerere_on(&state)
}

#[tauri::command]
fn set_rerere(enabled: bool, state: Aimed) -> Result<bool, String> {
    rebase::set_rerere(&state, enabled)
}

/// What fixing up, splitting or rewording one commit would involve.
#[tauri::command]
async fn rewrite_check(oid: String, state: Aimed) -> Result<rewrite::RewriteCheck, String> {
    rewrite::check(&state, &oid)
}

/// Turns what is staged into a fixup for an older commit, and folds it in.
#[tauri::command]
async fn fixup_commit(
    oid: String,
    fold: bool,
    allow_pushed: bool,
    state: Aimed,
) -> Result<String, String> {
    rewrite::fixup(&state, &oid, fold, allow_pushed)
}

/// Gives any commit on the branch a new message.
#[tauri::command]
async fn reword_commit(
    oid: String,
    message: String,
    allow_pushed: bool,
    state: Aimed,
) -> Result<String, String> {
    rewrite::reword_commit(&state, &oid, &message, allow_pushed)
}

/// Stops a rebase at a commit with its changes unstaged, to commit in pieces.
#[tauri::command]
async fn split_start(oid: String, allow_pushed: bool, state: Aimed) -> Result<String, String> {
    rewrite::split_start(&state, &oid, allow_pushed)
}

/// Carries on from a split once its pieces are committed.
#[tauri::command]
async fn split_continue(
    rest: rewrite::Rest,
    state: Aimed,
) -> Result<rewrite::SplitOutcome, String> {
    rewrite::split_continue(&state, rest)
}

/// Where each staged hunk would go if it were absorbed.
#[tauri::command]
async fn absorb_plan(state: Aimed) -> Result<rewrite::AbsorbPlan, String> {
    rewrite::absorb_plan(&state)
}

/// Absorbs the staged hunks into the commits the plan named.
#[tauri::command]
async fn absorb(fingerprint: String, fold: bool, state: Aimed) -> Result<String, String> {
    rewrite::absorb(&state, &fingerprint, fold)
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

// --- recovery ---------------------------------------------------------------

/// Takes a step that can no longer be taken out of the undo history.
#[tauri::command]
fn history_forget(id: usize, state: Aimed) -> Result<String, String> {
    journal::forget(&state, id)
}

/// Where HEAD and the current branch have been, newest first.
#[tauri::command]
async fn reflog_moments(
    limit: Option<usize>,
    state: Aimed,
) -> Result<Vec<recover::Moment>, String> {
    recover::moments(&state, limit.unwrap_or(200))
}

/// Stash commits that fell off the list and are still in the repository.
/// Runs `git fsck`, so only when asked.
#[tauri::command]
async fn lost_stashes(state: Aimed) -> Result<Vec<recover::LostStash>, String> {
    recover::lost_stashes(&state)
}

/// Puts a stash commit back on the stash list.
#[tauri::command]
async fn restore_stash(oid: String, state: Aimed) -> Result<String, String> {
    recover::restore_stash(&state, &oid)
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
// --- keeping in sync ----------------------------------------------------------

/// Fetches the repository's own remote only, for the fetch on a timer.
#[tauri::command]
async fn fetch_primary(state: Aimed) -> Result<git_cmd::CmdOutput, String> {
    remote::fetch_primary(&state)
}

/// Fast-forwards the local trunk to its upstream when that is safe and the
/// setting allows it.
#[tauri::command]
async fn keep_trunk_current(state: Aimed) -> Result<Option<remote::TrunkMove>, String> {
    remote::keep_trunk_current(&state)
}

/// What new work starts from: the trunk as its remote has it.
#[tauri::command]
fn trunk_target(state: Aimed) -> Option<String> {
    refs::trunk_target(&state)
}

/// Fetches, then branches from the trunk (or `from`) without tracking it.
#[tauri::command]
async fn start_work(name: String, from: Option<String>, state: Aimed) -> Result<String, String> {
    refs::start_work(&state, &name, from.as_deref())
}

/// Fetches, then rebases a branch onto the trunk or merges the trunk into it.
#[tauri::command]
async fn update_from_trunk(
    branch: String,
    how: Option<String>,
    state: Aimed,
) -> Result<remote::MergeOutcome, String> {
    remote::update_from_trunk(&state, &branch, how.as_deref())
}

/// The local branches that look merged, squashed in, or left behind by a
/// deleted upstream.
#[tauri::command]
async fn tidy_candidates(state: Aimed) -> Result<Vec<refs::Tidy>, String> {
    refs::tidy_candidates(&state)
}

/// Deletes local branches, each only while it points where the list said,
/// handing back the undo step that puts them back.
#[tauri::command]
async fn delete_branches(
    branches: Vec<refs::Doomed>,
    state: Aimed,
) -> Result<refs::Removal, String> {
    refs::delete_branches(&state, &branches)
}

/// The remotes gitnoob added to check out reviews from forks.
#[tauri::command]
fn review_remotes(state: Aimed) -> Vec<String> {
    remote::review_remotes(&state)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
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
            home_summary,
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
            squash_preview,
            squash,
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
            primary_remote,
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
            undo_step,
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
            conflict_discard,
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
            forge_search_reviews,
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
            forge_delete_comment,
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
            ai_squash_message,
            ai_review_message,
            ai_resolve_conflict,
            open_external,
            history_forget,
            reflog_moments,
            lost_stashes,
            restore_stash,
            fetch_primary,
            keep_trunk_current,
            trunk_target,
            start_work,
            update_from_trunk,
            tidy_candidates,
            delete_branches,
            review_remotes,
            commit_attempt,
            commit_guard,
            sequence_continue,
            sequence_skip,
            sequence_abort,
            conflict_resolved,
            conflict_reopen,
            rerere_enabled,
            set_rerere,
            rewrite_check,
            fixup_commit,
            reword_commit,
            split_start,
            split_continue,
            absorb_plan,
            absorb,
            forge_repo_info,
            forge_is_protected,
            forge_rerequest_review,
            forge_inbox,
            review_after_merge,
            review_delete_merged,
            review_restore_branch,
            review_branch_commits,
            profile_match,
            repo_profile_set,
            identity_check,
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
/// arrived and before the command runs. The command's own [`Aimed`] state
/// reads the same `__repo` from the message and keeps that path to the end,
/// since an async command may start after other calls have moved this. Reading
/// the shared path later was not enough: an async command waits for the
/// runtime, and a slow pull or commit reads the path again after git returns,
/// by which time another tab's call may have moved it.
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
                    if let Some(state) = invoke.message.webview_ref().try_state::<AppState>() {
                        state.set_path(PathBuf::from(repo));
                    }
                }
            }
        }
        handler(invoke)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call keeps the repository it named even when the shared path has
    /// moved on to another tab's before the command gets to run.
    #[test]
    fn a_call_is_aimed_at_the_repository_it_names_not_the_shared_path() {
        let dir = std::env::temp_dir().join(format!("gitnoob-aim-{}", std::process::id()));
        let state = AppState::new(dir.join("config"));
        state.set_path(PathBuf::from("/other-tab"));
        let named = tauri::ipc::InvokeBody::Json(serde_json::json!({ "__repo": "/mine" }));
        assert_eq!(aim(&state, &named).path().unwrap(), PathBuf::from("/mine"));

        let unnamed = tauri::ipc::InvokeBody::Json(serde_json::json!({ "__repo": "" }));
        assert_eq!(
            aim(&state, &unnamed).path().unwrap(),
            PathBuf::from("/other-tab")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
