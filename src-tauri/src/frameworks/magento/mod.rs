//! Magento 2: class names in XML config, template and asset strings, layout
//! handles, events, virtual types and generated classes.
//!
//! The component registry (`registry.rs`) says which module and theme is in
//! which folder; everything here is worked out from it and from the file the
//! click was in. Classes are found through Composer's maps (`deps.rs`), then
//! the module a class is named after, then the symbol index.

pub mod registry;

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::xml::{self, At};
use super::{
    target, utf16_at, CompletionItem, Completions, FrameworkInfo, Links, Provider, Target,
};
use crate::deps;
use registry::{registry, Registry};

pub struct Magento {
    version: Option<String>,
    how: &'static str,
}

fn is_file(root: &Path, relative: &str) -> bool {
    crate::files::resolve(root, relative, true).is_ok_and(|real| real.is_file())
}

/// A Magento 2 project, a Magento checkout, or one Magento module: `bin/magento`
/// with `app/etc/di.xml`, a `composer.json` that requires Magento, or a
/// `registration.php` that registers a component next to `etc/module.xml`.
pub fn detect(root: &Path) -> Option<Arc<dyn Provider>> {
    let how = if is_file(root, "bin/magento") && is_file(root, "app/etc/di.xml") {
        "bin/magento"
    } else if requires_magento(root) {
        "composer.json"
    } else if is_file(root, "etc/module.xml")
        && deps::read(root, "registration.php")
            .is_some_and(|text| text.contains("ComponentRegistrar::register"))
    {
        "module"
    } else {
        return None;
    };
    Some(Arc::new(Magento {
        version: version(root),
        how,
    }))
}

fn requires_magento(root: &Path) -> bool {
    let Some(json) = deps::read(root, "composer.json")
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    else {
        return false;
    };
    ["require", "require-dev"].iter().any(|section| {
        json[section].as_object().is_some_and(|map| {
            map.keys()
                .any(|name| name == "magento/framework" || name.starts_with("magento/product-"))
        })
    })
}

/// The Magento version `composer.lock` pins: a `magento/product-*` package,
/// else `magento/framework`. Read as text: the lock is large and only these
/// two strings matter.
pub fn version(root: &Path) -> Option<String> {
    let lock = deps::read(root, "composer.lock")?;
    let find = |name_starts: &str| -> Option<String> {
        let mut rest = lock.as_str();
        while let Some(at) = rest.find("\"name\"") {
            let after = &rest[at + 6..];
            rest = after;
            // `"name"` can also be a word in a list of keywords: not a key then.
            let Some(value) = after
                .trim_start()
                .strip_prefix(':')
                .and_then(|value| value.trim_start().strip_prefix('"'))
            else {
                continue;
            };
            let name = &value[..value.find('"')?];
            if !name.starts_with(name_starts) {
                continue;
            }
            // The version is the next one in the same package.
            let at = rest.find("\"version\"")?;
            let value = rest[at + 9..]
                .trim_start()
                .strip_prefix(':')?
                .trim_start()
                .strip_prefix('"')?;
            return Some(
                value[..value.find('"')?]
                    .trim_start_matches('v')
                    .to_string(),
            );
        }
        None
    };
    find("magento/product-").or_else(|| find("magento/framework"))
}

// --- what a token is --------------------------------------------------------------

/// `Foo\Bar\Baz` as written in config, without a leading `\` or a
/// `::method` after it: `Some` when it is shaped like a namespaced class.
pub fn class_token(value: &str) -> Option<&str> {
    let value = value.trim().trim_start_matches('\\');
    let value = value.split("::").next().unwrap_or(value);
    let shaped = !value.is_empty()
        && value.contains('\\')
        && value.split('\\').all(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    shaped.then_some(value)
}

/// A class-like name: `Foo\Bar`, or a single capitalised word.
fn class_like(value: &str) -> Option<&str> {
    let value = value.trim().trim_start_matches('\\');
    if let Some(class) = class_token(value) {
        return Some(class);
    }
    let mut chars = value.chars();
    (chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_'))
    .then_some(value)
}

/// The classes Magento generates a name from, best first, with what kind of
/// generated class it is: `FooFactory` from `Foo`, `Foo\Proxy` and
/// `Foo\Interceptor` from `Foo`, `FooExtensionInterface` and `FooExtension`
/// from `FooInterface`, `FooSearchResults` from `FooSearchResultsInterface`.
pub fn generated_sources(fqn: &str) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    if let Some(base) = fqn.strip_suffix("\\Proxy") {
        out.push((base.to_string(), "proxy"));
    } else if let Some(base) = fqn.strip_suffix("\\Interceptor") {
        out.push((base.to_string(), "interceptor"));
    } else if let Some(base) = fqn.strip_suffix("ExtensionInterface") {
        out.push((format!("{base}Interface"), "extension interface"));
    } else if let Some(base) = fqn.strip_suffix("Extension") {
        // `Foo\Extension` is a class called Extension, not one made from `Foo\`.
        if !base.is_empty() && !base.ends_with('\\') {
            out.push((format!("{base}Interface"), "extension"));
        }
    } else if fqn.ends_with("SearchResults") {
        out.push((format!("{fqn}Interface"), "search results"));
    } else if let Some(base) = fqn.strip_suffix("Factory") {
        out.push((base.to_string(), "factory"));
    }
    out.retain(|(base, _)| !base.is_empty() && !base.ends_with('\\'));
    out
}

/// Every class a generated name leads back to, nearest first:
/// `Foo\BarInterfaceFactory` gives `Foo\BarInterface`,
/// `Foo\BarExtensionInterfaceFactory` gives `Foo\BarExtensionInterface`
/// and then `Foo\BarInterface`, `Foo\Bar\ProxyFactory` gives `Foo\Bar\Proxy`
/// and then `Foo\Bar`.
pub fn generated_chain(fqn: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut name = fqn.to_string();
    for _ in 0..3 {
        let Some((source, _)) = generated_sources(&name).into_iter().next() else {
            break;
        };
        out.push(source.clone());
        name = source;
    }
    out
}

/// A plain identifier: a virtual type can be named like one (`orderGrid`).
fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn short(fqn: &str) -> &str {
    fqn.rsplit('\\').next().unwrap_or(fqn)
}

/// The area a file is about, from its path: `adminhtml` for `/adminhtml/`,
/// `Block/Adminhtml` and the like, `frontend` for `/frontend/`. `None` when
/// the path does not say, or says `base`.
pub fn area_of(path: &str) -> Option<&'static str> {
    if path.contains("/adminhtml/") || path.contains("/Adminhtml/") {
        Some("adminhtml")
    } else if path.contains("/frontend/") {
        Some("frontend")
    } else {
        None
    }
}

