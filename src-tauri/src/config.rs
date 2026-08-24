use std::fs;
use std::path::{Path, PathBuf};

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

/// Reads the config, falling back to defaults for a missing or unreadable file.
///
/// A corrupt file is moved aside rather than overwritten, so a hand-edit that
/// went wrong is still recoverable.
pub fn load(dir: &Path) -> Config {
    let path = file_path(dir);
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

const SERVICE: &str = "dev.gitui.app";

/// Stores a secret in the OS keychain. An empty value deletes the entry.
pub fn secret_set(key: &str, value: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, key).map_err(|e| e.to_string())?;
    if value.is_empty() {
        return match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }
    entry.set_password(value).map_err(|e| e.to_string())
}

pub fn secret_get(key: &str) -> Option<String> {
    keyring::Entry::new(SERVICE, key)
        .ok()
        .and_then(|entry| entry.get_password().ok())
        .filter(|value| !value.is_empty())
}

/// Keychain key for a profile's forge token.
pub fn forge_key(profile_id: &str) -> String {
    format!("forge:{profile_id}")
}

pub const OPENROUTER_KEY: &str = "openrouter";
