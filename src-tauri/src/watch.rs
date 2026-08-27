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
            note(&mut pending, first, &root);

            loop {
                match rx.recv_timeout(QUIET) {
                    Ok(event) => note(&mut pending, event, &root),
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

/// Folds one filesystem event into what will be reported.
fn note(pending: &mut Changed, event: notify::Result<notify::Event>, root: &Path) {
    let Ok(event) = event else { return };
    for path in &event.paths {
        match classify(path, root) {
            Some(true) => pending.git_dir = true,
            Some(false) => pending.work_tree = true,
            None => {}
        }
    }
}

/// `Some(true)` for a path inside `.git`, `Some(false)` for one in the work
/// tree, and `None` for anything not worth waking up for.
fn classify(path: &Path, root: &Path) -> Option<bool> {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let mut in_git_dir = false;

    for part in relative.components() {
        let name = part.as_os_str().to_string_lossy();
        if NOISE.contains(&name.as_ref()) {
            return None;
        }
        if name == ".git" {
            in_git_dir = true;
        }
    }

    if in_git_dir {
        // Git writes locks and temporary files constantly while it works, and
        // each one would otherwise be a refresh of its own.
        let name = relative.file_name()?.to_string_lossy().into_owned();
        if name.ends_with(".lock") || name.starts_with("tmp_") || name == "COMMIT_EDITMSG" {
            return None;
        }
    }

    Some(in_git_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/repo")
    }

    #[test]
    fn a_write_under_git_asks_for_everything() {
        assert_eq!(
            classify(&root().join(".git/refs/heads/main"), &root()),
            Some(true)
        );
        assert_eq!(classify(&root().join(".git/HEAD"), &root()), Some(true));
    }

    #[test]
    fn a_write_in_the_work_tree_asks_only_for_the_status() {
        assert_eq!(classify(&root().join("app/main.rs"), &root()), Some(false));
    }

    #[test]
    fn build_output_is_not_worth_waking_up_for() {
        assert_eq!(
            classify(&root().join("node_modules/x/index.js"), &root()),
            None
        );
        assert_eq!(
            classify(&root().join("src-tauri/target/debug/app.exe"), &root()),
            None
        );
        assert_eq!(classify(&root().join(".output/public/x.js"), &root()), None);
    }

    #[test]
    fn gits_own_scratch_files_are_ignored() {
        // Written and deleted constantly while git works; each one would
        // otherwise be a refresh.
        assert_eq!(classify(&root().join(".git/index.lock"), &root()), None);
        assert_eq!(classify(&root().join(".git/COMMIT_EDITMSG"), &root()), None);
        // But the index itself matters: staging changed.
        assert_eq!(classify(&root().join(".git/index"), &root()), Some(true));
    }

    #[test]
    fn a_burst_folds_into_one_report() {
        let mut pending = Changed::default();
        for path in [".git/refs/heads/main", "app/a.rs", "app/b.rs"] {
            let event = notify::Event::new(notify::EventKind::Any).add_path(root().join(path));
            note(&mut pending, Ok(event), &root());
        }
        assert_eq!(
            pending,
            Changed {
                git_dir: true,
                work_tree: true
            }
        );
    }
}
