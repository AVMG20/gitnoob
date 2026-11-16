//! Framework-aware navigation: what a framework wires together through
//! config files and strings, which the language alone cannot follow.
//!
//! A framework is a [`Provider`]. Each one has a `detect` function that looks
//! at a work tree with a few `stat` calls and says whether it is that
//! framework; the first one that says yes is kept for the work tree until a
//! file detection reads changes. Everything else is asked of the provider:
//!
//! - [`Provider::links_at`]: ⌘-click on a token in any file (a class name in
//!   XML, a template string, an event name), answered with the places it
//!   leads to.
//! - [`Provider::extra_references`]: config files that name a class, added to
//!   its usage count and keeping its methods from being marked unused.
//! - [`Provider::complete`]: names to offer inside config files.
//! - [`Provider::wiring`]: an outside tool's view of a class, when the
//!   framework has one (Magento's `magequery`).
//!
//! Magento is the only provider so far. Laravel or Nuxt would be a module
//! next to `magento`, with its own `detect`, added to [`DETECTORS`].

pub mod magento;
pub mod magequery;
pub mod xml;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use serde::Serialize;

use crate::state::AppState;

/// What the title bar says about a detected framework.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FrameworkInfo {
    /// `magento`.
    pub id: &'static str,
    /// `Magento`.
    pub name: &'static str,
    /// `2.4.7`, when a lock file says.
    pub version: Option<String>,
    /// How it was recognised: `bin/magento`, `composer.json`, `module`.
    pub how: &'static str,
    /// Whether an outside tool can describe a class's wiring here.
    pub wiring: bool,
}

/// A place a framework link leads to.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Target {
    pub path: String,
    /// From 1.
    pub line: u32,
    pub column: u32,
    /// `class`, `method`, `template`, `layout`, `observer`, `dispatch`,
    /// `virtualType`, `asset`, `config`.
    pub kind: &'static str,
    /// The line, trimmed, as the usages list shows one.
    pub text: String,
    /// In `vendor/`: opened read-only.
    pub dependency: bool,
    /// A short tag beside it: `adminhtml · module`, `frontend · Vendor/theme · override`.
    pub label: Option<String>,
}

/// What a ⌘-click on a framework token found.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Links {
    /// The token, in UTF-16 offsets, for the underline.
    pub from: u32,
    pub to: u32,
    /// The popup's title when there are several.
    pub title: String,
    /// Something to say, such as "OrderFactory is generated; opened Order",
    /// or why nothing was found.
    pub note: Option<String>,
    pub targets: Vec<Target>,
    /// The class the token names, for the "wiring" link under the popup.
    pub class: Option<String>,
}

/// A name to offer inside a config file.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    /// `class` or `namespace`.
    pub kind: &'static str,
    pub detail: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Completions {
    /// Where the typed name starts, UTF-16.
    pub from: u32,
    pub items: Vec<CompletionItem>,
}

/// One framework. Every method but `info` has a default that does nothing,
/// so a new provider can start with detection alone.
pub trait Provider: Send + Sync {
    fn info(&self) -> FrameworkInfo;

    /// Where the token at `byte` in `text` (the file `path`, relative) leads.
    /// `None` when there is no framework token there, so ⌘-click goes on to
    /// the language's own rules.
    fn links_at(&self, _root: &Path, _path: &str, _text: &str, _byte: usize) -> Option<Links> {
        None
    }

    /// Where a class is, by its fully qualified name, the framework's way.
    fn locate_class(&self, root: &Path, fqn: &str) -> Option<Target> {
        let (path, line, column) = crate::deps::class_location(root, fqn, None)?;
        Some(target(root, &path, line, column, "class", None))
    }

    /// Config files that name a class, by its fully qualified name.
    fn extra_references(&self, _root: &Path, _fqn: &str) -> Vec<Target> {
        Vec::new()
    }

    /// How many [`Provider::extra_references`] there are, when that is all
    /// that is wanted (the usage counts, on every keystroke's refresh).
    fn extra_reference_count(&self, root: &Path, fqn: &str) -> usize {
        self.extra_references(root, fqn).len()
    }

    /// What the group of [`Provider::extra_references`] is called.
    fn references_label(&self) -> &'static str {
        "Config"
    }

    fn complete(
        &self,
        _root: &Path,
        _path: &str,
        _text: &str,
        _byte: usize,
    ) -> Option<Completions> {
        None
    }

    /// An outside tool's account of how a class is wired, if there is one.
    fn wiring(
        &self,
        _root: &Path,
        _fqn: &str,
        _tool: Option<&Path>,
    ) -> Result<Option<magequery::Wiring>, String> {
        Ok(None)
    }

    /// Drops whatever was worked out about `root`, after files changed.
    fn forget(&self, _root: &Path) {}

    /// Whether a changed file (relative path) makes what was worked out stale.
    fn stale(&self, _relative: &str) -> bool {
        false
    }
}

type Detector = fn(&Path) -> Option<Arc<dyn Provider>>;

/// Every framework, asked in order.
const DETECTORS: &[Detector] = &[magento::detect];

/// The files detection reads, whose change means asking again.
const DETECTION_FILES: &[&str] = &[
    "composer.json",
    "composer.lock",
    "bin/magento",
    "registration.php",
];

