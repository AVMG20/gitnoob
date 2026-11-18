//! The files in the work tree, for the Files tree and the file finder.
//!
//! Everything here is a read of the filesystem, never a git operation. Two
//! rules hold for every path the window hands in:
//!
//! - It is relative to the work tree, and checked here rather than trusted. It
//!   is joined to the root, resolved through any symlinks, and refused when it
//!   lands outside the work tree or inside a `.git`. [`resolve`] is the one
//!   place that check lives, so a command added later cannot skip it.
//! - What git ignores is decided by the `ignore` crate, the same matcher
//!   ripgrep uses: `.gitignore` at every level, `.git/info/exclude` and the
//!   global excludes file. Ignored entries are listed only when the folder
//!   they are in is opened, and flagged, so `vendor/` and `node_modules/` cost
//!   nothing until somebody looks inside them.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::Serialize;

use crate::state::AppState;

/// One entry in a folder.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    /// Relative to the work tree, with `/` between the parts on every platform.
    pub path: String,
    pub dir: bool,
    /// Git ignores it, or the folder it is in.
    pub ignored: bool,
    /// A symbolic link. A link to a folder outside the work tree is listed but
    /// cannot be opened.
    pub link: bool,
}

/// One file, or folder, the finder matched.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Found {
    pub path: String,
    pub score: u32,
    /// Which characters of the path matched, for drawing them bold.
    pub indices: Vec<u32>,
    /// A folder rather than a file. Only when folders were asked for.
    pub dir: bool,
    /// Git ignores it (`app/etc/env.php`). Only when ignored files were asked
    /// for; never set on a dependency, which the window tells by its path.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ignored: bool,
}

/// Turns a path from the window into a real one inside the work tree.
///
/// `must_exist` false allows a file that is not there yet (a save of a new
/// file): its folder has to exist and be inside, and the name itself cannot be
/// `..` or `.git`.
pub fn resolve(root: &Path, relative: &str, must_exist: bool) -> Result<PathBuf, String> {
    let given = Path::new(relative);
    if given.is_absolute()
        || given
            .components()
            .any(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
    {
        return Err(format!("{relative} is not inside the work tree"));
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("Could not read the work tree: {e}"))?;
    let joined = root.join(given);
    let real = match joined.canonicalize() {
        Ok(real) => real,
        Err(e) if !must_exist && e.kind() == std::io::ErrorKind::NotFound => {
            let name = joined
                .file_name()
                .filter(|name| *name != ".." && *name != "." && !name.eq_ignore_ascii_case(".git"))
                .ok_or_else(|| format!("{relative} is not a file name"))?
                .to_owned();
            let parent = joined
                .parent()
                .ok_or_else(|| format!("{relative} is not inside the work tree"))?
                .canonicalize()
                .map_err(|e| format!("Could not read the folder of {relative}: {e}"))?;
            parent.join(name)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("{relative} is not there any more"))
        }
        Err(e) => return Err(format!("Could not read {relative}: {e}")),
    };
    let inner = real
        .strip_prefix(&root)
        .map_err(|_| format!("{relative} is not inside the work tree"))?;
    // Any case: on the case-insensitive filesystems macOS and Windows use by
    // default, `.GIT/config` is the repository's own config file.
    if inner
        .components()
        .any(|part| part.as_os_str().eq_ignore_ascii_case(".git"))
    {
        return Err(format!("{relative} is inside .git"));
    }
    Ok(real)
}

