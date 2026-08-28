use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// The key commits and tags made under this profile are signed with —
    /// `user.signingkey`. Unset leaves whatever the machine already says.
    #[serde(default)]
    pub signing_key: Option<String>,
    /// `gpg.format`: `openpgp`, `ssh` or `x509`.
    #[serde(default)]
    pub signing_format: Option<String>,
    /// `commit.gpgsign`. `None` means the profile has no opinion and the
    /// repository's own configuration is left exactly as it is.
    #[serde(default)]
    pub sign_commits: Option<bool>,
    /// `tag.gpgsign`.
    #[serde(default)]
    pub sign_tags: Option<bool>,
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
            signing_key: None,
            signing_format: None,
            sign_commits: None,
            sign_tags: None,
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
    /// What checking out a remote branch does when its local branch has
    /// commits of its own while the remote also moved on: `ask`, `rebase`,
    /// `merge` or `leave`. A branch that is merely behind is always
    /// fast-forwarded — that can lose nothing, so it is not a question.
    #[serde(default = "default_diverged_checkout")]
    pub diverged_checkout: String,
    /// Look up a picture for each commit author. Off, and the window draws
    /// initials instead and asks nobody anything.
    #[serde(default = "yes")]
    pub show_avatars: bool,
    /// Ask GitHub once at launch whether there is a newer release. Off, and the
    /// only check is the button in settings.
    #[serde(default = "yes")]
    pub check_updates: bool,
    /// Check the signature on every commit the graph draws. Off by default:
    /// it is a `git log` that runs gpg or ssh-keygen once per commit, and on a
    /// large page that is the slowest thing on the screen.
    #[serde(default)]
    pub verify_signatures: bool,
    /// How big the window was when it was last closed.
    #[serde(default)]
    pub window: Option<WindowSize>,
}

/// The size the window is reopened at.
///
/// Kept in the config rather than left to the platform, because the platform
/// only remembers it on one of the three this app runs on. Read back through
/// [`WindowSize::sane`], which is what stands between a stored number and a
/// window nobody can get hold of.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WindowSize {
    pub width: f64,
    pub height: f64,
    /// Full screen is a state rather than a size: restoring the size a window
    /// had while maximised would give a window exactly covering the screen but
    /// not maximised, which is not the same thing to drag or to unmaximise.
    #[serde(default)]
    pub maximized: bool,
}

impl WindowSize {
    /// The floor. Below this there is not enough window to find the corner of.
    pub const MIN: f64 = 700.0;
    /// The ceiling, when nothing is known about the screen: past this it is a
    /// number that came from somewhere other than a person resizing a window.
    pub const MAX: f64 = 8000.0;

    /// The stored size, made safe to open a window at.
    ///
    /// `screen` is what the display can actually show, when that is known: a
    /// window saved on a 5K monitor and reopened on a laptop is the commonest
    /// way to end up with a title bar somewhere off the top right of the world.
    /// Anything that is not a number at all — a hand-edited config, a crash
    /// mid-write — is refused outright rather than clamped, since there is no
    /// telling what it was meant to be.
    pub fn sane(&self, screen: Option<(f64, f64)>) -> Option<WindowSize> {
        if !self.width.is_finite() || !self.height.is_finite() {
            return None;
        }
        let (max_w, max_h) = screen
            .map(|(w, h)| (w.min(Self::MAX), h.min(Self::MAX)))
            .unwrap_or((Self::MAX, Self::MAX));
        Some(WindowSize {
            width: self.width.clamp(Self::MIN, max_w.max(Self::MIN)),
            height: self.height.clamp(Self::MIN, max_h.max(Self::MIN)),
            maximized: self.maximized,
        })
    }
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
/// Ask by default: the app is for people learning git, and quietly rebasing a
/// branch they did not know had diverged teaches nothing.
fn default_diverged_checkout() -> String {
    "ask".to_string()
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
            diverged_checkout: default_diverged_checkout(),
            show_avatars: true,
            check_updates: true,
            verify_signatures: false,
            window: None,
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
        Ok(mut config) => {
            tidy_projects(&mut config);
            config
        }
        Err(_) => {
            let _ = fs::rename(&path, path.with_extension("json.broken"));
            Config::default()
        }
    }
}