const AREAS: [&str; 2] = ["frontend", "adminhtml"];

/// The attributes whose value is a class.
const CLASS_ATTRS: &[&str] = &[
    "type",
    "for",
    "class",
    "instance",
    "model",
    "source_model",
    "backend_model",
    "frontend_model",
    "handler",
    "consumerInstance",
    "provider",
    "entity",
];

/// The elements whose text is a class.
const CLASS_ELEMENTS: &[&str] = &[
    "source_model",
    "backend_model",
    "frontend_model",
    "class",
    "instance",
    "model",
];

/// The attributes next to `method=` that name its class.
const METHOD_OWNERS: &[&str] = &["instance", "class", "type", "model", "service"];

/// `Vendor_Module` at the start of `Vendor_Module::path` or `Vendor_Module/path`.
fn module_prefix(value: &str) -> Option<(&str, &str, bool)> {
    let (module, rest, template) = if let Some((module, rest)) = value.split_once("::") {
        (module, rest, true)
    } else {
        let (module, rest) = value.split_once('/')?;
        (module, rest, false)
    };
    let (vendor, name) = module.split_once('_')?;
    let word = |part: &str| {
        part.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && part.chars().all(|c| c.is_ascii_alphanumeric())
    };
    (word(vendor) && word(name) && !rest.is_empty() && !rest.contains(char::is_whitespace))
        .then_some((module, rest, template))
}

/// Whether a value is shaped like a template or an asset: `x.phtml`,
/// `Vendor_Module::path`, `Vendor_Module/path`, with or without RequireJS's
/// `text!` before it.
fn view_shaped(value: &str) -> bool {
    let value = value.trim();
    let value = value.strip_prefix("text!").unwrap_or(value);
    value.ends_with(".phtml") || module_prefix(value).is_some()
}

/// The quoted string around `byte` on its line, and where its inside is.
fn string_at(text: &str, byte: usize) -> Option<(&str, usize, usize)> {
    let byte = byte.min(text.len());
    let line_start = text[..byte].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text[byte..].find('\n').map_or(text.len(), |at| byte + at);
    let line = &text[line_start..line_end];
    let at = byte - line_start;
    let mut open: Option<(usize, u8)> = None;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match open {
            None if c == b'\'' || c == b'"' => open = Some((i, c)),
            Some((_, quote)) if c == b'\\' && quote != 0 => {
                i += 1;
            }
            Some((from, quote)) if c == quote => {
                if from < at && at <= i {
                    let inside = &line[from + 1..i];
                    return Some((inside, line_start + from + 1, line_start + i));
                }
                open = None;
            }
            _ => {}
        }
        i += 1;
    }
    // Pairing quotes from the start of the line goes wrong after a lone
    // apostrophe (`Don't` in a template's HTML). The nearest quote on either
    // side of the caret, with no space or quote between, is still a string.
    for quote in ['\'', '"'] {
        let Some(open) = line[..at].rfind(quote) else {
            continue;
        };
        let Some(close) = line[at..].find(quote).map(|close| at + close) else {
            continue;
        };
        let inside = &line[open + 1..close];
        if !inside.is_empty()
            && !inside.contains(|c: char| c.is_whitespace() || c == '\'' || c == '"')
        {
            return Some((inside, line_start + open + 1, line_start + close));
        }
    }
    None
}

// --- the provider -----------------------------------------------------------------

impl Provider for Magento {
    fn info(&self) -> FrameworkInfo {
        FrameworkInfo {
            id: "magento",
            name: "Magento",
            version: self.version.clone(),
            how: self.how,
            wiring: true,
        }
    }

    fn links_at(&self, root: &Path, path: &str, text: &str, byte: usize) -> Option<Links> {
        let registry = registry(root);
        let lower = path.to_ascii_lowercase();
        let mut links = if lower.ends_with(".xml") {
            xml_links(root, &registry, path, text, byte)
        } else if lower.ends_with(".php")
            || lower.ends_with(".phtml")
            || lower.ends_with(".js")
            || lower.ends_with(".json")
            || lower.ends_with(".html")
        {
            string_links(root, &registry, path, text, byte)
        } else {
            None
        }?;
        links.from = utf16_at(text, links.from as usize);
        links.to = utf16_at(text, links.to as usize);
        Some(links)
    }

    fn locate_class(&self, root: &Path, fqn: &str) -> Option<Target> {
        locate_class(root, &registry(root), fqn)
    }

    fn extra_references(&self, root: &Path, fqn: &str) -> Vec<Target> {
        let registry = registry(root);
        config_places(root, &registry, fqn)
            .into_iter()
            .map(|place| target(root, &place.path, place.line, place.column, "config", None))
            .collect()
    }

    /// Only the number, for the usage counts: no file is read for its lines.
    fn extra_reference_count(&self, root: &Path, fqn: &str) -> usize {
        config_places(root, &registry(root), fqn).len()
    }