/// A relative path in the spelling the window uses: `/` on every platform.
pub fn slashed(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// The walker every listing here goes through, so they all agree about what
/// git ignores. Hidden files are files like any other; `.git` is never one.
fn walker(dir: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(dir);
    builder
        .hidden(false)
        .parents(true)
        .ignore(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git");
    builder
}

/// What is directly inside one folder of the work tree, folders first.
///
/// `relative` is the folder, `""` for the root.
pub fn list_dir(state: &AppState, relative: &str) -> Result<Vec<Entry>, String> {
    let root = state.path()?;
    list_dir_at(&root, relative)
}

pub fn list_dir_at(root: &Path, relative: &str) -> Result<Vec<Entry>, String> {
    let relative = relative.trim_matches('/');
    let dir = resolve(root, relative, true)?;
    if !dir.is_dir() {
        return Err(format!("{relative} is not a folder"));
    }
    let real_root = root.canonicalize().map_err(|e| e.to_string())?;
    // The path as the window should see it: through the folder it asked for,
    // not wherever a link resolved to.
    let base = relative.to_string();

    // A folder git ignores has nothing in it git does not ignore. The walker
    // matches each entry on its own, and `vendor/` says nothing about
    // `vendor/x/y.php`, so the folder itself is asked about first.
    let inherited = !relative.is_empty() && ignored_by_git(&real_root, relative);

    let kept: HashSet<std::ffi::OsString> = if inherited {
        HashSet::new()
    } else {
        walker(&dir)
            .max_depth(Some(1))
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.depth() == 1)
            .map(|entry| entry.file_name().to_owned())
            .collect()
    };

    // What git tracks is not ignored, whatever a later rule says about its
    // name: `git status` reports changes to it like any other file.
    let index = git2::Repository::open(&real_root)
        .ok()
        .and_then(|repo| repo.index().ok());

    let mut entries = Vec::new();
    for item in std::fs::read_dir(&dir).map_err(|e| format!("Could not read {relative}: {e}"))? {
        let Ok(item) = item else { continue };
        let name = item.file_name();
        if name == ".git" {
            continue;
        }
        let Ok(kind) = item.file_type() else { continue };
        let link = kind.is_symlink();
        let is_dir = if link {
            std::fs::metadata(item.path())
                .map(|meta| meta.is_dir())
                .unwrap_or(false)
        } else {
            kind.is_dir()
        };
        let text = name.to_string_lossy().into_owned();
        let path = if base.is_empty() {
            text.clone()
        } else {
            format!("{base}/{text}")
        };
        let ignored = (inherited || !kept.contains(&name))
            && !index
                .as_ref()
                .is_some_and(|index| tracked(index, &path, is_dir));
        entries.push(Entry {
            path,
            name: text,
            dir: is_dir,
            ignored,
            link,
        });
    }
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

/// Whether git ignores one path in the work tree, for the bar over an open
/// file. A path outside the work tree is not ignored, it is refused.
pub fn path_ignored(state: &AppState, relative: &str) -> Result<bool, String> {
    path_ignored_at(&state.path()?, relative)
}

pub fn path_ignored_at(root: &Path, relative: &str) -> Result<bool, String> {
    resolve(root, relative, false)?;
    let relative = relative.trim_matches('/');
    let is_tracked = git2::Repository::open(root)
        .ok()
        .and_then(|repo| repo.index().ok())
        .is_some_and(|index| tracked(&index, relative, false));
    Ok(!is_tracked && ignored_by_git(root, relative))
}

/// Whether the index holds a path: the file itself, or for a folder anything
/// below it. A tracked file is never an ignored one, even when a rule added
/// later matches its name.
fn tracked(index: &git2::Index, relative: &str, dir: bool) -> bool {
    if dir {
        index.find_prefix(format!("{relative}/")).is_ok()
    } else {
        // Stage 0 is the ordinary entry; a conflicted file has only 1 to 3.
        (0..=3).any(|stage| index.get_path(Path::new(relative), stage).is_some())
    }
}

/// Opens a work-tree file in whatever the desktop opens that kind of file
/// with. Only a file that is there and inside the work tree.
pub fn open_default(state: &AppState, relative: &str) -> Result<(), String> {
    let real = openable(&state.path()?, relative)?;
    let (program, args): (&str, Vec<std::ffi::OsString>) = if cfg!(target_os = "macos") {
        ("open", vec![real.into_os_string()])
    } else if cfg!(target_os = "windows") {
        // Explorer does not take the `\\?\` form `canonicalize` gives on
        // Windows, and opened a different window instead of the file.
        let plain = real.to_str().map(without_verbatim);
        let arg = plain.map_or_else(|| real.into_os_string(), std::ffi::OsString::from);
        ("explorer", vec![arg])
    } else {
        ("xdg-open", vec![real.into_os_string()])
    };
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open {relative}: {e}"))
}

/// A Windows path without the verbatim prefix: `\\?\C:\x` is `C:\x`, and
/// `\\?\UNC\server\share` is `\\server\share`. Anything else as it is.
pub fn without_verbatim(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match path.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
        _ => path.to_string(),
    }
}

/// The real path of a file `open_default` may hand to the desktop.
///
/// Not a program or a script: the desktop's "open" runs those (`open` on a
/// `.command` file or on anything executable starts Terminal and runs it,
/// Explorer runs a `.bat`), and a cloned repository can hold one with any
/// name. Asking to see a file should not run it.
pub fn openable(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let real = resolve(root, relative, true)?;
    if !real.is_file() {
        return Err(format!("{relative} is not a file"));
    }
    if runs_when_opened(&real) {
        return Err(format!(
            "{relative} is a program or a script, and opening it would run it. \
             Reveal it instead if that is what you want."
        ));
    }
    Ok(real)
}

/// Kinds of file the desktop runs rather than shows when asked to open them.
const RUNNABLE: [&str; 22] = [
    "app",
    "applescript",
    "bat",
    "cmd",
    "com",
    "command",
    "cpl",
    "desktop",
    "exe",
    "hta",
    "jar",
    "lnk",
    "msi",
    "pif",
    "ps1",
    "reg",
    "scpt",
    "scr",
    "sh",
    "tool",
    "vbs",
    "wsf",
];

/// Run by Windows Script Host when opened there; shown everywhere else.
const RUNNABLE_ON_WINDOWS: [&str; 2] = ["js", "jse"];

fn runs_when_opened(path: &Path) -> bool {
    let by_name = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| {
            RUNNABLE.contains(&ext.as_str())
                || (cfg!(windows) && RUNNABLE_ON_WINDOWS.contains(&ext.as_str()))
        });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let executable = std::fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
        by_name || executable
    }
    #[cfg(not(unix))]
    {
        by_name
    }
}

/// Whether git ignores a path, the folders it is in included.
///
/// Asked of libgit2, which checks every leading folder as git does. A
/// repository it cannot open ignores nothing.
fn ignored_by_git(root: &Path, relative: &str) -> bool {
    git2::Repository::open(root)
        .ok()
        .and_then(|repo| repo.is_path_ignored(Path::new(relative)).ok())
        .unwrap_or(false)
}

// --- every file, for the finder ------------------------------------------------

/// How long a list of paths is trusted when the watcher has said nothing.
///
/// The watcher drops the list as soon as a file appears or goes; this is for
/// the filesystems it cannot watch.
const STALE: Duration = Duration::from_secs(60);

struct Listed {
    at: Instant,
    paths: Arc<Vec<String>>,
}

fn cache() -> &'static Mutex<HashMap<PathBuf, Listed>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Listed>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Drops the list of paths for a work tree, so the next search walks it
/// again. The watcher calls this when a file is created, removed or renamed.
///
/// The ignored files and the dependencies are only marked stale: a project
/// file coming or going seldom changes them, and walking them again while
/// someone types would make every keystroke wait. The next ask hands out the
/// old list and walks again on a thread of its own.
pub fn forget(root: &Path) {
    let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if let Ok(mut cached) = cache().lock() {
        cached.remove(&key);
    }
    age_kept(Which::Dependencies, &key);
    age_kept(Which::Ignored, &key);
}

/// Drops the list of ignored files, for a `.gitignore` that changed or a
/// rescan: what it holds may be wrong now, not only old.
pub fn forget_ignore_rules(root: &Path) {
    let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    drop_kept(Which::Ignored, &key);
}

/// Drops only the list of dependency files, for a change under `vendor/`.
pub fn forget_dependencies(root: &Path) {
    let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    drop_kept(Which::Dependencies, &key);
}

