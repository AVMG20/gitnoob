//! Magento's components: which module and theme lives in which folder, and
//! the indexes over their config files that ⌘-click needs.
//!
//! Modules come from `app/code/*/*/registration.php`, a `registration.php`
//! at the root (a repository that is one module), and the packages Composer
//! installed as `magento2-module`, read from `vendor/composer/installed.json`
//! with each package's own `registration.php`. Themes the same way, from
//! `app/design/{frontend,adminhtml}/*/*` and `magento2-theme` packages, with
//! each one's `theme.xml` parent. `vendor/` is never walked.
//!
//! The list is made on the first framework request and kept per work tree
//! until `composer.lock`, `installed.json` or `app/etc/config.php` change
//! (checked by time on every request) or the watcher reports a changed
//! `registration.php`, `theme.xml` or config XML. The indexes over config
//! files (virtual types, observers, layout files, which classes config names)
//! are built the first time each is asked for.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use crate::deps;
use crate::frameworks::xml;

/// One registered module.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// `Vendor_Module`.
    pub name: String,
    /// Its folder, relative, without a trailing slash; empty for the root.
    pub dir: String,
    pub vendor: bool,
}

/// One registered theme.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// `frontend/Vendor/name`.
    pub key: String,
    /// `frontend` or `adminhtml`.
    pub area: String,
    /// `Vendor/name`.
    pub name: String,
    pub dir: String,
    /// The parent's key, from `theme.xml`.
    pub parent: Option<String>,
    pub vendor: bool,
}

/// A place in a config file.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub path: String,
    pub line: u32,
    pub column: u32,
    /// For a virtual type, the type it is made from.
    pub base: Option<String>,
}

/// One observer of an event.
#[derive(Debug, Clone, PartialEq)]
pub struct Observer {
    pub event: String,
    pub name: String,
    pub instance: String,
    pub area: String,
    pub disabled: bool,
    pub path: String,
    pub line: u32,
    pub column: u32,
}

/// A layout file: its path and the area it is for.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutFile {
    pub path: String,
    pub area: String,
    /// The theme it belongs to, `Vendor/name`, when it is a theme's.
    pub theme: Option<String>,
}

#[derive(Debug, Default)]
pub struct Registry {
    pub modules: Vec<Module>,
    pub themes: Vec<Theme>,
    virtual_types: OnceLock<HashMap<String, Vec<Place>>>,
    observers: OnceLock<HashMap<String, Vec<Observer>>>,
    layouts: OnceLock<Vec<LayoutFile>>,
    layout_names: OnceLock<HashMap<String, Vec<Place>>>,
    config_refs: OnceLock<HashMap<String, Vec<Place>>>,
}

// --- reading the work tree, always through the path guard ---------------------

fn read(root: &Path, relative: &str) -> Option<String> {
    deps::read(root, relative)
}

fn is_file(root: &Path, relative: &str) -> bool {
    crate::files::resolve(root, relative, true).is_ok_and(|real| real.is_file())
}

/// The names in a folder, `(name, is_dir)`, sorted. Empty when it is not
/// there or leads out of the work tree.
pub fn list(root: &Path, dir: &str) -> Vec<(String, bool)> {
    let Ok(real) = crate::files::resolve(root, if dir.is_empty() { "." } else { dir }, true) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(real) else {
        return Vec::new();
    };
    let mut out: Vec<(String, bool)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let dir = entry.path().is_dir();
            (!name.starts_with('.')).then_some((name, dir))
        })
        .collect();
    out.sort();
    out
}

pub fn join(dir: &str, rest: &str) -> String {
    deps::join(dir, rest)
}

// --- registration.php and theme.xml -------------------------------------------------

/// The components a `registration.php` registers: `(kind, name)` with kind
/// `MODULE` or `THEME`.
pub fn registrations(text: &str) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("ComponentRegistrar::register") {
        let after = &rest[at + "ComponentRegistrar::register".len()..];
        let Some(open) = after.find('(') else { break };
        let args = &after[open + 1..];
        let close = args.find(')').unwrap_or(args.len());
        let mut parts = args[..close].splitn(3, ',');
        let kind = parts.next().unwrap_or("");
        let name = parts
            .next()
            .map(|name| {
                name.trim()
                    .trim_matches(|c| c == '\'' || c == '"')
                    .to_string()
            })
            .unwrap_or_default();
        let kind = if kind.contains("MODULE") {
            Some("MODULE")
        } else if kind.contains("THEME") {
            Some("THEME")
        } else {
            None
        };
        if let Some(kind) = kind {
            if !name.is_empty() {
                out.push((kind, name));
            }
        }
        rest = &args[close..];
    }
    out
}