    fn references_label(&self) -> &'static str {
        "Magento config"
    }

    fn complete(&self, root: &Path, path: &str, text: &str, byte: usize) -> Option<Completions> {
        if !path.to_ascii_lowercase().ends_with(".xml") {
            return None;
        }
        let doc = xml::parse(text);
        let start = match doc.at(byte)? {
            At::Attr(element, attr) => {
                let named = attr.name == "name" && matches!(element.name, "type" | "virtualType");
                if !CLASS_ATTRS.contains(&attr.name) && !named {
                    return None;
                }
                attr.start
            }
            At::Text(Some(element), text) => {
                let object = element
                    .attr("xsi:type")
                    .is_some_and(|a| a.value == "object");
                if !object && !CLASS_ELEMENTS.contains(&element.local()) {
                    return None;
                }
                text.start
            }
            At::Text(None, _) => return None,
        };
        let typed = &text[start..byte.max(start)];
        let skip = typed.len() - typed.trim_start_matches(['\\', ' ']).len();
        let typed = &typed[skip..];
        if typed.contains(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '\\')) {
            return None;
        }
        let registry = registry(root);
        Some(Completions {
            from: utf16_at(text, start + skip),
            items: class_completions(root, &registry, path, typed),
        })
    }

    fn wiring(
        &self,
        root: &Path,
        fqn: &str,
        tool: Option<&Path>,
    ) -> Result<Option<super::magequery::Wiring>, String> {
        let Some(tool) = tool else {
            return Ok(None);
        };
        super::magequery::wiring(tool, root, fqn).map(Some)
    }

    fn forget(&self, root: &Path) {
        registry::forget(root);
    }

    fn stale(&self, relative: &str) -> bool {
        stale(relative)
    }
}

/// Whether a changed file makes the registry or its indexes stale: the files
/// it is built from, and XML where config, layout and UI components are
/// read from. Not an IDE's settings (`.idea/workspace.xml` is written all the
/// time) or what Magento writes itself (`var/`, `generated/`, `pub/static/`).
fn stale(relative: &str) -> bool {
    let parts: Vec<&str> = relative.split('/').collect();
    if matches!(
        parts.first().copied(),
        Some(".idea" | ".vscode" | ".git" | "var" | "generated" | "pub" | "node_modules")
    ) {
        return false;
    }
    let name = parts.last().copied().unwrap_or(relative);
    if matches!(
        name,
        "registration.php" | "theme.xml" | "composer.lock" | "installed.json" | "config.php"
    ) {
        return true;
    }
    name.ends_with(".xml")
        && parts[..parts.len() - 1]
            .iter()
            .any(|part| matches!(*part, "etc" | "layout" | "ui_component"))
}

// --- classes ------------------------------------------------------------------------

/// Where a class is: Composer's maps, then the module it is named after,
/// then the symbol index.
fn locate_class(root: &Path, registry: &Registry, fqn: &str) -> Option<Target> {
    let found = deps::class_location(root, fqn, None).or_else(|| {
        let path = registry.class_path(root, fqn)?;
        deps::class_location(root, fqn, Some(&path)).or(Some((path, 1, 1)))
    })?;
    let (path, line, column) = found;
    Some(target(root, &path, line, column, "class", None))
}

fn locate_method(root: &Path, registry: &Registry, fqn: &str, method: &str) -> Option<Target> {
    let hint = registry.class_path(root, fqn);
    let (path, line, column) = deps::method_location(root, fqn, None, method)
        .or_else(|| deps::method_location(root, fqn, hint.as_deref(), method))?;
    Some(target(root, &path, line, column, "method", None))
}

/// The places config names a class, without the ones that name a class of
/// its own that only looks generated (`Foo\BarFactory` written by hand is
/// no reference to `Foo\Bar`).
fn config_places(root: &Path, registry: &Registry, fqn: &str) -> Vec<registry::Place> {
    let key = fqn.trim_start_matches('\\').to_ascii_lowercase();
    let Some(places) = registry.config_refs(root).get(&key) else {
        return Vec::new();
    };
    let mut real: std::collections::HashMap<&str, bool> = std::collections::HashMap::new();
    places
        .iter()
        .filter(|place| match place.written.as_deref() {
            Some(written) => !*real
                .entry(written)
                .or_insert_with(|| locate_class(root, registry, written).is_some()),
            None => true,
        })
        .cloned()
        .collect()
}

/// A ⌘-click on a class name in config: the class, a virtual type of that
/// name (both, when a virtual type is named like a class), else the class a
/// generated name is made from. `Foo\Bar::baz` is that method.
fn class_link(
    root: &Path,
    registry: &Registry,
    value: &str,
    from: usize,
    to: usize,
) -> Option<Links> {
    let written = value.trim().trim_start_matches('\\');
    if let Some((class, method)) = written.split_once("::") {
        if class_token(class).is_some() && identifier(method) {
            return method_link(root, registry, class, method, from, to);
        }
    }
    // A virtual type can be any name; a class is `Foo\Bar` or a capitalised word.
    let fqn = match class_like(written) {
        Some(fqn) => fqn,
        None if identifier(written) => {
            let places = registry.virtual_types(root).get(written)?;
            return Some(Links {
                from: from as u32,
                to: to as u32,
                title: format!("Virtual type {written}"),
                targets: virtual_targets(root, places, None),
                ..Links::default()
            });
        }
        None => return None,
    };
    let mut links = Links {
        from: from as u32,
        to: to as u32,
        title: fqn.to_string(),
        class: Some(fqn.to_string()),
        ..Links::default()
    };
    let class = locate_class(root, registry, fqn);
    let virtual_types = registry.virtual_types(root).get(fqn);
    match (class, virtual_types) {
        (Some(mut found), Some(places)) => {
            // The object manager hands out the virtual type for this name.
            links.targets = virtual_targets(root, places, Some("virtual type"));
            found.label = Some("class".to_string());
            links.targets.push(found);
            links.title = format!("{fqn} · {}", links.targets.len());
            links.note = Some(format!(
                "{} is a class and also declared as a virtual type",
                short(fqn)
            ));
            return Some(links);
        }
        (Some(found), None) => {
            links.targets.push(found);
            return Some(links);
        }
        (None, Some(places)) => {
            links.title = format!("Virtual type {fqn}");
            links.class = None;
            links.targets = virtual_targets(root, places, None);
            return Some(links);
        }
        (None, None) => {}
    }
    let kind = generated_sources(fqn).first().map(|(_, kind)| *kind);
    for source in generated_chain(fqn) {
        if let Some(found) = locate_class(root, registry, &source) {
            links.note = Some(format!(
                "{} is generated ({}); opened {}",
                short(fqn),
                kind.unwrap_or("generated"),
                short(&source)
            ));
            links.class = Some(source);
            links.targets.push(found);
            return Some(links);
        }
    }
    // A single word that is no class is just a word.
    if !fqn.contains('\\') {
        return None;
    }
    links.note = Some(format!("No class {fqn} in the project or its dependencies"));
    Some(links)
}

