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
/// No class name is this long; one that is, is not passed on.
const MAX_CLASS: usize = 512;

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

/// Why there is no magequery to run, in words that point at the fix.
pub fn not_found(setting: Option<&str>) -> String {
    match setting.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => format!(
            "Settings → Frameworks names {path}, which is not a program gitnoob can find. \
             Give its full path (no ~), as `which magequery` prints it."
        ),
        None => "magequery is not bundled with this build, and Settings → Frameworks names none"
            .to_string(),
    }
}

/// Whether a class name is safe to pass: letters, digits, `_` and `\`, not
/// starting with a digit. Never a flag.
pub fn valid_class(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= MAX_CLASS
        && chars
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
    let mut command = Command::new(binary);
    // A console program started from the windowed app gets a console window
    // of its own on Windows, two per wiring lookup, unless told not to.
    crate::git_cmd::quiet(&mut command);
    let mut child = command
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

/// A path magequery printed as one in the work tree: absolute under the
/// root it was given, or relative to it. `None` for anything outside.
fn relative(root: &Path, path: &str) -> Option<String> {
    let path = Path::new(path);
    if path.is_relative() {
        let inner = crate::files::slashed(path);
        return crate::files::resolve(root, &inner, true)
            .is_ok()
            .then_some(inner);
    }
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

fn short(fqn: &str) -> &str {
    fqn.rsplit('\\').next().unwrap_or(fqn)
}

/// The strings of an array field.
fn strings(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|one| one.as_str())
                .filter(|one| !one.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// A row from one entry and its `source` (`{module, file, line, area}`).
/// The source's area is added to the detail when it is not the global one
/// and the detail does not say it already; line 0 is magequery's "known
/// file, unknown line".
fn row(
    root: &Path,
    item: &Value,
    label: String,
    mut detail: Vec<String>,
    class: Option<String>,
) -> Row {
    let source = &item["source"];
    if let Some(area) = text(source, "area") {
        if area != "global" && !detail.contains(&area) {
            detail.push(area);
        }
    }
    Row {
        label,
        detail: (!detail.is_empty()).then(|| detail.join(" · ")),
        class,
        path: text(source, "file").and_then(|file| relative(root, &file)),
        line: source["line"]
            .as_u64()
            .filter(|line| *line > 0)
            .map(|line| line as u32),
    }
}

/// A section from an array field, one row per object in it that `make`
/// can name.
fn section(title: &str, items: &Value, make: impl Fn(&Value) -> Option<Row>) -> Option<Section> {
    let rows: Vec<Row> = items
        .as_array()?
        .iter()
        .filter(|item| item.is_object())
        .filter_map(make)
        .collect();
    (!rows.is_empty()).then(|| Section {
        title: title.to_string(),
        rows,
    })
}

/// Rows that are only class names (`parents`, `interfaces`).
fn classes(title: &str, names: Vec<String>) -> Option<Section> {
    (!names.is_empty()).then(|| Section {
        title: title.to_string(),
        rows: names
            .into_iter()
            .map(|name| Row {
                label: name.clone(),
                class: Some(name),
                ..Row::default()
            })
            .collect(),
    })
}

/// One plugin of `plugins --json` (magequery's `Plugin`): `name`, `class`,
/// `sort_order`, `methods` (`{kind, target, plugin_method}`), `declared_on`,
/// `disabled`, `source` and `areas`.
fn plugin_row(root: &Path, asked: &str, item: &Value) -> Option<Row> {
    let mut label = text(item, "name")?;
    if item["disabled"].as_bool() == Some(true) {
        label.push_str(" (disabled)");
    }
    let class = text(item, "class");
    let mut detail: Vec<String> = Vec::new();
    detail.extend(class.clone());
    if let Some(order) = text(item, "sort_order") {
        detail.push(format!("sort {order}"));
    }
    // Declared on an interface or a parent: the case people miss.
    if let Some(on) = text(item, "declared_on") {
        if !on.eq_ignore_ascii_case(asked) {
            detail.push(format!("on {}", short(&on)));
        }
    }
    let areas = strings(item, "areas");
    if !areas.is_empty() && areas != ["global"] {
        detail.push(areas.join(", "));
    }
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
        detail.push(methods.join(", "));
    }
    Some(row(root, item, label, detail, class))
}

/// The wiring view from `whatis --json` and `plugins --json`, as magequery
/// 0.24 prints them (its `Whatis` and `Vec<Plugin>`, serialised by serde
/// with the field names as written in its source). Every field is read on
/// its own and a missing one is skipped.
/// Whether a `whatis` answer has any of the fields the wiring is read from.
fn understood(whatis: &Value) -> bool {
    ["class", "kind", "plugin_count", "resolves_to", "module"]
        .iter()
        .any(|key| whatis.get(key).is_some_and(|value| !value.is_null()))
}

pub fn read_wiring(root: &Path, fqn: &str, whatis: &Value, plugins: Option<&Value>) -> Wiring {
    let asked = fqn.trim().trim_start_matches('\\');
    let class = text(whatis, "class").unwrap_or_else(|| asked.to_string());
    let mut kind: Vec<String> = text(whatis, "kind").into_iter().collect();
    if whatis["is_virtual_type"].as_bool() == Some(true) {
        kind.push("virtual type".to_string());
    }
    let mut wiring = Wiring {
        kind: (!kind.is_empty()).then(|| kind.join(" · ")),
        module: text(whatis, "module"),
        path: text(whatis, "file").and_then(|file| relative(root, &file)),
        preference: text(whatis, "resolves_to"),
        sections: Vec::new(),
        class,
    };
    let mut push = |found: Option<Section>| {
        if let Some(found) = found {
            wiring.sections.push(found);
        }
    };
    // A virtual type: the real class it is made of.
    push(classes(
        "Instantiates",
        text(whatis, "instantiates").into_iter().collect(),
    ));
    let listed = plugins
        .and_then(|plugins| section("Plugins", plugins, |item| plugin_row(root, asked, item)));
    let counted = whatis["plugin_count"].as_u64().unwrap_or(0);
    match listed {
        Some(found) => push(Some(found)),
        // `whatis` counted plugins that `plugins` did not list (it failed).
        None if counted > 0 => push(Some(Section {
            title: "Plugins".to_string(),
            rows: vec![Row {
                label: format!("{counted} (magequery could not list them)"),
                ..Row::default()
            }],
        })),
        None => {}
    }
    let uses = &whatis["uses"];
    let named = |item: &Value| {
        let name = text(item, "name")?;
        Some(row(root, item, name.clone(), Vec::new(), Some(name)))
    };
    push(section("Preference for", &uses["preferred_for"], named));
    push(section("Virtual types", &uses["virtual_types"], |item| {
        // A virtual type has no file of its own: the row opens its declaration.
        Some(row(root, item, text(item, "name")?, Vec::new(), None))
    }));
    push(section("Injected into", &uses["injections"], |item| {
        let consumer = text(item, "consumer")?;
        let mut detail: Vec<String> = Vec::new();
        if let Some(argument) = text(item, "argument") {
            let keys: String = strings(item, "item_path")
                .iter()
                .map(|key| format!("[{key}]"))
                .collect();
            detail.push(format!("${argument}{keys}"));
        }
        if let Some(declared) = text(item, "declared") {
            if !declared.eq_ignore_ascii_case(asked) {
                detail.push(format!("as {}", short(&declared)));
            }
        }
        if item["as_string"].as_bool() == Some(true) {
            detail.push("by name".to_string());
        }
        let is_virtual = item["consumer_is_virtual"].as_bool() == Some(true);
        if is_virtual {
            detail.push("virtual type".to_string());
        }
        let class = (!is_virtual).then(|| consumer.clone());
        Some(row(root, item, consumer, detail, class))
    }));
    push(section("Observes", &whatis["observes"], |item| {
        let mut label = text(item, "event")?;
        if item["disabled"].as_bool() == Some(true) {
            label.push_str(" (disabled)");
        }
        Some(row(
            root,
            item,
            label,
            text(item, "name").into_iter().collect(),
            None,
        ))
    }));
    push(section("Cron jobs", &whatis["cron_jobs"], |item| {
        let detail = [
            text(item, "group"),
            text(item, "method"),
            text(item, "schedule").or_else(|| text(item, "config_path")),
        ];
        Some(row(
            root,
            item,
            text(item, "name")?,
            detail.into_iter().flatten().collect(),
            None,
        ))
    }));
    push(section("Web API", &whatis["webapi"], |item| {
        let mut detail: Vec<String> = [text(item, "method"), text(item, "service_method")]
            .into_iter()
            .flatten()
            .collect();
        let resources = strings(item, "resources");
        if !resources.is_empty() {
            detail.push(resources.join(", "));
        }
        Some(row(root, item, text(item, "url")?, detail, None))
    }));
    // `mq` and `graphql` are magequery's `ClassRef`: a `role` sentence
    // ("@resolver of Query.products") and a source.
    let role = |item: &Value| Some(row(root, item, text(item, "role")?, Vec::new(), None));
    push(section("Message queue", &whatis["mq"], role));
    push(section("GraphQL", &whatis["graphql"], role));
    push(section("URLs", &whatis["action_urls"], |item| {
        Some(row(
            root,
            item,
            text(item, "url")?,
            text(item, "area").into_iter().collect(),
            None,
        ))
    }));
    if whatis["command"].is_object() {
        let commands = Value::Array(vec![whatis["command"].clone()]);
        push(section("Console command", &commands, |item| {
            // `name` is null when the class builds it at run time.
            let label = text(item, "name").or_else(|| text(item, "item_key"))?;
            Some(row(
                root,
                item,
                label,
                text(item, "description").into_iter().collect(),
                None,
            ))
        }));
    }
    push(classes("Extends", strings(whatis, "parents")));
    push(classes("Implements", strings(whatis, "interfaces")));
    wiring
}

/// Runs `whatis` and `plugins` for a class and puts the answers together.
pub fn wiring(binary: &Path, root: &Path, fqn: &str) -> Result<Wiring, String> {
    let fqn = fqn.trim();
    if !valid_class(fqn) {
        return Err(format!("{fqn} is not a class name"));
    }
    let root_arg = root.to_string_lossy().into_owned();
    let args = |command: &[&str]| -> Vec<String> {
        let mut args: Vec<String> = command.iter().map(|part| part.to_string()).collect();
        args.extend([
            "--json".to_string(),
            "-r".to_string(),
            root_arg.clone(),
            "--".to_string(),
            fqn.trim_start_matches('\\').to_string(),
        ]);
        args
    };
    let whatis_args = args(&["whatis"]);
    // Every area: a plugin declared in `etc/frontend/di.xml` or
    // `etc/adminhtml/di.xml` is not in the global list.
    let plugins_args = args(&["plugins", "--all-areas"]);
    let binary_for_plugins = binary.to_path_buf();
    // Both at once: each reads the whole installation.
    let plugins = std::thread::spawn(move || run(&binary_for_plugins, &plugins_args));
    let whatis = run(binary, &whatis_args)?;
    let whatis: Value = serde_json::from_str(&whatis)
        .map_err(|e| format!("magequery's answer was not the JSON expected: {e}"))?;
    // Every field is optional to the reader, so an answer in some other shape
    // (another version, another tool by that name) read as a class nothing
    // touches, which is a confident wrong answer.
    if !understood(&whatis) {
        return Err(format!(
            "magequery answered in a shape gitnoob does not know. This build expects \
             magequery {PINNED}; check the one Settings → Frameworks points at."
        ));
    }
    let plugins = plugins
        .join()
        .ok()
        .and_then(|found| found.ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    Ok(read_wiring(root, fqn, &whatis, plugins.as_ref()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_setting_that_finds_nothing_is_named_in_the_error() {
        assert!(super::not_found(Some("~/bin/magequery")).contains("~/bin/magequery"));
        assert!(super::not_found(Some("  ")).contains("names none"));
        assert!(super::not_found(None).contains("names none"));
    }

    #[test]
    fn an_answer_without_any_known_field_is_not_read_as_an_empty_class() {
        assert!(!super::understood(&serde_json::json!({})));
        assert!(!super::understood(
            &serde_json::json!({ "error": "unknown command" })
        ));
        assert!(super::understood(
            &serde_json::json!({ "class": "Acme\\Foo" })
        ));
        assert!(super::understood(&serde_json::json!({ "plugin_count": 0 })));
    }

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
        assert!(!valid_class(&"A".repeat(513)));
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
