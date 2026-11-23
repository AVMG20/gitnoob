use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::{self, AiProvider, OPENROUTER_KEY};
use crate::conflict;
use crate::git_cmd;
use crate::state::AppState;

const BASE: &str = "https://openrouter.ai/api/v1";
/// Long enough that opening the picker twice does not hit the network twice,
/// short enough that new models show up the same day.
const MODEL_CACHE_SECS: u64 = 60 * 60 * 6;
/// Local CLI model lists are queried from the CLI and cached for two hours
/// (both in memory and on disk); Recheck bypasses the cache.
const CLI_MODEL_CACHE_SECS: u64 = 60 * 60 * 2;
/// Diffs get large; past this the model adds nothing but cost.
const MAX_DIFF_CHARS: usize = 48_000;
/// Git's empty tree, which is what a first commit is diffed against.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Serialize, Deserialize, Clone)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub context_length: u64,
    /// US dollars per million tokens, which is how everyone quotes them.
    pub prompt_price: f64,
    pub completion_price: f64,
    pub description: String,
    /// True when the model takes images as well as text.
    pub multimodal: bool,
}

#[derive(Serialize)]
pub struct AiStatus {
    pub configured: bool,
    pub provider: AiProvider,
    pub model: Option<String>,
    /// What the settings box starts from, and what "put the default back"
    /// puts back. Sent rather than repeated in the window, so the two cannot
    /// drift apart.
    pub default_commit_prompt: String,
}

/// Whether one of the local AI CLIs is installed on this machine.
#[derive(Serialize, Clone, Debug)]
pub struct CliInfo {
    pub provider: AiProvider,
    pub binary: String,
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

/// A model reported by a local AI CLI, together with the thinking/effort modes
/// it supports.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CliModel {
    pub id: String,
    pub name: String,
    pub note: String,
    /// Ordered thinking/effort levels supported by this model (e.g. `low`,
    /// `medium`, `high`, `xhigh`, `max`).
    #[serde(default)]
    pub effort_levels: Vec<String>,
    /// The model's default thinking/effort level, if reported by the CLI.
    #[serde(default)]
    pub default_effort: Option<String>,
    /// For CLIs whose effort levels are separate model ids (Antigravity), maps
    /// each effort level to the exact model id passed to `--model`.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub effort_models: HashMap<String, String>,
}

#[derive(Serialize)]
pub struct CommitMessage {
    pub summary: String,
    pub body: String,
}

pub fn status(state: &AppState) -> AiStatus {
    let config = state.config();
    let ai = &config.global.ai;
    let configured = match ai.provider {
        AiProvider::OpenRouter => {
            config::secret_get(OPENROUTER_KEY).is_some() && ai.active_model().is_some()
        }
        other => cli_binary_name(other).and_then(find_binary).is_some(),
    };
    AiStatus {
        configured,
        provider: ai.provider,
        model: ai.active_model(),
        default_commit_prompt: DEFAULT_COMMIT_PROMPT.to_string(),
    }
}

/// The executable name for a CLI provider.
fn cli_binary_name(provider: AiProvider) -> Option<&'static str> {
    match provider {
        AiProvider::OpenRouter => None,
        AiProvider::Claude => Some("claude"),
        AiProvider::Codex => Some("codex"),
        AiProvider::Antigravity => Some("agy"),
    }
}

fn cli_display_name(provider: AiProvider) -> &'static str {
    match provider {
        AiProvider::OpenRouter => "OpenRouter",
        AiProvider::Claude => "Claude CLI",
        AiProvider::Codex => "Codex CLI",
        AiProvider::Antigravity => "Antigravity CLI",
    }
}

/// The user's home directory, read from the platform's own variable.
fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let var = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let var = std::env::var_os("HOME");
    var.filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Directories to search for user-installed CLIs.
///
/// A macOS desktop app launched from Finder or the Dock starts with a minimal
/// `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`) that leaves out `~/.local/bin`,
/// Homebrew and package-manager bin folders — the exact places `claude`,
/// `codex` and `agy` live. Appending them after `PATH` finds the user's copy
/// without overriding anything already on `PATH`.
fn augmented_path_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();

    let mut extras: Vec<PathBuf> = Vec::new();
    #[cfg(not(windows))]
    {
        if let Some(home) = home_dir() {
            extras.push(home.join(".local/bin"));
            extras.push(home.join(".bun/bin"));
            extras.push(home.join(".npm-global/bin"));
            extras.push(home.join(".cargo/bin"));
            extras.push(home.join(".volta/bin"));
            extras.push(home.join(".nix-profile/bin"));
            extras.push(home.join(".asdf/shims"));
            extras.push(home.join(".local/share/mise/shims"));
        }
        extras.push(PathBuf::from("/opt/homebrew/bin"));
        extras.push(PathBuf::from("/opt/homebrew/sbin"));
        extras.push(PathBuf::from("/usr/local/bin"));
    }
    #[cfg(windows)]
    {
        if let Some(home) = home_dir() {
            extras.push(home.join(".local").join("bin"));
            extras.push(home.join(".bun").join("bin"));
            extras.push(home.join(".cargo").join("bin"));
            extras.push(home.join(".volta").join("bin"));
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            extras.push(PathBuf::from(appdata).join("npm"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            extras.push(PathBuf::from(local).join("Microsoft").join("WinGet").join("Links"));
        }
    }

    for extra in extras {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    dirs
}

fn augmented_path() -> OsString {
    std::env::join_paths(augmented_path_dirs())
        .unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Locates `name` on the augmented `PATH`, returning its full path when found.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    for dir in augmented_path_dirs() {
        #[cfg(windows)]
        {
            for ext in ["exe", "cmd", "bat"] {
                let candidate = dir.join(format!("{name}.{ext}"));
                if is_executable(&candidate) {
                    return Some(candidate);
                }
            }
            let bare = dir.join(name);
            if is_executable(&bare) {
                return Some(bare);
            }
        }
        #[cfg(not(windows))]
        {
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn cli_command(path: &Path) -> Command {
    let mut cmd = Command::new(path);
    git_cmd::quiet(&mut cmd);
    cmd.current_dir(std::env::temp_dir())
        .env("PATH", augmented_path())
        .env("NO_COLOR", "1");
    cmd
}

/// Checks whether each supported local AI CLI is installed, and reads its
/// version when it is.
pub fn cli_info() -> Vec<CliInfo> {
    const PROVIDERS: [AiProvider; 3] = [
        AiProvider::Claude,
        AiProvider::Codex,
        AiProvider::Antigravity,
    ];
    std::thread::scope(|scope| {
        let handles: Vec<_> = PROVIDERS
            .iter()
            .map(|&provider| {
                scope.spawn(move || {
                    let binary = cli_binary_name(provider).unwrap_or("").to_string();
                    let found = find_binary(&binary);
                    let version = found.as_deref().and_then(read_cli_version);
                    CliInfo {
                        provider,
                        binary,
                        available: found.is_some(),
                        path: found.map(|p| p.to_string_lossy().into_owned()),
                        version,
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .collect()
    })
}

fn read_cli_version(path: &Path) -> Option<String> {
    let mut cmd = cli_command(path);
    cmd.arg("--version")
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT");
    let out = run_cli_with_stdin(cmd, None, "cli", Duration::from_secs(5)).ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(60).collect())
}

#[derive(Serialize, Deserialize, Clone)]
struct CachedCliModels {
    fetched_at: u64,
    models: Vec<CliModel>,
}

type CliModelCache = HashMap<AiProvider, CachedCliModels>;

fn cli_model_cache() -> &'static Mutex<CliModelCache> {
    static CACHE: OnceLock<Mutex<CliModelCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Serializes read-modify-write updates to `cli-models.json` across concurrent
/// model fetches so two providers cannot clobber each other or fight over the
/// temporary file.
static DISK_CACHE_LOCK: Mutex<()> = Mutex::new(());

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cache_key(provider: AiProvider) -> &'static str {
    match provider {
        AiProvider::OpenRouter => "openrouter",
        AiProvider::Claude => "claude",
        AiProvider::Codex => "codex",
        AiProvider::Antigravity => "antigravity",
    }
}

fn disk_cache_path(config_dir: &Path) -> PathBuf {
    config_dir.join("cli-models.json")
}

fn read_disk_cache(config_dir: &Path, provider: AiProvider) -> Option<CachedCliModels> {
    let _guard = DISK_CACHE_LOCK.lock().ok();
    let text = std::fs::read_to_string(disk_cache_path(config_dir)).ok()?;
    let map: HashMap<String, CachedCliModels> = serde_json::from_str(&text).ok()?;
    map.get(cache_key(provider)).cloned()
}

fn write_disk_cache(config_dir: &Path, provider: AiProvider, entry: &CachedCliModels) {
    let _guard = DISK_CACHE_LOCK.lock().ok();
    let path = disk_cache_path(config_dir);
    let mut map: HashMap<String, CachedCliModels> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    map.insert(cache_key(provider).to_string(), entry.clone());
    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::create_dir_all(config_dir);
        let temp = path.with_extension("json.tmp");
        if std::fs::write(&temp, json).is_ok() {
            let _ = std::fs::rename(&temp, &path);
        }
    }
}

fn fresh_cached_cli_models(config_dir: &Path, provider: AiProvider) -> Option<Vec<CliModel>> {
    let now = unix_secs();
    if let Ok(cache) = cli_model_cache().lock() {
        if let Some(entry) = cache.get(&provider) {
            if now.saturating_sub(entry.fetched_at) < CLI_MODEL_CACHE_SECS
                && !entry.models.is_empty()
            {
                return Some(entry.models.clone());
            }
        }
    }
    if let Some(entry) = read_disk_cache(config_dir, provider) {
        if now.saturating_sub(entry.fetched_at) < CLI_MODEL_CACHE_SECS && !entry.models.is_empty() {
            if let Ok(mut cache) = cli_model_cache().lock() {
                cache.insert(provider, entry.clone());
            }
            return Some(entry.models);
        }
    }
    None
}

fn store_cached_cli_models(config_dir: &Path, provider: AiProvider, models: &[CliModel]) {
    let entry = CachedCliModels {
        fetched_at: unix_secs(),
        models: models.to_vec(),
    };
    if let Ok(mut cache) = cli_model_cache().lock() {
        cache.insert(provider, entry.clone());
    }
    write_disk_cache(config_dir, provider, &entry);
}

/// Queries the selectable models and their supported thinking/effort modes
/// directly from the local CLI, cached for two hours (in memory and on disk).
pub fn cli_models(
    config_dir: &Path,
    provider: AiProvider,
    refresh: bool,
) -> Result<Vec<CliModel>, String> {
    if provider == AiProvider::OpenRouter {
        return Ok(Vec::new());
    }
    if !refresh {
        if let Some(cached) = fresh_cached_cli_models(config_dir, provider) {
            return Ok(cached);
        }
    }

    let Some(binary) = cli_binary_name(provider) else {
        return Ok(Vec::new());
    };
    let Some(bin) = find_binary(binary) else {
        return Ok(Vec::new());
    };

    let models = match provider {
        AiProvider::OpenRouter => Vec::new(),
        AiProvider::Claude => fetch_claude_models(&bin)?,
        AiProvider::Codex => fetch_codex_models(&bin)?,
        AiProvider::Antigravity => fetch_antigravity_models(&bin)?,
    };

    if !models.is_empty() {
        store_cached_cli_models(config_dir, provider, &models);
    }
    Ok(models)
}

/// Runs a short-lived JSON-line handshake against a CLI (such as Claude's
/// stream-json `initialize` or Codex's `app-server` `model/list`), reading
/// stdout line-by-line until `extract` returns `Some(T)` or `timeout` elapses.
fn query_json_rpc_lines<T, F>(
    mut cmd: Command,
    requests: &[&str],
    timeout: Duration,
    mut extract: F,
) -> Result<Option<T>, String>
where
    F: FnMut(&serde_json::Value) -> Option<T>,
{
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "CLI stdin unavailable".to_string())?;
    for req in requests {
        stdin
            .write_all(req.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .map_err(|e| e.to_string())?;
    }
    let _ = stdin.flush();

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "CLI stdout unavailable".to_string())?;
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = std::time::Instant::now() + timeout;
    let mut answer = None;
    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(remaining) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    if let Some(found) = extract(&json) {
                        answer = Some(found);
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    Ok(answer)
}

/// Fetches Claude CLI's live model catalogue via the stream-json `initialize`
/// control request, falling back to the standard aliases if the handshake fails.
fn fetch_claude_models(bin: &Path) -> Result<Vec<CliModel>, String> {
    let mut cmd = cli_command(bin);
    cmd.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
        "--setting-sources",
        "",
    ])
    .env_remove("CLAUDECODE")
    .env_remove("CLAUDE_CODE_ENTRYPOINT");

    let init_req = r#"{"type":"control_request","request_id":"1","request":{"subtype":"initialize"}}"#;
    let live = query_json_rpc_lines(cmd, &[init_req], Duration::from_secs(12), |msg| {
        if msg.get("type").and_then(|v| v.as_str()) != Some("control_response") {
            return None;
        }
        let models = msg
            .get("response")?
            .get("response")?
            .get("models")?
            .as_array()?;
        Some(parse_claude_init_models(models))
    })
    .ok()
    .flatten()
    .unwrap_or_default();

    if !live.is_empty() {
        return Ok(live);
    }

    Ok(vec![
        CliModel {
            id: "haiku".to_string(),
            name: "Claude Haiku".to_string(),
            note: "Fastest for quick answers".to_string(),
            effort_levels: default_effort_levels(),
            default_effort: Some("low".to_string()),
            effort_models: HashMap::new(),
        },
        CliModel {
            id: "sonnet".to_string(),
            name: "Claude Sonnet".to_string(),
            note: "Balanced speed and quality".to_string(),
            effort_levels: default_effort_levels(),
            default_effort: None,
            effort_models: HashMap::new(),
        },
        CliModel {
            id: "opus".to_string(),
            name: "Claude Opus".to_string(),
            note: "Most capable".to_string(),
            effort_levels: default_effort_levels(),
            default_effort: None,
            effort_models: HashMap::new(),
        },
    ])
}

fn default_effort_levels() -> Vec<String> {
    vec![
        "low".to_string(),
        "medium".to_string(),
        "high".to_string(),
        "xhigh".to_string(),
        "max".to_string(),
    ]
}

fn parse_claude_init_models(items: &[serde_json::Value]) -> Vec<CliModel> {
    let mut models: Vec<CliModel> = items
        .iter()
        .filter_map(|item| {
            let id = item.get("value").and_then(|v| v.as_str())?.trim();
            if id.is_empty() || id == "default" {
                return None;
            }
            let name = item
                .get("displayName")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id);
            let note = item
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();

            let mut effort_levels = Vec::new();
            let supports_off = item
                .get("supportsThinkingOff")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if supports_off {
                effort_levels.push("off".to_string());
            }
            if let Some(levels) = item.get("supportedEffortLevels").and_then(|v| v.as_array()) {
                for level in levels.iter().filter_map(|v| v.as_str()) {
                    let level = level.trim();
                    if !level.is_empty() && !effort_levels.iter().any(|x| x == level) {
                        effort_levels.push(level.to_string());
                    }
                }
            }
            Some(CliModel {
                id: id.to_string(),
                name: name.to_string(),
                note,
                effort_levels,
                default_effort: None,
                effort_models: HashMap::new(),
            })
        })
        .collect();

    // Put the quick aliases (`haiku`, `sonnet`, `opus`, `fable`) at the top in
    // that order so Haiku (the default) is right at the start, followed by any
    // pinned model versions in the order Claude reported them.
    let rank = |id: &str| match id {
        "haiku" => 0,
        "sonnet" => 1,
        "opus" => 2,
        "fable" => 3,
        _ => 4,
    };
    models.sort_by_key(|m| rank(&m.id));
    models
}

/// Fetches Codex's live model catalogue via `codex app-server`'s `model/list`
/// RPC, falling back to `~/.codex/models_cache.json`.
fn fetch_codex_models(bin: &Path) -> Result<Vec<CliModel>, String> {
    let mut cmd = cli_command(bin);
    cmd.arg("app-server");

    let reqs = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"gitnoob","version":"1.0"},"capabilities":{}}}"#,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"model/list","params":{}}"#,
    ];

    let live = query_json_rpc_lines(cmd, &reqs, Duration::from_secs(10), |msg| {
        if msg.get("id").and_then(|v| v.as_u64()) != Some(2) {
            return None;
        }
        let items = msg.get("result")?.get("data")?.as_array()?;
        Some(parse_codex_app_server_models(items))
    })
    .ok()
    .flatten()
    .unwrap_or_default();

    if !live.is_empty() {
        return Ok(live);
    }

    Ok(read_codex_models_cache())
}