/// Whether the cached list for a work tree has this (absolute) path. False
/// when there is no list yet.
pub fn lists(root: &Path, path: &Path) -> bool {
    let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let Ok(inner) = real.strip_prefix(&key) else {
        return false;
    };
    let relative = slashed(inner);
    cache()
        .lock()
        .ok()
        .and_then(|cached| {
            cached
                .get(&key)
                .map(|found| found.paths.binary_search(&relative).is_ok())
        })
        .unwrap_or(false)
}

/// Every file git does not ignore, relative and sorted. Walked in parallel,
/// and kept until the watcher says the list changed.
pub fn all_files(root: &Path) -> Result<Arc<Vec<String>>, String> {
    let key = root
        .canonicalize()
        .map_err(|e| format!("Could not read the work tree: {e}"))?;
    if let Ok(cached) = cache().lock() {
        if let Some(found) = cached.get(&key) {
            if found.at.elapsed() < STALE {
                return Ok(found.paths.clone());
            }
        }
    }

    let found = Arc::new(Mutex::new(Vec::new()));
    walker(&key).build_parallel().run(|| {
        let found = found.clone();
        let root = key.clone();
        Box::new(move |entry| {
            if let Ok(entry) = entry {
                // A link to a folder is a folder here, as it is in the tree:
                // there is no file to open or to search behind its name.
                // Only regular files: a named pipe or a socket is nothing to
                // open or search, and reading one blocks.
                let is_file = entry.file_type().is_some_and(|kind| {
                    kind.is_file() || (kind.is_symlink() && entry.path().is_file())
                });
                if is_file && entry.depth() > 0 {
                    if let Ok(inner) = entry.path().strip_prefix(&root) {
                        if let Ok(mut list) = found.lock() {
                            list.push(slashed(inner));
                        }
                    }
                }
            }
            ignore::WalkState::Continue
        })
    });
    let mut paths = std::mem::take(&mut *found.lock().map_err(|e| e.to_string())?);
    // Files git tracks although a rule ignores their name: the walk leaves
    // them out, and git does not.
    if let Some(index) = git2::Repository::open(&key)
        .ok()
        .and_then(|repo| repo.index().ok())
    {
        let walked: HashSet<&str> = paths.iter().map(String::as_str).collect();
        let extra: Vec<String> = index
            .iter()
            .filter_map(|entry| String::from_utf8(entry.path).ok())
            .filter(|path| !walked.contains(path.as_str()) && key.join(path).is_file())
            .collect();
        paths.extend(extra);
    }
    paths.sort();
    paths.dedup();
    let paths = Arc::new(paths);
    if let Ok(mut cached) = cache().lock() {
        cached.insert(
            key,
            Listed {
                at: Instant::now(),
                paths: paths.clone(),
            },
        );
    }
    Ok(paths)
}

/// The folders whose files the finder lists only when asked: what Composer
/// and npm install. Git ignores them, so `all_files` never walks them.
pub const DEPENDENCY_DIRS: [&str; 2] = ["vendor", "node_modules"];

/// Build output and tool caches, left out of the list of ignored files
/// wherever they are: nobody looks for a file by name in `target/debug`, and
/// one Rust build fills it with hundreds of thousands. The watcher's own
/// list of noisy folders, less `vendor` (which is a dependency folder).
const BUILD_DIRS: [&str; 8] = [
    "node_modules",
    "target",
    ".output",
    ".nuxt",
    "dist",
    ".next",
    "__pycache__",
    ".venv",
];

/// At most this many dependency files are listed: `node_modules` can hold
/// more than anyone would scroll through.
const MAX_DEPENDENCY_FILES: usize = 300_000;

/// At most this many ignored files are listed. A Magento shop's `pub/media`
/// and `var/` can hold more than anyone would look through by name.
const MAX_IGNORED_FILES: usize = 300_000;

/// Files at most this deep are listed before any deeper one, so a cap that is
/// reached never leaves out `app/etc/env.php` or `.env`.
const SHALLOW: usize = 4;

/// A list of paths walked on demand, and whether the walk got to the end
/// (it stops at a cap).
#[derive(Clone, Debug, Default)]
pub struct Walked {
    pub paths: Arc<Vec<String>>,
    pub complete: bool,
}

/// The two lists walked on demand, kept per work tree.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Which {
    Dependencies,
    Ignored,
}

struct Kept {
    at: Instant,
    /// The watcher saw something change: handed out, and walked again.
    stale: bool,
    walked: Walked,
}

type KeptKey = (Which, PathBuf);

fn kept() -> &'static Mutex<HashMap<KeptKey, Kept>> {
    static KEPT: OnceLock<Mutex<HashMap<KeptKey, Kept>>> = OnceLock::new();
    KEPT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// How often a list was dropped, and how often aged. A walk stores what it
/// found only if no drop came since it began, so a walk that started before a
/// `.gitignore` changed cannot put back the old answer. An age since it began
/// stores it marked stale: kept, and walked again on the next ask, so a tree
/// that keeps changing still gets a list.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Generation {
    dropped: u64,
    aged: u64,
}

fn generations() -> &'static Mutex<HashMap<KeptKey, Generation>> {
    static GENERATIONS: OnceLock<Mutex<HashMap<KeptKey, Generation>>> = OnceLock::new();
    GENERATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn generation(key: &KeptKey) -> Generation {
    generations()
        .lock()
        .map(|all| all.get(key).copied().unwrap_or_default())
        .unwrap_or_default()
}

fn drop_kept(which: Which, root: &Path) {
    let key = (which, root.to_path_buf());
    let mut all = kept()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    all.remove(&key);
    if let Ok(mut generations) = generations().lock() {
        generations.entry(key).or_default().dropped += 1;
    }
}

fn age_kept(which: Which, root: &Path) {
    let key = (which, root.to_path_buf());
    let mut all = kept()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(one) = all.get_mut(&key) {
        one.stale = true;
    }
    if let Ok(mut generations) = generations().lock() {
        generations.entry(key).or_default().aged += 1;
    }
}