/// The parent a `theme.xml` names, as `Vendor/name`.
fn theme_parent(text: &str) -> Option<String> {
    let doc = xml::parse(text);
    doc.texts
        .iter()
        .find(|text| {
            text.parent
                .and_then(|at| doc.elements.get(at))
                .is_some_and(|element| element.name == "parent")
        })
        .map(|text| text.value.to_string())
}

impl Registry {
    fn add(&mut self, root: &Path, registration: &str, vendor: bool) {
        let Some(text) = read(root, registration) else {
            return;
        };
        let dir = registration
            .rsplit_once('/')
            .map_or(String::new(), |(dir, _)| dir.to_string());
        for (kind, name) in registrations(&text) {
            if kind == "MODULE" {
                if !self.modules.iter().any(|module| module.name == name) {
                    self.modules.push(Module {
                        name,
                        dir: dir.clone(),
                        vendor,
                    });
                }
                continue;
            }
            let Some((area, theme)) = name.split_once('/') else {
                continue;
            };
            if self.themes.iter().any(|known| known.key == name) {
                continue;
            }
            let parent = read(root, &join(&dir, "theme.xml"))
                .and_then(|text| theme_parent(&text))
                .map(|parent| format!("{area}/{parent}"));
            self.themes.push(Theme {
                key: name.clone(),
                area: area.to_string(),
                name: theme.to_string(),
                dir: dir.clone(),
                parent,
                vendor,
            });
        }
    }

    /// Reads the components of a work tree. Cheap: folder listings two levels
    /// deep and one JSON file, never a walk.
    pub fn build(root: &Path) -> Registry {
        let mut registry = Registry::default();
        if is_file(root, "registration.php") {
            registry.add(root, "registration.php", false);
        }
        for (vendor, dir) in list(root, "app/code") {
            if !dir {
                continue;
            }
            for (module, dir) in list(root, &format!("app/code/{vendor}")) {
                let registration = format!("app/code/{vendor}/{module}/registration.php");
                if dir && is_file(root, &registration) {
                    registry.add(root, &registration, false);
                }
            }
        }
        for area in ["frontend", "adminhtml"] {
            for (vendor, dir) in list(root, &format!("app/design/{area}")) {
                if !dir {
                    continue;
                }
                for (theme, dir) in list(root, &format!("app/design/{area}/{vendor}")) {
                    let registration =
                        format!("app/design/{area}/{vendor}/{theme}/registration.php");
                    if dir && is_file(root, &registration) {
                        registry.add(root, &registration, false);
                    }
                }
            }
        }
        for registration in installed(root) {
            registry.add(root, &registration, true);
        }
        registry
    }

    pub fn module(&self, name: &str) -> Option<&Module> {
        self.modules.iter().find(|module| module.name == name)
    }

    /// The module a file belongs to: the one whose folder holds it, or, for
    /// a file in a theme, the `Vendor_Module` folder it sits in there.
    pub fn module_of(&self, path: &str) -> Option<&Module> {
        if let Some(theme) = self.theme_of(path) {
            let inner = path.strip_prefix(&theme.dir)?.trim_start_matches('/');
            let first = inner.split('/').next()?;
            if let Some(module) = self.module(first) {
                return Some(module);
            }
        }
        self.modules
            .iter()
            .filter(|module| {
                module.dir.is_empty()
                    || path == module.dir
                    || path.starts_with(&format!("{}/", module.dir))
            })
            .max_by_key(|module| module.dir.len())
    }

    pub fn theme_of(&self, path: &str) -> Option<&Theme> {
        self.themes
            .iter()
            .filter(|theme| path.starts_with(&format!("{}/", theme.dir)))
            .max_by_key(|theme| theme.dir.len())
    }

    pub fn theme(&self, key: &str) -> Option<&Theme> {
        self.themes.iter().find(|theme| theme.key == key)
    }