fn parse_codex_app_server_models(items: &[serde_json::Value]) -> Vec<CliModel> {
    items
        .iter()
        .filter(|item| !item.get("hidden").and_then(|v| v.as_bool()).unwrap_or(false))
        .filter_map(|item| {
            let id = item
                .get("id")
                .or_else(|| item.get("model"))
                .and_then(|v| v.as_str())?
                .trim();
            if id.is_empty() {
                return None;
            }
            let name = item
                .get("displayName")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id);
            let note = item
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            let effort_levels: Vec<String> = item
                .get("supportedReasoningEfforts")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            e.get("reasoningEffort")
                                .or_else(|| e.get("effort"))
                                .and_then(|v| v.as_str())
                        })
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let default_effort = item
                .get("defaultReasoningEffort")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            Some(CliModel {
                id: id.to_string(),
                name: name.to_string(),
                note,
                effort_levels,
                default_effort,
                effort_models: HashMap::new(),
            })
        })
        .collect()
}

/// Fallback: reads Codex's local cache file (`~/.codex/models_cache.json`).
fn read_codex_models_cache() -> Vec<CliModel> {
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".codex")));
    if let Some(path) = codex_home.map(|dir| dir.join("models_cache.json")) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let parsed = parse_codex_models_cache(&text);
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }
    Vec::new()
}

fn parse_codex_models_cache(text: &str) -> Vec<CliModel> {
    let Ok(body) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(items) = body.get("models").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| {
            item.get("visibility")
                .and_then(|v| v.as_str())
                .map(|v| v != "hide")
                .unwrap_or(true)
        })
        .filter_map(|item| {
            let id = item.get("slug").and_then(|v| v.as_str())?.trim();
            if id.is_empty() {
                return None;
            }
            let name = item
                .get("display_name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id);
            let note = item
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            let effort_levels: Vec<String> = item
                .get("supported_reasoning_levels")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| e.get("effort").and_then(|v| v.as_str()))
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let default_effort = item
                .get("default_reasoning_level")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            Some(CliModel {
                id: id.to_string(),
                name: name.to_string(),
                note,
                effort_levels,
                default_effort,
                effort_models: HashMap::new(),
            })
        })
        .collect()
}

fn fetch_antigravity_models(bin: &Path) -> Result<Vec<CliModel>, String> {
    let mut cmd = cli_command(bin);
    cmd.arg("models");
    let out = run_cli_with_stdin(cmd, None, "agy models", Duration::from_secs(20))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.trim();
        return Err(if msg.is_empty() {
            "agy models failed".to_string()
        } else {
            format!("agy models failed: {msg}")
        });
    }
    Ok(parse_agy_models(&String::from_utf8_lossy(&out.stdout)))
}

fn effort_rank(level: &str) -> usize {
    match level {
        "off" => 0,
        "minimal" => 1,
        "low" => 2,
        "medium" => 3,
        "high" => 4,
        "xhigh" => 5,
        "max" => 6,
        "ultra" => 7,
        _ => 8,
    }
}

/// Splits an Antigravity effort-suffixed model (`gemini-3.8-flash-low`,
/// `Gemini 3.8 Flash (Low)`) into `(base_id, base_name, effort)`.
fn split_agy_effort(id: &str, name: &str) -> Option<(String, String, String)> {
    const SUFFIXES: &[(&str, &str)] = &[
        ("-low", "low"),
        ("-medium", "medium"),
        ("-high", "high"),
        ("-xhigh", "xhigh"),
        ("-max", "max"),
    ];
    for &(suffix, effort) in SUFFIXES {
        if let Some(base_id) = id.strip_suffix(suffix) {
            if base_id.is_empty() {
                continue;
            }
            let base_name = name
                .rsplit_once(" (")
                .filter(|(_, tail)| {
                    tail.trim_end_matches(')')
                        .eq_ignore_ascii_case(effort)
                })
                .map(|(head, _)| head.trim())
                .unwrap_or(name);
            return Some((
                base_id.to_string(),
                base_name.to_string(),
                effort.to_string(),
            ));
        }
    }
    None
}

/// Parses `agy models` output and groups effort-suffixed variants (e.g.
/// `gemini-3.8-flash-low` / `-medium` / `-high`) into one model entry with
/// selectable thinking levels.
fn parse_agy_models(stdout: &str) -> Vec<CliModel> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, CliModel> = HashMap::new();

    for raw_line in stdout.lines() {
        let line = raw_line.rsplit('\r').next().unwrap_or(raw_line).trim();
        if line.is_empty() || line.starts_with("Fetching ") {
            continue;
        }
        // Columns are separated by a tab (piped) or two+ spaces (TTY).
        let split = line
            .split_once('\t')
            .map(|(a, b)| (a.trim(), b.trim()))
            .or_else(|| {
                line.find("  ")
                    .map(|at| (&line[..at], line[at..].trim()))
            });
        let Some((id, name)) = split else {
            continue;
        };
        let id = id.trim();
        if id.is_empty() || id.contains(' ') {
            continue;
        }
        let display = if name.is_empty() { id } else { name };

        if let Some((base_id, base_name, effort)) = split_agy_effort(id, display) {
            let entry = grouped.entry(base_id.clone()).or_insert_with(|| {
                order.push(base_id.clone());
                CliModel {
                    id: base_id,
                    name: base_name,
                    note: String::new(),
                    effort_levels: Vec::new(),
                    default_effort: None,
                    effort_models: HashMap::new(),
                }
            });
            if !entry.effort_levels.iter().any(|e| e == &effort) {
                entry.effort_levels.push(effort.clone());
            }
            entry.effort_models.insert(effort, id.to_string());
        } else if !grouped.contains_key(id) {
            order.push(id.to_string());
            grouped.insert(
                id.to_string(),
                CliModel {
                    id: id.to_string(),
                    name: display.to_string(),
                    note: String::new(),
                    effort_levels: Vec::new(),
                    default_effort: None,
                    effort_models: HashMap::new(),
                },
            );
        }
    }

    order
        .into_iter()
        .filter_map(|id| {
            let mut model = grouped.remove(&id)?;
            model.effort_levels.sort_by_key(|e| effort_rank(e));
            if !model.effort_levels.is_empty() {
                model.default_effort = if model.effort_levels.iter().any(|e| e == "low") {
                    Some("low".to_string())
                } else if model.effort_levels.iter().any(|e| e == "medium") {
                    Some("medium".to_string())
                } else {
                    model.effort_levels.first().cloned()
                };
            }
            Some(model)
        })
        .collect()
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())
}

