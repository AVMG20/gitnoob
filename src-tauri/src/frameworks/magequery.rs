//! `magequery`, a command-line tool that reads a whole Magento installation's
//! config, run for the "Magento wiring" view of a class.
//!
//! It ships inside the app as a sidecar (`bundle.externalBin` in
//! tauri.conf.json, fetched by `scripts/magequery.mjs`), next to the app's
//! own executable. Settings → Frameworks can point at another copy instead.
//! Nothing else in the app needs it: every ⌘-click works without.
//!
//! The rules for running it: arguments as an array (no shell), the class
//! name checked against `^[A-Za-z_\\][A-Za-z0-9_\\]*$` and passed after `--`
//! so it can never be read as a flag, `--db` never passed, ten seconds at
//! most, and no more than 8 MB of output read.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::state::AppState;

/// The version `scripts/magequery.mjs` fetches. Keep the two in step.
pub const PINNED: &str = "0.24.1";

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTPUT: usize = 8 * 1024 * 1024;

/// The target triple this build is for, from build.rs.
const TARGET: &str = env!("GITNOOB_TARGET");

/// Where a sidecar can be, best first: next to the executable (where the
/// bundle and `tauri dev` put it), then `src-tauri/binaries` in a checkout.
pub fn sidecar_candidates(
    exe_dir: &Path,
    manifest_dir: Option<&Path>,
    triple: &str,
) -> Vec<PathBuf> {
    let windows = triple.contains("windows");
    let mut out = vec![exe_dir.join(if windows {
        "magequery.exe"
    } else {
        "magequery"
    })];
    if let Some(manifest) = manifest_dir {
        let name = format!("magequery-{triple}{}", if windows { ".exe" } else { "" });
        out.push(manifest.join("binaries").join(name));
    }
    out
}

/// A usable binary: a file with something in it. A debug build without the
/// sidecar fetched has an empty placeholder (build.rs).
fn usable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

/// The bundled binary, if this build has one.
pub fn bundled() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let manifest = cfg!(debug_assertions).then(|| Path::new(env!("CARGO_MANIFEST_DIR")));
    sidecar_candidates(dir, manifest, TARGET)
        .into_iter()
        .find(|path| usable(path))
}

/// The binary to run: the path in Settings when it is set, else the bundled one.
pub fn find_with(setting: Option<&str>) -> Option<PathBuf> {
    match setting.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => {
            let path = PathBuf::from(path);
            (path.is_absolute() && usable(&path)).then_some(path)
        }
        None => bundled(),
    }
}

pub fn find(state: &AppState) -> Option<PathBuf> {
    find_with(state.config().global.magequery_path.as_deref())
}

/// What Settings → Frameworks shows about it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Status {
    pub path: Option<String>,
    /// `setting` or `bundled`.
    pub source: Option<&'static str>,
    /// The version this build bundles.
    pub pinned: &'static str,
    /// The path in Settings is set but is not a file.
    pub missing_setting: bool,
}

pub fn status(state: &AppState) -> Status {
    let setting = state.config().global.magequery_path;
    let set = setting.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let found = find_with(set);
    Status {
        path: found
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        source: found
            .as_ref()
            .map(|_| if set.is_some() { "setting" } else { "bundled" }),
        pinned: PINNED,
        missing_setting: set.is_some() && found.is_none(),
    }
}

/// Whether a class name is safe to pass: letters, digits, `_` and `\`, not
/// starting with a digit. Never a flag.
pub fn valid_class(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '\\')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '\\')
}

/// Runs the tool with `args` and hands back what it printed, or what went wrong.
pub fn run(binary: &Path, args: &[String]) -> Result<String, String> {
    if args
        .iter()
        .any(|arg| arg == "--db" || arg.starts_with("--db="))
    {
        return Err("magequery is never run against the database".to_string());
    }
    let mut child = Command::new(binary)
        .args(args)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run magequery: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("magequery gave no output")?;
    let mut stderr = child.stderr.take().ok_or("magequery gave no output")?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = (&mut stdout)
            .take(MAX_OUTPUT as u64 + 1)
            .read_to_end(&mut out);
        // Whatever is left is drained so the tool is not stuck writing.
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
        out
    });
    let errors = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = (&mut stderr).take(64 * 1024).read_to_end(&mut out);
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
        out
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "magequery took longer than {} seconds and was stopped",
                    TIMEOUT.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(e) => return Err(format!("magequery: {e}")),
        }
    };
    let out = reader.join().unwrap_or_default();
    let err = errors.join().unwrap_or_default();
    if out.len() > MAX_OUTPUT {
        return Err("magequery printed more than 8 MB; not read".to_string());
    }
    if !status.success() {
        let said = String::from_utf8_lossy(&err);
        let said = said.trim().trim_start_matches("Error: ");
        return Err(if said.is_empty() {
            format!("magequery stopped with {status}")
        } else {
            said.lines().take(4).collect::<Vec<_>>().join(" ")
        });
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// One row of the wiring view.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Row {
    pub label: String,
    pub detail: Option<String>,
    /// The class the row is about, to open.
    pub class: Option<String>,
    /// Where it is declared: a config file in the work tree, and its line.
    pub path: Option<String>,
    pub line: Option<u32>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Section {
    pub title: String,
    pub rows: Vec<Row>,
}

/// A class as magequery describes it.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Wiring {
    pub class: String,
    pub kind: Option<String>,
    pub module: Option<String>,
    /// The class file, relative, when it is in the work tree.
    pub path: Option<String>,
    /// What the object manager hands out for it, when that is another class.
    pub preference: Option<String>,
    pub sections: Vec<Section>,
}