/// Keeps a walk's answer, unless the list was dropped since it began; marked
/// stale when it was aged meanwhile.
fn store_kept(key: &KeptKey, began: Generation, walked: Walked) {
    let mut all = kept()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Taken inside the list's lock, as `drop_kept` and `age_kept` do.
    let now = generation(key);
    if now.dropped != began.dropped {
        return;
    }
    all.insert(
        key.clone(),
        Kept {
            at: Instant::now(),
            stale: now.aged != began.aged,
            walked,
        },
    );
}

/// What is kept, and whether it should be walked again.
fn peek(key: &KeptKey) -> Option<(bool, Walked)> {
    let all = kept().lock().ok()?;
    all.get(key)
        .map(|one| (one.stale || one.at.elapsed() >= STALE, one.walked.clone()))
}

/// One lock per list and work tree, so two asks walk once.
fn walk_lock(key: &KeptKey) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<KeptKey, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut all = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    all.entry(key.clone()).or_default().clone()
}

/// A list kept per work tree. Fresh for [`STALE`], or until the watcher
/// marks it stale; after that the old list is handed out while a thread
/// walks it again, so a keystroke never waits on a walk it has already waited
/// on once. Walked while the caller waits only when there is nothing kept.
fn kept_list(which: Which, root: PathBuf, walk: fn(&Path) -> Walked) -> Walked {
    let key = (which, root);
    if let Some((stale, walked)) = peek(&key) {
        if stale {
            let lock = walk_lock(&key);
            std::thread::spawn(move || {
                // Somebody is walking it already.
                let Ok(_walking) = lock.try_lock() else {
                    return;
                };
                // Or has just finished.
                if !peek(&key).is_some_and(|(stale, _)| stale) {
                    return;
                }
                let began = generation(&key);
                store_kept(&key, began, walk(&key.1));
            });
        }
        return walked;
    }
    let lock = walk_lock(&key);
    let _walking = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // Walked by whoever held the lock before.
    if let Some((_, walked)) = peek(&key) {
        return walked;
    }
    let began = generation(&key);
    let walked = walk(&key.1);
    store_kept(&key, began, walked.clone());
    walked
}

fn canonical(root: &Path) -> Result<PathBuf, String> {
    root.canonicalize()
        .map_err(|e| format!("Could not read the work tree: {e}"))
}

/// Every file in `vendor/` and `node_modules/` at the root of the work tree,
/// relative and sorted, for the finder's and the search's dependencies
/// toggle. Walked the first time it is asked for, never at open. Links are
/// not followed, so nothing outside the work tree is listed.
pub fn dependency_files(root: &Path) -> Result<Arc<Vec<String>>, String> {
    Ok(dependency_walk(root)?.paths)
}

/// The same, and whether the walk got to the end.
pub fn dependency_walk(root: &Path) -> Result<Walked, String> {
    Ok(kept_list(
        Which::Dependencies,
        canonical(root)?,
        walk_dependencies,
    ))
}

/// The dependency files when they have been listed already; never walks.
pub fn dependency_walk_kept(root: &Path) -> Option<Walked> {
    let key = (Which::Dependencies, canonical(root).ok()?);
    peek(&key).map(|(_, walked)| walked)
}

fn walk_dependencies(key: &Path) -> Walked {
    // A file committed inside vendor/ (Magento's `vendor/.htaccess`, Go
    // vendoring) is a project file already: listed once, there.
    let listed = all_files(key).unwrap_or_default();
    let mut paths = Vec::new();
    let mut complete = true;
    for dir in DEPENDENCY_DIRS {
        let top = key.join(dir);
        if !top.is_dir() {
            continue;
        }
        let room = MAX_DEPENDENCY_FILES - paths.len();
        let found = walk_everything(key, &top, room + 1, &|| false).unwrap_or_default();
        if found.len() > room {
            complete = false;
        }
        paths.extend(
            found
                .into_iter()
                .take(room)
                .filter(|path| listed.binary_search(path).is_err()),
        );
    }
    paths.sort();
    Walked {
        paths: Arc::new(paths),
        complete,
    }
}