fn key() -> Result<String, String> {
    config::secret_get(OPENROUTER_KEY)
        .ok_or_else(|| "No OpenRouter API key set — add one in Settings › AI".to_string())
}

/// Fetches OpenRouter's catalogue, cached in memory.
///
/// This endpoint needs no key, so the picker works before the user has pasted
/// one and they can see prices while deciding.
pub async fn models(state: &AppState, refresh: bool) -> Result<Vec<Model>, String> {
    if !refresh {
        if let Some(cached) = state.cached_models(MODEL_CACHE_SECS) {
            return Ok(cached);
        }
    }

    let response = client()?
        .get(format!("{BASE}/models"))
        .send()
        .await
        .map_err(|e| format!("Could not reach OpenRouter: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("OpenRouter returned {}", response.status()));
    }
    let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let items = body
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "OpenRouter sent an unexpected model list".to_string())?;

    let mut models: Vec<Model> = items
        .iter()
        .map(|item| {
            let pricing = item.get("pricing");
            // Prices arrive as per-token decimal strings.
            let price = |field: &str| {
                pricing
                    .and_then(|p| p.get(field))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    * 1_000_000.0
            };
            let modalities = item
                .get("architecture")
                .and_then(|a| a.get("input_modalities"))
                .and_then(|v| v.as_array())
                .map(|list| list.iter().filter_map(|v| v.as_str()).any(|m| m == "image"))
                .unwrap_or(false);

            Model {
                id: item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                context_length: item
                    .get("context_length")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                prompt_price: price("prompt"),
                completion_price: price("completion"),
                description: item
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .chars()
                    .take(400)
                    .collect(),
                multimodal: modalities,
            }
        })
        .filter(|model| !model.id.is_empty())
        .collect();

    models.sort_by(|a, b| a.id.cmp(&b.id));
    state.cache_models(models.clone());
    Ok(models)
}

/// Turns the chosen thinking level into OpenRouter's `reasoning` object.
///
/// OpenRouter takes one shape for every provider: an effort level, which it
/// translates into whatever the model underneath wants, or `enabled: false` to
/// switch thinking off on a model that would otherwise do it. Anything the
/// reasoning tokens are spent on is billed, so "off" is a real choice rather
/// than a placeholder.
fn reasoning_field(level: &str) -> serde_json::Value {
    match level {
        "minimal" | "low" | "medium" | "high" => serde_json::json!({ "effort": level }),
        // "off", and anything unrecognised: no thinking at all.
        _ => serde_json::json!({ "enabled": false }),
    }
}

/// What one chat completion came back with, before anyone reads meaning
/// into it.
///
/// Kept apart from the parsing because every provider on OpenRouter shapes the
/// same answer slightly differently: content as a string or as a list of
/// parts, thinking in a `reasoning` field or inline in `<think>` tags, and an
/// answer that ran out of tokens looking exactly like one that was never
/// written. The caller decides what counts as an answer.
#[derive(Debug, Default, Clone, PartialEq)]
struct Reply {
    /// The message content as sent, indentation and all. Blank when the
    /// model wrote none.
    content: String,
    /// Whatever the model thought before answering, when the provider sends
    /// it back. Some providers put the whole answer here by mistake.
    reasoning: String,
    /// OpenRouter's normalised reason, e.g. `stop` or `length`.
    finish: String,
    /// How many tokens went on thinking, when the provider counts them.
    reasoning_tokens: Option<u64>,
}

impl Reply {
    fn cut_off(&self) -> bool {
        self.finish == "length"
    }

    fn is_blank(&self) -> bool {
        self.content.trim().is_empty()
    }

    /// Why an answer is missing, in words that say what to change.
    fn missing(&self, model: &str) -> String {
        let thought = match self.reasoning_tokens {
            Some(n) if n > 0 => format!(" after {n} reasoning tokens"),
            _ if !self.reasoning.is_empty() => " after thinking".to_string(),
            _ => String::new(),
        };
        if self.cut_off() {
            format!(
                "{model} ran out of tokens{thought} before writing an answer. \
                 Raise Max tokens or turn Thinking off in Settings › AI"
            )
        } else if !self.reasoning.is_empty() {
            format!(
                "{model} put its whole answer in the reasoning field and none in the reply \
                 (finish reason: {}). Try Thinking off, or another model",
                self.finish
            )
        } else {
            format!(
                "{model} returned an empty answer (finish reason: {})",
                if self.finish.is_empty() {
                    "none"
                } else {
                    &self.finish
                }
            )
        }
    }
}

/// Reads one chat completion body into a [`Reply`].
///
/// A 200 is not always an answer: OpenRouter forwards a provider's failure as
/// `choices[0].error` with the status of the gateway, not the provider. That
/// is caught here so it reads as the failure it is rather than as an empty
/// answer.
fn parse_reply(body: &serde_json::Value) -> Result<Reply, String> {
    let choice = body
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| "OpenRouter sent no choices back".to_string())?;

    if let Some(error) = choice.get("error") {
        let detail = error
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("no detail");
        return Err(format!("The provider failed: {detail}"));
    }

    let message = choice.get("message");
    let text = |field: &str| -> String {
        message
            .and_then(|m| m.get(field))
            .map(text_of)
            .unwrap_or_default()
    };
    let (content, inline_thinking) = strip_think_tags(&text("content"));
    let mut reasoning = text("reasoning");
    if reasoning.is_empty() {
        // DeepSeek's own name for the same field, which some hosts keep.
        reasoning = text("reasoning_content");
    }
    if reasoning.is_empty() {
        reasoning = inline_thinking;
    }

    let finish = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let reasoning_tokens = body
        .get("usage")
        .and_then(|u| u.get("completion_tokens_details"))
        .and_then(|d| d.get("reasoning_tokens"))
        .and_then(|v| v.as_u64());

    Ok(Reply {
        content,
        reasoning: reasoning.trim().to_string(),
        finish,
        reasoning_tokens,
    })
}

/// The text of a content field, whichever of the two shapes it takes.
///
/// A plain string most of the time; a list of `{type: "text", text}` parts
/// from providers that speak the multimodal dialect even for text.
fn text_of(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(|t| t.as_str())
                    .or_else(|| part.as_str())
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Splits `<think>…</think>` blocks out of an answer.
///
/// Returns the answer without them, and the thinking on its own. A hosted
/// model whose chat template leaks its thinking into the content is the usual
/// source; an unclosed `<think>` means the model never got to the answer, so
/// everything after it is thinking too.
fn strip_think_tags(text: &str) -> (String, String) {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";
    let mut answer = String::new();
    let mut thinking = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        answer.push_str(&rest[..start]);
        let inner = &rest[start + OPEN.len()..];
        match inner.find(CLOSE) {
            Some(end) => {
                thinking.push_str(&inner[..end]);
                rest = &inner[end + CLOSE.len()..];
            }
            None => {
                thinking.push_str(inner);
                rest = "";
            }
        }
    }
    answer.push_str(rest);
    (answer, thinking)
}

/// One chat completion round trip, returned as the model sent it.
///
/// `floor` is the fewest completion tokens the request may be allowed. The
/// settings cap is a cost control for commit messages; an answer that is a
/// block of code the user already has on screen needs room for that block,
/// and capping it below that room can only produce a cut-off answer.
///
/// An empty reply that was not cut off is asked for once more before it is
/// reported: providers behind OpenRouter do return nothing now and then, and
/// the second ask almost always lands.
async fn complete_raw(
    state: &AppState,
    system: &str,
    user: &str,
    floor: u32,
) -> Result<(Reply, String), String> {
    let ai = state.config().global.ai;
    match ai.provider {
        AiProvider::OpenRouter => complete_openrouter(&ai, system, user, floor).await,
        AiProvider::Claude | AiProvider::Codex | AiProvider::Antigravity => {
            let provider = ai.provider;
            let model = ai.active_model();
            let effort = ai.active_effort();
            let config_dir = state.config_dir();
            let system = system.to_string();
            let user = user.to_string();
            tauri::async_runtime::spawn_blocking(move || {
                complete_cli(&config_dir, provider, model, effort, &system, &user)
            })
            .await
            .map_err(|e| e.to_string())?
        }
    }
}

async fn complete_openrouter(
    ai: &config::Ai,
    system: &str,
    user: &str,
    floor: u32,
) -> Result<(Reply, String), String> {
    let model = ai
        .model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| "No model chosen — pick one in Settings › AI".to_string())?;
    let key = key()?;
    let max_tokens = ai.max_tokens.max(floor);
    let mut reasoning = Some(reasoning_field(&ai.reasoning));
    let client = client()?;

    let mut reply = Reply::default();
    // Two goes at most: a retry for a blank reply, or one for a thinking
    // switch the model refused, and never a third.
    for attempt in 0..2 {
        let mut request = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user }
            ]
        });
        if let Some(reasoning) = &reasoning {
            request["reasoning"] = reasoning.clone();
        }
        let response = client
            .post(format!("{BASE}/chat/completions"))
            .bearer_auth(&key)
            // OpenRouter uses these for its own attribution listings.
            .header("HTTP-Referer", "https://github.com/gitnoob")
            .header("X-Title", "gitnoob")
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("Could not reach OpenRouter: {e}"))?;

        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("OpenRouter sent something unreadable: {e}"))?;

        if !status.is_success() {
            let detail = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("no detail");
            // Some models think whether asked to or not, and OpenRouter
            // refuses the request rather than ignoring the switch. The
            // setting is a cost control, not a requirement, so the same
            // request goes again with the model's own default.
            if reasoning.is_some() && cannot_stop_thinking(detail) {
                reasoning = None;
                continue;
            }
            return Err(format!("{status}: {detail}"));
        }

        reply = parse_reply(&body)?;
        let worth_retrying = reply.is_blank() && reply.reasoning.is_empty() && !reply.cut_off();
        if !worth_retrying || attempt == 1 {
            break;
        }
    }
    Ok((reply, model))
}