fn virtual_targets(root: &Path, places: &[registry::Place], label: Option<&str>) -> Vec<Target> {
    places
        .iter()
        .map(|place| {
            target(
                root,
                &place.path,
                place.line,
                place.column,
                "virtualType",
                label.map(str::to_string),
            )
        })
        .collect()
}

fn method_link(
    root: &Path,
    registry: &Registry,
    class: &str,
    method: &str,
    from: usize,
    to: usize,
) -> Option<Links> {
    let fqn = class_like(class)?;
    let mut links = Links {
        from: from as u32,
        to: to as u32,
        title: format!("{fqn}::{method}()"),
        class: Some(fqn.to_string()),
        ..Links::default()
    };
    if let Some(found) = locate_method(root, registry, fqn, method) {
        links.targets.push(found);
    } else if let Some(found) = locate_class(root, registry, fqn) {
        links.note = Some(format!(
            "{} has no method {method}(); opened the class",
            short(fqn)
        ));
        links.targets.push(found);
    } else {
        links.note = Some(format!("No class {fqn} in the project or its dependencies"));
    }
    Some(links)
}

// --- templates and assets ---------------------------------------------------------------

/// The files a `Vendor_Module::path` (or a bare `path`, in the module of the
/// referring file) can be: the module's own for the area and `base`, then
/// each theme's override, parents first.
fn view_link(
    root: &Path,
    registry: &Registry,
    from_path: &str,
    value: &str,
    from: usize,
    to: usize,
) -> Option<Links> {
    let value = value.trim();
    // RequireJS's text plugin: `text!Vendor_Module/template/x.html`.
    let (value, from) = match value.strip_prefix("text!") {
        Some(rest) => (rest, from + 5),
        None => (value, from),
    };
    let extension = |rest: &str| {
        rest.rsplit('/')
            .next()
            .is_some_and(|last| last.contains('.'))
    };
    // A file type a view file can have. `jquery.min` or `slick.min` in a
    // RequireJS `paths` entry has a dot but no type: `.js` is left off there.
    let typed = |rest: &str| {
        rest.rsplit_once('.').is_some_and(|(_, ext)| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "js" | "html"
                    | "phtml"
                    | "css"
                    | "less"
                    | "json"
                    | "svg"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "ico"
                    | "woff"
                    | "woff2"
                    | "ttf"
                    | "eot"
            )
        })
    };
    let (module_name, rest, folder) = match module_prefix(value) {
        // `Vendor_Module::something` with no file extension is an ACL
        // resource or a menu id, not a file.
        Some((_, rest, true)) if !extension(rest) => return None,
        Some((module, rest, true)) => (
            module.to_string(),
            rest,
            if rest.ends_with(".phtml") {
                "templates"
            } else {
                "web"
            },
        ),
        Some((module, rest, false)) => (module.to_string(), rest, "web"),
        None if value.ends_with(".phtml") => (
            registry.module_of(from_path)?.name.clone(),
            value,
            "templates",
        ),
        None => return None,
    };
    let module = registry.module(&module_name);
    // A file in a theme is of that theme's area, whatever its path says
    // (`vendor/magento/theme-frontend-luma/…` has no `/frontend/` in it).
    let theme_area = registry
        .theme_of(from_path)
        .map(|theme| theme.area.as_str());
    let areas: Vec<&str> = match theme_area.or(area_of(from_path)) {
        Some(area) => vec![area],
        None => AREAS.to_vec(),
    };
    let names: Vec<String> = if folder == "web" && !typed(rest) {
        vec![
            format!("{rest}.js"),
            format!("{rest}.html"),
            // A Knockout template: `Vendor_Module/path` is `web/template/path.html`.
            format!("template/{rest}.html"),
            rest.to_string(),
        ]
    } else {
        vec![rest.to_string()]
    };
    let module_areas: Vec<&str> = areas.iter().copied().chain(["base"]).collect();
    let targets = view_files(
        root,
        registry,
        &module_name,
        folder,
        &names,
        &module_areas,
        &areas,
    );
    let what = if folder == "templates" {
        "Template"
    } else {
        "File"
    };
    let mut links = Links {
        from: from as u32,
        to: to as u32,
        title: format!("{what} {module_name}::{rest} · {}", targets.len()),
        targets,
        ..Links::default()
    };
    if links.targets.is_empty() {
        links.note = Some(if module.is_none() {
            format!("No module {module_name} is registered")
        } else {
            format!(
                "No {} {rest} in {module_name} or a theme",
                what.to_ascii_lowercase()
            )
        });
    }
    Some(links)
}

