use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Which forge a profile talks to.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    #[default]
    None,
    GitHub,
    GitLab,
}

impl ForgeKind {
    pub fn default_host(&self) -> &'static str {
        match self {
            ForgeKind::GitHub => "github.com",
            ForgeKind::GitLab => "gitlab.com",
            ForgeKind::None => "",
        }
    }
}

/// One repository in a profile's tab strip.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Project {
    pub path: String,
    pub name: String,
}

/// A profile is a working context: which forge, which identity, which
/// repositories are open. Switching profiles swaps all three at once.
#[derive(Serialize, Deserialize, Clone)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub forge: ForgeKind,
    /// Host for the forge, so self-hosted GitLab works.
    #[serde(default)]
    pub host: String,
    /// Overrides `user.name` / `user.email` when set on a repository.
    #[serde(default)]
    pub git_name: Option<String>,
    #[serde(default)]
    pub git_email: Option<String>,
    /// Private key this profile authenticates with. Unset means ssh decides for
    /// itself, the way it did before profiles existed. Set, and every git
    /// command run under this profile uses that key and no other, which is what
    /// lets a work account and a personal account share one machine.
    #[serde(default)]
    pub ssh_key: Option<String>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub active_project: Option<String>,
}

impl Profile {
    pub fn new(name: &str, forge: ForgeKind) -> Self {
        Profile {
            id: new_id(),
            name: name.to_string(),
            host: forge.default_host().to_string(),
            forge,
            git_name: None,
            git_email: None,
            ssh_key: None,
            projects: Vec::new(),
            active_project: None,
        }
    }
}

/// Settings that are the same whichever profile is active.
#[derive(Serialize, Deserialize, Clone)]
pub struct Global {
    #[serde(default)]
    pub ai: Ai,
    #[serde(default = "default_page_size")]
    pub graph_page_size: usize,
    /// Fetch as soon as a project tab is opened, so the ahead/behind counts are
    /// true rather than whatever they were last session.
    #[serde(default = "yes")]
    pub auto_fetch_on_open: bool,
    /// Keep fetching this often while a project is open. Zero turns it off.
    #[serde(default = "default_fetch_minutes")]
    pub auto_fetch_minutes: u32,
    /// Stash uncommitted work before a branch switch or pull, then put it back.
    #[serde(default = "yes")]
    pub auto_stash: bool,
    /// Look up a picture for each commit author. Off, and the window draws
    /// initials instead and asks nobody anything.
    #[serde(default = "yes")]
    pub show_avatars: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Ai {
    /// An OpenRouter model id, e.g. `anthropic/claude-sonnet-4.5`.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// How hard the model should think before answering, in OpenRouter's own
    /// terms: `off`, `minimal`, `low`, `medium` or `high`. A model that cannot
    /// reason ignores it.
    #[serde(default = "default_reasoning")]
    pub reasoning: String,
    /// `conventional` or `plain`.
    #[serde(default = "default_commit_style")]
    pub commit_style: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub active_profile: Option<String>,
    #[serde(default)]
    pub global: Global,
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

fn one() -> u32 {
    1
}
fn yes() -> bool {
    true
}
fn default_page_size() -> usize {
    500
}
fn default_max_tokens() -> u32 {
    1500
}
/// Off by default: a commit message does not need a reasoning budget, and
/// thinking tokens are billed.
fn default_reasoning() -> String {
    "off".to_string()
}
fn default_commit_style() -> String {
    "plain".to_string()
}
fn default_fetch_minutes() -> u32 {
    10
}

impl Default for Ai {
    fn default() -> Self {
        Ai {
            model: None,
            max_tokens: default_max_tokens(),
            reasoning: default_reasoning(),
            commit_style: default_commit_style(),
        }
    }
}

impl Default for Global {
    fn default() -> Self {
        Global {
            ai: Ai::default(),
            graph_page_size: default_page_size(),
            auto_fetch_on_open: true,
            auto_fetch_minutes: default_fetch_minutes(),
            auto_stash: true,
            show_avatars: true,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        // A brand new install still needs somewhere to put open projects.
        let profile = Profile::new("Personal", ForgeKind::None);
        Config {
            version: 1,
            active_profile: Some(profile.id.clone()),
            global: Global::default(),
            profiles: vec![profile],
        }
    }
}

impl Config {
    pub fn active(&self) -> Option<&Profile> {
        let id = self.active_profile.as_ref()?;
        self.profiles.iter().find(|p| &p.id == id)
    }

    pub fn active_mut(&mut self) -> Option<&mut Profile> {
        let id = self.active_profile.clone()?;
        self.profiles.iter_mut().find(|p| p.id == id)
    }
}

pub fn file_path(dir: &Path) -> PathBuf {
    dir.join("config.json")
}

/// The app's config directory is named after its bundle identifier, so renaming
/// the app from `gitui` to `gitnoob` moved it and left the old profiles behind.
/// On a first run under the new name, bring the old file across.
///
/// Copies rather than moves: if this turns out to be the wrong call, the
/// original is still where it was. Tokens cannot come along — they live in the
/// OS keychain under the old service name and have to be entered again.
fn adopt_previous_name(dir: &Path) {
    let Some(parent) = dir.parent() else { return };
    let previous = parent.join("dev.gitui.app").join("config.json");
    if !previous.is_file() {
        return;
    }
    if fs::create_dir_all(dir).is_ok() {
        let _ = fs::copy(&previous, file_path(dir));
    }
}

/// Reads the config, falling back to defaults for a missing or unreadable file.
///
/// A corrupt file is moved aside rather than overwritten, so a hand-edit that
/// went wrong is still recoverable.
pub fn load(dir: &Path) -> Config {
    let path = file_path(dir);
    if !path.exists() {
        adopt_previous_name(dir);
    }
    let Ok(text) = fs::read_to_string(&path) else {
        return Config::default();
    };
    match serde_json::from_str::<Config>(&text) {
        Ok(config) => config,
        Err(_) => {
            let _ = fs::rename(&path, path.with_extension("json.broken"));
            Config::default()
        }
    }
}

pub fn save(dir: &Path, config: &Config) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {}", dir.display(), e))?;
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let path = file_path(dir);
    // Write beside the target and rename, so an interrupted save cannot leave a
    // half-written config behind.
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text).map_err(|e| format!("Could not write {}: {}", temp.display(), e))?;
    fs::rename(&temp, &path).map_err(|e| e.to_string())
}

/// A short unique id; enough for a local config file.
pub fn new_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

// --- secrets ---------------------------------------------------------------

const SERVICE: &str = "dev.gitnoob.app";

/// The config directory, for the development token file. Set once at startup.
static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Tells the secret store where the config lives.
pub fn use_dir(dir: &Path) {
    *DIR.lock().unwrap() = Some(dir.to_path_buf());
}

/// Secrets read since the app started, including the ones that were not there.
///
/// The keychain asks the user to authorise a read whenever it does not
/// recognise the program doing it, and one refresh reads the same token four
/// times over — the forge status, the account, the review list and the AI
/// settings all want it. That is four dialogs for one branch switch, which is
/// the difference between an app you can use and one you cannot. Reading each
/// key once a run makes it at most one.
static CACHED: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

fn cached(key: &str) -> Option<Option<String>> {
    CACHED
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|seen| seen.get(key).cloned())
}