/// Runs a local AI CLI (`claude`, `codex` or `agy`) non-interactively and wraps
/// its stdout into a [`Reply`].
fn complete_cli(
    config_dir: &Path,
    provider: AiProvider,
    model: Option<String>,
    effort: Option<String>,
    system: &str,
    user: &str,
) -> Result<(Reply, String), String> {
    let binary = cli_binary_name(provider)
        .ok_or_else(|| "Unsupported CLI provider".to_string())?;
    let bin_path = find_binary(binary).ok_or_else(|| {
        format!(
            "{} (`{binary}`) was not found on your PATH — install it or pick another provider in Settings › AI",
            cli_display_name(provider)
        )
    })?;
    let label = match &model {
        Some(m) => format!("{} ({m})", cli_display_name(provider)),
        None => cli_display_name(provider).to_string(),
    };

    let raw = match provider {
        AiProvider::Claude => run_claude(
            &bin_path,
            model.as_deref().unwrap_or("haiku"),
            effort.as_deref(),
            system,
            user,
        )?,
        AiProvider::Codex => {
            run_codex(&bin_path, model.as_deref(), effort.as_deref(), system, user)?
        }
        AiProvider::Antigravity => run_antigravity(
            config_dir,
            &bin_path,
            model.as_deref(),
            effort.as_deref(),
            system,
            user,
        )?,
        AiProvider::OpenRouter => unreachable!(),
    };

    let (content, reasoning) = strip_think_tags(&raw);
    Ok((
        Reply {
            content,
            reasoning: reasoning.trim().to_string(),
            finish: "stop".to_string(),
            reasoning_tokens: None,
        },
        label,
    ))
}

/// Runs a child command with optional stdin text and a hard timeout so a
/// stalled CLI (waiting on auth or network) never hangs the app indefinitely.
fn run_cli_with_stdin(
    mut cmd: Command,
    stdin_text: Option<String>,
    name: &str,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    use std::io::Read;

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin_text.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start {name}: {e}"))?;

    let writer = if let (Some(text), Some(mut stdin)) = (stdin_text, child.stdin.take()) {
        Some(std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        }))
    } else {
        None
    };

    let stdout_reader = child.stdout.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    });
    let stderr_reader = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    });

    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    if let Some(handle) = writer {
                        let _ = handle.join();
                    }
                    if let Some(handle) = stdout_reader {
                        let _ = handle.join();
                    }
                    if let Some(handle) = stderr_reader {
                        let _ = handle.join();
                    }
                    return Err(format!(
                        "{name} timed out after {} seconds",
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(format!("{name} did not finish: {e}")),
        }
    };

    if let Some(handle) = writer {
        let _ = handle.join();
    }
    let stdout = stdout_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

const CLI_COMPLETE_TIMEOUT: Duration = Duration::from_secs(120);