type Stamp = Vec<Option<SystemTime>>;

type Detected = HashMap<PathBuf, (Stamp, Option<Arc<dyn Provider>>)>;

fn detected() -> &'static Mutex<Detected> {
    static DETECTED: OnceLock<Mutex<Detected>> = OnceLock::new();
    DETECTED.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key(root: &Path) -> PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
}

fn stamp_of(root: &Path) -> Stamp {
    DETECTION_FILES
        .iter()
        .map(|file| {
            std::fs::metadata(root.join(file))
                .and_then(|meta| meta.modified())
                .ok()
        })
        .collect()
}

/// The framework a work tree uses, if any: a few `stat` calls once the
/// answer is cached.
pub fn detect(root: &Path) -> Option<Arc<dyn Provider>> {
    let root = key(root);
    let stamp = stamp_of(&root);
    if let Ok(cache) = detected().lock() {
        if let Some((seen, found)) = cache.get(&root) {
            if *seen == stamp {
                return found.clone();
            }
        }
    }
    let found = DETECTORS.iter().find_map(|detector| detector(&root));
    if let Ok(mut cache) = detected().lock() {
        cache.insert(root, (stamp, found.clone()));
    }
    found
}

/// Whether Settings → Frameworks has framework support on.
pub fn enabled(state: &AppState) -> bool {
    state.config().global.frameworks
}

/// The framework of the open repository, when framework support is on.
pub fn active(state: &AppState) -> Option<(PathBuf, Arc<dyn Provider>)> {
    if !enabled(state) {
        return None;
    }
    let root = key(&state.path().ok()?);
    let provider = detect(&root)?;
    Some((root, provider))
}

/// What the title bar shows.
pub fn info(state: &AppState) -> Option<FrameworkInfo> {
    let (_, provider) = active(state)?;
    let mut info = provider.info();
    info.wiring = info.wiring && magequery::find(state).is_some();
    Some(info)
}

/// Where a ⌘-click on a framework token leads, in `text` or the file on disk.
pub fn links_at(
    state: &AppState,
    path: &str,
    offset: u32,
    text: Option<&str>,
) -> Result<Option<Links>, String> {
    let Some((root, provider)) = active(state) else {
        return Ok(None);
    };
    let text = match text {
        Some(text) => text.to_string(),
        None => crate::deps::read(&root, path).ok_or_else(|| format!("Could not read {path}"))?,
    };
    let byte = crate::deps::byte_at_utf16(&text, offset);
    Ok(provider.links_at(&root, path, &text, byte))
}

/// Config files that name the class `fqn`, with the label of their group.
pub fn extra_references(state: &AppState, fqn: &str) -> Option<(&'static str, Vec<Target>)> {
    let (root, provider) = active(state)?;
    Some((
        provider.references_label(),
        provider.extra_references(&root, fqn),
    ))
}

/// How many config files name the class `fqn`; `None` without a framework.
pub fn extra_reference_count(state: &AppState, fqn: &str) -> Option<usize> {
    let (root, provider) = active(state)?;
    Some(provider.extra_reference_count(&root, fqn))
}

/// Where a class is, for the wiring view's rows.
pub fn locate_class(state: &AppState, fqn: &str) -> Option<Target> {
    let (root, provider) = active(state)?;
    provider.locate_class(&root, fqn.trim().trim_start_matches('\\'))
}

pub fn complete(state: &AppState, path: &str, offset: u32, text: &str) -> Option<Completions> {
    let (root, provider) = active(state)?;
    let byte = crate::deps::byte_at_utf16(text, offset);
    provider.complete(&root, path, text, byte)
}

/// The watcher's news: files changed under `root`.
pub fn touched(root: &Path, paths: &HashSet<PathBuf>) {
    let root = key(root);
    let provider = match detected().lock() {
        Ok(cache) => cache.get(&root).and_then(|(_, found)| found.clone()),
        Err(_) => None,
    };
    let Some(provider) = provider else { return };
    let stale = paths.iter().any(|path| {
        let relative = path
            .strip_prefix(&root)
            .ok()
            .map(crate::files::slashed)
            .unwrap_or_else(|| crate::files::slashed(path));
        provider.stale(&relative)
    });
    if stale {
        provider.forget(&root);
    }
}

/// Forgets what was detected and worked out for a work tree, for tests.
pub fn forget(root: &Path) {
    let root = key(root);
    let found = match detected().lock() {
        Ok(mut cache) => cache.remove(&root).and_then(|(_, found)| found),
        Err(_) => None,
    };
    if let Some(provider) = found {
        provider.forget(&root);
    }
}

/// A target from a place in a file: the line's text, and whether it is in a
/// dependency.
pub fn target(
    root: &Path,
    path: &str,
    line: u32,
    column: u32,
    kind: &'static str,
    label: Option<String>,
) -> Target {
    Target {
        text: crate::deps::line_of(root, path, line),
        dependency: path.starts_with("vendor/") || crate::deps::is_dependency(root, path),
        path: path.to_string(),
        line,
        column,
        kind,
        label,
    }
}

/// A UTF-16 offset for a byte offset.
pub fn utf16_at(text: &str, byte: usize) -> u32 {
    text.get(..byte.min(text.len()))
        .map_or(0, |before| before.encode_utf16().count() as u32)
}
