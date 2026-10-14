//! Watching the repository so the window does not go stale.
//!
//! Anything done outside the app — a commit in a terminal, a checkout by a
//! script, an editor saving a file — leaves what is on screen describing a
//! repository that no longer exists. Fetching happened to fix it only because a
//! fetch triggers a refresh; nothing was actually watching.
//!
//! Two kinds of change come out of this, because they cost very different
//! amounts to answer. A write under `.git` means refs or HEAD moved, so the
//! branch list and the graph have to be rebuilt. A write in the work tree means
//! only the status changed, which is cheap. Telling them apart is what keeps a
//! file save from re-walking the whole history.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// How long to wait for a burst to finish before reporting it.
///
/// A checkout touches thousands of files and an `npm install` touches far more;
/// reporting each one would refresh the window hundreds of times. Everything
/// inside this window is coalesced into one message.
const QUIET: Duration = Duration::from_millis(350);

/// Directories never worth watching. Build output and dependencies change
/// constantly, are almost always ignored by git anyway, and on this repository
/// alone `target` would drown everything else.
const NOISE: [&str; 8] = [
    "node_modules",
    "target",
    ".output",
    ".nuxt",
    "dist",
    ".next",
    ".venv",
    "__pycache__",
];

/// What changed, as coarsely as the frontend needs to know.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Changed {
    /// A write under `.git`: refs, HEAD, the index. Rebuild everything.
    pub git_dir: bool,
    /// A write in the work tree. The status is enough.
    pub work_tree: bool,
}

/// The event name the frontend listens on.
pub const EVENT: &str = "repo-changed";

/// A watcher, kept alive for as long as its repository is open.
///
/// Dropping this stops the thread: the watcher goes with it, the channel closes,
/// and the loop falls out.
pub struct Watch {
    _watcher: RecommendedWatcher,
}

/// The active watch, replaced whenever a different repository is opened.
pub type Slot = Arc<Mutex<Option<Watch>>>;

/// Starts watching `root`, emitting [`EVENT`] when something changes.
///
/// Failure is not fatal and not reported: a repository on a filesystem the
/// platform cannot watch still works, it just needs the refresh button, which
/// is how the whole app worked until now.
pub fn start(app: AppHandle, root: PathBuf) -> Option<Watch> {
    let (tx, rx) = channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        // A send failure means the receiving thread is gone, which happens on
        // shutdown; nothing to do about it here.
        let _ = tx.send(event);
    })
    .ok()?;
    watcher.watch(&root, RecursiveMode::Recursive).ok()?;
    // A linked worktree keeps its index and HEAD in `.git/worktrees/<name>`
    // of the repository it belongs to, and its branches in that repository's
    // `.git`; a submodule keeps everything in the parent's `.git/modules`.
    // Neither is under the folder that is open, so watching only that folder
    // missed every commit, stage and checkout made there.
    let places = places(root);
    for dir in places.own.iter().chain(&places.shared) {
        if !inside(dir, &places.root) {
            let _ = watcher.watch(dir, RecursiveMode::Recursive);
        }
    }

    std::thread::spawn(move || {
        let mut pending = Changed::default();
        loop {
            // Block until something happens, then keep draining until the burst
            // goes quiet. Reporting mid-burst would show a half-finished
            // checkout.
            let first = match rx.recv() {
                Ok(event) => event,
                Err(_) => return,
            };
            note(&mut pending, first, &places);

            loop {
                match rx.recv_timeout(QUIET) {
                    Ok(event) => note(&mut pending, event, &places),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }

            if pending != Changed::default() {
                let _ = app.emit(EVENT, pending.clone());
            }
            pending = Changed::default();
        }
    });

    Some(Watch { _watcher: watcher })
}

/// Where a repository lives on disk: the folder that is open, and the git
/// directories that belong to it wherever they are.
///
/// Each directory is held as given and as the filesystem resolves it, since
/// events arrive with the resolved path (`/private/var` for `/var` on a Mac).
struct Places {
    root: PathBuf,
    /// This checkout's own git directory: its HEAD and its index.
    own: Vec<PathBuf>,
    /// The directory it shares refs with, when that is somewhere else — the
    /// main repository's `.git` for a linked worktree.
    shared: Vec<PathBuf>,
}

/// Both forms of a path: as given, and resolved.
fn both(dir: PathBuf) -> Vec<PathBuf> {
    let resolved = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    if resolved == dir {
        vec![dir]
    } else {
        vec![dir, resolved]
    }
}