fn run_claude(
    bin: &Path,
    model: &str,
    effort: Option<&str>,
    system: &str,
    user: &str,
) -> Result<String, String> {
    let mut cmd = cli_command(bin);
    cmd.args([
        "-p",
        "--model",
        model,
        "--system-prompt",
        system,
        "--tools",
        "",
        "--no-session-persistence",
        "--disable-slash-commands",
        "--output-format",
        "text",
    ]);
    if let Some(level) = effort {
        if level == "off" {
            cmd.args(["--settings", r#"{"alwaysThinkingEnabled":false}"#]);
        } else {
            cmd.args(["--effort", level]);
        }
    }
    // Clear nested-session guards in case gitnoob itself was started from
    // inside a Claude Code terminal.
    cmd.env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT");

    let out = run_cli_with_stdin(cmd, Some(user.to_string()), "claude", CLI_COMPLETE_TIMEOUT)?;
    if !out.status.success() {
        return Err(cli_error("Claude CLI", &out));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Deletes a temporary file when dropped, even on early return.
struct TempFileGuard(PathBuf);
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run_codex(
    bin: &Path,
    model: Option<&str>,
    effort: Option<&str>,
    system: &str,
    user: &str,
) -> Result<String, String> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let out_file = std::env::temp_dir().join(format!(
        "gitnoob-codex-{}-{seq}.txt",
        std::process::id()
    ));
    let _guard = TempFileGuard(out_file.clone());

    let mut cmd = cli_command(bin);
    cmd.args([
        "exec",
        "--ephemeral",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--color",
        "never",
        "-o",
    ])
    .arg(&out_file);
    if let Some(model) = model {
        cmd.args(["-m", model]);
    }
    if let Some(level) = effort {
        cmd.args(["-c", &format!("model_reasoning_effort={level}")]);
    }
    cmd.arg("-");

    let combined = format!("{system}\n\n---\n\n{user}");
    let out = run_cli_with_stdin(cmd, Some(combined), "codex", CLI_COMPLETE_TIMEOUT)?;
    if !out.status.success() {
        let detail = extract_codex_error(&String::from_utf8_lossy(&out.stderr))
            .unwrap_or_else(|| cli_error("Codex CLI", &out));
        return Err(detail);
    }
    if let Ok(text) = std::fs::read_to_string(&out_file) {
        if !text.trim().is_empty() {
            return Ok(text);
        }
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Resolves a grouped Antigravity model id + effort (e.g. `gemini-3.8-flash` +
/// `low`) to the exact model slug `agy --model` expects (`gemini-3.8-flash-low`),
/// returning `(resolved_model, pass_effort_flag)`.
fn resolve_antigravity_args(
    models: &[CliModel],
    model: Option<&str>,
    effort: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(model_id) = model else {
        return (None, effort.map(str::to_string));
    };
    let entry = models
        .iter()
        .find(|m| m.id == model_id || m.effort_models.values().any(|v| v == model_id));
    if let Some(entry) = entry {
        if !entry.effort_models.is_empty() {
            let chosen = effort
                .and_then(|e| entry.effort_models.get(e))
                .or_else(|| {
                    // If the saved id is already an exact variant slug and no
                    // explicit effort override was chosen, keep that slug.
                    entry
                        .effort_models
                        .values()
                        .find(|v| v.as_str() == model_id)
                })
                .or_else(|| {
                    entry
                        .default_effort
                        .as_deref()
                        .and_then(|e| entry.effort_models.get(e))
                })
                .or_else(|| {
                    entry
                        .effort_levels
                        .first()
                        .and_then(|e| entry.effort_models.get(e))
                });
            if let Some(slug) = chosen {
                return (Some(slug.clone()), None);
            }
        }
    }
    (Some(model_id.to_string()), effort.map(str::to_string))
}

fn run_antigravity(
    config_dir: &Path,
    bin: &Path,
    model: Option<&str>,
    effort: Option<&str>,
    system: &str,
    user: &str,
) -> Result<String, String> {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut combined = format!("{system}\n\n---\n\n{user}");
    // Windows command lines cap at 32,767 UTF-16 code units; keep headroom.
    #[cfg(windows)]
    if combined.len() > 26_000 {
        let cut = combined.floor_char_boundary_compat(26_000);
        combined.truncate(cut);
        combined.push_str("\n… (truncated for command-line limit)");
    }

    let known = fresh_cached_cli_models(config_dir, AiProvider::Antigravity)
        .or_else(|| cli_models(config_dir, AiProvider::Antigravity, false).ok())
        .unwrap_or_default();
    let (resolved_model, effort_flag) = resolve_antigravity_args(&known, model, effort);

    let mut cmd = cli_command(bin);
    if let Some(model) = resolved_model {
        cmd.args(["--model", &model]);
    }
    if let Some(level) = effort_flag {
        cmd.args(["--effort", &level]);
    }
    cmd.args(["--disable-slash-commands", "--output-format", "text", "-p"])
        .arg(&combined);

    let out = run_cli_with_stdin(cmd, None, "agy", CLI_COMPLETE_TIMEOUT)?;
    if !out.status.success() {
        return Err(cli_error("Antigravity CLI", &out));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(windows)]
trait FloorCharBoundary {
    fn floor_char_boundary_compat(&self, index: usize) -> usize;
}
#[cfg(windows)]
impl FloorCharBoundary for str {
    fn floor_char_boundary_compat(&self, index: usize) -> usize {
        let mut i = index.min(self.len());
        while i > 0 && !self.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

fn cli_error(name: &str, out: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let raw = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        return format!("{name} exited with status {}", out.status);
    };
    let short: String = raw.chars().take(400).collect();
    format!("{name} failed: {short}")
}

/// Pulls the actual `ERROR: …` line out of `codex exec`'s verbose stderr
/// banner so the toast names the failure instead of the session header.
fn extract_codex_error(stderr: &str) -> Option<String> {
    let mut found: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("ERROR:").map(str::trim))
        .filter(|line| !line.is_empty())
        .collect();
    found.dedup();
    if found.is_empty() {
        None
    } else {
        Some(format!("Codex CLI failed: {}", found.join(" — ")))
    }
}

/// Whether an error says the model will think regardless of the setting.
///
/// OpenRouter's wording as of this writing: "Reasoning is mandatory for this
/// endpoint and cannot be disabled." Matched loosely, since the sentence is
/// theirs to change.
fn cannot_stop_thinking(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("reasoning")
        && (lower.contains("mandatory") || lower.contains("cannot be disabled"))
}

/// One chat completion round trip, as the text the model wrote.
async fn complete(state: &AppState, system: &str, user: String) -> Result<String, String> {
    let (reply, model) = complete_raw(state, system, &user, 0).await?;
    if reply.is_blank() {
        return Err(reply.missing(&model));
    }
    Ok(reply.content.trim().to_string())
}

/// Writes a commit message from what is staged.
///
/// `amend` describes the commit an amend will make, which is HEAD's own change
/// with whatever is staged on top: describing the staged part alone wrote a
/// message for a fraction of the commit and replaced the one it had.
///
/// `draft` is whatever is in the box already. It is handed over to be
/// finished rather than thrown away — a ticket prefix, half a sentence, the
/// message being amended — so pressing the button never costs what was typed.
pub async fn commit_message(
    state: &AppState,
    amend: bool,
    draft: Option<String>,
) -> Result<CommitMessage, String> {
    let root = state.path()?;
    // Gather everything before the await; no git handles cross it.
    let base = if amend {
        // HEAD's parent, or the empty tree for a first commit.
        Some(
            git_cmd::run(&root, &["rev-parse", "--verify", "--quiet", "HEAD^"])
                .ok()
                .filter(|out| out.ok)
                .map(|out| out.stdout.trim().to_string())
                .filter(|oid| !oid.is_empty())
                .unwrap_or_else(|| EMPTY_TREE.to_string()),
        )
    } else {
        None
    };
    // `--` after the base, here and below: a ref is never read as a path, nor
    // a path as a ref.
    let mut args = crate::work::plain_diff();
    args.push("--cached");
    if let Some(base) = &base {
        args.push(base);
    }
    args.push("--");
    let diff = git_cmd::run_checked(&root, &args)?;
    if diff.trim().is_empty() {
        return Err(if amend {
            "The amended commit would change nothing, so there is nothing to describe".to_string()
        } else {
            "Nothing is staged, so there is nothing to describe".to_string()
        });
    }
    let mut numbers = crate::work::plain_diff();
    numbers.extend(["--cached", "--numstat"]);
    if let Some(base) = &base {
        numbers.push(base);
    }
    numbers.push("--");
    let numstat = git_cmd::run_checked(&root, &numbers).unwrap_or_default();
    let mut names = crate::work::plain_diff();
    names.extend(["--cached", "--name-status"]);
    if let Some(base) = &base {
        names.push(base);
    }
    names.push("--");
    let files = git_cmd::run_checked(&root, &names)?;
    // The commit being amended is the one being described, so its own subject
    // is not "recent history" — it is in the draft, if anywhere.
    let recent = if amend {
        git_cmd::run_checked(&root, &["log", "-8", "--format=%s", "HEAD^", "--"])
            .unwrap_or_default()
    } else {
        git_cmd::run_checked(&root, &["log", "-8", "--format=%s"]).unwrap_or_default()
    };

    let mut context = String::new();
    if let Some(branch) = crate::journal::current_branch(state) {
        let ticket_prefix = state.config().global.commit.ticket_prefix;
        context.push_str(&branch_context(&branch, ticket_prefix));
    }
    if let Some(draft) = draft.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        context.push_str(&format!(
            "The developer has already started this message. Keep what they wrote — its \
             wording, any ticket prefix, any facts in it — and complete or tighten it \
             rather than replacing it:\n{draft}\n\n"
        ));
    }

    let fitted = fit_diff(&diff, &numstat, MAX_DIFF_CHARS);
    let label = if amend {
        "Diff of the amended commit"
    } else {
        "Staged diff"
    };
    describe_change(state, label, &files, &fitted, &recent, &context).await
}

/// What the model is told about the branch.
///
/// The ticket prefix follows the same switch as the commit box. With it off,
/// the model is told to leave the branch name out, because left to itself it
/// copies a ticket id from the branch into the summary anyway.
fn branch_context(branch: &str, ticket_prefix: bool) -> String {
    if ticket_prefix {
        format!(
            "Branch: {branch}\nIf the branch name carries a ticket id such as ABC-123, the \
             team expects it at the start of the summary as `ABC-123: `.\n\n"
        )
    } else {
        format!(
            "Branch: {branch}\nDo not put the branch name or a ticket id from it in the \
             message.\n\n"
        )
    }
}

/// Lockfiles: thousands of lines a commit message has nothing to say about.
const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "deno.lock",
    "Cargo.lock",
    "composer.lock",
    "Gemfile.lock",
    "poetry.lock",
    "Pipfile.lock",
    "uv.lock",
    "go.sum",
    "flake.lock",
    "mix.lock",
    "pubspec.lock",
    "Podfile.lock",
    "packages.lock.json",
];

/// Why a file's diff is left out of what the model reads, if it is.
fn left_out(path: &str, chunk: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if LOCKFILES.contains(&name) {
        return Some("lockfile");
    }
    if chunk.contains("\nBinary files ") || chunk.contains("\nGIT binary patch") {
        return Some("binary");
    }
    let lower = name.to_ascii_lowercase();
    const MADE: [&str; 9] = [
        ".min.js",
        ".min.css",
        ".js.map",
        ".css.map",
        ".pb.go",
        "_pb2.py",
        ".g.dart",
        ".freezed.dart",
        ".snap",
    ];
    if MADE.iter().any(|tail| lower.ends_with(tail)) || lower.contains(".generated.") {
        return Some("generated");
    }
    let mut body = chunk.lines().skip_while(|line| !line.starts_with("@@"));
    if body
        .by_ref()
        .take(12)
        .any(|line| line.contains("@generated") || line.contains("DO NOT EDIT"))
    {
        return Some("generated");
    }
    if chunk.lines().any(|line| line.len() > 1_000) {
        return Some("minified");
    }
    None
}

/// The diff cut down to `budget` characters so that every file is in it.
///
/// It used to be the first 48,000 characters, which is the first few files —
/// a lockfile that sorted early could be the whole of what the model saw.
/// Now lockfiles, binaries and generated or minified files are one line each,
/// and what is left is shared out: small files whole, and the big ones an
/// equal slice of the rest, so a large rewrite cannot crowd out the one-line
/// fix next to it.
fn fit_diff(diff: &str, numstat: &str, budget: usize) -> String {
    // `added  removed  path` per file, `-` for a binary.
    let counts: std::collections::HashMap<&str, (&str, &str)> = numstat
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let added = parts.next()?;
            let removed = parts.next()?;
            Some((parts.next()?, (added, removed)))
        })
        .collect();

    let mut chunks: Vec<&str> = Vec::new();
    let mut start = 0;
    for (at, _) in diff.match_indices("diff --git ") {
        if at > 0 && diff.as_bytes()[at - 1] == b'\n' && at > start {
            chunks.push(&diff[start..at]);
            start = at;
        }
    }
    chunks.push(&diff[start..]);

    let mut out: Vec<String> = Vec::with_capacity(chunks.len());
    let mut kept: Vec<(usize, &str)> = Vec::new();
    for chunk in &chunks {
        let path = chunk_path(chunk);
        match left_out(&path, chunk) {
            Some(why) => {
                let (added, removed) = counts.get(path.as_str()).copied().unwrap_or(("?", "?"));
                out.push(format!(
                    "{path}: {why}, +{added} −{removed} lines (contents left out)\n"
                ));
            }
            None => {
                kept.push((out.len(), chunk));
                out.push(String::new());
            }
        }
    }

    let spent: usize = out.iter().map(|line| line.chars().count()).sum();
    let mut left = budget.saturating_sub(spent);
    // Smallest first, so whatever a small file does not use is there for the
    // larger ones after it.
    let mut order: Vec<usize> = (0..kept.len()).collect();
    order.sort_by_key(|&i| kept[i].1.len());
    for (taken, &i) in order.iter().enumerate() {
        let (slot, chunk) = kept[i];
        let share = left / (order.len() - taken);
        let fitted = cut_to(chunk, share);
        left = left.saturating_sub(fitted.chars().count());
        out[slot] = fitted;
    }
    out.concat()
}

/// The path a file's piece of a diff is about, from its `+++` line (or `---`
/// for a deletion).
fn chunk_path(chunk: &str) -> String {
    let named = |prefix: &str, side: &str| {
        chunk
            .lines()
            .find(|line| line.starts_with(prefix))
            .map(|line| line[prefix.len()..].trim_matches('"').to_string())
            .filter(|name| name != "/dev/null")
            .map(|name| name.strip_prefix(side).map(str::to_string).unwrap_or(name))
    };
    named("+++ ", "b/")
        .or_else(|| named("--- ", "a/"))
        .or_else(|| {
            let first = chunk.lines().next()?;
            Some(first.rsplit(" b/").next()?.to_string())
        })
        .unwrap_or_default()
}

/// A file's piece of the diff in at most `limit` characters, cut at a line and
/// saying how much is missing.
fn cut_to(chunk: &str, limit: usize) -> String {
    if chunk.chars().count() <= limit {
        return chunk.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    let mut lines = chunk.lines();
    for line in lines.by_ref() {
        let cost = line.chars().count() + 1;
        if used + cost > limit {
            // That line is the first one left out.
            let rest = 1 + lines.count();
            out.push_str(&format!("… {rest} more lines of this file left out\n"));
            return out;
        }
        out.push_str(line);
        out.push('\n');
        used += cost;
    }
    out
}

/// Writes a commit message from a commit that already exists.
///
/// The same job as [`commit_message`] with a different subject: the message
/// being replaced is the one already on the commit, so what the model is shown
/// is that commit's own diff rather than the index. Its own subject is left
/// out of the recent ones deliberately — the point of asking is that the
/// message it has is not the wanted one.
pub async fn commit_message_for(state: &AppState, oid: &str) -> Result<CommitMessage, String> {
    let root = state.path()?;
    let oid = checked_ref(oid)?;
    let diff = git_cmd::run_checked(
        &root,
        &[
            "show",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--unified=3",
            "--format=",
            &oid,
            "--",
        ],
    )?;
    if diff.trim().is_empty() {
        return Err("That commit changed nothing, so there is nothing to describe".to_string());
    }
    let files = git_cmd::run_checked(&root, &["show", "--name-status", "--format=", &oid, "--"])?;
    // A root commit has no parent to walk back from; tone is a nicety anyway.
    let recent = git_cmd::run_checked(
        &root,
        &["log", "-8", "--format=%s", &format!("{oid}^"), "--"],
    )
    .unwrap_or_default();

    let numstat = git_cmd::run_checked(&root, &["show", "--numstat", "--format=", &oid, "--"])
        .unwrap_or_default();
    let fitted = fit_diff(&diff, &numstat, MAX_DIFF_CHARS);
    describe_change(state, "Diff", &files, &fitted, &recent, "").await
}

/// Writes one message for the commits a squash would fold.
///
/// The dialog starts from git's own join — every message one after another —
/// which says what each commit did and never what the fold as a whole did. So
/// the model is shown the range as a single diff, the way the folded commit
/// will read afterwards, together with the messages being replaced: they carry
/// the reasoning that the diff alone does not.
pub async fn squash_message(state: &AppState, oids: &[String]) -> Result<CommitMessage, String> {
    let root = state.path()?;
    let mut wanted: Vec<String> = Vec::new();
    for oid in oids {
        let oid = checked_ref(oid)?;
        if !wanted.contains(&oid) {
            wanted.push(oid);
        }
    }
    if wanted.len() < 2 {
        return Err("Squashing folds commits together, so it takes at least two.".to_string());
    }

    let ordered = crate::work::in_history_order(state, &wanted)?;
    let oldest = ordered.first().expect("checked non-empty above");
    let newest = ordered.last().expect("checked non-empty above");
    // The parent of the oldest, or the empty tree when the run starts at the
    // root: a first commit has nothing behind it to diff against.
    let base = git_cmd::run(&root, &["rev-parse", "--verify", &format!("{oldest}^")])
        .ok()
        .filter(|out| out.ok)
        .map(|out| out.stdout.trim().to_string())
        .filter(|oid| !oid.is_empty())
        .unwrap_or_else(|| EMPTY_TREE.to_string());

    let mut args = crate::work::plain_diff();
    args.extend([base.as_str(), newest.as_str(), "--"]);
    let diff = git_cmd::run_checked(&root, &args)?;
    if diff.trim().is_empty() {
        return Err(
            "Those commits change nothing together, so there is nothing to describe.".to_string(),
        );
    }
    let files = git_cmd::run_checked(&root, &["diff", "--name-status", &base, newest, "--"])?;
    // What the commits say about themselves, which is the part of the join
    // worth keeping. Subjects from before the run set the tone.
    let folded = git_cmd::run_checked(
        &root,
        &[
            "log",
            "--reverse",
            "--format=%B",
            &format!("{base}..{newest}"),
            "--",
        ],
    )
    .unwrap_or_default();
    let recent =
        git_cmd::run_checked(&root, &["log", "-8", "--format=%s", &base, "--"]).unwrap_or_default();

    let truncated = diff.chars().count() > MAX_DIFF_CHARS;
    let short: String = diff.chars().take(MAX_DIFF_CHARS).collect();
    let system = commit_prompt(&state.config().global.ai);
    let prompt = format!(
        "These commits are being folded into one. Their own messages:\n{folded}\n\n         Files changed:\n{files}\n\nRecent commit subjects in this repository:\n{recent}\n\n         The folded diff{}:\n{short}",
        if truncated { " (truncated)" } else { "" }
    );

    let answer = complete(state, &system, prompt).await?;
    Ok(split_message(&answer))
}

/// Asks the model for a subject and body describing one set of changes.
async fn describe_change(
    state: &AppState,
    label: &str,
    files: &str,
    diff: &str,
    recent: &str,
    context: &str,
) -> Result<CommitMessage, String> {
    // Already fitted to the budget by the caller; this is only the backstop.
    let truncated = diff.chars().count() > MAX_DIFF_CHARS;
    let diff: String = diff.chars().take(MAX_DIFF_CHARS).collect();
    // Whatever the user has in the settings box, which is the default until
    // they change it.
    let system = commit_prompt(&state.config().global.ai);

    let prompt = format!(
        "{context}Files changed:\n{files}\n\nRecent commit subjects in this repository, \
         for the vocabulary this codebase uses:\n{recent}\n\n{label}{}:\n{diff}",
        if truncated { " (truncated)" } else { "" }
    );

    let answer = complete(state, &system, prompt).await?;
    Ok(split_message(&answer))
}

/// The instructions a commit message is written under.
///
/// An empty box means the default rather than no instructions at all: a model
/// told nothing writes a paragraph of prose and calls it a commit message.
pub fn commit_prompt(ai: &config::Ai) -> String {
    match ai.commit_prompt.as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => text.to_string(),
        _ => DEFAULT_COMMIT_PROMPT.to_string(),
    }
}

/// Writes the title and description of a review from the commits behind it.
///
/// The commits are the argument the branch is making, in the author's own
/// words; the diffstat is there so a subject like "fix the thing" can still be
/// placed in the right part of the codebase.
pub async fn review_message(
    state: &AppState,
    source: String,
    target: String,
) -> Result<CommitMessage, String> {
    let root = state.path()?;
    // The target as the forge has it: a local `main` that has not been pulled
    // for a week would put everybody else's week into "what this branch
    // does". The source as it is here, or on a remote when it is not.
    let preferred = crate::forge::known_target(state).map(|chosen| chosen.remote);
    let target = resolve_target(&root, &checked_ref(&target)?, preferred.as_deref())?;
    let source = resolve_ref(&root, &checked_ref(&source)?)?;
    if source == target {
        return Err("The two branches are the same".to_string());
    }

    let range = format!("{target}..{source}");
    let log = git_cmd::run_checked(
        &root,
        &[
            "log",
            "--no-color",
            "--max-count=40",
            "--format=%s%n%b%n---",
            &range,
            "--",
        ],
    )?;
    if log.trim().is_empty() {
        return Err(format!(
            "{source} has no commits that {target} does not already have"
        ));
    }
    // Three dots: what the branch did, not what happened on the target while
    // it was away.
    let stat = git_cmd::run_checked(
        &root,
        &[
            "diff",
            "--stat",
            "--stat-width=200",
            &format!("{target}...{source}"),
            "--",
        ],
    )
    .unwrap_or_default();

    let truncated = log.chars().count() > MAX_DIFF_CHARS;
    let log: String = log.chars().take(MAX_DIFF_CHARS).collect();
    let stat: String = stat.chars().take(8_000).collect();

    let prompt = format!(
        "Branch: {source}\nMerging into: {target}\n\n\
         Commits on the branch, newest first, each ending with `---`{}:\n{log}\n\n\
         Files changed:\n{stat}",
        if truncated { " (truncated)" } else { "" }
    );

    let answer = complete(state, REVIEW_SYSTEM, prompt).await?;
    Ok(split_message(&answer))
}

/// Refuses a branch name git would read as an option.
///
/// The arguments never reach a shell, so this is not about quoting: a branch
/// called `--all` would simply be understood as a flag.
fn checked_ref(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("No branch given".to_string());
    }
    if name.starts_with('-') {
        return Err(format!("{name} is not a usable branch name"));
    }
    Ok(name.to_string())
}

/// Finds the ref a branch name means here.
///
/// The target of a review is often a branch nobody has checked out — `develop`
/// exists on the forge and locally only as `origin/develop` — so a plain name
/// that resolves to nothing is tried again against each remote.
pub fn resolve_ref(root: &std::path::Path, name: &str) -> Result<String, String> {
    let exists = |candidate: &str| {
        git_cmd::run_checked(
            root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{candidate}^{{commit}}"),
            ],
        )
        .is_ok()
    };
    if exists(name) {
        return Ok(name.to_string());
    }
    let remotes = git_cmd::run_checked(root, &["remote"]).unwrap_or_default();
    for remote in remotes.lines().map(str::trim).filter(|r| !r.is_empty()) {
        let candidate = format!("{remote}/{name}");
        if exists(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "{name} is not a branch this repository knows about"
    ))
}

/// Finds the ref a review's target means: the remote-tracking copy first —
/// the reviews' own remote, then origin, then any — and the local branch only
/// when no remote has it. A name that already says which remote is taken as
/// it is.
pub fn resolve_target(
    root: &std::path::Path,
    name: &str,
    preferred: Option<&str>,
) -> Result<String, String> {
    let exists = |candidate: &str| {
        git_cmd::run_checked(
            root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{candidate}^{{commit}}"),
            ],
        )
        .is_ok()
    };
    let remotes: Vec<String> = git_cmd::run_checked(root, &["remote"])
        .unwrap_or_default()
        .lines()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    if remotes.iter().any(|r| name.starts_with(&format!("{r}/"))) && exists(name) {
        return Ok(name.to_string());
    }
    let mut order: Vec<&str> = Vec::new();
    order.extend(preferred.filter(|p| remotes.iter().any(|r| r == p)));
    if remotes.iter().any(|r| r == "origin") {
        order.push("origin");
    }
    order.extend(remotes.iter().map(String::as_str));
    for remote in order {
        let candidate = format!("refs/remotes/{remote}/{name}");
        if exists(&candidate) {
            return Ok(format!("{remote}/{name}"));
        }
    }
    resolve_ref(root, name)
}