/// Every regular file under `dir`, whatever git ignores, relative to `key`
/// (the canonical work tree). Links are not followed and `.git` is skipped.
/// At most `room` of them; `None` when `stop` said to give up.
fn walk_everything(
    key: &Path,
    dir: &Path,
    room: usize,
    stop: &dyn Fn() -> bool,
) -> Option<Vec<String>> {
    let mut paths = Vec::new();
    let walk = WalkBuilder::new(dir)
        .standard_filters(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    for (seen, entry) in walk.filter_map(Result::ok).enumerate() {
        if seen % 1024 == 0 && seen > 0 && stop() {
            return None;
        }
        if paths.len() >= room {
            break;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if let Ok(inner) = entry.path().strip_prefix(key) {
            paths.push(slashed(inner));
        }
    }
    Some(paths)
}

/// Every file git ignores, outside `vendor/` and `node_modules/` (those are
/// [`dependency_files`]) and build output ([`BUILD_DIRS`]): `app/etc/env.php`,
/// `.env`, `generated/`, `var/`. Relative and sorted, walked the first time
/// it is asked for and kept like the dependencies. Links are not followed.
pub fn ignored_files(root: &Path) -> Result<Arc<Vec<String>>, String> {
    Ok(ignored_walk(root)?.paths)
}

/// The same, and whether the walk got to the end.
pub fn ignored_walk(root: &Path) -> Result<Walked, String> {
    Ok(kept_list(Which::Ignored, canonical(root)?, walk_ignored))
}

/// The ignored files when they have been listed already; never walks.
pub fn ignored_walk_kept(root: &Path) -> Option<Walked> {
    let key = (Which::Ignored, canonical(root).ok()?);
    peek(&key).map(|(_, walked)| walked)
}

/// Whether the walk for ignored files leaves this folder out: a dependency
/// folder at the root, or build output anywhere.
pub fn left_out_of_ignored(relative: &str) -> bool {
    in_dependency(relative) || relative.split('/').any(|part| BUILD_DIRS.contains(&part))
}

fn walk_ignored(key: &Path) -> Walked {
    let listed = all_files(key).unwrap_or_default();
    let found = Arc::new(Mutex::new(Vec::new()));
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // The shallow files first, all of them, then the deeper ones until the
    // cap: which files a cap leaves out never depends on the threads.
    for deep in [false, true] {
        let mut walk = WalkBuilder::new(key);
        walk.standard_filters(false)
            .follow_links(false)
            .filter_entry(|entry| {
                let name = entry.file_name();
                if name == ".git" || BUILD_DIRS.iter().any(|dir| name == *dir) {
                    return false;
                }
                !(entry.depth() == 1 && DEPENDENCY_DIRS.iter().any(|dir| name == *dir))
            });
        if !deep {
            walk.max_depth(Some(SHALLOW));
        }
        walk.build_parallel().run(|| {
            let found = found.clone();
            let count = count.clone();
            let listed = listed.clone();
            let root = key.to_path_buf();
            Box::new(move |entry| {
                let Ok(entry) = entry else {
                    return ignore::WalkState::Continue;
                };
                if deep && entry.depth() <= SHALLOW {
                    return ignore::WalkState::Continue;
                }
                if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                    return ignore::WalkState::Continue;
                }
                let Ok(inner) = entry.path().strip_prefix(&root) else {
                    return ignore::WalkState::Continue;
                };
                let relative = slashed(inner);
                if listed.binary_search(&relative).is_ok() {
                    return ignore::WalkState::Continue;
                }
                if count.fetch_add(1, Ordering::SeqCst) >= MAX_IGNORED_FILES {
                    return ignore::WalkState::Quit;
                }
                if let Ok(mut list) = found.lock() {
                    list.push(relative);
                }
                ignore::WalkState::Continue
            })
        });
    }
    let mut paths = std::mem::take(&mut *found.lock().unwrap_or_else(|p| p.into_inner()));
    paths.sort();
    Walked {
        complete: count.load(Ordering::SeqCst) <= MAX_IGNORED_FILES,
        paths: Arc::new(paths),
    }
}

/// Whether a relative path is inside `vendor/` or `node_modules/` at the
/// root, or is one of them.
pub fn in_dependency(relative: &str) -> bool {
    let first = relative.trim_matches('/').split('/').next().unwrap_or("");
    DEPENDENCY_DIRS.contains(&first)
}

/// Whether git ignores what is in a folder: the folder itself (`/vendor/`),
/// or everything in it (Magento's `/generated/*` with `!/generated/.htaccess`,
/// which leaves the folder itself not ignored).
pub fn folder_ignored(root: &Path, relative: &str) -> Result<bool, String> {
    let relative = relative.trim_matches('/');
    Ok(path_ignored_at(root, relative)?
        || path_ignored_at(root, &format!("{relative}/gitnoob-probe"))?)
}

/// Every file in one folder of the work tree, what git ignores included,
/// relative and sorted: for a search inside a folder git ignores, such as
/// `vendor/magento/module-catalog` or `generated`, when the lists above do not
/// have it. `None` when `stop` said to give up.
pub fn files_under(
    root: &Path,
    relative: &str,
    stop: &dyn Fn() -> bool,
) -> Result<Option<Vec<String>>, String> {
    let key = canonical(root)?;
    let dir = resolve(&key, relative, true)?;
    if !dir.is_dir() {
        return Err(format!("{relative} is not a folder"));
    }
    let Some(mut paths) = walk_everything(&key, &dir, MAX_DEPENDENCY_FILES, stop) else {
        return Ok(None);
    };
    paths.sort();
    Ok(Some(paths))
}

/// A test file: under a `tests`, `test` (Magento's `Test/Unit`),
/// `__tests__`, `spec` or `e2e` folder, or named the way a test runner finds
/// tests (`FooTest.php`, `foo.test.ts`, `foo.spec.js`, `foo_test.go`,
/// `test_foo.py`). `isTestPath` in `app/composables/pathKind.ts` says the
/// same for the window.
pub fn is_test_path(path: &str) -> bool {
    let mut parts: Vec<&str> = path.split('/').collect();
    let name = parts.pop().unwrap_or("");
    if parts.iter().any(|dir| {
        matches!(
            dir.to_ascii_lowercase().as_str(),
            "tests" | "test" | "__tests__" | "spec" | "e2e"
        )
    }) {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    let extension_after = |marker: &str| {
        lower.rfind(marker).is_some_and(|at| {
            at > 0 && !lower[at + marker.len()..].contains('.') && lower.len() > at + marker.len()
        })
    };
    (name.ends_with("Test.php") && name.len() > "Test.php".len())
        || extension_after(".test.")
        || extension_after(".spec.")
        || lower.ends_with("_test.go")
        || lower.ends_with("_test.py")
        || (lower.starts_with("test_") && lower.ends_with(".py") && lower.len() > "test_.py".len())
}

/// The newest finder request per work tree. An older one still scoring
/// stops early: its answer would only be thrown away.
fn latest(root: &Path) -> Arc<AtomicU64> {
    static LATEST: OnceLock<Mutex<HashMap<PathBuf, Arc<AtomicU64>>>> = OnceLock::new();
    let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut all = LATEST
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    all.entry(key).or_default().clone()
}

/// Files whose path matches `query`, best first.
///
/// Fuzzy, on the whole path: `invsvc` finds `app/Billing/InvoiceService.php`.
/// `request` is a number that goes up with each call; a call overtaken by a
/// newer one gives up and says so.
pub fn find_files(
    state: &AppState,
    query: &str,
    limit: usize,
    request: u64,
    also: &[String],
    dependencies: bool,
) -> Result<Vec<Found>, String> {
    let root = state.path()?;
    find_files_with(&root, query, limit, request, also, dependencies)
}

/// `also` are paths kept in the answer whenever they match, however far down
/// the list they score: the window puts recent and changed files first.
pub fn find_files_at(
    root: &Path,
    query: &str,
    limit: usize,
    request: u64,
    also: &[String],
) -> Result<Vec<Found>, String> {
    find_files_with(root, query, limit, request, also, false)
}

/// The same, and with `dependencies` the files in `vendor/` and
/// `node_modules/` too, after every project file that matched, `limit` more
/// at most.
pub fn find_files_with(
    root: &Path,
    query: &str,
    limit: usize,
    request: u64,
    also: &[String],
    dependencies: bool,
) -> Result<Vec<Found>, String> {
    find_paths(
        root,
        query,
        limit,
        request,
        also,
        dependencies,
        false,
        false,
    )
}

/// What a fuzzy score is worth in a file git ignores: a little less than in a
/// project file, so `generated/` and `pub/static/` copies of a file come
/// after it, while `env.php` for `app/etc/env.php` still beats a scatter of
/// the same letters across the project.
fn demoted(score: u32) -> u32 {
    let tier = score / SHAPED;
    tier * SHAPED + (score % SHAPED) * 3 / 4
}

/// The same, and with `folders` the folders whose own name matches too, for
/// the window's finder (the Files tree's filter lists files only).
///
/// A folder named exactly what was typed (`RMA` or `rma` for `app/code/Acme/RMA`)
/// comes before everything; other folders take their place among the files by
/// score. Folders never fill more than a sixth of `limit`, so a query like
/// `model` in a tree full of `Model/` folders still lists files.
///
/// With `ignored` the files git ignores (outside the dependencies) are ranked
/// among the project's, a little lower ([`demoted`]) and marked. The
/// dependencies come after all of those, with a `limit` of their own, so a
/// good match in `vendor/` is listed even when the project fills its limit
/// with poor ones.
#[allow(clippy::too_many_arguments)]
pub fn find_paths(
    root: &Path,
    query: &str,
    limit: usize,
    request: u64,
    also: &[String],
    dependencies: bool,
    ignored: bool,
    folders: bool,
) -> Result<Vec<Found>, String> {
    let newest = latest(root);
    newest.fetch_max(request, Ordering::SeqCst);
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let paths = all_files(root)?;
    let superseded = || request < newest.load(Ordering::SeqCst);
    if superseded() {
        return Err("superseded".into());
    }
    let wanted = limit.max(1);
    let room = if folders { (wanted / 6).max(1) } else { 0 };
    let mut found = rank(&paths, query, limit, also, &superseded)?;
    let ignored_list = if ignored {
        Some(ignored_files(root)?)
    } else {
        None
    };
    if let Some(extra) = &ignored_list {
        let mut more = rank(extra, query, limit, also, &superseded)?;
        for one in &mut more {
            one.score = demoted(one.score);
            one.ignored = true;
        }
        found = merged_files(found, more, limit, also);
    }
    let mut used = 0;
    if room > 0 {
        let mut dirs = rank_folders(&folders_of(&paths), query, room, &superseded)?;
        if let Some(extra) = &ignored_list {
            let seen: HashSet<String> = dirs.iter().map(|one| one.path.clone()).collect();
            let own = folders_of(&paths);
            let project: HashSet<&str> = own.iter().map(String::as_str).collect();
            let extra = folders_of(extra);
            let only: Vec<String> = extra
                .iter()
                .filter(|dir| !project.contains(dir.as_str()) && !seen.contains(*dir))
                .cloned()
                .collect();
            let mut more = rank_folders(&only, query, room, &superseded)?;
            for one in &mut more {
                one.ignored = true;
            }
            dirs.extend(more);
            dirs.sort_by(better);
            dirs.truncate(room);
        }
        used = dirs.len();
        found = merged(found, dirs, limit, also);
    }
    if dependencies {
        let extra = dependency_files(root)?;
        let mut more = rank(&extra, query, wanted, &[], &superseded)?;
        if room > used {
            let dirs = rank_folders(&folders_of(&extra), query, room - used, &superseded)?;
            more = merged(more, dirs, wanted, &[]);
        }
        found.extend(more);
    }
    Ok(found)
}

/// Two lists of files in one, best first, `limit` of them and any of `also`.
fn merged_files(
    mut files: Vec<Found>,
    more: Vec<Found>,
    limit: usize,
    also: &[String],
) -> Vec<Found> {
    files.extend(more);
    files.sort_by(better);
    let wanted: HashSet<&str> = also.iter().map(String::as_str).collect();
    let mut kept = 0usize;
    files.retain(|one| {
        kept += 1;
        kept <= limit.max(1) || wanted.contains(one.path.as_str())
    });
    files
}

/// Every folder the files in `paths` sit in, sorted. Worked out once per
/// list: the answer is kept for as long as the list itself is.
fn folders_of(paths: &Arc<Vec<String>>) -> Arc<Vec<String>> {
    type Kept = Vec<(Weak<Vec<String>>, Arc<Vec<String>>)>;
    static KEPT: OnceLock<Mutex<Kept>> = OnceLock::new();
    let kept = KEPT.get_or_init(|| Mutex::new(Vec::new()));
    // A list still alive is at its own address, so the address names it.
    let same = |from: &Weak<Vec<String>>| {
        from.strong_count() > 0 && std::ptr::eq(from.as_ptr(), Arc::as_ptr(paths))
    };
    {
        let mut all = kept.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        all.retain(|(from, _)| from.strong_count() > 0);
        if let Some((_, dirs)) = all.iter().find(|(from, _)| same(from)) {
            return dirs.clone();
        }
    }
    let mut dirs: HashSet<&str> = HashSet::new();
    for path in paths.iter() {
        let mut end = path.len();
        while let Some(at) = path[..end].rfind('/') {
            // Its parents went in with it the first time.
            if !dirs.insert(&path[..at]) {
                break;
            }
            end = at;
        }
    }
    let mut dirs: Vec<String> = dirs.into_iter().map(str::to_string).collect();
    dirs.sort();
    let dirs = Arc::new(dirs);
    let mut all = kept.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !all.iter().any(|(from, _)| same(from)) {
        all.push((Arc::downgrade(paths), dirs.clone()));
    }
    dirs
}

/// Best first; between equals the shorter path, then the alphabet.
fn better(a: &Found, b: &Found) -> std::cmp::Ordering {
    b.score
        .cmp(&a.score)
        .then_with(|| a.path.len().cmp(&b.path.len()))
        .then_with(|| a.path.cmp(&b.path))
}

/// Files and folders in one list, best first, `limit` of them and any file
/// of `also` the files kept.
fn merged(files: Vec<Found>, dirs: Vec<Found>, limit: usize, also: &[String]) -> Vec<Found> {
    if dirs.is_empty() {
        return files;
    }
    let mut all = files;
    all.extend(dirs);
    all.sort_by(better);
    let wanted: HashSet<&str> = also.iter().map(String::as_str).collect();
    let mut kept = 0usize;
    all.retain(|one| {
        kept += 1;
        kept <= limit.max(1) || (!one.dir && wanted.contains(one.path.as_str()))
    });
    all
}

/// A folder named what was typed: above every shaped tier.
const NAMED: u32 = 4;

/// The folders whose own name matches, best first, at most `room` of them.
///
/// The query has to reach into the last part of the folder: `RMA` finds
/// `app/code/Acme/RMA` and not every folder inside it. A folder whose name
/// is the query (or, with a slash, whose name is the last part of it) ranks
/// [`NAMED`], the shortest path first.
fn rank_folders(
    dirs: &[String],
    query: &str,
    room: usize,
    superseded: &dyn Fn() -> bool,
) -> Result<Vec<Found>, String> {
    if room == 0 {
        return Ok(Vec::new());
    }
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let shaped = Shaped::of(query);
    let typed = match &shaped {
        Shaped::Segments(parts) => parts.last().cloned().unwrap_or_default(),
        _ => query.to_lowercase(),
    };
    let segments = matches!(shaped, Shaped::Segments(_));
    let mut buffer = Vec::new();
    let mut found = Vec::new();
    for (at, path) in dirs.iter().enumerate() {
        if at % 4096 == 0 && at > 0 && superseded() {
            return Err("superseded".into());
        }
        let fuzzy = pattern.score(Utf32Str::new(path, &mut buffer), &mut matcher);
        let name = path.rsplit('/').next().unwrap_or(path);
        let name_at = (path.chars().count() - name.chars().count()) as u32;
        // A slash query fits only when its last part is this folder's name.
        let shape = match shaped.score(path) {
            Some((tier, _)) if segments && tier < 2 => None,
            shape => shape,
        };
        // Named whatever the case: `RMA` and `rma` both find `RMA/`.
        let named = name.to_lowercase() == typed && (shape.is_some() || !segments);
        let (score, mut indices) = match (shape, fuzzy) {
            (Some((_, indices)), _) if named => (NAMED * SHAPED, indices),
            (None, _) if named => (
                NAMED * SHAPED,
                (name_at..name_at + name.chars().count() as u32).collect(),
            ),
            (Some((tier, indices)), fuzzy) => {
                (tier * SHAPED + fuzzy.unwrap_or(0).min(SHAPED - 1), indices)
            }
            (None, Some(fuzzy)) => {
                let mut indices = Vec::new();
                pattern.indices(Utf32Str::new(path, &mut buffer), &mut matcher, &mut indices);
                (fuzzy.min(SHAPED - 1), indices)
            }
            (None, None) => continue,
        };
        indices.sort_unstable();
        indices.dedup();
        // Matched only in the folders above it: one of theirs, not this one.
        if !indices.last().is_some_and(|&last| last >= name_at) {
            continue;
        }
        found.push(Found {
            path: path.clone(),
            score,
            indices,
            dir: true,
            ignored: false,
        });
    }
    found.sort_by(better);
    found.truncate(room);
    Ok(found)
}

/// The paths that match, best first, `limit` of them and any of `also`.
fn rank(
    paths: &[String],
    query: &str,
    limit: usize,
    also: &[String],
    superseded: &dyn Fn() -> bool,
) -> Result<Vec<Found>, String> {
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let shaped = Shaped::of(query);
    let mut buffer = Vec::new();
    let mut scored: Vec<(u32, usize)> = Vec::new();
    // Where a shaped match picked its characters, which the window marks
    // instead of the fuzzy matcher's.
    let mut picked: HashMap<usize, Vec<u32>> = HashMap::new();
    for (at, path) in paths.iter().enumerate() {
        if at % 4096 == 0 && at > 0 && superseded() {
            return Err("superseded".into());
        }
        let fuzzy = pattern.score(Utf32Str::new(path, &mut buffer), &mut matcher);
        let shape = shaped.score(path);
        match (fuzzy, shape) {
            (_, Some((tier, indices))) => {
                scored.push((tier * SHAPED + fuzzy.unwrap_or(0).min(SHAPED - 1), at));
                picked.insert(at, indices);
            }
            (Some(score), None) => scored.push((score.min(SHAPED - 1), at)),
            (None, None) => {}
        }
    }
    // Best score first; between equals the shorter path, which is usually
    // the one meant, then the alphabet so the order never jumps about.
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| paths[a.1].len().cmp(&paths[b.1].len()))
            .then_with(|| paths[a.1].cmp(&paths[b.1]))
    });
    let wanted: HashSet<&str> = also.iter().map(String::as_str).collect();
    let mut kept = 0usize;
    scored.retain(|(_, at)| {
        kept += 1;
        kept <= limit.max(1) || wanted.contains(paths[*at].as_str())
    });

    Ok(scored
        .into_iter()
        .map(|(score, at)| {
            let path = &paths[at];
            let mut indices = picked.remove(&at).unwrap_or_default();
            if indices.is_empty() {
                pattern.indices(Utf32Str::new(path, &mut buffer), &mut matcher, &mut indices);
            }
            indices.sort_unstable();
            indices.dedup();
            Found {
                path: path.clone(),
                score,
                indices,
                dir: false,
                ignored: false,
            }
        })
        .collect())
}