/// The files `names` can be in `folder` of a module's view: the module's
/// own for each of `module_areas` in turn, then each theme's override for
/// `theme_areas`, parents first. Only the ones that are there.
fn view_files(
    root: &Path,
    registry: &Registry,
    module_name: &str,
    folder: &str,
    names: &[String],
    module_areas: &[&str],
    theme_areas: &[&str],
) -> Vec<Target> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut targets: Vec<Target> = Vec::new();
    let mut push = |path: String, label: String, targets: &mut Vec<Target>| {
        // `..` in what was written is worked out here, so the path handed
        // back is the file's own; one that climbs out of the tree is nothing.
        let Some(path) = deps::normal(&path) else {
            return;
        };
        if seen.insert(path.clone()) && is_file(root, &path) {
            targets.push(target(root, &path, 1, 1, kind_of(folder), Some(label)));
        }
    };
    if let Some(module) = registry.module(module_name) {
        for area in module_areas {
            for name in names {
                let path = registry::join(&module.dir, &format!("view/{area}/{folder}/{name}"));
                push(path, format!("{area} · module"), &mut targets);
            }
        }
    }
    for theme in registry.themes_in(theme_areas) {
        for name in names {
            let path = registry::join(&theme.dir, &format!("{module_name}/{folder}/{name}"));
            push(
                path,
                format!(
                    "{} · {}{} · override",
                    theme.area,
                    theme.name,
                    inherits(theme)
                ),
                &mut targets,
            );
        }
    }
    targets
}

/// ` (inherits luma)` after a theme with a parent.
fn inherits(theme: &registry::Theme) -> String {
    theme
        .parent
        .as_deref()
        .and_then(|key| key.split_once('/'))
        .map(|(_, name)| format!(" (inherits {name})"))
        .unwrap_or_default()
}

/// An email template's `file=` in `email_templates.xml`: the module's
/// `view/<area>/email/<file>`, then `base`, then the other area, then each
/// theme's `<Vendor_Module>/email/<file>` for the area.
fn email_link(
    root: &Path,
    registry: &Registry,
    from_path: &str,
    element: &xml::Element,
    file: &str,
    from: usize,
    to: usize,
) -> Option<Links> {
    let module_name = match element.attr("module") {
        Some(attr) => attr.value.trim().to_string(),
        None => registry.module_of(from_path)?.name.clone(),
    };
    let area = element
        .attr("area")
        .map(|attr| attr.value.trim())
        .and_then(|area| AREAS.iter().copied().find(|known| *known == area));
    let (module_areas, theme_areas): (Vec<&str>, Vec<&str>) = match area {
        Some(area) => {
            let other = AREAS.iter().copied().filter(|one| *one != area);
            (
                std::iter::once(area).chain(["base"]).chain(other).collect(),
                vec![area],
            )
        }
        None => (["base"].into_iter().chain(AREAS).collect(), AREAS.to_vec()),
    };
    let names = [file.to_string()];
    let targets = view_files(
        root,
        registry,
        &module_name,
        "email",
        &names,
        &module_areas,
        &theme_areas,
    );
    let module = registry.module(&module_name);
    let mut links = Links {
        from: from as u32,
        to: to as u32,
        title: format!("Email template {module_name}::{file} · {}", targets.len()),
        targets,
        ..Links::default()
    };
    if links.targets.is_empty() {
        links.note = Some(if module.is_none() {
            format!("No module {module_name} is registered")
        } else {
            format!("No email template {file} in {module_name} or a theme")
        });
    }
    Some(links)
}