/// Records what the keychain said, so it is not asked again this run. Also the
/// write-through for [`secret_set`]: a token replaced in settings has to be the
/// one every later read sees.
fn cache(key: &str, value: Option<String>) {
    CACHED
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(key.to_string(), value);
}

/// Stores a secret. An empty value deletes it.
pub fn secret_set(key: &str, value: &str) -> Result<(), String> {
    let value = (!value.is_empty()).then(|| value.to_string());
    store_set(key, value.as_deref())?;
    cache(key, value);
    Ok(())
}

pub fn secret_get(key: &str) -> Option<String> {
    if let Some(known) = cached(key) {
        return known;
    }
    let found = store_get(key).filter(|value| !value.is_empty());
    cache(key, found.clone());
    found
}

#[cfg(not(debug_assertions))]
fn store_set(key: &str, value: Option<&str>) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, key).map_err(|e| e.to_string())?;
    match value {
        Some(value) => entry.set_password(value).map_err(|e| e.to_string()),
        None => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        },
    }
}

#[cfg(not(debug_assertions))]
fn store_get(key: &str) -> Option<String> {
    keyring::Entry::new(SERVICE, key)
        .ok()
        .and_then(|entry| entry.get_password().ok())
}

/// Where a development build keeps its tokens.
///
/// macOS ties a keychain item's permission to the exact binary that made it, so
/// every rebuild is a stranger asking for a password — and a rebuild happens
/// dozens of times a day. That question is worth asking of the app someone
/// installed, not of the one being written, so a debug build keeps its tokens
/// in a file beside the config, readable by its owner and nobody else. The
/// keychain code above is what a release build compiles.
#[cfg(debug_assertions)]
fn dev_file() -> Option<PathBuf> {
    Some(DIR.lock().unwrap().clone()?.join("secrets.dev.json"))
}

#[cfg(debug_assertions)]
fn dev_all() -> HashMap<String, String> {
    dev_file()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Reads the development file, falling back to the keychain the first time.
///
/// A build that has never been asked for a key takes whatever the installed app
/// left there, so switching to the file store does not sign the user out of
/// profiles they set up. That costs one authorisation dialog per key, once. A
/// key that turns out not to be in the keychain either is written down as
/// empty, which counts as an answer: the question is not asked twice.
#[cfg(debug_assertions)]
fn store_get(key: &str) -> Option<String> {
    if let Some(known) = dev_all().get(key) {
        return Some(known.clone());
    }
    let adopted = match keyring::Entry::new(SERVICE, key).map(|entry| entry.get_password()) {
        Ok(Ok(found)) => found,
        // The keychain has nothing under that name, which is an answer.
        Ok(Err(keyring::Error::NoEntry)) => String::new(),
        // Refused, or no keychain at all. Write nothing down: the user may have
        // dismissed the dialog by reflex, and a token they own should not be
        // out of reach for the rest of the build's life because of it.
        _ => return None,
    };
    let _ = store_set(key, Some(&adopted));
    Some(adopted)
}

#[cfg(debug_assertions)]
fn store_set(key: &str, value: Option<&str>) -> Result<(), String> {
    let Some(path) = dev_file() else {
        return Err("No config directory to keep tokens in".to_string());
    };
    let mut all = dev_all();
    match value {
        Some(value) => all.insert(key.to_string(), value.to_string()),
        None => all.remove(key),
    };
    let text = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    fs::write(&path, text).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Keychain key for a profile's forge token.
pub fn forge_key(profile_id: &str) -> String {
    format!("forge:{profile_id}")
}

pub const OPENROUTER_KEY: &str = "openrouter";