/// A shaped match outranks any fuzzy one: each tier is worth this much.
const SHAPED: u32 = 100_000;

/// Two ways of typing a path that a fuzzy matcher scores no better than any
/// scatter of the same letters, though they are plainly what was meant:
///
/// - Folders with a slash: `exact/make` is a folder starting "exact" with,
///   right inside it, something starting "make".
/// - Capitals: `MECC` is a name whose capitals start M, E, C, C, the way
///   PhpStorm reads it: MakeExactConnectionCommand.
enum Shaped {
    None,
    Segments(Vec<String>),
    Initials(String),
}

impl Shaped {
    fn of(query: &str) -> Shaped {
        let query = query.trim();
        if query.contains('/') {
            let parts: Vec<String> = query
                .split('/')
                .filter(|part| !part.is_empty())
                .map(str::to_lowercase)
                .collect();
            if !parts.is_empty() {
                return Shaped::Segments(parts);
            }
        }
        if is_initials(query) {
            return Shaped::Initials(query.to_string());
        }
        Shaped::None
    }

    /// The tier (higher is better) and the characters it matched, by index
    /// in `path`'s characters.
    fn score(&self, path: &str) -> Option<(u32, Vec<u32>)> {
        match self {
            Shaped::None => None,
            Shaped::Segments(parts) => segments(path, parts),
            Shaped::Initials(query) => initials(path, query),
        }
    }
}