/// A `module="Vendor_Module"` attribute (email templates, menus, widgets):
/// the module's `etc/module.xml` and `registration.php`.
fn module_link(
    root: &Path,
    registry: &Registry,
    name: &str,
    from: usize,
    to: usize,
) -> Option<Links> {
    let (vendor, rest) = name.split_once('_')?;
    if vendor.is_empty()
        || rest.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    let module = registry.module(name);
    let targets: Vec<Target> = module
        .map(|module| {
            ["etc/module.xml", "registration.php"]
                .iter()
                .map(|file| registry::join(&module.dir, file))
                .filter(|path| is_file(root, path))
                .map(|path| {
                    target(
                        root,
                        &path,
                        1,
                        1,
                        "config",
                        Some(format!("{name} · module")),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let mut links = Links {
        from: from as u32,
        to: to as u32,
        title: format!("Module {name} · {}", targets.len()),
        targets,
        ..Links::default()
    };
    if links.targets.is_empty() {
        links.note = Some(format!("No module {name} is registered"));
    }
    Some(links)
}

/// A layout's `<css src>` or `<script src>` with no `Vendor_Module::`: a
/// file of the theme (`web/<src>`, or the `.less` it is built from) or of
/// `lib/web`.
fn asset_link(
    root: &Path,
    registry: &Registry,
    from_path: &str,
    src: &str,
    from: usize,
    to: usize,
) -> Option<Links> {
    if src.contains("://") || src.starts_with("//") || !src.contains('.') {
        return None;
    }
    let theme_area = registry
        .theme_of(from_path)
        .map(|theme| theme.area.as_str());
    let areas: Vec<&str> = match theme_area.or(area_of(from_path)) {
        Some(area) => vec![area],
        None => AREAS.to_vec(),
    };
    let mut names = vec![src.to_string()];
    if let Some(stem) = src.strip_suffix(".css") {
        names.push(format!("{stem}.less"));
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut targets: Vec<Target> = Vec::new();
    let mut push = |path: String, label: String| {
        let Some(path) = deps::normal(&path) else {
            return;
        };
        if seen.insert(path.clone()) && is_file(root, &path) {
            targets.push(target(root, &path, 1, 1, "asset", Some(label)));
        }
    };
    for theme in registry.themes_in(&areas) {
        for name in &names {
            push(
                registry::join(&theme.dir, &format!("web/{name}")),
                format!("{} · {}{}", theme.area, theme.name, inherits(theme)),
            );
        }
    }
    for lib in ["lib/web", "vendor/magento/magento2-base/lib/web"] {
        for name in &names {
            push(format!("{lib}/{name}"), "library".to_string());
        }
    }
    let mut links = Links {
        from: from as u32,
        to: to as u32,
        title: format!("File {src} · {}", targets.len()),
        targets,
        ..Links::default()
    };
    if links.targets.is_empty() {
        links.note = Some(format!("No {src} in a theme or lib/web"));
    }
    Some(links)
}

fn kind_of(folder: &str) -> &'static str {
    if folder == "templates" || folder == "email" {
        "template"
    } else {
        "asset"
    }
}

// --- layout ---------------------------------------------------------------------------

fn handle_link(root: &Path, registry: &Registry, handle: &str, from: usize, to: usize) -> Links {
    let file = format!("{handle}.xml");
    let targets: Vec<Target> = registry
        .layouts(root)
        .iter()
        .filter(|layout| layout.path.rsplit('/').next() == Some(file.as_str()))
        .map(|layout| {
            let label = match &layout.theme {
                Some(theme) => format!("{} · {theme}", layout.area),
                None => format!(
                    "{} · {}",
                    layout.area,
                    registry
                        .module_of(&layout.path)
                        .map_or("", |module| module.name.as_str())
                ),
            };
            target(root, &layout.path, 1, 1, "layout", Some(label))
        })
        .collect();
    Links {
        from: from as u32,
        to: to as u32,
        title: format!("Layout handle {handle} · {}", targets.len()),
        note: targets
            .is_empty()
            .then(|| format!("No layout file is called {file}")),
        targets,
        class: None,
    }
}

fn name_link(root: &Path, registry: &Registry, name: &str, from: usize, to: usize) -> Links {
    let targets: Vec<Target> = registry
        .layout_names(root)
        .get(name)
        .map(|places| {
            places
                .iter()
                .map(|place| {
                    let label = registry
                        .module_of(&place.path)
                        .map(|module| module.name.clone());
                    target(root, &place.path, place.line, place.column, "layout", label)
                })
                .collect()
        })
        .unwrap_or_default();
    Links {
        from: from as u32,
        to: to as u32,
        title: format!("Declarations of {name} · {}", targets.len()),
        note: targets
            .is_empty()
            .then(|| format!("No layout file declares a block or container called {name}")),
        targets,
        class: None,
    }
}

// --- events ---------------------------------------------------------------------------

fn observers_link(root: &Path, registry: &Registry, event: &str, from: usize, to: usize) -> Links {
    let mut targets = Vec::new();
    for observer in registry.observers(root).get(event).into_iter().flatten() {
        let mut label = format!("{} · {}", observer.area, observer.name);
        if observer.disabled {
            label.push_str(" · disabled");
        }
        let found = (!observer.instance.is_empty())
            .then(|| locate_class(root, registry, &observer.instance))
            .flatten();
        targets.push(match found {
            Some(mut found) => {
                found.kind = "observer";
                found.label = Some(label);
                found
            }
            None => target(
                root,
                &observer.path,
                observer.line,
                observer.column,
                "observer",
                Some(label),
            ),
        });
    }
    Links {
        from: from as u32,
        to: to as u32,
        title: format!("Observers of {event} · {}", targets.len()),
        note: targets
            .is_empty()
            .then(|| format!("No module observes {event}")),
        targets,
        class: None,
    }
}

/// How long and how far a search for dispatch sites goes.
const DISPATCH_FILES: usize = 60_000;
const DISPATCH_HITS: usize = 100;
const DISPATCH_TIME: Duration = Duration::from_secs(6);

/// The largest PHP file a dispatch search reads.
const DISPATCH_FILE_SIZE: u64 = 2 * 1024 * 1024;

/// Where an event is dispatched by name: `'event'` or `"event"` on a line
/// with `dispatch`, in the project's PHP and in every vendor module's.
/// Listing the files, reading them and the hits all stop at their limits;
/// the second value says whether one of them was reached.
pub fn dispatch_sites(root: &Path, registry: &Registry, event: &str) -> (Vec<Target>, bool) {
    let needles = [format!("'{event}'"), format!("\"{event}\"")];
    let started = Instant::now();
    let late = || started.elapsed() > DISPATCH_TIME;
    let mut cut = false;
    let mut seen: HashSet<String> = HashSet::new();
    let mut paths: Vec<String> = Vec::new();
    if let Ok(all) = crate::files::all_files(root) {
        for path in all.iter() {
            if paths.len() >= DISPATCH_FILES {
                cut = true;
                break;
            }
            if (path.ends_with(".php") || path.ends_with(".phtml")) && seen.insert(path.clone()) {
                paths.push(path.clone());
            }
        }
    }
    let base = root.canonicalize().ok();
    'modules: for module in registry.modules.iter().filter(|module| module.vendor) {
        // A vendor module is a folder under vendor/, never the work tree itself.
        if module.dir.is_empty() {
            continue;
        }
        let (Ok(real), Some(base)) = (crate::files::resolve(root, &module.dir, true), &base) else {
            continue;
        };
        let walk = ignore::WalkBuilder::new(&real)
            .standard_filters(false)
            .follow_links(false)
            .max_depth(Some(24))
            .build();
        for entry in walk.flatten() {
            if paths.len() >= DISPATCH_FILES || late() {
                cut = true;
                break 'modules;
            }
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext == "php" || ext == "phtml")
            {
                if let Ok(inner) = path.strip_prefix(base) {
                    let inner = crate::files::slashed(inner);
                    // Already listed when vendor/ is not ignored.
                    if seen.insert(inner.clone()) {
                        paths.push(inner);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    'files: for path in &paths {
        if late() {
            cut = true;
            break;
        }
        let Some(text) = registry::read_capped(root, path, DISPATCH_FILE_SIZE) else {
            continue;
        };
        if !needles.iter().any(|needle| text.contains(needle.as_str())) {
            continue;
        }
        for (at, line) in text.lines().enumerate() {
            if line.contains("dispatch")
                && needles.iter().any(|needle| line.contains(needle.as_str()))
            {
                if out.len() >= DISPATCH_HITS {
                    cut = true;
                    break 'files;
                }
                let column = line.find("dispatch").map_or(1, |c| c as u32 + 1);
                out.push(target(root, path, at as u32 + 1, column, "dispatch", None));
            }
        }
    }
    (out, cut)
}

fn dispatch_link(root: &Path, registry: &Registry, event: &str, from: usize, to: usize) -> Links {
    let (targets, cut) = dispatch_sites(root, registry, event);
    let note = if targets.is_empty() {
        Some(format!(
            "Nothing dispatches '{event}' by name. Names built at run time, like an _eventPrefix, are not looked through."
        ))
    } else if cut {
        Some("The search stopped early; there may be more.".to_string())
    } else {
        None
    };
    Links {
        from: from as u32,
        to: to as u32,
        title: format!("Dispatches of {event} · {}", targets.len()),
        note,
        targets,
        class: None,
    }
}

// --- reading a click --------------------------------------------------------------

fn xml_links(
    root: &Path,
    registry: &Registry,
    path: &str,
    text: &str,
    byte: usize,
) -> Option<Links> {
    let doc = xml::parse(text);
    match doc.at(byte)? {
        At::Attr(element, attr) => {
            let value = attr.value.trim();
            let (from, to) = (attr.start, attr.end);
            if value.is_empty() {
                return None;
            }
            let local = element.local();
            match attr.name {
                "template" => view_link(root, registry, path, value, from, to),
                "file" if local == "template" && path.ends_with("email_templates.xml") => {
                    email_link(root, registry, path, element, value, from, to)
                }
                "module" => module_link(root, registry, value, from, to)
                    .or_else(|| loose(root, registry, path, value, from, to)),
                "src" if matches!(local, "css" | "script" | "link") && !view_shaped(value) => {
                    asset_link(root, registry, path, value, from, to)
                }
                "method" => {
                    let class = METHOD_OWNERS.iter().find_map(|owner| element.attr(owner))?;
                    method_link(root, registry, class.value, value, from, to)
                }
                "handle" if local == "update" => Some(handle_link(root, registry, value, from, to)),
                "name" if matches!(local, "referenceBlock" | "referenceContainer") => {
                    Some(name_link(root, registry, value, from, to))
                }
                "element" | "destination" if local == "move" => {
                    Some(name_link(root, registry, value, from, to))
                }
                "name" if local == "event" => Some(dispatch_link(root, registry, value, from, to)),
                "name" if local == "virtualType" => {
                    let base = element.attr("type")?.value;
                    let mut links = class_link(root, registry, base, from, to)?;
                    if !links.targets.is_empty() {
                        links.note = Some(format!(
                            "{} is a virtual type of {}",
                            short(value),
                            short(base)
                        ));
                    }
                    Some(links)
                }
                "name" if local == "type" => class_link(root, registry, value, from, to),
                "component" => view_link(root, registry, path, value, from, to),
                name if CLASS_ATTRS.contains(&name) => class_link(root, registry, value, from, to),
                _ => loose(root, registry, path, value, from, to),
            }
        }
        At::Text(parent, node) => {
            let value = node.value.trim();
            let (from, to) = (node.start, node.end);
            let parent = parent?;
            let object = parent.attr("xsi:type").is_some_and(|a| a.value == "object");
            let named = parent.attr("name").map(|a| a.value);
            if object || CLASS_ELEMENTS.contains(&parent.local()) {
                return class_link(root, registry, value, from, to);
            }
            if named == Some("template") || named == Some("component") {
                return view_link(root, registry, path, value, from, to);
            }
            loose(root, registry, path, value, from, to)
        }
    }
}

/// Any other attribute or text: a template, an asset or a namespaced class
/// when it is shaped like one.
fn loose(
    root: &Path,
    registry: &Registry,
    path: &str,
    value: &str,
    from: usize,
    to: usize,
) -> Option<Links> {
    if view_shaped(value) {
        return view_link(root, registry, path, value, from, to);
    }
    class_token(value)?;
    class_link(root, registry, value, from, to)
}

/// A string in PHP, a template, a script or JSON: a template or asset, or an
/// event name being dispatched.
fn string_links(
    root: &Path,
    registry: &Registry,
    path: &str,
    text: &str,
    byte: usize,
) -> Option<Links> {
    let (value, from, to) = string_at(text, byte)?;
    let before = text[..from.saturating_sub(1)].trim_end();
    if before.ends_with("dispatch(") || before.ends_with("dispatch (") {
        return Some(observers_link(root, registry, value, from, to));
    }
    if view_shaped(value) {
        return view_link(root, registry, path, value, from, to);
    }
    None
}

// --- completion ---------------------------------------------------------------------

const COMPLETION_LIMIT: usize = 200;

/// Classes and namespaces for a partly typed `Vendor\Module\Na`: the
/// folders Composer's PSR-4 maps and the registered modules give for the
/// namespace typed so far, and the project's classes from the index.
fn class_completions(
    root: &Path,
    registry: &Registry,
    path: &str,
    typed: &str,
) -> Vec<CompletionItem> {
    let (space, partial) = match typed.rfind('\\') {
        Some(at) => (&typed[..at], &typed[at + 1..]),
        None => ("", typed),
    };
    let lower = partial.to_ascii_lowercase();
    let mut items: Vec<CompletionItem> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut add = |items: &mut Vec<CompletionItem>,
                   label: String,
                   kind: &'static str,
                   detail: Option<String>| {
        if items.len() < COMPLETION_LIMIT && seen.insert(label.clone()) {
            items.push(CompletionItem {
                label,
                kind,
                detail,
            });
        }
    };
    let prefix = if space.is_empty() {
        String::new()
    } else {
        format!("{space}\\")
    };
    // The folders that hold the namespace typed so far.
    let mut folders: Vec<String> = Vec::new();
    for (namespace, dirs) in deps::psr4_prefixes(root) {
        if let Some(rest) = prefix.strip_prefix(namespace.as_str()) {
            for dir in &dirs {
                folders.push(registry::join(dir, &rest.replace('\\', "/")));
            }
        } else if namespace.starts_with(&prefix) {
            // The next part of a namespace Composer knows.
            let next = namespace[prefix.len()..].split('\\').next().unwrap_or("");
            if !next.is_empty() && next.to_ascii_lowercase().starts_with(&lower) {
                add(&mut items, format!("{prefix}{next}\\"), "namespace", None);
            }
        }
    }
    for module in &registry.modules {
        let Some((vendor, name)) = module.name.split_once('_') else {
            continue;
        };
        let space_of = format!("{vendor}\\{name}\\");
        if let Some(rest) = prefix.strip_prefix(space_of.as_str()) {
            folders.push(registry::join(&module.dir, &rest.replace('\\', "/")));
        } else if space_of.starts_with(&prefix) {
            let next = space_of[prefix.len()..].split('\\').next().unwrap_or("");
            if !next.is_empty() && next.to_ascii_lowercase().starts_with(&lower) {
                add(&mut items, format!("{prefix}{next}\\"), "namespace", None);
            }
        }
    }
    folders.sort();
    folders.dedup();
    for folder in folders {
        for (name, dir) in registry::list(root, &folder) {
            if !name.to_ascii_lowercase().starts_with(&lower) {
                continue;
            }
            if dir {
                if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                    add(&mut items, format!("{prefix}{name}\\"), "namespace", None);
                }
            } else if let Some(class) = name.strip_suffix(".php") {
                add(
                    &mut items,
                    format!("{prefix}{class}"),
                    "class",
                    Some(folder.clone()),
                );
            }
        }
    }
    // The project's own classes by their short name, wherever they are.
    if partial.len() >= 2 {
        for found in crate::complete::complete_symbols(root, partial, path, 40, &[]) {
            if found.kind != "class" && found.kind != "interface" {
                continue;
            }
            let Some(text) = deps::read(root, &found.path) else {
                continue;
            };
            let space = deps::php_imports(&text).namespace;
            let fqn = if space.is_empty() {
                found.name.clone()
            } else {
                format!("{space}\\{}", found.name)
            };
            if fqn
                .to_ascii_lowercase()
                .starts_with(&typed.to_ascii_lowercase())
                || prefix.is_empty()
            {
                add(&mut items, fqn, "class", Some(found.path.clone()));
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_names_point_at_their_source() {
        assert_eq!(
            generated_sources("Magento\\Sales\\Model\\OrderFactory"),
            vec![("Magento\\Sales\\Model\\Order".to_string(), "factory")]
        );
        assert_eq!(
            generated_sources("Foo\\Bar\\Proxy")[0].0,
            "Foo\\Bar".to_string()
        );
        assert_eq!(
            generated_sources("Magento\\Catalog\\Api\\Data\\ProductExtensionInterface")[0].0,
            "Magento\\Catalog\\Api\\Data\\ProductInterface".to_string()
        );
        assert!(generated_sources("Foo\\Bar").is_empty());
    }

    #[test]
    fn tells_class_names_from_other_values() {
        assert_eq!(class_token("\\Foo\\Bar"), Some("Foo\\Bar"));
        assert_eq!(class_token("Foo\\Bar::execute"), Some("Foo\\Bar"));
        assert_eq!(class_token("select"), None);
        assert_eq!(class_token("Foo\\Bar Baz"), None);
    }

    #[test]
    fn finds_the_string_under_the_caret() {
        let text = "protected $_template = 'A_B::x.phtml';";
        let at = text.find("x.phtml").unwrap();
        let (value, from, to) = string_at(text, at).unwrap();
        assert_eq!(value, "A_B::x.phtml");
        assert_eq!(&text[from..to], "A_B::x.phtml");
        assert!(string_at(text, 3).is_none());
    }

    #[test]
    fn only_config_changes_make_the_registry_stale() {
        for path in [
            "app/code/A/B/etc/di.xml",
            "app/code/A/B/etc/frontend/events.xml",
            "app/code/A/B/view/frontend/layout/default.xml",
            "app/code/A/B/view/adminhtml/ui_component/a_listing.xml",
            "app/design/frontend/V/t/A_B/layout/default.xml",
            "app/design/frontend/V/t/theme.xml",
            "app/code/A/B/registration.php",
            "etc/di.xml",
            "vendor/composer/installed.json",
            "composer.lock",
            "app/etc/config.php",
        ] {
            assert!(stale(path), "{path}");
        }
        for path in [
            ".idea/workspace.xml",
            "var/cache/x.xml",
            "generated/metadata/x.xml",
            "pub/static/frontend/V/t/en_US/x.xml",
            "phpunit.xml",
            "app/code/A/B/Model/X.php",
            "app/code/A/B/view/frontend/templates/x.phtml",
        ] {
            assert!(!stale(path), "{path}");
        }
    }

    #[test]
    fn reads_the_area_from_a_path() {
        assert_eq!(
            area_of("app/code/A/B/Block/Adminhtml/X.php"),
            Some("adminhtml")
        );
        assert_eq!(
            area_of("app/code/A/B/view/frontend/layout/x.xml"),
            Some("frontend")
        );
        assert_eq!(area_of("app/code/A/B/Model/X.php"), None);
    }

    #[test]
    fn reads_a_module_prefix() {
        assert_eq!(
            module_prefix("A_B::x/y.phtml"),
            Some(("A_B", "x/y.phtml", true))
        );
        assert_eq!(
            module_prefix("Magento_Ui/js/form"),
            Some(("Magento_Ui", "js/form", false))
        );
        assert_eq!(module_prefix("jquery/ui"), None);
    }
}