/// Resolves one conflict region.
///
/// The model gets the merge base as well as both sides, which is what lets it
/// tell an addition from a deletion instead of guessing.
pub async fn resolve_conflict(
    state: &AppState,
    path: String,
    index: usize,
) -> Result<Vec<String>, String> {
    let file = conflict::read(state, &path)?;
    let mut ours = Vec::new();
    let mut base = Vec::new();
    let mut theirs = Vec::new();
    let mut has_base = false;
    let mut before: Vec<String> = Vec::new();
    let mut after: Vec<String> = Vec::new();
    let mut seen = false;

    // Walk the blocks once, keeping the context on either side of the region.
    for block in &file.blocks {
        match block {
            conflict::Block::Context { lines } => {
                if seen {
                    if after.len() < 15 {
                        after.extend(lines.iter().take(15 - after.len()).cloned());
                    }
                } else {
                    before = lines.iter().rev().take(15).rev().cloned().collect();
                }
            }
            conflict::Block::Conflict {
                index: i,
                ours: o,
                base: b,
                theirs: t,
                has_base: hb,
                ..
            } => {
                if *i == index {
                    ours = o.clone();
                    base = b.clone();
                    theirs = t.clone();
                    has_base = *hb;
                    seen = true;
                } else if !seen {
                    before.clear();
                }
            }
        }
    }

    if !seen {
        return Err(format!("No conflict number {index} in {path}"));
    }

    let prompt = format!(
        "File: {path}\n\n\
         Lines before the conflict:\n{}\n\n\
         {}OUR side:\n{}\n\nTHEIR side:\n{}\n\n\
         Lines after the conflict:\n{}",
        before.join("\n"),
        if has_base {
            format!("COMMON ANCESTOR:\n{}\n\n", base.join("\n"))
        } else {
            String::new()
        },
        ours.join("\n"),
        theirs.join("\n"),
        after.join("\n")
    );

    // Room for the answer: it is at most about the size of both sides put
    // together, and a cap below that can only cut it off.
    let floor = u32::try_from(
        ours.iter()
            .chain(&theirs)
            .map(|l| l.len() + 1)
            .sum::<usize>()
            / 2
            + 512,
    )
    .unwrap_or(u32::MAX);

    let (reply, model) = complete_raw(state, CONFLICT_SYSTEM, &prompt, floor).await?;
    resolution_of(&reply, &model)
}

/// The line the model is told to put before its answer, and the one after.
///
/// An answer with edges is one that can be found wherever the model put it:
/// after a sentence of commentary it was told not to write, inside a fence it
/// was told not to add, or in the reasoning field on a host whose chat
/// template loses the final turn. It is also the only way an empty
/// resolution — both sides deleted the lines — can be told from no answer.
const RESOLUTION_OPEN: &str = "=====BEGIN RESOLUTION=====";
const RESOLUTION_CLOSE: &str = "=====END RESOLUTION=====";

/// Reads the resolved lines out of a reply.
///
/// Looks for the delimited block in the content first and the reasoning
/// second. Without a delimited block, the content is the answer if there is
/// one, minus any fence the model added anyway.
fn resolution_of(reply: &Reply, model: &str) -> Result<Vec<String>, String> {
    if reply.cut_off() {
        return Err(if reply.is_blank() {
            reply.missing(model)
        } else {
            format!(
                "{model} ran out of tokens part way through the answer. \
                 Raise Max tokens or turn Thinking off in Settings › AI"
            )
        });
    }
    if let Some(lines) = delimited(&reply.content).or_else(|| delimited(&reply.reasoning)) {
        return Ok(lines);
    }
    if reply.is_blank() {
        return Err(reply.missing(model));
    }
    Ok(strip_fences(reply.content.trim_matches(['\r', '\n'])))
}

/// The lines between the delimiters, when both are there in order.
///
/// A fence the model wrapped the block in, inside or outside the delimiters,
/// is dropped; the delimiter lines themselves may carry stray whitespace.
fn delimited(text: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = text.lines().collect();
    let open = lines.iter().position(|l| l.trim() == RESOLUTION_OPEN)?;
    let close = lines
        .iter()
        .skip(open + 1)
        .position(|l| l.trim() == RESOLUTION_CLOSE)
        .map(|at| at + open + 1)?;
    let inner = strip_fences(&lines[open + 1..close].join("\n"));
    Some(inner)
}

/// Splits a model's answer into a summary line and a body.
fn split_message(answer: &str) -> CommitMessage {
    // Models like to wrap prose in fences even when told not to.
    let cleaned = strip_fences(answer).join("\n");
    let mut lines = cleaned.lines();
    let summary = lines
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches("Summary:")
        .trim()
        .to_string();
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    CommitMessage { summary, body }
}

/// Removes a surrounding markdown code fence, if the answer has one.
fn strip_fences(answer: &str) -> Vec<String> {
    let lines: Vec<&str> = answer.lines().collect();
    let opens = lines
        .first()
        .is_some_and(|l| l.trim_start().starts_with("```"));
    let closes = lines.len() > 1 && lines.last().is_some_and(|l| l.trim() == "```");
    let slice = if opens && closes {
        &lines[1..lines.len() - 1]
    } else if opens {
        &lines[1..]
    } else {
        &lines[..]
    };
    slice.iter().map(|l| l.to_string()).collect()
}

/// What the model is told when the settings box has not been changed.
///
/// One line, most of the time. A body is the exception rather than the shape,
/// because a commit that needs three sentences of explanation is rarer than
/// the models writing them believe.
///
/// It says nothing about `feat:` and `fix:` on purpose. A type prefix is a
/// convention a repository either has or does not, and a default that reaches
/// for one writes it into repositories that never asked. Anyone who wants them
/// has the settings box to say so.
pub const DEFAULT_COMMIT_PROMPT: &str = "\
You write git commit messages for a working developer. Reply with the message \
and nothing else: no preamble, no markdown, no code fences, no quotes.

Line 1 is the message, and usually the whole of it: imperative mood, no \
trailing period, under 72 characters, specific about what changed.

Add a body only where the summary cannot carry the change on its own. When you \
do, leave a blank line after the summary and keep it to one or two sentences \
on WHY. Most commits need no body at all. Never list the files, never restate \
the diff, never pad.";