/// `parts` as the starts of consecutive folders; tier 2 when the last part
/// is the start of the file name, 1 when it is a folder further up.
fn segments(path: &str, parts: &[String]) -> Option<(u32, Vec<u32>)> {
    // Each segment with the index of its first character.
    let mut segs: Vec<(u32, String)> = Vec::new();
    let mut start = 0u32;
    let mut current = String::new();
    for (at, c) in path.chars().enumerate() {
        if c == '/' {
            segs.push((start, std::mem::take(&mut current)));
            start = at as u32 + 1;
        } else {
            current.extend(c.to_lowercase());
        }
    }
    segs.push((start, current));
    let fits = |from: usize| {
        parts.iter().enumerate().all(|(j, part)| {
            segs.get(from + j)
                .is_some_and(|(_, seg)| seg.starts_with(part.as_str()))
        })
    };
    // The match closest to the file name wins: that is where people start.
    let from = (0..segs.len()).rev().find(|&from| fits(from))?;
    let tier = if from + parts.len() == segs.len() {
        2
    } else {
        1
    };
    let indices = parts
        .iter()
        .enumerate()
        .flat_map(|(j, part)| {
            let first = segs[from + j].0;
            (0..part.chars().count() as u32).map(move |k| first + k)
        })
        .collect();
    Some((tier, indices))
}