    /// How many parents a theme has, for putting parents before children.
    pub fn depth(&self, theme: &Theme) -> usize {
        let mut depth = 0;
        let mut seen: HashSet<&str> = HashSet::new();
        let mut parent = theme.parent.as_deref();
        while let Some(key) = parent {
            if !seen.insert(key) || depth > 20 {
                break;
            }
            depth += 1;
            parent = self.theme(key).and_then(|theme| theme.parent.as_deref());
        }
        depth
    }

    /// The themes of the given areas, parents first.
    pub fn themes_in(&self, areas: &[&str]) -> Vec<&Theme> {
        let mut themes: Vec<&Theme> = self
            .themes
            .iter()
            .filter(|theme| areas.contains(&theme.area.as_str()))
            .collect();
        themes.sort_by_key(|theme| (self.depth(theme), theme.vendor, theme.key.clone()));
        themes.dedup_by(|a, b| a.key == b.key);
        themes
    }

    /// The file a class would be in going by the module it is named after:
    /// `Vendor\Module\Model\Foo` in `<Vendor_Module>/Model/Foo.php`.
    pub fn class_path(&self, root: &Path, fqn: &str) -> Option<String> {
        let mut parts = fqn.trim_start_matches('\\').split('\\');
        let vendor = parts.next()?;
        let module = parts.next()?;
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() {
            return None;
        }
        let module = self.module(&format!("{vendor}_{module}"))?;
        let path = join(&module.dir, &format!("{}.php", rest.join("/")));
        is_file(root, &path).then_some(path)
    }

