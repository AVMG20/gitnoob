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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// How long to wait for a burst to finish before reporting it.
///
/// A checkout touches thousands of files and an `npm install` touches far more;
/// reporting each one would refresh the window hundreds of times. Everything
/// inside this window is coalesced into one message.
const QUIET: Duration = Duration::from_millis(350);

/// The longest a burst is held before it is reported anyway.
///
/// Something that writes every few hundred milliseconds for as long as it
/// runs (a dev server's log, a queue worker, a cache warming up) never goes
/// quiet, and waiting for quiet left the window stale for as long as it ran.
const MAX_BURST: Duration = Duration::from_secs(2);

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
        // The work-tree files written in this burst, for the code index.
        let mut touched: HashSet<PathBuf> = HashSet::new();
        let mut ignore = Ignore::new(&places.root);
        loop {
            // Block until something happens, then keep draining until the burst
            // goes quiet. Reporting mid-burst would show a half-finished
            // checkout.
            let first = match rx.recv() {
                Ok(event) => event,
                Err(_) => return,
            };
            let mut rescan = false;
            let mut fold = |event: notify::Result<notify::Event>| {
                if event.as_ref().is_ok_and(|event| event.need_rescan()) {
                    rescan = true;
                }
                note(&mut pending, &mut touched, event, &places, &mut ignore);
            };
            fold(first);
            if !drain(&rx, QUIET, MAX_BURST, &mut fold) {
                return;
            }

            if rescan {
                // The platform dropped events and says so: which files
                // changed is unknown, so everything that was read from the
                // tree is read again.
                pending.git_dir = true;
                pending.work_tree = true;
                touched.clear();
                crate::files::forget(&places.root);
                crate::frameworks::forget(&places.root);
                crate::symbols::start(places.root.clone());
            }
            if pending != Changed::default() {
                let _ = app.emit(EVENT, pending.clone());
            }
            // After the window has heard, and on a thread of its own: reading
            // the files again must not hold up the next report.
            if !touched.is_empty() {
                let root = places.root.clone();
                let paths = std::mem::take(&mut touched);
                std::thread::spawn(move || {
                    crate::frameworks::touched(&root, &paths);
                    crate::symbols::touched(&root, &paths);
                });
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

/// Takes events until none has come for `quiet`, or until `max` has passed
/// since the call, whichever is first. False once the sender is gone.
fn drain<T>(rx: &Receiver<T>, quiet: Duration, max: Duration, each: &mut impl FnMut(T)) -> bool {
    let started = Instant::now();
    loop {
        let left = max.saturating_sub(started.elapsed());
        if left.is_zero() {
            return true;
        }
        match rx.recv_timeout(quiet.min(left)) {
            Ok(event) => each(event),
            Err(RecvTimeoutError::Timeout) => return true,
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

/// What git ignores in the work tree, asked of libgit2 and remembered per
/// folder for the folders named like build output.
///
/// A missing repository answers "not ignored" for files and "ignored" for
/// folders named like build output, which is the old behaviour.
struct Ignore {
    repo: Option<git2::Repository>,
    /// The work tree as given and as the filesystem resolves it.
    roots: Vec<PathBuf>,
    folders: HashMap<PathBuf, bool>,
}

impl Ignore {
    fn new(root: &Path) -> Self {
        Ignore {
            repo: git2::Repository::open(root).ok(),
            roots: both(root.to_path_buf()),
            folders: HashMap::new(),
        }
    }

    /// No repository: nothing is ignored. For tests and unwatchable places.
    #[cfg(test)]
    fn none() -> Self {
        Ignore {
            repo: None,
            roots: Vec::new(),
            folders: HashMap::new(),
        }
    }

    fn relative<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        self.roots
            .iter()
            .find_map(|root| path.strip_prefix(root).ok())
    }

    /// Whether git ignores this work-tree file.
    fn file(&self, path: &Path) -> bool {
        let (Some(repo), Some(relative)) = (&self.repo, self.relative(path)) else {
            return false;
        };
        repo.is_path_ignored(relative).unwrap_or(false)
    }

    /// Whether git ignores this folder, relative to the work tree. With no
    /// repository to ask, a folder named like build output is taken to be it.
    fn folder(&mut self, relative: &Path) -> bool {
        if let Some(known) = self.folders.get(relative) {
            return *known;
        }
        let ignored = self
            .repo
            .as_ref()
            .map_or(true, |repo| repo.is_path_ignored(relative).unwrap_or(false));
        self.folders.insert(relative.to_path_buf(), ignored);
        ignored
    }

    /// A `.gitignore` changed: what was remembered may be wrong now.
    fn reset(&mut self) {
        self.folders.clear();
    }
}

/// Folds one filesystem event into what will be reported.
fn note(
    pending: &mut Changed,
    touched: &mut HashSet<PathBuf>,
    event: notify::Result<notify::Event>,
    places: &Places,
    ignore: &mut Ignore,
) {
    let Ok(event) = event else { return };
    // The editor's own save goes through a temp file renamed over the real
    // one: the temp file is nobody's business, and the rename only replaced
    // a file that was already there.
    let own_save = event.paths.iter().any(|path| editor_temp(path));
    for path in &event.paths {
        if editor_temp(path) {
            continue;
        }
        if path.file_name().is_some_and(|name| name == ".gitignore") {
            ignore.reset();
        }
        match classify(path, places, ignore) {
            Some(true) => pending.git_dir = true,
            Some(false) => {
                pending.work_tree = true;
                touched.insert(path.clone());
                if own_save
                    || !changes_listing(&event.kind, path)
                    || replaced_in_place(&event.kind, path, &places.root)
                {
                    continue;
                }
                // A file git ignores is not in the finder's list, so a cache
                // or log folder that churns files costs no re-walk. What
                // Composer installs is ignored too, and has a list of its own.
                if !ignore.file(path) {
                    crate::files::forget(&places.root);
                } else if ignore.relative(path).is_some_and(|relative| {
                    relative.components().next().is_some_and(|first| {
                        crate::files::DEPENDENCY_DIRS
                            .iter()
                            .any(|dir| first.as_os_str() == *dir)
                    })
                }) {
                    crate::files::forget_dependencies(&places.root);
                }
            }
            None => {}
        }
    }
}

/// A temp file `edit.rs` writes before renaming it over the file it saves.
fn editor_temp(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with('.') && name.contains(".gitnoob-") && name.ends_with(".tmp")
        })
}

/// A rename that landed on a file the listing already has: an atomic save,
/// which leaves the same files there were.
fn replaced_in_place(kind: &notify::EventKind, path: &Path, root: &Path) -> bool {
    use notify::event::{EventKind, ModifyKind};
    matches!(kind, EventKind::Modify(ModifyKind::Name(_)))
        && path.is_file()
        && crate::files::lists(root, path)
}

/// Whether a work-tree event changes which files there are, or which of them
/// git ignores: the file finder's list of paths is out of date after one.
/// Saving an existing file does neither.
fn changes_listing(kind: &notify::EventKind, path: &Path) -> bool {
    use notify::event::{EventKind, ModifyKind};
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    ) || path.file_name().is_some_and(|name| name == ".gitignore")
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
///
/// A folder named like build output (`dist`, `target`, ...) is skipped when it
/// sits at the top of the work tree, or when git ignores it. A tracked
/// `app/code/Acme/Foo/dist` deeper down is somebody's source.
fn classify(path: &Path, places: &Places, ignore: &mut Ignore) -> Option<bool> {
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
            // Events arrive with the resolved path (`/private/var` for a root
            // opened as `/var`); strip whichever form matches, so the folders
            // above the work tree are never mistaken for ones inside it.
            let relative = ignore
                .relative(path)
                .or_else(|| path.strip_prefix(&places.root).ok())?;
            let mut in_git_dir = false;
            let mut folder = PathBuf::new();
            for (depth, name) in names(relative).into_iter().enumerate() {
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
                folder.push(&name);
                if NOISE.contains(&name.as_str()) && (depth == 0 || ignore.folder(&folder)) {
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
    // A loose object is written by every `git add`, and an object changes
    // nothing on screen until a ref or the index points at it, which is a
    // write of its own.
    if parts.first().is_some_and(|first| first == "objects") {
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

    /// `classify` with nothing ignored.
    fn sort(path: &Path, places: &Places) -> Option<bool> {
        classify(path, places, &mut Ignore::none())
    }

    #[test]
    fn a_burst_that_never_goes_quiet_is_still_reported() {
        let (tx, rx) = channel();
        let writer = std::thread::spawn(move || {
            for _ in 0..200 {
                if tx.send(()).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let started = Instant::now();
        let mut seen = 0;
        assert!(drain(
            &rx,
            Duration::from_millis(100),
            Duration::from_millis(300),
            &mut |_| seen += 1
        ));
        assert!(started.elapsed() < Duration::from_millis(1500));
        assert!(seen > 0);
        drop(rx);
        writer.join().unwrap();
    }

    #[test]
    fn only_an_ignored_folder_named_like_build_output_is_skipped() {
        let dir = std::env::temp_dir().join(format!("gitnoob-watch-noise-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("packages/x/node_modules/y")).unwrap();
        std::fs::create_dir_all(dir.join("packages/x/dist")).unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        std::fs::write(dir.join(".gitignore"), "node_modules/\n").unwrap();
        let places = Places {
            root: dir.clone(),
            own: vec![dir.join(".git")],
            shared: Vec::new(),
        };
        let mut ignore = Ignore::new(&dir);
        assert_eq!(
            classify(
                &dir.join("packages/x/node_modules/y/i.js"),
                &places,
                &mut ignore
            ),
            None
        );
        // Deeper than the top, a `dist` git does not ignore is source.
        assert_eq!(
            classify(&dir.join("packages/x/dist/a.js"), &places, &mut ignore),
            Some(false)
        );
        // At the top it is build output, ignored or not.
        assert_eq!(classify(&dir.join("dist/a.js"), &places, &mut ignore), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_loose_object_is_not_news() {
        // Every stage writes one; the index write that follows is the news.
        assert_eq!(sort(&at(".git/objects/ab/cdef0123"), &plain()), None);
        assert_eq!(sort(&at(".git/objects/pack/pack-1.pack"), &plain()), None);
    }

    #[test]
    fn a_write_under_git_asks_for_everything() {
        assert_eq!(sort(&at(".git/refs/heads/main"), &plain()), Some(true));
        assert_eq!(sort(&at(".git/HEAD"), &plain()), Some(true));
    }

    #[test]
    fn a_write_in_the_work_tree_asks_only_for_the_status() {
        assert_eq!(sort(&at("app/main.rs"), &plain()), Some(false));
    }

    #[test]
    fn build_output_is_not_worth_waking_up_for() {
        assert_eq!(sort(&at("node_modules/x/index.js"), &plain()), None);
        assert_eq!(sort(&at("src-tauri/target/debug/app.exe"), &plain()), None);
        assert_eq!(sort(&at(".output/public/x.js"), &plain()), None);
    }

    #[test]
    fn a_branch_named_like_build_output_is_still_a_branch() {
        assert_eq!(sort(&at(".git/refs/heads/dist"), &plain()), Some(true));
    }

    #[test]
    fn gits_own_scratch_files_are_ignored() {
        // Written and deleted constantly while git works; each one would
        // otherwise be a refresh.
        assert_eq!(sort(&at(".git/index.lock"), &plain()), None);
        assert_eq!(sort(&at(".git/COMMIT_EDITMSG"), &plain()), None);
    }

    #[test]
    fn the_index_is_only_the_status() {
        // Staging writes it and nothing else; the graph has not moved.
        assert_eq!(sort(&at(".git/index"), &plain()), Some(false));
        // Another worktree's, or a submodule's, is not this window's.
        assert_eq!(sort(&at(".git/worktrees/other/index"), &plain()), None);
        assert_eq!(sort(&at(".git/modules/sub/index"), &plain()), None);
        // Reached through the folder rather than through the git directory,
        // which is how it arrives when nothing resolved the directory.
        let unknown = Places {
            root: root(),
            own: Vec::new(),
            shared: Vec::new(),
        };
        assert_eq!(sort(&at(".git/index"), &unknown), Some(false));
        assert_eq!(sort(&at(".git/refs/heads/x"), &unknown), Some(true));
    }

    #[test]
    fn a_linked_worktree_hears_its_own_git_directory() {
        let wt = linked();
        // Its own HEAD and index live in the main repository's `.git`.
        assert_eq!(
            sort(Path::new("/repo/.git/worktrees/wt/HEAD"), &wt),
            Some(true)
        );
        assert_eq!(
            sort(Path::new("/repo/.git/worktrees/wt/index"), &wt),
            Some(false)
        );
        // Branches are shared.
        assert_eq!(
            sort(Path::new("/repo/.git/refs/heads/feature"), &wt),
            Some(true)
        );
        // The main checkout's index and another worktree's are not its own.
        assert_eq!(sort(Path::new("/repo/.git/index"), &wt), None);
        assert_eq!(
            sort(Path::new("/repo/.git/worktrees/other/index"), &wt),
            None
        );
        // The pointer file in its own folder is not a git directory.
        assert_eq!(sort(Path::new("/wt/.git"), &wt), None);
        assert_eq!(sort(Path::new("/wt/src/a.rs"), &wt), Some(false));
    }

    #[test]
    fn a_burst_folds_into_one_report() {
        let mut pending = Changed::default();
        let mut touched = HashSet::new();
        for path in [".git/refs/heads/main", "app/a.rs", "app/b.rs"] {
            let event = notify::Event::new(notify::EventKind::Any).add_path(at(path));
            note(
                &mut pending,
                &mut touched,
                Ok(event),
                &plain(),
                &mut Ignore::none(),
            );
        }
        // Only the work-tree files go to the code index.
        assert_eq!(touched, HashSet::from([at("app/a.rs"), at("app/b.rs")]));

        // The editor's temp file is not a file anybody has.
        let mut touched = HashSet::new();
        let mut quiet = Changed::default();
        let event =
            notify::Event::new(notify::EventKind::Any).add_path(at("app/.a.rs.gitnoob-1-0.tmp"));
        note(
            &mut quiet,
            &mut touched,
            Ok(event),
            &plain(),
            &mut Ignore::none(),
        );
        assert_eq!(quiet, Changed::default());
        assert!(touched.is_empty());
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
        assert_eq!(sort(&index, &wt), Some(false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