/// Whether `query` is typed as capitals (`MECC`), for the symbol finder too.
pub fn is_initials(query: &str) -> bool {
    query.chars().count() >= 2 && query.chars().all(|c| c.is_ascii_uppercase())
}

/// The capitals of the file name: tier 3 when they are the query exactly, 2
/// when they start with it. A name with no folder is its own file name.
pub fn initials(path: &str, query: &str) -> Option<(u32, Vec<u32>)> {
    let name_at = path.chars().count() - path.rsplit('/').next().unwrap_or(path).chars().count();
    let name: Vec<char> = path.chars().skip(name_at).collect();
    // Up to the extension: `.php` has no capitals worth reading.
    let stem = name
        .iter()
        .position(|&c| c == '.')
        .filter(|&at| at > 0)
        .unwrap_or(name.len());
    let mut caps = String::new();
    let mut indices = Vec::new();
    for at in 0..stem {
        let c = name[at];
        let starts = at == 0 || matches!(name[at - 1], '_' | '-' | ' ');
        if c.is_ascii_uppercase() || (starts && c.is_ascii_alphabetic()) {
            caps.push(c.to_ascii_uppercase());
            indices.push((name_at + at) as u32);
        }
    }
    if !caps.starts_with(query) {
        return None;
    }
    indices.truncate(query.len());
    Some((if caps.len() == query.len() { 3 } else { 2 }, indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_gets_a_path_without_the_verbatim_prefix() {
        assert_eq!(without_verbatim(r"\\?\C:\repo\a.txt"), r"C:\repo\a.txt");
        assert_eq!(
            without_verbatim(r"\\?\UNC\server\share\a.txt"),
            r"\\server\share\a.txt"
        );
        assert_eq!(without_verbatim(r"C:\repo\a.txt"), r"C:\repo\a.txt");
        assert_eq!(without_verbatim("/repo/a.txt"), "/repo/a.txt");
    }

    #[test]
    fn tells_a_test_file_by_its_folder_or_its_name() {
        for path in [
            "tests/Feature/InvoiceTest.php",
            "app/code/Acme/Shop/Test/Unit/Model/ProductTest.php",
            "dev/tests/integration/x.php",
            "src/__tests__/a.ts",
            "app/Billing/InvoiceTest.php",
            "src/a.test.ts",
            "src/a.spec.js",
            "pkg/a_test.go",
            "lib/test_a.py",
        ] {
            assert!(is_test_path(path), "{path}");
        }
        for path in [
            "app/Latest.php",
            "app/Test.php",
            "src/contest.ts",
            ".test.ts",
            "src/a.test.ts.bak.d",
            "app/Testing/Helper.php",
            "lib/test_.py",
        ] {
            assert!(!is_test_path(path), "{path}");
        }
    }

    #[test]
    fn spells_paths_with_forward_slashes() {
        let path: PathBuf = ["app", "Billing", "Tax.php"].iter().collect();
        assert_eq!(slashed(&path), "app/Billing/Tax.php");
    }

    #[test]
    fn refuses_an_absolute_path_before_touching_the_disk() {
        let root = std::env::temp_dir();
        let absolute = if cfg!(windows) { "C:\\Windows" } else { "/etc" };
        assert!(resolve(&root, absolute, true).is_err());
    }
}