    /// The config files of every module in `etc/` (and one folder down, for
    /// the areas) whose name is `file`.
    fn etc_files(&self, root: &Path, file: &str, vendor: bool) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for module in self
            .modules
            .iter()
            .filter(|module| vendor || !module.vendor)
        {
            let etc = join(&module.dir, "etc");
            for (name, dir) in list(root, &etc) {
                if !dir && name == file {
                    out.push((join(&etc, &name), "global".to_string()));
                } else if dir {
                    let inner = join(&join(&etc, &name), file);
                    if is_file(root, &inner) {
                        out.push((inner, name));
                    }
                }
            }
        }
        if file == "di.xml" && is_file(root, "app/etc/di.xml") {
            out.push(("app/etc/di.xml".to_string(), "global".to_string()));
        }
        out
    }

    /// Every `<virtualType>` by name, in any module's `di.xml`.
    pub fn virtual_types(&self, root: &Path) -> &HashMap<String, Vec<Place>> {
        self.virtual_types.get_or_init(|| {
            let mut out: HashMap<String, Vec<Place>> = HashMap::new();
            for (path, _) in self.etc_files(root, "di.xml", true) {
                let Some(text) = read(root, &path) else {
                    continue;
                };
                if !text.contains("virtualType") {
                    continue;
                }
                let doc = xml::parse(&text);
                for element in doc.elements.iter().filter(|e| e.name == "virtualType") {
                    let Some(name) = element.attr("name") else {
                        continue;
                    };
                    let (line, column) = xml::line_col(&text, name.start);
                    out.entry(name.value.trim_start_matches('\\').to_string())
                        .or_default()
                        .push(Place {
                            path: path.clone(),
                            line,
                            column,
                            base: element
                                .attr("type")
                                .map(|base| base.value.trim_start_matches('\\').to_string()),
                        });
                }
            }
            out
        })
    }

    /// Every observer, by event name, from every area's `events.xml`.
    pub fn observers(&self, root: &Path) -> &HashMap<String, Vec<Observer>> {
        self.observers.get_or_init(|| {
            let mut out: HashMap<String, Vec<Observer>> = HashMap::new();
            for (path, area) in self.etc_files(root, "events.xml", true) {
                let Some(text) = read(root, &path) else {
                    continue;
                };
                let doc = xml::parse(&text);
                for element in doc.elements.iter().filter(|e| e.name == "observer") {
                    let Some(event) = doc
                        .parent_of(element)
                        .filter(|parent| parent.name == "event")
                        .and_then(|parent| parent.attr("name"))
                    else {
                        continue;
                    };
                    let instance = element.attr("instance").map_or("", |a| a.value);
                    let at = element.attr("instance").or(element.attr("name"));
                    let (line, column) =
                        xml::line_col(&text, at.map_or(element.start, |a| a.start));
                    out.entry(event.value.to_string())
                        .or_default()
                        .push(Observer {
                            event: event.value.to_string(),
                            name: element.attr("name").map_or("", |a| a.value).to_string(),
                            instance: instance.trim_start_matches('\\').to_string(),
                            area: area.clone(),
                            disabled: element.attr("disabled").is_some_and(|a| a.value == "true"),
                            path: path.clone(),
                            line,
                            column,
                        });
                }
            }
            out
        })
    }

    /// Every layout file: each module's `view/<area>/layout/*.xml` and each
    /// theme's `<Vendor_Module>/layout/*.xml`. Folder listings only.
    pub fn layouts(&self, root: &Path) -> &Vec<LayoutFile> {
        self.layouts.get_or_init(|| {
            let mut out = Vec::new();
            for module in &self.modules {
                let view = join(&module.dir, "view");
                for (area, dir) in list(root, &view) {
                    if !dir {
                        continue;
                    }
                    let layout = format!("{view}/{area}/layout");
                    for (name, dir) in list(root, &layout) {
                        if !dir && name.ends_with(".xml") {
                            out.push(LayoutFile {
                                path: format!("{layout}/{name}"),
                                area: area.clone(),
                                theme: None,
                            });
                        }
                    }
                }
            }
            for theme in &self.themes {
                for (folder, dir) in list(root, &theme.dir) {
                    if !dir || !folder.contains('_') {
                        continue;
                    }
                    let layout = format!("{}/{folder}/layout", theme.dir);
                    for (name, dir) in list(root, &layout) {
                        if !dir && name.ends_with(".xml") {
                            out.push(LayoutFile {
                                path: format!("{layout}/{name}"),
                                area: theme.area.clone(),
                                theme: Some(theme.name.clone()),
                            });
                        }
                    }
                }
            }
            out
        })
    }

    /// Where each block and container is declared (`<block name=…>`,
    /// `<container name=…>`), across every layout file.
    pub fn layout_names(&self, root: &Path) -> &HashMap<String, Vec<Place>> {
        self.layout_names.get_or_init(|| {
            let mut out: HashMap<String, Vec<Place>> = HashMap::new();
            for file in self.layouts(root) {
                let Some(text) = read(root, &file.path) else {
                    continue;
                };
                let doc = xml::parse(&text);
                for element in doc
                    .elements
                    .iter()
                    .filter(|e| e.name == "block" || e.name == "container")
                {
                    let Some(name) = element.attr("name") else {
                        continue;
                    };
                    let (line, column) = xml::line_col(&text, name.start);
                    out.entry(name.value.to_string()).or_default().push(Place {
                        path: file.path.clone(),
                        line,
                        column,
                        base: None,
                    });
                }
            }
            out
        })
    }

    /// The config files of the project's own modules and themes: what can
    /// name one of its classes. Not vendor's.
    fn project_config(&self, root: &Path) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for module in self.modules.iter().filter(|module| !module.vendor) {
            let etc = join(&module.dir, "etc");
            for (name, dir) in list(root, &etc) {
                if !dir && name.ends_with(".xml") {
                    out.push(join(&etc, &name));
                } else if dir {
                    let inner = join(&etc, &name);
                    for (file, dir) in list(root, &inner) {
                        if !dir && file.ends_with(".xml") {
                            out.push(join(&inner, &file));
                        }
                    }
                }
            }
            let view = join(&module.dir, "view");
            for (area, dir) in list(root, &view) {
                if !dir {
                    continue;
                }
                for kind in ["layout", "ui_component"] {
                    let folder = format!("{view}/{area}/{kind}");
                    for (file, dir) in list(root, &folder) {
                        if !dir && file.ends_with(".xml") {
                            out.push(format!("{folder}/{file}"));
                        }
                    }
                }
            }
        }
        for file in self.layouts(root) {
            if file.theme.is_some() && !file.path.starts_with("vendor/") {
                out.push(file.path.clone());
            }
        }
        if is_file(root, "app/etc/di.xml") {
            out.push("app/etc/di.xml".to_string());
        }
        out.sort();
        out.dedup();
        out
    }

    /// Which classes the project's config names, by lowercased fully
    /// qualified name. A generated name (`FooFactory`, `Foo\Proxy`) counts for
    /// the class it is made from too.
    pub fn config_refs(&self, root: &Path) -> &HashMap<String, Vec<Place>> {
        self.config_refs.get_or_init(|| {
            let mut out: HashMap<String, Vec<Place>> = HashMap::new();
            for path in self.project_config(root) {
                let Some(text) = read(root, &path) else {
                    continue;
                };
                if !text.contains('\\') {
                    continue;
                }
                let doc = xml::parse(&text);
                let mut found: Vec<(&str, usize)> = Vec::new();
                for element in &doc.elements {
                    for attr in &element.attrs {
                        found.push((attr.value, attr.start));
                    }
                }
                for text in &doc.texts {
                    found.push((text.value, text.start));
                }
                for (value, at) in found {
                    let Some(class) = super::class_token(value) else {
                        continue;
                    };
                    let (line, column) = xml::line_col(&text, at);
                    let place = Place {
                        path: path.clone(),
                        line,
                        column,
                        base: None,
                    };
                    let mut keys = vec![class.to_ascii_lowercase()];
                    for (source, _) in super::generated_sources(class) {
                        keys.push(source.to_ascii_lowercase());
                    }
                    keys.dedup();
                    for key in keys {
                        let list = out.entry(key).or_default();
                        if !list.contains(&place) {
                            list.push(place.clone());
                        }
                    }
                }
            }
            out
        })
    }
}