/// Kept for the settings that were written before the box existed: a config
/// that said `conventional` is migrated into the box with this text, so the
/// choice survives the change rather than being quietly dropped.
pub const CONVENTIONAL_COMMIT_PROMPT: &str = "\
You write git commit messages in Conventional Commits form. Reply with the \
message and nothing else: no preamble, no markdown, no code fences.

Line 1: `type(scope): summary` where type is one of feat, fix, docs, style, \
refactor, perf, test, build, ci, chore. The scope is optional. Imperative \
mood, no trailing period, under 72 characters total.

Then a blank line, then one to three sentences on WHY the change was made. \
Skip the body for a trivial change. Do not list files.";

const REVIEW_SYSTEM: &str = "\
You write the title and description of a pull request, for the reviewer who \
has to read it. Reply with the text and nothing else: no preamble, no code \
fences, no quotes.

Line 1 is the title: what the branch does as a whole, imperative mood, no \
trailing period, under 70 characters. It is not a list of the commits; if they \
are one piece of work, name the piece. Never prefix it with the branch name or \
a ticket number that is already in the branch name.

Then a blank line, then the description in plain GitHub-flavoured markdown, \
usually 60 to 200 words:

- A short paragraph on WHY the change was made and what problem it solves.
- A `## What changed` list of the substantial changes, one line each, only \
where there is more than one worth separating. Group by intent, not by file, \
and leave out anything mechanical.
- A `## How to test` list, only when there is something a reviewer should \
actually run or click.

Write what the commits and the file list support and nothing more. Do not \
invent tickets, screenshots, breaking changes or migration steps. Do not \
thank anyone, do not summarise your own summary, and do not add a checklist \
the project has not asked for.";

const CONFLICT_SYSTEM: &str = "\
You resolve a single git merge conflict.

You are given the lines around the conflict, the common ancestor when it is \
available, and both sides. Work out what each side was trying to do and produce \
the text that keeps BOTH intentions where they are compatible. If they are \
genuinely incompatible, keep the side whose change is clearly newer or more \
specific.

Reply with the resolved lines between two marker lines, exactly like this:

=====BEGIN RESOLUTION=====
<the resolved lines>
=====END RESOLUTION=====

