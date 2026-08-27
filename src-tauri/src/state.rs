use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use git2::Repository;

use crate::ai::Model;
use crate::config::{self, Config};
use crate::journal::Journal;

/// Process-wide state.
///
/// `git2::Repository` is not `Sync` and a handle is cheap to create, so only the
/// repository path is kept here; each command opens its own handle.
pub struct AppState {
    path: Mutex<Option<PathBuf>>,
    config_dir: Mutex<PathBuf>,
    config: Mutex<Config>,
    /// OpenRouter's model list, with the time it was fetched.
    models: Mutex<Option<(Instant, Vec<Model>)>>,
    /// Undo and redo history for this session.
    journal: Mutex<Journal>,
}

impl AppState {
    pub fn new(config_dir: PathBuf) -> Self {
        let config = config::load(&config_dir);
        AppState {
            path: Mutex::new(None),
            config_dir: Mutex::new(config_dir),
            config: Mutex::new(config),
            models: Mutex::new(None),
            journal: Mutex::new(Journal::default()),
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
        self.config.lock().unwrap().clone()
    }

    /// Where settings live, and with them anything else worth keeping between
    /// runs — the cached author pictures, for one.
    pub fn config_dir(&self) -> PathBuf {
        self.config_dir.lock().unwrap().clone()
    }

    /// Mutates the config and writes it out in one step, so what is in memory
    /// and what is on disk cannot drift.
    pub fn update_config<T>(&self, edit: impl FnOnce(&mut Config) -> T) -> Result<T, String> {
        let mut config = self.config.lock().unwrap();
        let result = edit(&mut config);
        config::save(&self.config_dir.lock().unwrap(), &config)?;
        Ok(result)
    }

    /// The profile id in force, used to look up the right forge token.
    pub fn active_profile_id(&self) -> Option<String> {
        self.config.lock().unwrap().active_profile.clone()
    }

    /// Cached model list, if it was fetched recently enough to still be useful.
    pub fn cached_models(&self, max_age_secs: u64) -> Option<Vec<Model>> {
        let guard = self.models.lock().unwrap();
        let (at, models) = guard.as_ref()?;
        (at.elapsed().as_secs() < max_age_secs).then(|| models.clone())
    }

    /// Borrows the undo history. Kept behind a closure so the lock is never held
    /// across anything that could block.
    pub fn journal<T>(&self, edit: impl FnOnce(&mut Journal) -> T) -> T {
        edit(&mut self.journal.lock().unwrap())
    }

    pub fn cache_models(&self, models: Vec<Model>) {
        *self.models.lock().unwrap() = Some((Instant::now(), models));
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
