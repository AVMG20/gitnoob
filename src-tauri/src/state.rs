use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use git2::Repository;

use crate::ai::Model;
use crate::config::{self, Config};
use crate::journal::Journal;

/// Process-wide state.
///
/// `git2::Repository` is not `Sync` and a handle is cheap to create, so only the
/// repository path is kept here; each command opens its own handle.
///
/// The path is the one thing that is not shared between copies. The copy the
/// app manages follows the tab in front; a command gets a copy of its own,
/// pinned to the repository its call named (see [`AppState::pinned`]), so a
/// tab switched while it runs cannot move the path out from under it. The
/// settings, the model list and the undo histories live behind one `Arc` and
/// are the same for every copy.
pub struct AppState {
    path: Mutex<Option<PathBuf>>,
    shared: Arc<Shared>,
}

struct Shared {
    config_dir: Mutex<PathBuf>,
    config: Mutex<Config>,
    /// OpenRouter's model list, with the time it was fetched.
    models: Mutex<Option<(Instant, Vec<Model>)>>,
    /// Undo and redo history, one per work tree, keyed by its canonical path
    /// and loaded from disk the first time it is asked for.
    journals: Mutex<HashMap<PathBuf, Journal>>,
}

impl AppState {
    pub fn new(config_dir: PathBuf) -> Self {
        let config = config::load(&config_dir);
        AppState {
            path: Mutex::new(None),
            shared: Arc::new(Shared {
                config_dir: Mutex::new(config_dir),
                config: Mutex::new(config),
                models: Mutex::new(None),
                journals: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// A copy of this state whose path stays where it is now, whatever the
    /// original is pointed at afterwards. Everything else is shared.
    ///
    /// Every command from the window is handed one of these, made the moment
    /// its call arrives (`Aimed` in `lib.rs`). A pull that takes ten seconds,
    /// a commit waiting on a slow hook, a merge that fetches first: each reads
    /// the path again half-way, to record its undo step or to look at where
    /// the branch ended up, and with one path for the whole window a tab
    /// switched in between sent that second half to the other repository.
    pub fn pinned(&self) -> AppState {
        AppState {
            path: Mutex::new(self.path.lock().unwrap().clone()),
            shared: Arc::clone(&self.shared),
        }
    }

    /// A copy of this state pointed at `path`, whatever the original is
    /// pointed at. Everything else is shared, as for [`AppState::pinned`].
    pub fn pinned_to(&self, path: PathBuf) -> AppState {
        AppState {
            path: Mutex::new(Some(path)),
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn set_path(&self, path: PathBuf) {
        *self.path.lock().unwrap() = Some(path);
    }

    pub fn clear_path(&self) {
        *self.path.lock().unwrap() = None;
    }

    pub fn path(&self) -> Result<PathBuf, String> {
        self.path
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "No repository is open".to_string())
    }

    pub fn repo(&self) -> Result<Repository, String> {
        let path = self.path()?;
        Repository::open(&path).map_err(|e| format!("Could not open {}: {}", path.display(), e))
    }

    pub fn config(&self) -> Config {
        self.shared.config.lock().unwrap().clone()
    }

    /// Where settings live, and with them anything else worth keeping between
    /// runs — the cached author pictures, for one.
    pub fn config_dir(&self) -> PathBuf {
        self.shared.config_dir.lock().unwrap().clone()
    }

    /// Mutates the config and writes it out in one step, so what is in memory
    /// and what is on disk cannot drift.
    pub fn update_config<T>(&self, edit: impl FnOnce(&mut Config) -> T) -> Result<T, String> {
        let mut config = self.shared.config.lock().unwrap();
        let result = edit(&mut config);
        config::save(&self.shared.config_dir.lock().unwrap(), &config)?;
        Ok(result)
    }

    /// The profile id in force, used to look up the right forge token.
    pub fn active_profile_id(&self) -> Option<String> {
        self.shared.config.lock().unwrap().active_profile.clone()
    }

    /// Cached model list, if it was fetched recently enough to still be useful.
    pub fn cached_models(&self, max_age_secs: u64) -> Option<Vec<Model>> {
        let guard = self.shared.models.lock().unwrap();
        let (at, models) = guard.as_ref()?;
        (at.elapsed().as_secs() < max_age_secs).then(|| models.clone())
    }

    /// Borrows the undo history of the open work tree. Kept behind a closure so
    /// the lock is never held across anything that could block.
    pub fn journal<T>(&self, edit: impl FnOnce(&mut Journal) -> T) -> T {
        match self.path() {
            Ok(root) => self.journal_at(&root, edit),
            // Nothing open: a history that belongs to nothing and goes nowhere.
            Err(_) => edit(&mut Journal::default()),
        }
    }

    /// Borrows the undo history of one work tree, read once by the caller.
    ///
    /// One per work tree rather than one for the window: two tabs, or two
    /// worktrees of the same repository, each undo their own steps. Anything
    /// the edit changed is written back to disk before the lock is let go.
    pub fn journal_at<T>(&self, root: &Path, edit: impl FnOnce(&mut Journal) -> T) -> T {
        let key = crate::journal::key(root);
        let mut all = self.shared.journals.lock().unwrap();
        let journal = all
            .entry(key.clone())
            .or_insert_with(|| Journal::load(root, &key));
        // Another window of the app may have written the file since; its
        // steps are caught up with before any are added, numbered or written.
        journal.refresh();
        let out = edit(journal);
        journal.save();
        out
    }

    pub fn cache_models(&self, models: Vec<Model>) {
        *self.shared.models.lock().unwrap() = Some((Instant::now(), models));
    }
}

/// Resolves a path the user picked to the root of its work tree, so opening a
/// subdirectory of a repository works the way it does on the command line.
pub fn discover_workdir(input: &Path) -> Result<PathBuf, String> {
    let repo = Repository::discover(input)
        .map_err(|_| format!("{} is not inside a Git repository", input.display()))?;
    repo.workdir()
        .map(trim_separator)
        .ok_or_else(|| "Bare repositories are not supported yet".to_string())
}

/// A work tree path without the separator libgit2 leaves on the end of it.
///
/// The path recorded for an open project is compared against the ones already
/// recorded, and `C:/repos/thing` and `C:/repos/thing/` are the same repository
/// spelled two ways. Comparing them as strings said they were not, so opening a
/// repository whose entry had been written the other way added a *second* entry
/// for it — and the window drew two tabs, both called the same thing, both
/// pointing at one folder.
pub fn trim_separator(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(['/', '\\']);
    // A root — `/` or `C:\` — is all separator, and trimming it away would
    // leave something that is not a path at all.
    if trimmed.is_empty() || trimmed.ends_with(':') {
        path.to_path_buf()
    } else {
        PathBuf::from(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command's copy keeps the repository it was called for while the tab
    /// in front changes, and still shares the settings with the app.
    #[test]
    fn a_pinned_copy_keeps_its_path_and_shares_the_rest() {
        let dir = std::env::temp_dir().join(format!("gitnoob-pinned-{}", std::process::id()));
        let state = AppState::new(dir.join("config"));
        state.set_path(PathBuf::from("/one"));
        let pinned = state.pinned();
        state.set_path(PathBuf::from("/two"));
        assert_eq!(pinned.path().unwrap(), PathBuf::from("/one"));
        assert_eq!(state.path().unwrap(), PathBuf::from("/two"));

        state.cache_models(Vec::new());
        assert!(
            pinned.cached_models(60).is_some(),
            "the model list is shared"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