Nothing else: no conflict markers, no markdown, no code fences, no commentary, \
no explanation before or after the markers. Preserve the file's existing \
indentation style and language exactly. If the right answer is to keep one side \
unchanged, output that side verbatim. If the right answer is to delete the \
lines entirely, put nothing between the markers.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ticket_prefix_switch_decides_what_the_model_is_told() {
        let on = branch_context("feature/ABC-123-login", true);
        assert!(on.contains("`ABC-123: `"));
        let off = branch_context("feature/ABC-123-login", false);
        assert!(!off.contains("expects it at the start"));
        assert!(off.contains("Do not put the branch name"));
    }

    #[test]
    fn an_empty_box_means_the_default_rather_than_no_instructions() {
        let mut ai = config::Ai::default();
        assert_eq!(commit_prompt(&ai), DEFAULT_COMMIT_PROMPT);

        ai.commit_prompt = Some(String::new());
        assert_eq!(commit_prompt(&ai), DEFAULT_COMMIT_PROMPT);

        ai.commit_prompt = Some("   \n  ".to_string());
        assert_eq!(commit_prompt(&ai), DEFAULT_COMMIT_PROMPT);
    }

    #[test]
    fn what_is_in_the_box_is_what_the_model_is_told() {
        let ai = config::Ai {
            commit_prompt: Some("  Only ever write one line.  ".to_string()),
            ..Default::default()
        };
        assert_eq!(commit_prompt(&ai), "Only ever write one line.");
    }

    #[test]
    fn the_default_asks_for_one_line_and_a_body_only_where_it_is_needed() {
        // The wording is the feature: a default that asks for three sentences
        // of body gets three sentences of body on every commit.
        assert!(DEFAULT_COMMIT_PROMPT.contains("usually the whole of it"));
        assert!(DEFAULT_COMMIT_PROMPT.contains("Add a body only where"));
        assert!(DEFAULT_COMMIT_PROMPT.contains("Most commits need no body at all"));
    }

    #[test]
    fn the_default_asks_for_no_type_prefix() {
        // A prefix is a convention a repository either has or does not, and
        // the default writing `feat:` into one that never used it is the thing
        // people turn the whole feature off over. Conventional Commits are
        // still there for anyone who wants them, in the settings box.
        assert!(!DEFAULT_COMMIT_PROMPT.contains("feat:"));
        assert!(!DEFAULT_COMMIT_PROMPT.contains("fix:"));
        assert!(CONVENTIONAL_COMMIT_PROMPT.contains("feat"));
    }

    #[test]
    fn a_thinking_level_becomes_an_effort() {
        assert_eq!(
            reasoning_field("high"),
            serde_json::json!({ "effort": "high" })
        );
        assert_eq!(
            reasoning_field("minimal"),
            serde_json::json!({ "effort": "minimal" })
        );
    }

    #[test]
    fn off_switches_thinking_off_rather_than_omitting_it() {
        assert_eq!(
            reasoning_field("off"),
            serde_json::json!({ "enabled": false })
        );
        // An unknown value from a hand-edited config must not turn thinking on.
        assert_eq!(
            reasoning_field("whatever"),
            serde_json::json!({ "enabled": false })
        );
    }

    #[test]
    fn splits_a_summary_from_a_body() {
        let message = split_message("Add a thing\n\nBecause it was missing.");
        assert_eq!(message.summary, "Add a thing");
        assert_eq!(message.body, "Because it was missing.");
    }

    #[test]
    fn tolerates_a_summary_only_answer() {
        let message = split_message("Fix the off-by-one");
        assert_eq!(message.summary, "Fix the off-by-one");
        assert_eq!(message.body, "");
    }

    #[test]
    fn strips_fences_a_model_added_anyway() {
        let message = split_message("```\nAdd a thing\n\nBody here.\n```");
        assert_eq!(message.summary, "Add a thing");
        assert_eq!(message.body, "Body here.");

        assert_eq!(
            strip_fences("```rust\nlet x = 1;\n```"),
            vec!["let x = 1;".to_string()]
        );
        assert_eq!(
            strip_fences("no fence here"),
            vec!["no fence here".to_string()]
        );
    }

    #[test]
    fn refuses_a_branch_name_git_would_read_as_a_flag() {
        assert_eq!(checked_ref(" main ").unwrap(), "main");
        assert!(checked_ref("--all").is_err());
        assert!(checked_ref("   ").is_err());
    }

    #[test]
    fn drops_a_label_the_model_prefixed() {
        let message = split_message("Summary: Tidy the parser");
        assert_eq!(message.summary, "Tidy the parser");
    }

    fn reply(body: serde_json::Value) -> Reply {
        parse_reply(&body).unwrap()
    }

    #[test]
    fn reads_content_as_a_string_or_as_parts() {
        let plain = reply(serde_json::json!({
            "choices": [{ "message": { "content": "  hello \n" }, "finish_reason": "stop" }]
        }));
        assert_eq!(plain.content, "  hello \n");
        assert_eq!(plain.finish, "stop");

        let parts = reply(serde_json::json!({
            "choices": [{ "message": { "content": [
                { "type": "text", "text": "hel" },
                { "type": "text", "text": "lo" }
            ] } }]
        }));
        assert_eq!(parts.content, "hello");
    }

    #[test]
    fn a_null_content_with_reasoning_is_not_an_answer() {
        let r = reply(serde_json::json!({
            "choices": [{
                "message": { "content": null, "reasoning": "Let me think..." },
                "finish_reason": "length"
            }],
            "usage": { "completion_tokens_details": { "reasoning_tokens": 1500 } }
        }));
        assert_eq!(r.content, "");
        assert_eq!(r.reasoning, "Let me think...");
        assert!(r.cut_off());
        let why = r.missing("deepseek/deepseek-v4.1-flash");
        assert!(why.contains("ran out of tokens"), "{why}");
        assert!(why.contains("1500 reasoning tokens"), "{why}");
        assert!(why.contains("Max tokens"), "{why}");
    }

    #[test]
    fn reasoning_content_is_read_under_deepseeks_name_too() {
        let r = reply(serde_json::json!({
            "choices": [{
                "message": { "content": "", "reasoning_content": "thinking" },
                "finish_reason": "stop"
            }]
        }));
        assert_eq!(r.reasoning, "thinking");
        assert!(r.missing("m").contains("reasoning field"));
    }

    #[test]
    fn inline_think_tags_are_thinking_not_answer() {
        let r = reply(serde_json::json!({
            "choices": [{
                "message": { "content": "<think>\nhmm\n</think>\nAdd a thing" },
                "finish_reason": "stop"
            }]
        }));
        assert_eq!(r.content.trim(), "Add a thing");
        assert_eq!(r.reasoning, "hmm");

        // Never closed: the model ran out before it got to the answer.
        let (answer, thinking) = strip_think_tags("<think>still going");
        assert_eq!(answer, "");
        assert_eq!(thinking, "still going");
        assert_eq!(
            strip_think_tags("no tags"),
            ("no tags".to_string(), String::new())
        );
    }

    #[test]
    fn a_first_line_keeps_its_indentation() {
        let r = reply(serde_json::json!({
            "choices": [{ "message": { "content": "    let x = 1;\n    let y = 2;\n" }, "finish_reason": "stop" }]
        }));
        assert_eq!(
            resolution_of(&r, "m").unwrap(),
            vec!["    let x = 1;".to_string(), "    let y = 2;".to_string()]
        );
    }

    #[test]
    fn a_model_that_must_think_is_recognised_from_the_refusal() {
        assert!(cannot_stop_thinking(
            "Reasoning is mandatory for this endpoint and cannot be disabled."
        ));
        assert!(!cannot_stop_thinking("Invalid API key"));
        assert!(!cannot_stop_thinking(
            "reasoning.effort must be one of low, medium, high"
        ));
    }

    #[test]
    fn a_provider_error_inside_a_200_is_an_error() {
        let err = parse_reply(&serde_json::json!({
            "choices": [{ "error": { "message": "upstream overloaded" }, "finish_reason": "error" }]
        }))
        .unwrap_err();
        assert!(err.contains("upstream overloaded"), "{err}");

        let err = parse_reply(&serde_json::json!({ "id": "x" })).unwrap_err();
        assert!(err.contains("no choices"), "{err}");
    }

    #[test]
    fn an_empty_answer_says_so_without_guessing() {
        let r = reply(serde_json::json!({
            "choices": [{ "message": { "content": "" }, "finish_reason": "stop" }]
        }));
        assert_eq!(
            r.missing("m"),
            "m returned an empty answer (finish reason: stop)"
        );
    }

    fn done(content: &str, reasoning: &str) -> Reply {
        Reply {
            content: content.to_string(),
            reasoning: reasoning.to_string(),
            finish: "stop".to_string(),
            reasoning_tokens: None,
        }
    }

    #[test]
    fn the_system_prompt_asks_for_the_markers_the_parser_looks_for() {
        assert!(CONFLICT_SYSTEM.contains(RESOLUTION_OPEN));
        assert!(CONFLICT_SYSTEM.contains(RESOLUTION_CLOSE));
    }

    #[test]
    fn a_delimited_answer_is_found_wherever_the_model_put_it() {
        let block =
            "=====BEGIN RESOLUTION=====\n  let x = 1;\n  let y = 2;\n=====END RESOLUTION=====";
        let want = vec!["  let x = 1;".to_string(), "  let y = 2;".to_string()];

        assert_eq!(resolution_of(&done(block, ""), "m").unwrap(), want);
        // Commentary around it, which the model was told not to write.
        let chatty = format!("Sure, here is the merge:\n{block}\nLet me know if that helps.");
        assert_eq!(resolution_of(&done(&chatty, ""), "m").unwrap(), want);
        // Fenced, inside the markers.
        let fenced = "=====BEGIN RESOLUTION=====\n```rust\n  let x = 1;\n  let y = 2;\n```\n=====END RESOLUTION=====";
        assert_eq!(resolution_of(&done(fenced, ""), "m").unwrap(), want);
        // Stray whitespace on the marker lines.
        let loose =
            " =====BEGIN RESOLUTION===== \n  let x = 1;\n  let y = 2;\n=====END RESOLUTION=====  ";
        assert_eq!(resolution_of(&done(loose, ""), "m").unwrap(), want);
        // The whole answer landed in the reasoning field.
        assert_eq!(
            resolution_of(&done("", &format!("I will keep both.\n{block}")), "m").unwrap(),
            want
        );
    }

    #[test]
    fn an_empty_resolution_is_an_answer_when_it_is_delimited() {
        let block = "=====BEGIN RESOLUTION=====\n=====END RESOLUTION=====";
        assert_eq!(
            resolution_of(&done(block, ""), "m").unwrap(),
            Vec::<String>::new()
        );
        // But nothing at all is still nothing.
        assert!(resolution_of(&done("", ""), "m").is_err());
        assert!(resolution_of(&done("", "only thinking"), "m").is_err());
    }

    #[test]
    fn an_undelimited_answer_is_still_taken_minus_its_fence() {
        assert_eq!(
            resolution_of(&done("```\nlet x = 1;\n```", ""), "m").unwrap(),
            vec!["let x = 1;".to_string()]
        );
        // Only an opening marker: not a block, so the content stands as is.
        let half = "=====BEGIN RESOLUTION=====\nlet x = 1;";
        assert_eq!(
            resolution_of(&done(half, ""), "m").unwrap(),
            vec![
                "=====BEGIN RESOLUTION=====".to_string(),
                "let x = 1;".to_string()
            ]
        );
    }

    #[test]
    fn a_cut_off_resolution_is_refused_rather_than_written_half() {
        let mut r = done("=====BEGIN RESOLUTION=====\nlet x = 1;", "");
        r.finish = "length".to_string();
        let err = resolution_of(&r, "m").unwrap_err();
        assert!(err.contains("part way"), "{err}");

        let mut r = done("", "thinking");
        r.finish = "length".to_string();
        assert!(resolution_of(&r, "m")
            .unwrap_err()
            .contains("ran out of tokens"));
    }

    fn file_diff(path: &str, lines: usize, width: usize) -> String {
        let mut text = format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,0 +1,{lines} @@\n"
        );
        for n in 0..lines {
            text.push_str(&format!("+{}\n", format!("{n} ").repeat(width / 2)));
        }
        text
    }

    #[test]
    fn a_lockfile_is_one_line_and_never_crowds_out_the_code() {
        let diff = format!(
            "{}{}",
            file_diff("Cargo.lock", 5_000, 40),
            file_diff("src/main.rs", 3, 10)
        );
        let numstat = "5000\t0\tCargo.lock\n3\t0\tsrc/main.rs\n";
        let fitted = fit_diff(&diff, numstat, 4_000);
        assert!(fitted.contains("Cargo.lock: lockfile, +5000 −0 lines (contents left out)"));
        assert!(fitted.contains("+++ b/src/main.rs"));
        assert!(fitted.chars().count() <= 4_000);
    }

    #[test]
    fn every_file_gets_a_share_of_the_budget() {
        let diff = format!(
            "{}{}{}",
            file_diff("big.rs", 2_000, 60),
            file_diff("small.rs", 2, 10),
            file_diff("other.rs", 2_000, 60)
        );
        let fitted = fit_diff(&diff, "", 6_000);
        // The small one whole, both big ones present and cut.
        assert!(fitted.contains("+++ b/small.rs"));
        assert!(fitted.contains("+++ b/big.rs"));
        assert!(fitted.contains("+++ b/other.rs"));
        assert_eq!(
            fitted.matches("more lines of this file left out").count(),
            2
        );
        assert!(fitted.chars().count() <= 6_000);
        // In the order git printed them.
        assert!(fitted.find("big.rs").unwrap() < fitted.find("small.rs").unwrap());
    }

    #[test]
    fn minified_and_binary_files_are_summarised() {
        let minified = file_diff("app.bundle.js", 1, 3_000);
        let binary = "diff --git a/logo.png b/logo.png\nindex 1..2 100644\nBinary files a/logo.png and b/logo.png differ\n";
        let fitted = fit_diff(
            &format!("{minified}{binary}"),
            "1\t0\tapp.bundle.js\n-\t-\tlogo.png\n",
            10_000,
        );
        assert!(fitted.contains("app.bundle.js: minified"));
        assert!(fitted.contains("logo.png: binary"));
    }

    #[test]
    fn a_small_diff_passes_through_untouched() {
        let diff = file_diff("a.rs", 3, 10);
        assert_eq!(fit_diff(&diff, "", 48_000), diff);
    }

    #[test]
    fn claude_defaults_to_haiku_and_preserves_other_providers_models() {
        let mut ai = config::Ai::default();
        assert_eq!(ai.provider, AiProvider::OpenRouter);
        assert_eq!(ai.active_model(), None);

        ai.model = Some("anthropic/claude-sonnet-4.5".to_string());
        assert_eq!(ai.active_model().as_deref(), Some("anthropic/claude-sonnet-4.5"));

        ai.provider = AiProvider::Claude;
        assert_eq!(ai.active_model().as_deref(), Some("haiku"));

        ai.claude_model = Some("sonnet".to_string());
        assert_eq!(ai.active_model().as_deref(), Some("sonnet"));

        ai.claude_model = Some("   ".to_string());
        assert_eq!(ai.active_model().as_deref(), Some("haiku"));

        ai.provider = AiProvider::Codex;
        assert_eq!(ai.active_model(), None);
        ai.codex_model = Some("gpt-5.4-mini".to_string());
        assert_eq!(ai.active_model().as_deref(), Some("gpt-5.4-mini"));

        ai.provider = AiProvider::Antigravity;
        assert_eq!(ai.active_model(), None);
        ai.antigravity_model = Some("gemini-3.8-flash-low".to_string());
        assert_eq!(ai.active_model().as_deref(), Some("gemini-3.8-flash-low"));
    }

    #[test]
    fn old_config_without_provider_defaults_to_openrouter_and_haiku_for_claude() {
        let parsed: config::Ai = serde_json::from_str(
            r#"{
                "model": "openai/gpt-4o",
                "max_tokens": 3000,
                "reasoning": "off",
                "commit_style": "plain",
                "commit_prompt": null
            }"#,
        )
        .unwrap();
        assert_eq!(parsed.provider, AiProvider::OpenRouter);
        assert_eq!(parsed.model.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(parsed.claude_model.as_deref(), Some("haiku"));
    }

    #[test]
    fn parses_agy_models_output_grouping_effort_variants() {
        let raw = "\r⠋ Fetching available models...\rgemini-4-neon-high\tGemini 4 Neon (High)\ngemini-4-neon-low\tGemini 4 Neon (Low)\ngemini-4-neon-xhigh\tGemini 4 Neon (XHigh)\ngemini-pro-orchestrator-neon\tOrchestrator (Neon)\n";
        let list = parse_agy_models(raw);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "gemini-4-neon");
        assert_eq!(list[0].name, "Gemini 4 Neon");
        assert_eq!(list[0].effort_levels, vec!["low", "high", "xhigh"]);
        assert_eq!(list[0].default_effort.as_deref(), Some("low"));
        assert_eq!(
            resolve_antigravity_args(&list, Some("gemini-4-neon"), Some("xhigh")),
            (Some("gemini-4-neon-xhigh".to_string()), None)
        );
        assert_eq!(
            resolve_antigravity_args(&list, Some("gemini-4-neon"), None),
            (Some("gemini-4-neon-low".to_string()), None)
        );
        assert_eq!(list[1].id, "gemini-pro-orchestrator-neon");
        assert!(list[1].effort_levels.is_empty());
    }

    #[test]
    fn parses_claude_init_models_ordering_haiku_first_with_efforts() {
        let items = vec![
            serde_json::json!({"value": "default", "displayName": "Default"}),
            serde_json::json!({
                "value": "opus",
                "displayName": "Opus 5.5",
                "description": "Complex work",
                "supportedEffortLevels": ["low", "medium", "high"],
                "supportsThinkingOff": false
            }),
            serde_json::json!({
                "value": "haiku",
                "displayName": "Haiku 5.5",
                "description": "Fastest",
                "supportedEffortLevels": ["low", "medium", "high"],
                "supportsThinkingOff": true
            }),
        ];
        let list = parse_claude_init_models(&items);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "haiku");
        assert_eq!(list[0].name, "Haiku 5.5");
        assert_eq!(list[0].effort_levels, vec!["off", "low", "medium", "high"]);
        assert_eq!(list[1].id, "opus");
        assert_eq!(list[1].effort_levels, vec!["low", "medium", "high"]);
    }

    #[test]
    fn parses_codex_app_server_and_cache_models_with_efforts() {
        let rpc = vec![
            serde_json::json!({"id": "hidden-one", "hidden": true}),
            serde_json::json!({
                "id": "gpt-6-luna",
                "displayName": "GPT-6-Luna",
                "description": "Fast",
                "hidden": false,
                "defaultReasoningEffort": "medium",
                "supportedReasoningEfforts": [
                    {"reasoningEffort": "low"},
                    {"reasoningEffort": "medium"},
                    {"reasoningEffort": "high"}
                ]
            }),
        ];
        let from_rpc = parse_codex_app_server_models(&rpc);
        assert_eq!(from_rpc.len(), 1);
        assert_eq!(from_rpc[0].id, "gpt-6-luna");
        assert_eq!(from_rpc[0].effort_levels, vec!["low", "medium", "high"]);
        assert_eq!(from_rpc[0].default_effort.as_deref(), Some("medium"));

        let raw = r#"{
            "models": [
                {"slug": "secret-model", "display_name": "Secret", "visibility": "hide"},
                {"slug": "gpt-6-luna", "display_name": "GPT-6-Luna", "description": "Fast coding model", "visibility": "list", "default_reasoning_level": "medium", "supported_reasoning_levels": [{"effort": "low"}, {"effort": "high"}]}
            ]
        }"#;
        let list = parse_codex_models_cache(raw);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "gpt-6-luna");
        assert_eq!(list[0].effort_levels, vec!["low", "high"]);
        assert_eq!(list[0].default_effort.as_deref(), Some("medium"));
    }

    #[test]
    fn extracts_codex_error_from_verbose_stderr() {
        let stderr = "OpenAI Codex v0.169.0\n--------\nworkdir: /tmp\nERROR: The 'foo' model is not supported.\nERROR: The 'foo' model is not supported.\n";
        assert_eq!(
            extract_codex_error(stderr).as_deref(),
            Some("Codex CLI failed: The 'foo' model is not supported.")
        );
        assert_eq!(extract_codex_error("nothing wrong here"), None);
    }
}