/// The git directory and the common directory, as libgit2 finds them.
fn places(root: PathBuf) -> Places {
    let Ok(repo) = git2::Repository::open(&root) else {
        return Places {
            root,
            own: Vec::new(),
            shared: Vec::new(),
        };
    };
    let own = both(repo.path().to_path_buf());
    let shared = both(repo.commondir().to_path_buf())
        .into_iter()
        .filter(|dir| !own.contains(dir))
        .collect();
    drop(repo);
    Places { root, own, shared }
}

fn inside(path: &Path, dir: &Path) -> bool {
    path.starts_with(dir)
        || dir
            .canonicalize()
            .ok()
            .zip(path.canonicalize().ok())
            .is_some_and(|(dir, path)| path.starts_with(dir))
}

/// Folds one filesystem event into what will be reported.
fn note(pending: &mut Changed, event: notify::Result<notify::Event>, places: &Places) {
    let Ok(event) = event else { return };
    for path in &event.paths {
        match classify(path, places) {
            Some(true) => pending.git_dir = true,
            Some(false) => pending.work_tree = true,
            None => {}
        }
    }
}

/// `Some(true)` for a write that moved refs or HEAD, `Some(false)` for one
/// that only changed what the status says, and `None` for anything not worth
/// waking up for.
///
/// The index is the status: staging writes it and nothing else, and a stage
/// done in the window is already followed by a read of the status. Calling it
/// a change to the repository rebuilt the branch list and walked the history
/// again, once per click. A commit or a checkout writes refs or HEAD as well
/// as the index, so they still rebuild everything.
fn classify(path: &Path, places: &Places) -> Option<bool> {
    // The names inside a git directory, and whether it is this checkout's.
    let found = places
        .own
        .iter()
        .find(|dir| path.starts_with(dir))
        .map(|dir| (dir, true))
        .or_else(|| {
            places
                .shared
                .iter()
                .find(|dir| path.starts_with(dir))
                .map(|dir| (dir, false))
        });
    let mut parts: Vec<String> = Vec::new();
    let mut own = true;
    let in_git_dir = match found {
        Some((dir, is_own)) => {
            own = is_own;
            parts = names(path.strip_prefix(dir).unwrap_or(path));
            true
        }
        None => {
            let relative = path.strip_prefix(&places.root).unwrap_or(path);
            let mut in_git_dir = false;
            for name in names(relative) {
                if in_git_dir {
                    parts.push(name);
                    continue;
                }
                // Only up to `.git`: past it the names are git's, and a
                // branch called `dist` or `target` is not build output.
                if name == ".git" {
                    in_git_dir = true;
                    continue;
                }
                if NOISE.contains(&name.as_str()) {
                    return None;
                }
            }
            in_git_dir
        }
    };

    if !in_git_dir {
        return Some(false);
    }

    // Git writes locks and temporary files constantly while it works, and
    // each one would otherwise be a refresh of its own.
    let name = parts.last()?;
    // The undo history this app keeps beside git's own state is written
    // after every step, and is not news about the repository.
    if name.ends_with(".lock")
        || name.starts_with("tmp_")
        || name == "COMMIT_EDITMSG"
        || name.starts_with("gitnoob-journal.json")
    {
        return None;
    }
    if name == "index" {
        // This checkout's index is its status. Another worktree's, or a
        // submodule's under `modules/`, is nothing this window shows.
        return (own && parts.len() == 1).then_some(false);
    }
    Some(true)
}