/// The `registration.php` of every Magento package Composer installed.
fn installed(root: &Path) -> Vec<String> {
    let Some(text) = read(root, "vendor/composer/installed.json") else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let packages = json
        .get("packages")
        .and_then(|packages| packages.as_array())
        .or_else(|| json.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for package in packages {
        let kind = package["type"].as_str().unwrap_or("");
        if kind != "magento2-module" && kind != "magento2-theme" {
            continue;
        }
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        let dir = package["install-path"]
            .as_str()
            .and_then(|path| deps::normal(&join("vendor/composer", path)))
            .unwrap_or_else(|| format!("vendor/{name}"));
        let mut files: Vec<String> = package["autoload"]["files"]
            .as_array()
            .map(|files| {
                files
                    .iter()
                    .filter_map(|file| file.as_str())
                    .filter(|file| file.ends_with("registration.php"))
                    .filter_map(|file| deps::normal(&join(&dir, file)))
                    .collect()
            })
            .unwrap_or_default();
        if files.is_empty() {
            files.push(join(&dir, "registration.php"));
        }
        out.extend(files);
    }
    out
}

// --- the cache -------------------------------------------------------------------

/// The files whose change means reading the components again, checked by
/// time on every request.
const STAMPED: [&str; 3] = [
    "composer.lock",
    "vendor/composer/installed.json",
    "app/etc/config.php",
];

type Stamp = Vec<Option<SystemTime>>;

type Cache = HashMap<PathBuf, (Stamp, Arc<Registry>)>;

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn stamp_of(root: &Path) -> Stamp {
    STAMPED
        .iter()
        .map(|file| {
            std::fs::metadata(root.join(file))
                .and_then(|meta| meta.modified())
                .ok()
        })
        .collect()
}

/// The registry of a work tree, read on first use.
pub fn registry(root: &Path) -> Arc<Registry> {
    let stamp = stamp_of(root);
    if let Ok(cached) = cache().lock() {
        if let Some((seen, registry)) = cached.get(root) {
            if *seen == stamp {
                return registry.clone();
            }
        }
    }
    let registry = Arc::new(Registry::build(root));
    if let Ok(mut cached) = cache().lock() {
        cached.insert(root.to_path_buf(), (stamp, registry.clone()));
    }
    registry
}

pub fn forget(root: &Path) {
    if let Ok(mut cached) = cache().lock() {
        cached.remove(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_a_registration_registers() {
        let text = r#"<?php
use Magento\Framework\Component\ComponentRegistrar;
ComponentRegistrar::register(ComponentRegistrar::MODULE, 'BigBridge_Foo', __DIR__);
\Magento\Framework\Component\ComponentRegistrar::register(
    \Magento\Framework\Component\ComponentRegistrar::THEME,
    "frontend/BigBridge/default",
    __DIR__
);"#;
        assert_eq!(
            registrations(text),
            vec![
                ("MODULE", "BigBridge_Foo".to_string()),
                ("THEME", "frontend/BigBridge/default".to_string())
            ]
        );
    }

    #[test]
    fn reads_a_themes_parent() {
        let text = "<theme><title>X</title><parent>Magento/luma</parent></theme>";
        assert_eq!(theme_parent(text), Some("Magento/luma".to_string()));
    }
}