/// Settles a project list written by an earlier run, or by hand.
///
/// A work tree used to be recorded exactly as libgit2 handed it over, which is
/// with a separator on the end, while a path typed or restored from elsewhere
/// has none. The two were compared as strings, so one repository could be
/// listed twice and open as two identical tabs. Both spellings collapse to one
/// here, and the first entry keeps its place — the tab strip's order is the
/// user's, not the file's.
fn tidy_projects(config: &mut Config) {
    for profile in &mut config.profiles {
        let mut seen: Vec<String> = Vec::new();
        profile.projects.retain_mut(|project| {
            project.path = trim_separator(&project.path);
            if seen.contains(&project.path) {
                false
            } else {
                seen.push(project.path.clone());
                true
            }
        });
        if let Some(active) = profile.active_project.as_mut() {
            *active = trim_separator(active);
        }
        // An active project that is no longer in the list cannot be restored.
        if let Some(active) = profile.active_project.clone() {
            if !seen.contains(&active) {
                profile.active_project = None;
            }
        }
    }
}

/// The same path, without the separator some spellings of it end on.
fn trim_separator(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed.ends_with(':') {
        path.to_string()
    } else {
        trimmed.to_string()
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
///
/// The clock alone is not enough. Two profiles made in the same tick of it —
/// the first run, which writes a default profile and can have another added
/// straight after — came out carrying the same id, and the one the config
/// points at is then whichever the search reaches first. The counter is what
/// makes them differ; the clock is what keeps them ordered and unlike the ids
/// from any earlier run.
pub fn new_id() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let count = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}{count:x}")
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

/// The one keychain item every secret lives in, as a JSON object.
///
/// macOS ties an item's permission to the code signature of the program that
/// made it, and the app is not signed, so each build is a program the keychain
/// has never seen and every update is a stranger asking. Nothing in the app can
/// stop that question; what it can decide is how many times it gets asked. One
/// item for all the tokens is one dialog per update instead of one per token,
/// which was three for a single profile with an AI key.
const ALL: &str = "secrets";

/// Whether the keychain may be asked at all.
///
/// Reading a secret on an unsigned build makes macOS put a password dialog on
/// screen. That is the right thing when somebody is using the app and the wrong
/// thing everywhere else: a test suite that stops to ask for a password is a
/// test suite nobody can run, and it asks once per test that touches a token.
/// Off by default under `cargo test`, and the end-to-end tests — which build
/// the library without `cfg(test)` — turn it off themselves.
static ASK_KEYCHAIN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(!cfg!(test));

/// Stops this process ever asking the keychain for anything. For tests.
pub fn silence_keychain() {
    ASK_KEYCHAIN.store(false, std::sync::atomic::Ordering::Relaxed);
}

fn may_ask() -> bool {
    ASK_KEYCHAIN.load(std::sync::atomic::Ordering::Relaxed)
}

fn entry(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, account).map_err(|e| e.to_string())
}

/// The item as it was last read or written, so the keychain is asked once a run.
static BLOB: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Reads the one item. `None` means the keychain would not answer.
///
/// A refusal is not remembered as an empty item: the next write would then save
/// a map missing every token it could not see, and deleting the item is what
/// that would come to.
fn blob() -> Option<HashMap<String, String>> {
    if !may_ask() {
        return Some(HashMap::new());
    }
    let mut held = BLOB.lock().unwrap();
    if let Some(known) = held.as_ref() {
        return Some(known.clone());
    }
    let all = match entry(ALL).map(|entry| entry.get_password()) {
        Ok(Ok(text)) => serde_json::from_str(&text).unwrap_or_default(),
        // No item yet, which is an answer: there are no secrets.
        Ok(Err(keyring::Error::NoEntry)) => HashMap::new(),
        _ => return None,
    };
    *held = Some(all.clone());
    Some(all)
}

/// Writes the one item, or removes it once nothing is left to keep.
#[cfg(not(debug_assertions))]
fn blob_set(all: HashMap<String, String>) -> Result<(), String> {
    let entry = entry(ALL)?;
    if all.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.to_string()),
        }
    } else {
        let text = serde_json::to_string(&all).map_err(|e| e.to_string())?;
        entry.set_password(&text).map_err(|e| e.to_string())?;
    }
    *BLOB.lock().unwrap() = Some(all);
    Ok(())
}