/// A relative path's components, as strings.
fn names(path: &Path) -> Vec<String> {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/repo")
    }

    /// An ordinary repository: its git directory is `.git` inside the folder.
    fn plain() -> Places {
        Places {
            root: root(),
            own: vec![root().join(".git")],
            shared: Vec::new(),
        }
    }

    /// A linked worktree at `/wt` of the repository at `/repo`.
    fn linked() -> Places {
        Places {
            root: PathBuf::from("/wt"),
            own: vec![PathBuf::from("/repo/.git/worktrees/wt")],
            shared: vec![PathBuf::from("/repo/.git")],
        }
    }

    fn at(path: &str) -> PathBuf {
        root().join(path)
    }

    #[test]
    fn a_write_under_git_asks_for_everything() {
        assert_eq!(classify(&at(".git/refs/heads/main"), &plain()), Some(true));
        assert_eq!(classify(&at(".git/HEAD"), &plain()), Some(true));
    }

    #[test]
    fn a_write_in_the_work_tree_asks_only_for_the_status() {
        assert_eq!(classify(&at("app/main.rs"), &plain()), Some(false));
    }

    #[test]
    fn build_output_is_not_worth_waking_up_for() {
        assert_eq!(classify(&at("node_modules/x/index.js"), &plain()), None);
        assert_eq!(
            classify(&at("src-tauri/target/debug/app.exe"), &plain()),
            None
        );
        assert_eq!(classify(&at(".output/public/x.js"), &plain()), None);
    }

    #[test]
    fn a_branch_named_like_build_output_is_still_a_branch() {
        assert_eq!(classify(&at(".git/refs/heads/dist"), &plain()), Some(true));
    }

    #[test]
    fn gits_own_scratch_files_are_ignored() {
        // Written and deleted constantly while git works; each one would
        // otherwise be a refresh.
        assert_eq!(classify(&at(".git/index.lock"), &plain()), None);
        assert_eq!(classify(&at(".git/COMMIT_EDITMSG"), &plain()), None);
    }

    #[test]
    fn the_index_is_only_the_status() {
        // Staging writes it and nothing else; the graph has not moved.
        assert_eq!(classify(&at(".git/index"), &plain()), Some(false));
        // Another worktree's, or a submodule's, is not this window's.
        assert_eq!(classify(&at(".git/worktrees/other/index"), &plain()), None);
        assert_eq!(classify(&at(".git/modules/sub/index"), &plain()), None);
        // Reached through the folder rather than through the git directory,
        // which is how it arrives when nothing resolved the directory.
        let unknown = Places {
            root: root(),
            own: Vec::new(),
            shared: Vec::new(),
        };
        assert_eq!(classify(&at(".git/index"), &unknown), Some(false));
        assert_eq!(classify(&at(".git/refs/heads/x"), &unknown), Some(true));
    }

    #[test]
    fn a_linked_worktree_hears_its_own_git_directory() {
        let wt = linked();
        // Its own HEAD and index live in the main repository's `.git`.
        assert_eq!(
            classify(Path::new("/repo/.git/worktrees/wt/HEAD"), &wt),
            Some(true)
        );
        assert_eq!(
            classify(Path::new("/repo/.git/worktrees/wt/index"), &wt),
            Some(false)
        );
        // Branches are shared.
        assert_eq!(
            classify(Path::new("/repo/.git/refs/heads/feature"), &wt),
            Some(true)
        );
        // The main checkout's index and another worktree's are not its own.
        assert_eq!(classify(Path::new("/repo/.git/index"), &wt), None);
        assert_eq!(
            classify(Path::new("/repo/.git/worktrees/other/index"), &wt),
            None
        );
        // The pointer file in its own folder is not a git directory.
        assert_eq!(classify(Path::new("/wt/.git"), &wt), None);
        assert_eq!(classify(Path::new("/wt/src/a.rs"), &wt), Some(false));
    }

    #[test]
    fn a_burst_folds_into_one_report() {
        let mut pending = Changed::default();
        for path in [".git/refs/heads/main", "app/a.rs", "app/b.rs"] {
            let event = notify::Event::new(notify::EventKind::Any).add_path(at(path));
            note(&mut pending, Ok(event), &plain());
        }
        assert_eq!(
            pending,
            Changed {
                git_dir: true,
                work_tree: true
            }
        );
    }

    #[test]
    fn finds_both_git_directories_of_a_linked_worktree() {
        let dir = std::env::temp_dir().join(format!("gitnoob-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main");
        let run = |cwd: &Path, args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(args)
                .current_dir(cwd)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        std::fs::create_dir_all(&main).unwrap();
        run(&main, &["init", "-q"]);
        run(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
        run(&main, &["worktree", "add", "-q", "-b", "side", "../wt"]);

        let wt = places(dir.join("wt"));
        let main_git = main.join(".git").canonicalize().unwrap();
        assert!(wt
            .own
            .iter()
            .any(|d| d.canonicalize().unwrap() == main_git.join("worktrees/wt")));
        assert!(wt
            .shared
            .iter()
            .any(|d| d.canonicalize().unwrap() == main_git));
        // Its index, written by a stage made there, reads as a status change.
        let index = main_git.join("worktrees/wt/index");
        assert_eq!(classify(&index, &wt), Some(false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