/// A path magequery printed (absolute) as one in the work tree.
fn relative(root: &Path, path: &str) -> Option<String> {
    let path = Path::new(path);
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let inner = path
        .strip_prefix(&root)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            let real = path.canonicalize().ok()?;
            real.strip_prefix(&root).ok().map(Path::to_path_buf)
        })?;
    Some(crate::files::slashed(&inner))
}

fn text(value: &Value, key: &str) -> Option<String> {
    match &value[key] {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A row from one of magequery's entries, whatever its shape: the first
/// name-like field as the label, a few others as the detail, its `source`
/// as the place.
fn row(
    root: &Path,
    item: &Value,
    label_keys: &[&str],
    detail_keys: &[&str],
    class_key: Option<&str>,
) -> Row {
    let label = label_keys
        .iter()
        .find_map(|key| text(item, key))
        .unwrap_or_else(|| "(unnamed)".to_string());
    let detail: Vec<String> = detail_keys
        .iter()
        .filter_map(|key| {
            let value = text(item, key)?;
            Some(match *key {
                "sort_order" => format!("sort {value}"),
                "area" => value,
                _ => value,
            })
        })
        .collect();
    let source = &item["source"];
    let mut detail = detail;
    if let Some(area) = text(source, "area") {
        detail.push(area);
    }
    Row {
        label,
        detail: (!detail.is_empty()).then(|| detail.join(" · ")),
        class: class_key.and_then(|key| text(item, key)),
        path: text(source, "file").and_then(|file| relative(root, &file)),
        line: source["line"].as_u64().map(|line| line as u32),
    }
}

fn section(
    root: &Path,
    title: &str,
    items: &Value,
    label_keys: &[&str],
    detail_keys: &[&str],
    class_key: Option<&str>,
) -> Option<Section> {
    let rows: Vec<Row> = items
        .as_array()?
        .iter()
        .filter(|item| item.is_object())
        .map(|item| row(root, item, label_keys, detail_keys, class_key))
        .collect();
    (!rows.is_empty()).then(|| Section {
        title: title.to_string(),
        rows,
    })
}

/// The wiring view from `whatis --json` and `plugins --json`.
pub fn read_wiring(root: &Path, fqn: &str, whatis: &Value, plugins: Option<&Value>) -> Wiring {
    let mut wiring = Wiring {
        class: text(whatis, "class").unwrap_or_else(|| fqn.to_string()),
        kind: text(whatis, "kind"),
        module: text(whatis, "module"),
        path: text(whatis, "file").and_then(|file| relative(root, &file)),
        preference: text(whatis, "resolves_to"),
        sections: Vec::new(),
    };
    let mut push = |found: Option<Section>| {
        if let Some(found) = found {
            wiring.sections.push(found);
        }
    };
    if let Some(plugins) = plugins {
        // Each plugin with the methods it wraps.
        let rows: Vec<Row> = plugins
            .as_array()
            .map(|all| {
                all.iter()
                    .filter(|item| item.is_object())
                    .map(|item| {
                        let mut found = row(
                            root,
                            item,
                            &["name"],
                            &["class", "sort_order"],
                            Some("class"),
                        );
                        let methods: Vec<String> = item["methods"]
                            .as_array()
                            .map(|methods| {
                                methods
                                    .iter()
                                    .filter_map(|method| {
                                        Some(format!(
                                            "{} {}",
                                            method["kind"].as_str()?,
                                            method["target"].as_str()?
                                        ))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        if !methods.is_empty() {
                            let detail = found.detail.take().unwrap_or_default();
                            found.detail = Some(format!("{detail} · {}", methods.join(", ")));
                        }
                        if item["disabled"].as_bool() == Some(true) {
                            found.label.push_str(" (disabled)");
                        }
                        found
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !rows.is_empty() {
            push(Some(Section {
                title: "Plugins".to_string(),
                rows,
            }));
        }
    }
    let uses = &whatis["uses"];
    push(section(
        root,
        "Preference for",
        &uses["preferred_for"],
        &["name"],
        &[],
        Some("name"),
    ));
    push(section(
        root,
        "Virtual types",
        &uses["virtual_types"],
        &["name", "virtual_type"],
        &[],
        None,
    ));
    push(section(
        root,
        "Injected into",
        &uses["injections"],
        &["consumer"],
        &["argument"],
        Some("consumer"),
    ));
    push(section(
        root,
        "Observes",
        &whatis["observes"],
        &["event"],
        &["name"],
        None,
    ));
    push(section(
        root,
        "Cron jobs",
        &whatis["cron_jobs"],
        &["name"],
        &["group", "method", "schedule"],
        None,
    ));
    push(section(
        root,
        "Web API",
        &whatis["webapi"],
        &["url"],
        &["method", "service_method"],
        None,
    ));
    push(section(
        root,
        "Message queue",
        &whatis["mq"],
        &["topic", "name", "consumer"],
        &["queue"],
        None,
    ));
    push(section(
        root,
        "GraphQL",
        &whatis["graphql"],
        &["type", "name", "field"],
        &["field"],
        None,
    ));
    push(section(
        root,
        "URLs",
        &whatis["action_urls"],
        &["url", "path", "route"],
        &["area"],
        None,
    ));
    if whatis["command"].is_object() {
        push(section(
            root,
            "Console command",
            &Value::Array(vec![whatis["command"].clone()]),
            &["name", "command"],
            &[],
            None,
        ));
    }
    wiring
}

/// Runs `whatis` and `plugins` for a class and puts the answers together.
pub fn wiring(binary: &Path, root: &Path, fqn: &str) -> Result<Wiring, String> {
    let fqn = fqn.trim();
    if !valid_class(fqn) {
        return Err(format!("{fqn} is not a class name"));
    }
    let root_arg = root.to_string_lossy().into_owned();
    let args = |command: &str| -> Vec<String> {
        vec![
            command.to_string(),
            "--json".to_string(),
            "-r".to_string(),
            root_arg.clone(),
            "--".to_string(),
            fqn.trim_start_matches('\\').to_string(),
        ]
    };
    let whatis_args = args("whatis");
    let plugins_args = args("plugins");
    let binary_for_plugins = binary.to_path_buf();
    // Both at once: each reads the whole installation.
    let plugins = std::thread::spawn(move || run(&binary_for_plugins, &plugins_args));
    let whatis = run(binary, &whatis_args)?;
    let whatis: Value = serde_json::from_str(&whatis)
        .map_err(|e| format!("magequery's answer was not the JSON expected: {e}"))?;
    let plugins = plugins
        .join()
        .ok()
        .and_then(|found| found.ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    Ok(read_wiring(root, fqn, &whatis, plugins.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_class_name_is_never_a_flag() {
        assert!(valid_class("Magento\\Catalog\\Model\\Product"));
        assert!(valid_class("\\Foo_Bar"));
        assert!(!valid_class("--db"));
        assert!(!valid_class("-r"));
        assert!(!valid_class("Foo Bar"));
        assert!(!valid_class("Foo;rm"));
        assert!(!valid_class(""));
    }

    #[test]
    fn the_sidecar_is_looked_for_next_to_the_app_then_in_the_checkout() {
        let found = sidecar_candidates(
            Path::new("/Applications/gitnoob.app/Contents/MacOS"),
            Some(Path::new("/src/gitnoob/src-tauri")),
            "aarch64-apple-darwin",
        );
        assert_eq!(
            found,
            vec![
                PathBuf::from("/Applications/gitnoob.app/Contents/MacOS/magequery"),
                PathBuf::from("/src/gitnoob/src-tauri/binaries/magequery-aarch64-apple-darwin"),
            ]
        );
        let windows = sidecar_candidates(Path::new("C:/gitnoob"), None, "x86_64-pc-windows-msvc");
        assert_eq!(windows, vec![PathBuf::from("C:/gitnoob/magequery.exe")]);
    }

    #[test]
    fn a_db_flag_is_refused_before_anything_runs() {
        let refused = run(
            Path::new("/nonexistent"),
            &["config".to_string(), "--db".to_string()],
        );
        assert!(refused.unwrap_err().contains("database"));
    }

    #[test]
    fn a_setting_that_is_not_absolute_is_not_used() {
        assert_eq!(find_with(Some("magequery")), None);
    }
}
