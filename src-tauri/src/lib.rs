pub mod ai;
pub mod conflict;
pub mod config;
pub mod diff;
pub mod forge;
pub mod git_cmd;
pub mod graph;
pub mod journal;
pub mod refs;
pub mod remote;
pub mod ssh;
pub mod state;
pub mod work;

use std::path::PathBuf;

use state::AppState;
use tauri::{Manager, State};

// --- repository -------------------------------------------------------------

/// Opens a repository and records it in the active profile's tab strip.
#[tauri::command]
async fn open_repo(path: String, state: State<'_, AppState>) -> Result<refs::RepoInfo, String> {
    let root = state::discover_workdir(&PathBuf::from(&path))?;
    let recorded = root.to_string_lossy().into_owned();
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| recorded.clone());

    state.set_path(root);
    state.update_config(|config| {
        if let Some(profile) = config.active_mut() {
            if !profile.projects.iter().any(|p| p.path == recorded) {
                profile.projects.push(config::Project {
                    path: recorded.clone(),
                    name,
                });
            }
            profile.active_project = Some(recorded.clone());
        }
    })?;

    refs::describe(&state)
}

/// The repository named on the command line, if any: `gitui /path/to/repo`.
#[tauri::command]
fn startup_repo() -> Option<String> {
    std::env::args()
        .skip(1)
        // Skip anything that looks like a flag; only a bare path names a repo.
        .find(|arg| !arg.starts_with('-'))
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
async fn commit_graph(limit: Option<usize>, state: State<'_, AppState>) -> Result<graph::GraphPage, String> {
    let fallback = state.config().global.graph_page_size;
    graph::build(&state, limit.unwrap_or(fallback))
}

#[tauri::command]
async fn commit_detail(oid: String, state: State<'_, AppState>) -> Result<diff::CommitDetail, String> {
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

// --- branches ---------------------------------------------------------------

#[tauri::command]
async fn checkout(name: String, state: State<'_, AppState>) -> Result<String, String> {
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

#[tauri::command]
async fn delete_branch(
    name: String,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    refs::delete_branch(&state, &name, force.unwrap_or(false))
}

#[tauri::command]
async fn rename_branch(from: String, to: String, state: State<'_, AppState>) -> Result<String, String> {
    refs::rename_branch(&state, &from, &to)
}

#[tauri::command]
fn set_upstream(branch: String, upstream: String, state: State<'_, AppState>) -> Result<String, String> {
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

#[tauri::command]
fn add_to_gitignore(pattern: String, state: State<'_, AppState>) -> Result<String, String> {
    refs::add_to_gitignore(&state, &pattern)
}

#[tauri::command]
fn remotes(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    remote::remotes(&state)
}

#[tauri::command]
fn can_fast_forward(
    branch: String,
    onto: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    remote::can_fast_forward(&state, &branch, &onto)
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
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::apply_hunk(&state, &path, hunk_index, action)
}

#[tauri::command]
async fn commit(message: String, amend: Option<bool>, state: State<'_, AppState>) -> Result<String, String> {
    work::commit(&state, &message, amend.unwrap_or(false))
}

#[tauri::command]
fn amend_draft(state: State<'_, AppState>) -> Result<work::AmendDraft, String> {
    work::amend_draft(&state)
}

#[tauri::command]
async fn stash_push(
    message: Option<String>,
    include_untracked: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    work::stash_push(&state, message.as_deref(), include_untracked.unwrap_or(true))
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

#[tauri::command]
async fn stash_drop(index: usize, state: State<'_, AppState>) -> Result<String, String> {
    work::stash_drop(&state, index)
}

#[tauri::command]
async fn stash_branch(index: usize, name: String, state: State<'_, AppState>) -> Result<String, String> {
    work::stash_branch(&state, index, &name)
}

/// The commit a stash points at, so its diff can be shown like any commit's.
#[tauri::command]
fn stash_oid(index: usize, state: State<'_, AppState>) -> Result<String, String> {
    work::stash_oid(&state, index)
}

// --- moving a branch and replaying commits ----------------------------------

#[tauri::command]
async fn reset_preview(oid: String, state: State<'_, AppState>) -> Result<work::ResetPreview, String> {
    work::reset_preview(&state, &oid)
}

#[tauri::command]
async fn reset(oid: String, mode: work::ResetMode, state: State<'_, AppState>) -> Result<String, String> {
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
async fn fetch(remote: Option<String>, state: State<'_, AppState>) -> Result<git_cmd::CmdOutput, String> {
    remote::fetch(&state, remote.as_deref())
}

#[tauri::command]
async fn pull(rebase: Option<bool>, state: State<'_, AppState>) -> Result<git_cmd::CmdOutput, String> {
    remote::pull(&state, rebase.unwrap_or(false))
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

// --- conflicts --------------------------------------------------------------

#[tauri::command]
fn conflict_list(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    conflict::list(&state)
}

#[tauri::command]
async fn conflict_read(path: String, state: State<'_, AppState>) -> Result<conflict::ConflictFile, String> {
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
fn config_set_global(global: config::Global, state: State<'_, AppState>) -> Result<config::Config, String> {
    state.update_config(|config| config.global = global)?;
    Ok(state.config())
}

/// Creates the profile when its id is unknown, updates it otherwise.
#[tauri::command]
fn profile_save(profile: config::Profile, state: State<'_, AppState>) -> Result<config::Config, String> {
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
fn project_reorder(paths: Vec<String>, state: State<'_, AppState>) -> Result<config::Config, String> {
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

/// Writes the active profile's identity into this repository's local config.
///
/// Deliberately explicit: opening a repository never rewrites its config on its
/// own.
#[tauri::command]
fn apply_identity(state: State<'_, AppState>) -> Result<String, String> {
    let root = state.path()?;
    let config = state.config();
    let profile = config
        .active()
        .ok_or_else(|| "No profile is active".to_string())?;

    let name = profile
        .git_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("The {} profile has no name set", profile.name))?;
    let email = profile
        .git_email
        .clone()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("The {} profile has no email set", profile.name))?;

    git_cmd::run_checked(&root, &["config", "--local", "user.name", &name])?;
    git_cmd::run_checked(&root, &["config", "--local", "user.email", &email])?;
    Ok(format!("This repository will now commit as {name} <{email}>"))
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

/// The page where a token for this forge is created.
#[tauri::command]
fn forge_signin_url(kind: config::ForgeKind, host: String) -> Option<String> {
    forge::signin_url(kind, &host)
}

#[tauri::command]
fn forge_status(state: State<'_, AppState>) -> forge::ForgeStatus {
    forge::status(&state)
}

#[tauri::command]
async fn forge_check(state: State<'_, AppState>) -> Result<String, String> {
    forge::check(&state).await
}

#[tauri::command]
async fn forge_reviews(state: State<'_, AppState>) -> Result<Vec<forge::Review>, String> {
    forge::reviews(&state).await
}

#[tauri::command]
async fn forge_create_review(
    title: String,
    body: String,
    target: String,
    draft: Option<bool>,
    state: State<'_, AppState>,
) -> Result<forge::Review, String> {
    forge::create_review(&state, title, body, target, draft.unwrap_or(false)).await
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let state = AppState::new(dir);
            // Before any git command runs, so the very first fetch already uses
            // the right key.
            ssh::apply(&state.config());
            app.manage(state);
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
        .invoke_handler(tauri::generate_handler![
            open_repo,
            startup_repo,
            repo_info,
            ref_tree,
            working_status,
            commit_graph,
            commit_detail,
            commit_file_diff,
            working_file_diff,
            checkout,
            create_branch,
            delete_branch,
            rename_branch,
            set_upstream,
            unset_upstream,
            stale_branches,
            add_to_gitignore,
            remotes,
            can_fast_forward,
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
            stash_push,
            stash_pop,
            stash_list,
            stash_apply,
            stash_drop,
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
            push_preview,
            push,
            merge,
            abort_merge,
            rebase,
            abort_rebase,
            continue_rebase,
            in_progress,
            conflict_list,
            conflict_read,
            conflict_preview,
            conflict_resolve,
            conflict_resolve_whole,
            config_get,
            config_set_global,
            profile_save,
            profile_delete,
            profile_activate,
            project_close,
            project_reorder,
            apply_identity,
            ssh_keys,
            ssh_test,
            secret_set,
            secret_status,
            forge_secret_key,
            forge_status,
            forge_signin_url,
            forge_check,
            forge_reviews,
            forge_create_review,
            ai_status,
            ai_models,
            ai_commit_message,
            ai_resolve_conflict,
            open_external,
        ])
        .run(tauri::generate_context!())
        .expect("error while running gitui");
}