#[cfg(not(debug_assertions))]
fn store_set(key: &str, value: Option<&str>) -> Result<(), String> {
    let mut all = blob().ok_or("The keychain would not open")?;
    match value {
        Some(value) => all.insert(key.to_string(), value.to_string()),
        None => all.remove(key),
    };
    blob_set(all)
}

#[cfg(not(debug_assertions))]
fn store_get(key: &str) -> Option<String> {
    if let Some(found) = blob()?.get(key) {
        return Some(found.clone());
    }
    // Every secret used to have an item of its own. One that is still there is
    // moved across the first time it is asked for, so an install that predates
    // the single item keeps its tokens; the old item goes only once the new one
    // holds the value.
    let old = entry(key).ok()?;
    let found = old.get_password().ok()?;
    if store_set(key, Some(&found)).is_ok() {
        let _ = old.delete_credential();
    }
    Some(found)
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

/// What the installed app left in the keychain for a key: `None` when the
/// keychain would not answer, `Some(None)` when it answered that there is
/// nothing. Looks in the one item first, then under the key's own old name.
#[cfg(debug_assertions)]
fn keychain_get(key: &str) -> Option<Option<String>> {
    if !may_ask() {
        // Answered, and the answer is that there is nothing. `None` would mean
        // "the keychain would not say", which sends the caller back to ask
        // again on the next read.
        return Some(None);
    }
    if let Some(found) = blob()?.get(key) {
        return Some(Some(found.clone()));
    }
    match entry(key).ok()?.get_password() {
        Ok(found) => Some(Some(found)),
        Err(keyring::Error::NoEntry) => Some(None),
        Err(_) => None,
    }
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
    let adopted = match keychain_get(key) {
        Some(Some(found)) => found,
        // The keychain has nothing under that name, which is an answer.
        Some(None) => String::new(),
        // Refused, or no keychain at all. Write nothing down: the user may have
        // dismissed the dialog by reflex, and a token they own should not be
        // out of reach for the rest of the build's life because of it.
        None => return None,
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

#[cfg(test)]
mod tests {
    use super::{Config, ForgeKind, Profile, Project};

    /// One repository, spelled two ways, is one tab.
    #[test]
    fn a_project_listed_with_and_without_its_trailing_slash_is_one_project() {
        let mut config = Config {
            profiles: vec![Profile {
                projects: vec![
                    Project {
                        path: "C:/repos/thing".to_string(),
                        name: "thing".to_string(),
                    },
                    Project {
                        path: "C:/repos/thing/".to_string(),
                        name: "thing".to_string(),
                    },
                    Project {
                        path: "C:/repos/other".to_string(),
                        name: "other".to_string(),
                    },
                ],
                active_project: Some("C:/repos/thing/".to_string()),
                ..Profile::new("Personal", ForgeKind::None)
            }],
            ..Config::default()
        };

        super::tidy_projects(&mut config);

        let profile = &config.profiles[0];
        assert_eq!(
            profile
                .projects
                .iter()
                .map(|p| p.path.as_str())
                .collect::<Vec<_>>(),
            vec!["C:/repos/thing", "C:/repos/other"]
        );
        // The one that was open is still the one that is open.
        assert_eq!(profile.active_project.as_deref(), Some("C:/repos/thing"));
    }

    /// A root is all separator, and trimming it away leaves nothing usable.
    #[test]
    fn a_root_path_keeps_its_separator() {
        assert_eq!(super::trim_separator("C:/"), "C:/");
        assert_eq!(super::trim_separator("/"), "/");
        assert_eq!(super::trim_separator("/home/arno/repo/"), "/home/arno/repo");
    }

    /// An active project that de-duplication removed cannot be restored.
    #[test]
    fn an_active_project_that_is_not_listed_is_forgotten() {
        let mut config = Config {
            profiles: vec![Profile {
                projects: vec![Project {
                    path: "/repos/kept".to_string(),
                    name: "kept".to_string(),
                }],
                active_project: Some("/repos/gone".to_string()),
                ..Profile::new("Personal", ForgeKind::None)
            }],
            ..Config::default()
        };
        super::tidy_projects(&mut config);
        assert_eq!(config.profiles[0].active_project, None);
    }

    #[test]
    fn a_saved_window_size_is_clamped_to_something_reachable() {
        let tiny = WindowSize {
            width: 40.0,
            height: 12.0,
            maximized: false,
        };
        let fitted = tiny.sane(None).expect("a size");
        assert_eq!(fitted.width, WindowSize::MIN);
        assert_eq!(fitted.height, WindowSize::MIN);

        // A window bigger than the screen it is opening on is a title bar out
        // of reach, which is the way this goes wrong that nobody can undo.
        let huge = WindowSize {
            width: 5000.0,
            height: 3000.0,
            maximized: false,
        };
        let fitted = huge.sane(Some((1440.0, 900.0))).expect("a size");
        assert_eq!(fitted.width, 1440.0);
        assert_eq!(fitted.height, 900.0);

        // With no screen to measure against, only the absurd is refused.
        let vast = WindowSize {
            width: 99_000.0,
            height: 99_000.0,
            maximized: false,
        };
        assert_eq!(vast.sane(None).unwrap().width, WindowSize::MAX);
    }

    #[test]
    fn a_size_that_is_not_a_number_is_refused_rather_than_repaired() {
        let broken = WindowSize {
            width: f64::NAN,
            height: 800.0,
            maximized: false,
        };
        assert!(broken.sane(None).is_none());
        let endless = WindowSize {
            width: 900.0,
            height: f64::INFINITY,
            maximized: false,
        };
        assert!(endless.sane(None).is_none());
    }

    #[test]
    fn a_size_that_was_already_sensible_is_left_alone() {
        let mine = WindowSize {
            width: 1680.0,
            height: 1040.0,
            maximized: false,
        };
        assert_eq!(mine.sane(Some((3840.0, 2160.0))), Some(mine));
    }
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A config directory of its own, removed when the test ends. Nothing here
    /// may touch the real one: it holds the user's whole setup.
    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "gitnoob-config-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Dir(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_first_run_gets_a_profile_that_is_already_active() {
        let dir = Dir::new("first-run");
        let config = load(dir.path());
        assert_eq!(config.profiles.len(), 1);
        assert!(
            config.active().is_some(),
            "a config whose active id names nothing has no profile to open projects under"
        );
    }

    #[test]
    fn what_was_saved_is_what_comes_back() {
        let dir = Dir::new("round-trip");
        let mut config = Config::default();
        let profile = config.profiles.first_mut().unwrap();
        profile.name = "Work".into();
        profile.forge = ForgeKind::GitLab;
        profile.host = "gitlab.example.com".into();
        profile.git_email = Some("me@example.com".into());
        profile.ssh_key = Some("/home/me/.ssh/id_work".into());
        profile.projects = vec![Project {
            path: "/repos/widget".into(),
            name: "widget".into(),
        }];
        profile.active_project = Some("/repos/widget".into());
        config.global.auto_fetch_minutes = 7;
        config.global.auto_stash = false;

        save(dir.path(), &config).unwrap();
        let read = load(dir.path());

        let back = read
            .active()
            .expect("the active profile should survive a save");
        assert_eq!(back.name, "Work");
        assert_eq!(back.forge, ForgeKind::GitLab);
        assert_eq!(back.host, "gitlab.example.com");
        assert_eq!(back.ssh_key.as_deref(), Some("/home/me/.ssh/id_work"));
        assert_eq!(back.projects.len(), 1);
        assert_eq!(back.active_project.as_deref(), Some("/repos/widget"));
        assert_eq!(read.global.auto_fetch_minutes, 7);
        assert!(!read.global.auto_stash);
    }

    #[test]
    fn saving_leaves_no_half_written_file_behind() {
        let dir = Dir::new("atomic");
        save(dir.path(), &Config::default()).unwrap();
        save(dir.path(), &Config::default()).unwrap();
        assert!(file_path(dir.path()).is_file());
        assert!(
            !dir.path().join("config.json.tmp").exists(),
            "the file written beside the target should have been renamed over it"
        );
    }

    /// Overwriting a config that failed to parse would throw away every profile
    /// on the machine, which is the one thing this file must never do.
    #[test]
    fn a_config_that_will_not_parse_is_moved_aside_rather_than_lost() {
        let dir = Dir::new("corrupt");
        let broken = "{ \"profiles\": [ oh dear";
        fs::write(file_path(dir.path()), broken).unwrap();

        let config = load(dir.path());
        assert_eq!(config.profiles.len(), 1, "the app still opens on defaults");

        let kept = dir.path().join("config.json.broken");
        assert_eq!(
            fs::read_to_string(&kept).unwrap(),
            broken,
            "the original text has to be recoverable, byte for byte"
        );
        assert!(!file_path(dir.path()).exists());
    }

    /// Every field added since a config was written carries a `serde(default)`.
    /// Without one, adding a setting would make every older config unparseable
    /// and send it to `.broken` on the next launch.
    #[test]
    fn a_config_written_by_an_older_version_keeps_its_profiles() {
        let dir = Dir::new("older");
        // A profile and a global block as they were before ssh keys, avatars,
        // the fetch interval and the AI settings existed.
        fs::write(
            file_path(dir.path()),
            r#"{
              "version": 1,
              "active_profile": "abc",
              "global": { "ai": {} },
              "profiles": [{ "id": "abc", "name": "Personal" }]
            }"#,
        )
        .unwrap();

        let config = load(dir.path());
        let profile = config.active().expect("the profile should still be found");
        assert_eq!(profile.name, "Personal");
        assert_eq!(profile.forge, ForgeKind::None);
        assert!(profile.projects.is_empty());
        assert!(profile.ssh_key.is_none());
        // The fields it never had take the values a new install would get.
        assert!(config.global.show_avatars);
        assert_eq!(config.global.auto_fetch_minutes, default_fetch_minutes());
        assert_eq!(config.global.graph_page_size, default_page_size());
    }

    #[test]
    fn the_config_from_the_old_app_name_is_adopted_once() {
        let parent = Dir::new("rename");
        let previous = parent.path().join("dev.gitui.app");
        fs::create_dir_all(&previous).unwrap();
        fs::write(
            previous.join("config.json"),
            r#"{"version":1,"active_profile":"old","global":{"ai":{}},
                "profiles":[{"id":"old","name":"From before the rename"}]}"#,
        )
        .unwrap();

        let now = parent.path().join("dev.gitnoob.app");
        let config = load(&now);
        assert_eq!(
            config.active().map(|p| p.name.as_str()),
            Some("From before the rename")
        );
        assert!(
            previous.join("config.json").is_file(),
            "the old file is copied, not moved, so a wrong call is recoverable"
        );
    }

    #[test]
    fn a_config_that_already_exists_is_not_replaced_by_the_old_one() {
        let parent = Dir::new("rename-existing");
        let previous = parent.path().join("dev.gitui.app");
        fs::create_dir_all(&previous).unwrap();
        fs::write(
            previous.join("config.json"),
            r#"{"version":1,"active_profile":"old","global":{"ai":{}},
                "profiles":[{"id":"old","name":"Stale"}]}"#,
        )
        .unwrap();

        let now = parent.path().join("dev.gitnoob.app");
        fs::create_dir_all(&now).unwrap();
        let mut current = Config::default();
        current.profiles[0].name = "Current".into();
        save(&now, &current).unwrap();

        assert_eq!(
            load(&now).active().map(|p| p.name.as_str()),
            Some("Current")
        );
    }

    #[test]
    fn the_active_profile_is_the_one_the_id_names() {
        let mut config = Config::default();
        let second = Profile::new("Work", ForgeKind::GitHub);
        let id = second.id.clone();
        config.profiles.push(second);
        config.active_profile = Some(id);
        assert_eq!(config.active().map(|p| p.name.as_str()), Some("Work"));

        config.active_mut().unwrap().name = "Renamed".into();
        assert_eq!(config.active().map(|p| p.name.as_str()), Some("Renamed"));

        // A deleted profile leaves the id behind; nothing may claim to be it.
        config.active_profile = Some("gone".into());
        assert!(config.active().is_none());
    }

    #[test]
    fn a_profile_starts_on_its_forges_own_host() {
        assert_eq!(Profile::new("A", ForgeKind::GitHub).host, "github.com");
        assert_eq!(Profile::new("B", ForgeKind::GitLab).host, "gitlab.com");
        assert_eq!(Profile::new("C", ForgeKind::None).host, "");
    }

    /// Tokens are keyed by profile, so two profiles on the same forge do not
    /// read each other's.
    #[test]
    fn forge_keys_are_per_profile() {
        assert_eq!(forge_key("abc"), "forge:abc");
        assert_ne!(forge_key("abc"), forge_key("def"));
    }
}
