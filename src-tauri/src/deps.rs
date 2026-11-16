//! Definitions in dependencies: ⌘-click from the project into `vendor/` and
//! `node_modules/`, worked out when it is asked for.
//!
//! Dependencies stay out of the symbol index (`symbols.rs`): they would swamp
//! the usage counts and the dimming of unused code, and walking them at every
//! open would cost more than the whole project. Instead a name is resolved
//! the way the language itself does it, one file at a time:
//!
//! - PHP: the file's namespace and `use` imports give the fully qualified
//!   name ([`php_imports`]), Composer's own autoload maps give the file
//!   ([`resolve_class`]), and tree-sitter finds the class and its members in
//!   it, walking up `extends`, traits and interfaces ([`class_members`]).
//!   Laravel facades are followed to the class they stand for where the
//!   facade says which it is.
//! - TypeScript and JavaScript: a bare import goes through `node_modules`,
//!   the package's `types`, `exports` or `main`, and the exported name is
//!   found in that file, one `export … from` deep.
//! - Rust: the crates are in cargo's registry, outside the work tree, so they
//!   are not followed.
//!
//! Every path is checked by [`files::resolve`], so a link that leads out of
//! the work tree (a Composer path repository, a pnpm store elsewhere) is
//! refused rather than followed.
//!
//! The autoload maps are read on the first ⌘-click and kept per work tree
//! until a file in `vendor/composer/` or `composer.json` changes.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use serde::Serialize;
use tree_sitter::{Node, Parser, Tree};

use crate::files;

/// How far up `extends`, traits and interfaces a member is looked for.
const MAX_DEPTH: usize = 10;

/// The Laravel class every facade extends.
const FACADE: &str = "Illuminate\\Support\\Facades\\Facade";

// --- where a ⌘-click lands ----------------------------------------------------

/// A definition found outside the index.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Jump {
    pub path: String,
    /// From 1.
    pub line: u32,
    pub column: u32,
    pub kind: &'static str,
    /// The line, trimmed, as the usages list shows one.
    pub text: String,
    /// In `vendor/` or `node_modules/`: opened read-only.
    pub dependency: bool,
    /// Something to say about where it landed, such as a facade whose method
    /// lives elsewhere.
    pub note: Option<String>,
}

/// Whether a path is a dependency: inside a `vendor` or `node_modules`
/// folder that git ignores. Read-only in the editor, never counted.
pub fn is_dependency(root: &Path, relative: &str) -> bool {
    let inside = relative
        .split('/')
        .any(|part| part == "vendor" || part == "node_modules");
    inside && files::path_ignored_at(root, relative).unwrap_or(false)
}

/// Where the name at `offset` (UTF-16, in `text` or the file on disk) is
/// defined, when that is in a dependency or the language's own rules say
/// exactly which file. `None` leaves it to the index.
pub fn dependency_definition(
    root: &Path,
    path: &str,
    offset: u32,
    text: Option<&str>,
) -> Result<Option<Jump>, String> {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let text = match text {
        Some(text) => text.to_string(),
        None => {
            let real = files::resolve(root, path, true)?;
            String::from_utf8_lossy(&std::fs::read(&real).map_err(|e| e.to_string())?).into_owned()
        }
    };
    let byte = byte_at_utf16(&text, offset);
    let found = match ext.as_str() {
        "php" => php_jump(root, path, &text, byte),
        "ts" | "mts" | "cts" | "tsx" | "js" | "mjs" | "cjs" | "jsx" | "vue" => {
            let direct = if ext == "vue" {
                None
            } else {
                script_jump(root, path, &text, byte, &ext)
            };
            // A member on a chain, through the return types it declares.
            direct.or_else(|| {
                crate::complete::script_member_definition(root, path, &text, byte)
                    .map(|(path, line, column)| (path, line, column, "method", None))
            })
        }
        _ => None,
    };
    Ok(found.map(|(path, line, column, kind, note)| Jump {
        text: line_of(root, &path, line),
        dependency: is_dependency(root, &path),
        path,
        line,
        column,
        kind,
        note,
    }))
}

type Landing = (String, u32, u32, &'static str, Option<String>);

pub(crate) fn byte_at_utf16(text: &str, offset: u32) -> usize {
    let mut units = 0u32;
    for (at, c) in text.char_indices() {
        if units >= offset {
            return at;
        }
        units += c.len_utf16() as u32;
    }
    text.len()
}

pub(crate) fn line_of(root: &Path, path: &str, line: u32) -> String {
    let Ok(real) = files::resolve(root, path, true) else {
        return String::new();
    };
    let text = std::fs::read(real).unwrap_or_default();
    String::from_utf8_lossy(&text)
        .lines()
        .nth(line.saturating_sub(1) as usize)
        .map(|one| one.trim().chars().take(200).collect())
        .unwrap_or_default()
}

fn text_of<'a>(node: Node, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

fn named_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

/// A path inside the work tree, `..` worked out, or `None` if it leaves it.
pub(crate) fn normal(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// A file that is there and, once links are followed, inside the work tree.
fn exists(root: &Path, relative: &str) -> bool {
    files::resolve(root, relative, true)
        .map(|real| real.is_file())
        .unwrap_or(false)
}

fn is_dir(root: &Path, relative: &str) -> bool {
    files::resolve(root, relative, true)
        .map(|real| real.is_dir())
        .unwrap_or(false)
}

pub(crate) fn read(root: &Path, relative: &str) -> Option<String> {
    let real = files::resolve(root, relative, true).ok()?;
    let bytes = std::fs::read(real).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

// --- PHP: names -----------------------------------------------------------------

pub(crate) fn parse_php(text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .ok()?;
    parser.parse(text, None)
}

/// A PHP file's namespace and the classes it imports.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct PhpImports {
    pub namespace: String,
    /// The fully qualified name of each imported class, by the name the file
    /// uses for it: its alias, or the last part of the name.
    pub classes: HashMap<String, String>,
}

impl PhpImports {
    /// The fully qualified name of a class as written in this file: `\A\B` as
    /// it is, an imported name or alias through its `use`, anything else in
    /// the file's namespace.
    pub fn qualify(&self, written: &str) -> String {
        let written = written.trim();
        if let Some(absolute) = written.strip_prefix('\\') {
            return absolute.to_string();
        }
        let (first, rest) = match written.split_once('\\') {
            Some((first, rest)) => (first, Some(rest)),
            None => (written, None),
        };
        // Class names are not case-sensitive in PHP; an exact match first.
        let imported = self.classes.get(first).or_else(|| {
            self.classes
                .iter()
                .find(|(alias, _)| alias.eq_ignore_ascii_case(first))
                .map(|(_, fqn)| fqn)
        });
        if let Some(fqn) = imported {
            return match rest {
                Some(rest) => format!("{fqn}\\{rest}"),
                None => fqn.clone(),
            };
        }
        if self.namespace.is_empty() {
            written.to_string()
        } else {
            format!("{}\\{written}", self.namespace)
        }
    }
}

/// The namespace and class imports of a PHP file: plain, aliased and grouped
/// `use`, but not `use function` or `use const`.
pub fn php_imports(text: &str) -> PhpImports {
    match parse_php(text) {
        Some(tree) => imports_in(tree.root_node(), text),
        None => PhpImports::default(),
    }
}

fn imports_in(root: Node, source: &str) -> PhpImports {
    let mut found = PhpImports::default();
    let mut statements = named_children(root);
    let mut at = 0;
    while at < statements.len() {
        let node = statements[at];
        at += 1;
        match node.kind() {
            "namespace_definition" => {
                if let Some(name) = node.child_by_field_name("name") {
                    found.namespace = text_of(name, source).trim_start_matches('\\').to_string();
                }
                // `namespace A { … }`: what is inside counts too.
                if let Some(body) = node.child_by_field_name("body") {
                    statements.extend(named_children(body));
                }
            }
            "namespace_use_declaration" => {
                for (alias, fqn) in use_clauses(node, source) {
                    found.classes.insert(alias, fqn);
                }
            }
            _ => {}
        }
    }
    found
}

/// Each class a `use` declaration imports, as (the name it goes by, the fully
/// qualified name). Nothing for `use function` and `use const`.
fn use_clauses(node: Node, source: &str) -> Vec<(String, String)> {
    let after = text_of(node, source)
        .trim_start()
        .trim_start_matches("use")
        .trim_start();
    if after.starts_with("function ") || after.starts_with("const ") {
        return Vec::new();
    }
    let prefix = named_children(node)
        .into_iter()
        .find(|child| child.kind() == "namespace_name")
        .map(|child| text_of(child, source).trim_start_matches('\\').to_string());
    let mut clauses: Vec<Node> = Vec::new();
    for child in named_children(node) {
        match child.kind() {
            "namespace_use_clause" => clauses.push(child),
            "namespace_use_group" => clauses.extend(
                named_children(child)
                    .into_iter()
                    .filter(|one| one.kind() == "namespace_use_clause"),
            ),
            _ => {}
        }
    }
    clauses
        .into_iter()
        .filter_map(|clause| clause_import(clause, prefix.as_deref(), source))
        .collect()
}

fn clause_import(clause: Node, prefix: Option<&str>, source: &str) -> Option<(String, String)> {
    let alias = clause.child_by_field_name("alias");
    let name = named_children(clause)
        .into_iter()
        .find(|child| matches!(child.kind(), "qualified_name" | "name") && Some(*child) != alias)?;
    let written = text_of(name, source).trim_start_matches('\\');
    let fqn = match prefix {
        Some(prefix) => format!("{prefix}\\{written}"),
        None => written.to_string(),
    };
    let goes_by = match alias {
        Some(alias) => text_of(alias, source).to_string(),
        None => fqn.rsplit('\\').next().unwrap_or(&fqn).to_string(),
    };
    Some((goes_by, fqn))
}

// --- PHP: Composer's autoload maps -----------------------------------------------

#[derive(Default, Debug)]
struct Autoload {
    /// Class to file.
    classmap: HashMap<String, String>,
    /// Namespace prefix (with its trailing `\`) to folders, longest first.
    psr4: Vec<(String, Vec<String>)>,
    psr0: Vec<(String, Vec<String>)>,
}

const AUTOLOAD_FILES: [&str; 4] = [
    "vendor/composer/autoload_classmap.php",
    "vendor/composer/autoload_psr4.php",
    "vendor/composer/autoload_psr0.php",
    "composer.json",
];

type Stamp = Vec<Option<SystemTime>>;

type Loaded = HashMap<PathBuf, (Stamp, Arc<Autoload>)>;

fn maps() -> &'static Mutex<Loaded> {
    static MAPS: OnceLock<Mutex<Loaded>> = OnceLock::new();
    MAPS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn stamp_of(root: &Path) -> Stamp {
    AUTOLOAD_FILES
        .iter()
        .map(|file| {
            std::fs::metadata(root.join(file))
                .and_then(|meta| meta.modified())
                .ok()
        })
        .collect()
}

/// The autoload maps of a work tree: read once, and again when Composer
/// rewrote them.
fn autoload(root: &Path) -> Arc<Autoload> {
    let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let stamp = stamp_of(&key);
    if let Ok(cached) = maps().lock() {
        if let Some((seen, loaded)) = cached.get(&key) {
            if *seen == stamp {
                return loaded.clone();
            }
        }
    }
    let loaded = Arc::new(load_autoload(&key));
    if let Ok(mut cached) = maps().lock() {
        cached.insert(key, (stamp, loaded.clone()));
    }
    loaded
}

/// Forgets a work tree's autoload maps, for the watcher and for tests.
pub fn forget(root: &Path) {
    let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if let Ok(mut cached) = maps().lock() {
        cached.remove(&key);
    }
}

fn load_autoload(root: &Path) -> Autoload {
    let mut loaded = Autoload::default();
    let read_map = |file: &str| {
        std::fs::read_to_string(root.join(file))
            .map(|text| php_array(&text, root))
            .unwrap_or_default()
    };
    for (class, mut paths) in read_map(AUTOLOAD_FILES[0]) {
        if let Some(path) = paths.pop() {
            loaded.classmap.insert(class, path);
        }
    }
    loaded.psr4 = read_map(AUTOLOAD_FILES[1]);
    loaded.psr0 = read_map(AUTOLOAD_FILES[2]);
    // Without `vendor/` installed, the project's own namespaces from
    // composer.json still say where its classes are.
    if let Some(json) = std::fs::read_to_string(root.join("composer.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    {
        for section in ["autoload", "autoload-dev"] {
            let Some(map) = json[section]["psr-4"].as_object() else {
                continue;
            };
            for (prefix, dirs) in map {
                if loaded.psr4.iter().any(|(known, _)| known == prefix) {
                    continue;
                }
                let dirs: Vec<String> = match dirs {
                    serde_json::Value::String(one) => vec![one.clone()],
                    serde_json::Value::Array(many) => many
                        .iter()
                        .filter_map(|one| one.as_str().map(str::to_string))
                        .collect(),
                    _ => Vec::new(),
                };
                let dirs = dirs.iter().filter_map(|dir| normal(dir)).collect();
                loaded.psr4.push((prefix.clone(), dirs));
            }
        }
    }
    loaded
        .psr4
        .sort_by_key(|one| std::cmp::Reverse(one.0.len()));
    loaded
        .psr0
        .sort_by_key(|one| std::cmp::Reverse(one.0.len()));
    loaded
}

/// One of Composer's generated `return array(…)` files, as its keys and the
/// paths each maps to, relative to the work tree. A path that leads out of
/// the work tree is left out.
fn php_array(text: &str, root: &Path) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('\'') {
            continue;
        }
        let Some((key, rest)) = quoted(line) else {
            continue;
        };
        let Some(rhs) = rest.trim_start().strip_prefix("=>") else {
            continue;
        };
        let paths = values(rhs)
            .into_iter()
            .filter_map(|(base, tail)| place(base, &tail, root))
            .collect();
        out.push((key, paths));
    }
    out
}

/// A single-quoted PHP string at the start of `text`, unescaped, and what
/// follows it.
fn quoted(text: &str) -> Option<(String, &str)> {
    let body = text.strip_prefix('\'')?;
    let mut out = String::new();
    let mut chars = body.char_indices();
    while let Some((at, c)) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some((_, next @ ('\\' | '\''))) => out.push(next),
                Some((_, next)) => {
                    out.push('\\');
                    out.push(next);
                }
                None => out.push('\\'),
            },
            '\'' => return Some((out, &body[at + 1..])),
            other => out.push(other),
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq)]
enum Base {
    Vendor,
    Root,
    Absolute,
}

/// The values on the right of `=>`: `$vendorDir . '/a'`, `array($baseDir .
/// '/b', …)`, or a plain string.
fn values(rhs: &str) -> Vec<(Base, String)> {
    let mut out = Vec::new();
    let mut base: Option<Base> = None;
    let mut tail = String::new();
    let mut seen = false;
    let mut rest = rhs;
    while let Some(c) = rest.chars().next() {
        if c == '\'' {
            match quoted(rest) {
                Some((part, after)) => {
                    tail.push_str(&part);
                    seen = true;
                    rest = after;
                }
                None => break,
            }
            continue;
        }
        if c == '$' {
            let name: String = rest[1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            base = match name.as_str() {
                "vendorDir" => Some(Base::Vendor),
                "baseDir" => Some(Base::Root),
                _ => base,
            };
            seen = true;
            rest = &rest[1 + name.len()..];
            continue;
        }
        if c == ',' || c == ')' {
            if seen {
                out.push((base.unwrap_or(Base::Absolute), std::mem::take(&mut tail)));
            }
            base = None;
            seen = false;
        }
        rest = &rest[c.len_utf8()..];
    }
    if seen {
        out.push((base.unwrap_or(Base::Absolute), tail));
    }
    out
}

fn place(base: Base, tail: &str, root: &Path) -> Option<String> {
    match base {
        Base::Vendor => normal(&format!("vendor/{tail}")),
        Base::Root => normal(tail),
        Base::Absolute => {
            let inner = Path::new(tail).strip_prefix(root).ok()?;
            if inner
                .components()
                .any(|part| matches!(part, Component::ParentDir))
            {
                return None;
            }
            Some(files::slashed(inner))
        }
    }
}

/// The file a class is in, relative to the work tree, by Composer's rules:
/// the class map, then PSR-4, then PSR-0. `None` when none of them has a
/// file that is there, or the file is reached through a link out of the work
/// tree.
pub fn resolve_class(root: &Path, fqn: &str) -> Option<String> {
    let fqn = fqn.trim_start_matches('\\');
    if fqn.is_empty() {
        return None;
    }
    let maps = autoload(root);
    let mut candidates: Vec<String> = Vec::new();
    if let Some(path) = maps.classmap.get(fqn) {
        candidates.push(path.clone());
    }
    for (prefix, dirs) in &maps.psr4 {
        if let Some(rest) = fqn.strip_prefix(prefix.as_str()) {
            let rest = rest.replace('\\', "/");
            for dir in dirs {
                candidates.push(join(dir, &format!("{rest}.php")));
            }
        }
    }
    for (prefix, dirs) in &maps.psr0 {
        if !fqn.starts_with(prefix.as_str()) {
            continue;
        }
        let (space, class) = fqn.rsplit_once('\\').unwrap_or(("", fqn));
        let mut path = space.replace('\\', "/");
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(&class.replace('_', "/"));
        for dir in dirs {
            candidates.push(join(dir, &format!("{path}.php")));
        }
    }
    candidates
        .into_iter()
        .filter_map(|path| normal(&path))
        .find(|path| exists(root, path))
}

pub(crate) fn join(dir: &str, rest: &str) -> String {
    if dir.is_empty() {
        rest.to_string()
    } else {
        format!("{}/{rest}", dir.trim_end_matches('/'))
    }
}

// --- PHP: classes and their members ---------------------------------------------

/// A method, constant, property or enum case a class has, its own or one it
/// gets from a parent, a trait or an interface.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Member {
    pub name: String,
    /// `method`, `constant`, `property` or `case`.
    pub kind: &'static str,
    #[serde(rename = "static")]
    pub is_static: bool,
    /// `public`, `protected` or `private`.
    pub visibility: &'static str,
    /// From 1.
    pub line: u32,
    pub column: u32,
    /// The class it is declared in, fully qualified.
    pub class: String,
    /// The file that class is in.
    pub path: String,
    /// Only written in a docblock (`@method`), not declared.
    pub doc: bool,
    /// What it takes and gives back, as written: `(int $id): User` for a
    /// method, the type for a property. Whitespace squashed, cut short.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// The first line of its docblock.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The class a method returns or a property holds, fully qualified, or
    /// `static` for the class it was reached through (`static`, `self` and
    /// `$this`). From the declared type, else `@return` or `@var`.
    #[serde(skip)]
    pub returns: Option<String>,
}

/// One class, interface, trait or enum as declared in a file.
#[derive(Clone, Debug)]
struct ClassDecl {
    fqn: String,
    /// The file it is declared in.
    path: String,
    line: u32,
    column: u32,
    extends: Vec<String>,
    interfaces: Vec<String>,
    traits: Vec<String>,
    members: Vec<Member>,
    doc: Option<(String, u32)>,
    /// What a facade's `getFacadeAccessor` returns, when it is `X::class`.
    accessor: Option<String>,
    imports: PhpImports,
}

fn visibility_of(node: Node, source: &str) -> &'static str {
    named_children(node)
        .into_iter()
        .find(|child| child.kind() == "visibility_modifier")
        .map(|child| match text_of(child, source) {
            "private" => "private",
            "protected" => "protected",
            _ => "public",
        })
        .unwrap_or("public")
}

fn has_static(node: Node) -> bool {
    named_children(node)
        .into_iter()
        .any(|child| child.kind() == "static_modifier")
}

fn names_in(node: Node, source: &str, imports: &PhpImports) -> Vec<String> {
    named_children(node)
        .into_iter()
        .filter(|child| matches!(child.kind(), "name" | "qualified_name"))
        .map(|child| imports.qualify(text_of(child, source)))
        .collect()
}

const CLASS_KINDS: [&str; 4] = [
    "class_declaration",
    "interface_declaration",
    "trait_declaration",
    "enum_declaration",
];

/// Every class-like declaration in a PHP file.
fn classes_in(text: &str, path: &str) -> Vec<ClassDecl> {
    let Some(tree) = parse_php(text) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let imports = imports_in(root, text);
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if CLASS_KINDS.contains(&node.kind()) {
            if let Some(decl) = class_decl(node, text, path, &imports) {
                out.push(decl);
            }
            continue;
        }
        if matches!(
            node.kind(),
            "program" | "namespace_definition" | "compound_statement"
        ) {
            stack.extend(named_children(node));
        }
    }
    out
}

fn class_decl(node: Node, source: &str, path: &str, imports: &PhpImports) -> Option<ClassDecl> {
    let name = node.child_by_field_name("name")?;
    let short = text_of(name, source);
    let mut decl = ClassDecl {
        fqn: String::new(),
        path: path.to_string(),
        line: name.start_position().row as u32 + 1,
        column: name.start_position().column as u32,
        extends: Vec::new(),
        interfaces: Vec::new(),
        traits: Vec::new(),
        members: Vec::new(),
        doc: None,
        accessor: None,
        imports: imports.clone(),
    };
    // `namespace\Name` is the class itself, wherever the file's namespace is.
    decl.fqn = if imports.namespace.is_empty() {
        short.to_string()
    } else {
        format!("{}\\{short}", imports.namespace)
    };
    if let Some(doc) = node.prev_named_sibling() {
        let text = text_of(doc, source);
        if doc.kind() == "comment" && text.starts_with("/**") {
            decl.doc = Some((text.to_string(), doc.start_position().row as u32 + 1));
        }
    }
    for child in named_children(node) {
        match child.kind() {
            "base_clause" => decl.extends = names_in(child, source, imports),
            "class_interface_clause" => decl.interfaces = names_in(child, source, imports),
            _ => {}
        }
    }
    let member =
        |name: Node, kind: &'static str, is_static: bool, visibility: &'static str| Member {
            name: text_of(name, source).trim_start_matches('$').to_string(),
            kind,
            is_static,
            visibility,
            line: name.start_position().row as u32 + 1,
            column: name.start_position().column as u32,
            class: decl.fqn.clone(),
            path: path.to_string(),
            doc: false,
            signature: None,
            summary: None,
            returns: None,
        };
    let mut members = Vec::new();
    let mut traits = Vec::new();
    let mut accessor = None;
    if let Some(body) = node.child_by_field_name("body") {
        for item in named_children(body) {
            let doc = doc_before(item, source);
            let summary = doc.and_then(doc_summary);
            match item.kind() {
                "use_declaration" => traits.extend(names_in(item, source, imports)),
                "const_declaration" => {
                    let visibility = visibility_of(item, source);
                    for element in named_children(item) {
                        if element.kind() != "const_element" {
                            continue;
                        }
                        if let Some(name) = named_children(element)
                            .into_iter()
                            .find(|one| one.kind() == "name")
                        {
                            let mut one = member(name, "constant", true, visibility);
                            one.summary = summary.clone();
                            members.push(one);
                        }
                    }
                }
                "property_declaration" => {
                    let visibility = visibility_of(item, source);
                    let is_static = has_static(item);
                    let declared = item.child_by_field_name("type");
                    let returns = declared
                        .and_then(|ty| type_class(ty, source, imports))
                        .or_else(|| {
                            doc.and_then(|doc| doc_tag(doc, "@var"))
                                .and_then(|ty| doc_class(ty, imports))
                        });
                    let signature = declared
                        .map(|ty| squash(text_of(ty, source)))
                        .or_else(|| doc.and_then(|doc| doc_tag(doc, "@var")).map(squash));
                    for element in named_children(item) {
                        if element.kind() != "property_element" {
                            continue;
                        }
                        if let Some(name) = element.child_by_field_name("name") {
                            let mut one = member(name, "property", is_static, visibility);
                            one.signature = signature.clone();
                            one.summary = summary.clone();
                            one.returns = returns.clone();
                            members.push(one);
                        }
                    }
                }
                "method_declaration" => {
                    let Some(name) = item.child_by_field_name("name") else {
                        continue;
                    };
                    if text_of(name, source) == "getFacadeAccessor" {
                        accessor = class_constant_in(item, source, imports);
                    }
                    let mut one = member(
                        name,
                        "method",
                        has_static(item),
                        visibility_of(item, source),
                    );
                    let declared = item.child_by_field_name("return_type");
                    one.returns = declared
                        .and_then(|ty| type_class(ty, source, imports))
                        .or_else(|| {
                            doc.and_then(|doc| doc_tag(doc, "@return"))
                                .and_then(|ty| doc_class(ty, imports))
                        });
                    one.signature = method_signature(item, source, doc);
                    one.summary = summary;
                    members.push(one);
                    // Promoted constructor parameters are properties too.
                    if text_of(name, source).eq_ignore_ascii_case("__construct") {
                        let parameters = item
                            .child_by_field_name("parameters")
                            .map(named_children)
                            .unwrap_or_default();
                        for parameter in parameters {
                            if parameter.kind() != "property_promotion_parameter" {
                                continue;
                            }
                            let Some(var) = parameter.child_by_field_name("name") else {
                                continue;
                            };
                            let var = if var.kind() == "variable_name" {
                                var
                            } else {
                                match named_children(var)
                                    .into_iter()
                                    .find(|one| one.kind() == "variable_name")
                                {
                                    Some(inner) => inner,
                                    None => continue,
                                }
                            };
                            let mut one =
                                member(var, "property", false, visibility_of(parameter, source));
                            let declared = parameter.child_by_field_name("type");
                            one.returns = declared.and_then(|ty| type_class(ty, source, imports));
                            one.signature = declared.map(|ty| squash(text_of(ty, source)));
                            members.push(one);
                        }
                    }
                }
                "enum_case" => {
                    if let Some(name) = item.child_by_field_name("name") {
                        let mut one = member(name, "case", true, "public");
                        one.summary = summary;
                        members.push(one);
                    }
                }
                _ => {}
            }
        }
    }
    // `@method` tags: what a class says it answers to without declaring it.
    if let Some((doc, first)) = &decl.doc {
        for (at, line) in doc.lines().enumerate() {
            if let Some(tag) = method_tag(line) {
                let returns = tag.returns.as_deref().and_then(|ty| doc_class(ty, imports));
                members.push(Member {
                    name: tag.name,
                    kind: "method",
                    is_static: tag.is_static,
                    visibility: "public",
                    line: first + at as u32,
                    column: 0,
                    class: decl.fqn.clone(),
                    path: path.to_string(),
                    doc: true,
                    signature: tag.signature,
                    summary: None,
                    returns,
                });
            }
        }
    }
    decl.members = members;
    decl.traits = traits;
    decl.accessor = accessor;
    Some(decl)
}

/// The first `X::class` in a method, as a fully qualified name.
fn class_constant_in(method: Node, source: &str, imports: &PhpImports) -> Option<String> {
    let mut stack = vec![method];
    while let Some(node) = stack.pop() {
        if node.kind() == "class_constant_access_expression" {
            let parts = named_children(node);
            if parts.len() == 2
                && text_of(parts[1], source) == "class"
                && matches!(parts[0].kind(), "name" | "qualified_name")
            {
                return Some(imports.qualify(text_of(parts[0], source)));
            }
        }
        stack.extend(named_children(node));
    }
    None
}

/// The docblock right above a declaration, if there is one.
fn doc_before<'s>(node: Node, source: &'s str) -> Option<&'s str> {
    let before = node.prev_named_sibling()?;
    let text = text_of(before, source);
    (before.kind() == "comment" && text.starts_with("/**")).then_some(text)
}

/// The first line of a docblock that is prose, not a tag.
fn doc_summary(doc: &str) -> Option<String> {
    doc.lines()
        .map(|line| {
            line.trim()
                .trim_start_matches("/**")
                .trim_end_matches("*/")
                .trim_start_matches('*')
                .trim()
        })
        .find(|line| !line.is_empty())
        .filter(|line| !line.starts_with('@'))
        .map(|line| line.chars().take(160).collect())
}

/// The type a docblock tag names: `Foo|null` in `@return Foo|null the thing`.
fn doc_tag<'d>(doc: &'d str, tag: &str) -> Option<&'d str> {
    doc.lines().find_map(|line| {
        let at = line.find(tag)?;
        let rest = &line[at + tag.len()..];
        // `@return` and not `@returns-something`.
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        rest.split_whitespace().next()
    })
}

/// Whitespace squashed to single spaces, and cut at 120 characters.
fn squash(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let joined = joined.replace("( ", "(").replace(" )", ")");
    if joined.chars().count() > 120 {
        let cut: String = joined.chars().take(119).collect();
        format!("{cut}…")
    } else {
        joined
    }
}

/// `(int $id, ?Foo $foo = null): Bar` for a method, the return type from
/// `@return` when none is declared.
fn method_signature(method: Node, source: &str, doc: Option<&str>) -> Option<String> {
    let params = squash(text_of(method.child_by_field_name("parameters")?, source));
    let returns = method
        .child_by_field_name("return_type")
        .map(|ty| squash(text_of(ty, source)))
        .or_else(|| doc.and_then(|doc| doc_tag(doc, "@return")).map(squash));
    Some(match returns {
        Some(returns) => format!("{params}: {returns}"),
        None => params,
    })
}

/// Whether a type as written means the class it is reached through.
fn is_self_type(written: &str) -> bool {
    let written = written.trim().trim_start_matches('?');
    ["static", "self", "$this"]
        .iter()
        .any(|one| written.eq_ignore_ascii_case(one))
}

/// The class a declared type means, fully qualified: the type itself, the
/// type in `?Foo`, the first class in a union. `static` for `static`, `self`
/// and `$this`. `None` for a scalar, `array`, `mixed` and the like.
fn type_class(node: Node, source: &str, imports: &PhpImports) -> Option<String> {
    let written = text_of(node, source).trim();
    if is_self_type(written) {
        return Some("static".into());
    }
    match node.kind() {
        "named_type" => {
            let name = named_children(node).into_iter().next()?;
            let written = text_of(name, source).trim();
            if is_self_type(written) {
                return Some("static".into());
            }
            if written.eq_ignore_ascii_case("parent") {
                return None;
            }
            Some(imports.qualify(written))
        }
        "optional_type" | "union_type" | "type" => named_children(node)
            .into_iter()
            .filter(|one| one.kind() != "primitive_type" || is_self_type(text_of(*one, source)))
            .find_map(|one| type_class(one, source, imports)),
        "primitive_type" | "bottom_type" => None,
        "name" | "qualified_name" => Some(imports.qualify(written)),
        _ => None,
    }
}

/// Names that are never a class in a docblock type.
const DOC_SCALARS: &[&str] = &[
    "int",
    "integer",
    "float",
    "double",
    "string",
    "bool",
    "boolean",
    "array",
    "mixed",
    "object",
    "callable",
    "iterable",
    "null",
    "void",
    "never",
    "false",
    "true",
    "resource",
    "list",
    "class-string",
];

/// The class a docblock type names: `Foo`, `\A\Foo`, `?Foo`, the first class
/// of `Foo|null`, `Collection` of `Collection<int, User>`. `static` for
/// `static`, `self` and `$this`. Not `Foo[]`, which is an array.
fn doc_class(written: &str, imports: &PhpImports) -> Option<String> {
    let written = written.trim().trim_start_matches('?');
    // `Collection<int, User>|null`: the generic part says what is inside.
    let outer = written.split('<').next().unwrap_or(written);
    outer.split('|').find_map(|part| {
        let part = part.trim().trim_start_matches('?');
        if part.is_empty() || part.ends_with("[]") || part.contains(['(', '{', '[']) {
            return None;
        }
        if is_self_type(part) {
            return Some("static".to_string());
        }
        if DOC_SCALARS.iter().any(|one| part.eq_ignore_ascii_case(one)) {
            return None;
        }
        let first = part.chars().next()?;
        (first == '\\' || first.is_alphabetic() || first == '_').then(|| imports.qualify(part))
    })
}

struct MethodTag {
    name: String,
    is_static: bool,
    /// The type it says the method returns, as written.
    returns: Option<String>,
    /// `(string $id): Factory`, for the completion list.
    signature: Option<String>,
}

/// `@method static \A\B name(…)` in a docblock line.
fn method_tag(line: &str) -> Option<MethodTag> {
    let at = line.find("@method")?;
    let rest = line[at + "@method".len()..].trim_start();
    let paren = rest.find('(')?;
    let head = rest[..paren].trim();
    let mut words: Vec<&str> = head.split_whitespace().collect();
    let name = words.pop()?.to_string();
    let is_static = words.first() == Some(&"static");
    if is_static {
        words.remove(0);
    }
    let returns = words
        .first()
        .map(|kind| {
            kind.split('|')
                .next()
                .unwrap_or(kind)
                .trim_end_matches("[]")
                .to_string()
        })
        .filter(|kind| {
            kind.chars()
                .next()
                .is_some_and(|c| c == '\\' || c.is_ascii_uppercase())
        });
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || name.is_empty() {
        return None;
    }
    let params = rest[paren..]
        .find(')')
        .map(|close| &rest[paren..=paren + close])
        .unwrap_or(&rest[paren..]);
    let signature = match words.first() {
        Some(kind) => format!("{}: {kind}", squash(params)),
        None => squash(params),
    };
    Some(MethodTag {
        name,
        is_static,
        returns,
        signature: Some(signature),
    })
}

/// The classes a docblock names with `@see`, as written.
fn see_tags(doc: &str) -> Vec<String> {
    doc.lines()
        .filter_map(|line| {
            let at = line.find("@see")?;
            let target = line[at + 4..].split_whitespace().next()?;
            let class = target.split("::").next()?;
            class
                .chars()
                .next()
                .is_some_and(|c| c == '\\' || c.is_ascii_uppercase())
                .then(|| class.to_string())
        })
        .collect()
}

/// Classes parsed from a file, kept by path and the hash of the text they
/// were read from: completion asks after every keystroke, and a vendor
/// class's parents are the same files each time.
type Parsed = HashMap<String, (u64, Arc<Vec<ClassDecl>>)>;

fn parsed_classes() -> &'static Mutex<Parsed> {
    static PARSED: OnceLock<Mutex<Parsed>> = OnceLock::new();
    PARSED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// [`classes_in`], from the cache when the text is the one parsed last time.
fn classes_cached(text: &str, path: &str) -> Arc<Vec<ClassDecl>> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    let hash = hasher.finish();
    if let Ok(cache) = parsed_classes().lock() {
        if let Some((seen, classes)) = cache.get(path) {
            if *seen == hash {
                return classes.clone();
            }
        }
    }
    let classes = Arc::new(classes_in(text, path));
    if let Ok(mut cache) = parsed_classes().lock() {
        if cache.len() > 400 {
            cache.clear();
        }
        cache.insert(path.to_string(), (hash, classes.clone()));
    }
    classes
}

/// A class's declaration: from `text` when it is in that file (the editor's
/// copy, which may be ahead of the disk), else through the autoload maps,
/// else, for a project without Composer's maps, a file the symbol index
/// says declares a class of that name.
fn find_decl(root: &Path, fqn: &str, here: Option<(&str, &str)>) -> Option<ClassDecl> {
    let fqn = fqn.trim_start_matches('\\');
    if let Some((path, text)) = here {
        if let Some(decl) = classes_cached(text, path)
            .iter()
            .find(|decl| decl.fqn.eq_ignore_ascii_case(fqn))
        {
            return Some(decl.clone());
        }
    }
    let short = fqn.rsplit('\\').next().unwrap_or(fqn);
    let Some(path) = resolve_class(root, fqn) else {
        // Only an exact match: the index knows short names.
        return crate::symbols::class_paths(root, short, true)
            .into_iter()
            .filter(|path| here.map_or(true, |(own, _)| own != path.as_str()))
            .find_map(|path| {
                let text = read(root, &path)?;
                classes_cached(&text, &path)
                    .iter()
                    .find(|decl| decl.fqn.eq_ignore_ascii_case(fqn))
                    .cloned()
            });
    };
    let text = read(root, &path)?;
    let all = classes_cached(&text, &path);
    let exact = all
        .iter()
        .position(|decl| decl.fqn.eq_ignore_ascii_case(fqn));
    let by_name = all.iter().position(|decl| {
        decl.fqn
            .rsplit('\\')
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case(short))
    });
    exact.or(by_name).map(|at| all[at].clone())
}

/// Walks a class and what it inherits, nearest first: its own members, its
/// traits, its parent and theirs, then its interfaces. `visit` stops the walk
/// by returning true.
fn walk(
    root: &Path,
    decl: &ClassDecl,
    here: Option<(&str, &str)>,
    seen: &mut HashSet<String>,
    depth: usize,
    visit: &mut dyn FnMut(&ClassDecl) -> bool,
) -> bool {
    if depth > MAX_DEPTH || !seen.insert(decl.fqn.to_ascii_lowercase()) {
        return false;
    }
    if visit(decl) {
        return true;
    }
    let next = decl
        .traits
        .iter()
        .chain(&decl.extends)
        .chain(&decl.interfaces);
    for fqn in next {
        if let Some(parent) = find_decl(root, fqn, here) {
            if walk(root, &parent, here, seen, depth + 1, visit) {
                return true;
            }
        }
    }
    false
}

/// Every member of a class, its own and what it inherits from parents,
/// traits and interfaces (up to ten levels), nearest first and each name
/// once. `path` is the file the class is in; `fqn` its fully qualified name.
pub fn class_members(root: &Path, path: &str, fqn: &str) -> Vec<Member> {
    let here = read(root, path);
    let here = here.as_deref().map(|text| (path, text));
    let Some(decl) = find_decl(root, fqn, here) else {
        return Vec::new();
    };
    let mut out: Vec<Member> = Vec::new();
    let mut names: HashSet<(bool, String)> = HashSet::new();
    walk(root, &decl, here, &mut HashSet::new(), 0, &mut |decl| {
        for member in &decl.members {
            // Methods are not case-sensitive; properties and constants are.
            let method = member.kind == "method";
            let name = if method {
                member.name.to_ascii_lowercase()
            } else {
                member.name.clone()
            };
            if names.insert((method, name)) {
                out.push(member.clone());
            }
        }
        false
    });
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Want {
    Method,
    Constant,
    Property,
}

fn wanted(member: &Member, name: &str, want: Want) -> bool {
    match want {
        Want::Method => member.kind == "method" && member.name.eq_ignore_ascii_case(name),
        Want::Constant => matches!(member.kind, "constant" | "case") && member.name == name,
        Want::Property => member.kind == "property" && member.name == name,
    }
}

/// A member by name, declared before documented: a real method wins over a
/// `@method` tag anywhere up the hierarchy.
fn find_member(
    root: &Path,
    decl: &ClassDecl,
    here: Option<(&str, &str)>,
    name: &str,
    want: Want,
) -> Option<Member> {
    for doc in [false, true] {
        let mut found = None;
        walk(root, decl, here, &mut HashSet::new(), 0, &mut |decl| {
            found = decl
                .members
                .iter()
                .find(|member| member.doc == doc && wanted(member, name, want))
                .cloned();
            found.is_some()
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Whether a class is a Laravel facade: it, or something it extends, is
/// `Illuminate\Support\Facades\Facade`.
fn is_facade(root: &Path, decl: &ClassDecl, here: Option<(&str, &str)>) -> bool {
    let mut facade = false;
    walk(root, decl, here, &mut HashSet::new(), 0, &mut |decl| {
        facade = decl
            .extends
            .iter()
            .any(|one| one.eq_ignore_ascii_case(FACADE));
        facade
    });
    facade
}

/// Where a facade's method really is: on the class `getFacadeAccessor`
/// names, on the type its `@method` tag returns, or on a class its `@see`
/// names. With the class it was found on.
fn through_facade(root: &Path, decl: &ClassDecl, name: &str) -> Option<(Member, String)> {
    let mut targets: Vec<String> = Vec::new();
    if let Some(accessor) = &decl.accessor {
        targets.push(accessor.clone());
    }
    if let Some((doc, _)) = &decl.doc {
        for line in doc.lines() {
            if let Some(tag) = method_tag(line) {
                if tag.name.eq_ignore_ascii_case(name) {
                    if let Some(kind) = tag.returns {
                        targets.push(decl.imports.qualify(&kind));
                    }
                }
            }
        }
        targets.extend(see_tags(doc).iter().map(|one| decl.imports.qualify(one)));
    }
    targets.into_iter().find_map(|fqn| {
        let target = find_decl(root, &fqn, None)?;
        find_member(root, &target, None, name, Want::Method)
            .filter(|member| !member.doc)
            .map(|member| (member, target.fqn.clone()))
    })
}

enum Target {
    Class(String),
    Member {
        class: String,
        name: String,
        want: Want,
    },
}

fn php_jump(root: &Path, path: &str, text: &str, byte: usize) -> Option<Landing> {
    let tree = parse_php(text)?;
    let imports = imports_in(tree.root_node(), text);
    let node = tree
        .root_node()
        .named_descendant_for_byte_range(byte, byte)?;
    let scope = Here {
        root,
        path,
        source: text,
        imports: &imports,
    };
    let target = php_target(&scope, node)?;
    let here = Some((path, text));
    match target {
        Target::Class(fqn) => {
            // A project class Composer does not map is the index's to find.
            let mapped = resolve_class(root, &fqn).is_some()
                || classes_cached(text, path)
                    .iter()
                    .any(|decl| decl.fqn.eq_ignore_ascii_case(fqn.trim_start_matches('\\')));
            if !mapped {
                return None;
            }
            let decl = find_decl(root, &fqn, here)?;
            let file = decl.path.clone();
            Some((file, decl.line, decl.column, "class", None))
        }
        Target::Member { class, name, want } => {
            let decl = find_decl(root, &class, here)?;
            if let Some(member) = find_member(root, &decl, here, &name, want) {
                if !member.doc {
                    return Some((member.path, member.line, member.column, member.kind, None));
                }
            }
            if want == Want::Method && is_facade(root, &decl, here) {
                if let Some((member, _)) = through_facade(root, &decl, &name) {
                    return Some((member.path, member.line, member.column, member.kind, None));
                }
                let short = decl
                    .fqn
                    .rsplit('\\')
                    .next()
                    .unwrap_or(&decl.fqn)
                    .to_string();
                let file = decl.path.clone();
                return Some((
                    file,
                    decl.line,
                    decl.column,
                    "class",
                    Some(format!(
                        "{short} is a facade; the method lives on the class it resolves to"
                    )),
                ));
            }
            // Only written in a docblock: that is where it can be read about.
            find_member(root, &decl, here, &name, want)
                .map(|member| (member.path, member.line, member.column, member.kind, None))
        }
    }
}

/// Where a class is meant by a bare name in these places.
const CLASS_PLACES: &[&str] = &[
    "base_clause",
    "class_interface_clause",
    "object_creation_expression",
    "use_declaration",
    "named_type",
    "optional_type",
    "union_type",
    "intersection_type",
    "type_list",
    "catch_clause",
    "binary_expression",
    "attribute",
    "scoped_call_expression",
    "class_constant_access_expression",
    "scoped_property_access_expression",
    "instanceof_expression",
];

fn php_target(scope: &Here, node: Node) -> Option<Target> {
    let source = scope.source;
    let imports = scope.imports;
    // A name in a `use` line: the class it imports.
    let mut up = Some(node);
    for _ in 0..4 {
        let Some(at) = up else { break };
        if at.kind() == "namespace_use_clause" {
            let declaration = at.parent().and_then(|parent| {
                if parent.kind() == "namespace_use_group" {
                    parent.parent()
                } else {
                    Some(parent)
                }
            })?;
            let prefix = named_children(declaration)
                .into_iter()
                .find(|child| child.kind() == "namespace_name")
                .map(|child| text_of(child, source).trim_start_matches('\\').to_string());
            let after = text_of(declaration, source)
                .trim_start()
                .trim_start_matches("use")
                .trim_start();
            if after.starts_with("function ") || after.starts_with("const ") {
                return None;
            }
            let (_, fqn) = clause_import(at, prefix.as_deref(), source)?;
            return Some(Target::Class(fqn));
        }
        up = at.parent();
    }

    // `X::$name`: the name sits inside a variable_name.
    if node.kind() == "name" {
        if let Some(variable) = node.parent().filter(|p| p.kind() == "variable_name") {
            let access = variable.parent()?;
            if access.kind() == "scoped_property_access_expression"
                && access.child_by_field_name("name") == Some(variable)
            {
                let class = scope_class(access.child_by_field_name("scope")?, source, imports)?;
                return Some(Target::Member {
                    class,
                    name: text_of(node, source).to_string(),
                    want: Want::Property,
                });
            }
            return None;
        }
    }
    if !matches!(node.kind(), "name" | "qualified_name" | "namespace_name") {
        return None;
    }
    // The whole of `A\B\C`, wherever in it the click was.
    let mut name = node;
    while let Some(parent) = name.parent() {
        if matches!(parent.kind(), "qualified_name" | "namespace_name") {
            name = parent;
        } else {
            break;
        }
    }
    let parent = name.parent()?;
    let written = text_of(name, source);
    match parent.kind() {
        "scoped_call_expression" => {
            if parent.child_by_field_name("name") == Some(name) {
                let class = scope_class(parent.child_by_field_name("scope")?, source, imports)?;
                return Some(Target::Member {
                    class,
                    name: written.to_string(),
                    want: Want::Method,
                });
            }
            return scope_class(name, source, imports).map(Target::Class);
        }
        "class_constant_access_expression" => {
            let parts = named_children(parent);
            if parts.len() == 2 && parts[1] == name {
                let class = scope_class(parts[0], source, imports)?;
                if written == "class" {
                    return Some(Target::Class(class));
                }
                return Some(Target::Member {
                    class,
                    name: written.to_string(),
                    want: Want::Constant,
                });
            }
            return scope_class(name, source, imports).map(Target::Class);
        }
        "member_call_expression"
        | "nullsafe_member_call_expression"
        | "member_access_expression"
        | "nullsafe_member_access_expression" => {
            if parent.child_by_field_name("name") != Some(name) {
                return None;
            }
            // The receiver's class: `$this`, a typed variable or property,
            // or a chain of calls followed through their return types. One
            // that cannot be worked out is left to the index.
            let object = parent.child_by_field_name("object")?;
            let class = php_type(scope, object, 0)?;
            let want = if parent.kind().contains("call") {
                Want::Method
            } else {
                Want::Property
            };
            return Some(Target::Member {
                class,
                name: written.to_string(),
                want,
            });
        }
        "function_call_expression" | "method_declaration" | "function_definition" => return None,
        _ => {}
    }
    if CLASS_KINDS.contains(&parent.kind()) {
        return None;
    }
    let looks_like_class = written
        .trim_start_matches('\\')
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase());
    if CLASS_PLACES.contains(&parent.kind()) || (looks_like_class && written.contains('\\')) {
        return Some(Target::Class(imports.qualify(written)));
    }
    None
}

/// The class a `X::` scope means: a name, or `self`, `static` and `parent`.
fn scope_class(scope: Node, source: &str, imports: &PhpImports) -> Option<String> {
    match scope.kind() {
        "name" | "qualified_name" => Some(imports.qualify(text_of(scope, source))),
        "relative_scope" => {
            let class = enclosing_class(scope, source, imports)?;
            if text_of(scope, source).eq_ignore_ascii_case("parent") {
                let decl = enclosing_decl(scope)?;
                let base = named_children(decl)
                    .into_iter()
                    .find(|child| child.kind() == "base_clause")?;
                return names_in(base, source, imports).into_iter().next();
            }
            Some(class)
        }
        _ => None,
    }
}

fn enclosing_decl(node: Node) -> Option<Node> {
    let mut at = node.parent();
    while let Some(here) = at {
        if CLASS_KINDS.contains(&here.kind()) {
            return Some(here);
        }
        at = here.parent();
    }
    None
}

fn enclosing_class(node: Node, source: &str, imports: &PhpImports) -> Option<String> {
    let decl = enclosing_decl(node)?;
    let name = text_of(decl.child_by_field_name("name")?, source);
    Some(if imports.namespace.is_empty() {
        name.to_string()
    } else {
        format!("{}\\{name}", imports.namespace)
    })
}

// --- PHP: what an expression holds ------------------------------------------------

/// How many steps of a chain (`a()->b()->c()`) are followed.
const MAX_CHAIN: usize = 8;

/// The file an expression is in: where to look for its classes.
struct Here<'a> {
    root: &'a Path,
    path: &'a str,
    source: &'a str,
    imports: &'a PhpImports,
}

impl Here<'_> {
    fn here(&self) -> Option<(&str, &str)> {
        Some((self.path, self.source))
    }
}

/// The class an expression holds an instance of, fully qualified: `$this`, a
/// typed parameter, a variable assigned `new Foo` or a chain, a typed or
/// documented property, and each call of a chain by the type its method
/// declares it returns (`: Foo`, `?Foo`, the first class of a union,
/// `@return Foo`, `static`, `self` and `$this` keep the class). `None` as
/// soon as one step cannot be worked out: no guessing.
fn php_type(scope: &Here, node: Node, depth: usize) -> Option<String> {
    if depth > MAX_CHAIN {
        return None;
    }
    let source = scope.source;
    match node.kind() {
        "variable_name" => {
            let name = text_of(node, source).trim_start_matches('$');
            if name == "this" {
                return enclosing_class(node, source, scope.imports);
            }
            var_type(scope, node, name, depth)
        }
        "member_access_expression" | "nullsafe_member_access_expression" => {
            let class = php_type(scope, node.child_by_field_name("object")?, depth + 1)?;
            let name = node
                .child_by_field_name("name")
                .filter(|n| n.kind() == "name")?;
            member_returns(scope, &class, text_of(name, source), Want::Property)
        }
        "member_call_expression" | "nullsafe_member_call_expression" => {
            let class = php_type(scope, node.child_by_field_name("object")?, depth + 1)?;
            let name = node
                .child_by_field_name("name")
                .filter(|n| n.kind() == "name")?;
            member_returns(scope, &class, text_of(name, source), Want::Method)
        }
        "scoped_call_expression" => {
            let class = scope_class(node.child_by_field_name("scope")?, source, scope.imports)?;
            let name = node
                .child_by_field_name("name")
                .filter(|n| n.kind() == "name")?;
            member_returns(scope, &class, text_of(name, source), Want::Method)
        }
        "object_creation_expression" => {
            let name = named_children(node)
                .into_iter()
                .find(|n| matches!(n.kind(), "name" | "qualified_name"))?;
            let written = text_of(name, source);
            if is_self_type(written) {
                return enclosing_class(node, source, scope.imports);
            }
            Some(scope.imports.qualify(written))
        }
        "parenthesized_expression" => named_children(node)
            .into_iter()
            .next()
            .and_then(|inner| php_type(scope, inner, depth + 1)),
        // `app(Foo::class)` and `resolve(Foo::class)`.
        "function_call_expression" => {
            let function = node.child_by_field_name("function")?;
            if !matches!(text_of(function, source), "app" | "resolve") {
                return None;
            }
            let arguments = node.child_by_field_name("arguments")?;
            let first = named_children(arguments)
                .into_iter()
                .find(|n| n.kind() == "argument")?;
            let value = named_children(first).into_iter().last()?;
            let parts = named_children(value);
            (value.kind() == "class_constant_access_expression"
                && parts.len() == 2
                && text_of(parts[1], source) == "class")
                .then(|| scope_class(parts[0], source, scope.imports))
                .flatten()
        }
        _ => None,
    }
}

/// What a member of `class` gives back: a method's return type or a
/// property's type, `static` meaning `class` itself. A facade's method that
/// the facade only names is followed to the class it stands for.
fn member_returns(scope: &Here, class: &str, name: &str, want: Want) -> Option<String> {
    let here = scope.here();
    let decl = find_decl(scope.root, class, here)?;
    let found = find_member(scope.root, &decl, here, name, want);
    if let Some(returns) = found.as_ref().and_then(|member| member.returns.clone()) {
        return Some(if returns == "static" {
            decl.fqn.clone()
        } else {
            returns
        });
    }
    let only_documented = found.as_ref().map_or(true, |member| member.doc);
    if want == Want::Method && only_documented && is_facade(scope.root, &decl, here) {
        let (member, target) = through_facade(scope.root, &decl, name)?;
        let returns = member.returns?;
        return Some(if returns == "static" { target } else { returns });
    }
    None
}

/// The nearest function, method, closure or arrow function around a node.
fn enclosing_function(node: Node) -> Option<Node> {
    let mut at = node.parent();
    while let Some(here) = at {
        if matches!(
            here.kind(),
            "method_declaration" | "function_definition" | "anonymous_function" | "arrow_function"
        ) {
            return Some(here);
        }
        if CLASS_KINDS.contains(&here.kind()) {
            return None;
        }
        at = here.parent();
    }
    None
}

/// `@var Foo $name` or `@var Foo` in a comment: the type and the name, if
/// it says one.
fn var_comment(comment: &str) -> Option<(&str, Option<&str>)> {
    let rest = comment.split("@var").nth(1)?;
    let mut words = rest.split_whitespace();
    let ty = words.next()?;
    let var = words
        .next()
        .and_then(|word| word.strip_prefix('$'))
        .map(|word| word.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_'));
    Some((ty, var))
}

/// A variable's class where it is used: the last assignment before the use
/// in the same function (`$x = new Foo`, `$x = Http::withToken($t)`), a
/// `@var` comment, or the parameter's type. Closures look through `use`, and
/// arrow functions at the function around them.
fn var_type(scope: &Here, used: Node, name: &str, depth: usize) -> Option<String> {
    if depth > MAX_CHAIN {
        return None;
    }
    let source = scope.source;
    let function = enclosing_function(used)?;
    let before = used.start_byte();
    // The latest thing before the use that says what the variable is.
    let mut best: Option<(usize, Option<String>)> = None;
    let mut consider = |at: usize, class: Option<String>| {
        if best.as_ref().map_or(true, |(seen, _)| at >= *seen) {
            best = Some((at, class));
        }
    };
    let mut declared_here = false;
    if let Some(parameters) = function.child_by_field_name("parameters") {
        for parameter in named_children(parameters) {
            let Some(var) = parameter.child_by_field_name("name") else {
                continue;
            };
            if text_of(var, source).trim_start_matches(['&', '$']) != name {
                continue;
            }
            declared_here = true;
            let class = (parameter.kind() != "variadic_parameter")
                .then(|| parameter.child_by_field_name("type"))
                .flatten()
                .and_then(|ty| type_class(ty, source, scope.imports));
            consider(function.start_byte(), class);
        }
    }
    if let Some(body) = function.child_by_field_name("body") {
        let mut stack = vec![body];
        while let Some(node) = stack.pop() {
            if node.start_byte() >= before {
                continue;
            }
            match node.kind() {
                "assignment_expression" if node.end_byte() <= before => {
                    let left = node.child_by_field_name("left");
                    let mine = left.is_some_and(|left| {
                        left.kind() == "variable_name"
                            && text_of(left, source).trim_start_matches('$') == name
                    });
                    if mine {
                        declared_here = true;
                        let documented = node
                            .parent()
                            .filter(|p| p.kind() == "expression_statement")
                            .and_then(|statement| statement.prev_named_sibling())
                            .filter(|comment| comment.kind() == "comment")
                            .and_then(|comment| var_comment(text_of(comment, source)))
                            .filter(|(_, var)| var.map_or(true, |var| var == name))
                            .and_then(|(ty, _)| doc_class(ty, scope.imports));
                        let class = documented.or_else(|| {
                            node.child_by_field_name("right")
                                .and_then(|right| php_type(scope, right, depth + 1))
                        });
                        consider(node.start_byte(), class);
                        continue;
                    }
                }
                "comment" => {
                    if let Some((ty, Some(var))) = var_comment(text_of(node, source)) {
                        if var == name {
                            declared_here = true;
                            consider(node.start_byte(), doc_class(ty, scope.imports));
                        }
                    }
                    continue;
                }
                // Their own scopes.
                "anonymous_function" | "arrow_function" | "function_definition" => continue,
                _ => {}
            }
            if CLASS_KINDS.contains(&node.kind()) || node.kind() == "anonymous_class" {
                continue;
            }
            stack.extend(named_children(node));
        }
    }
    if let Some((_, class)) = best {
        let class = class?;
        return if class == "static" {
            enclosing_class(used, source, scope.imports)
        } else {
            Some(class)
        };
    }
    // A closure's `use ($x)` and an arrow function's whole outer scope.
    let shared = match function.kind() {
        "arrow_function" => true,
        "anonymous_function" => named_children(function)
            .into_iter()
            .find(|n| n.kind() == "anonymous_function_use_clause")
            .is_some_and(|clause| {
                named_children(clause)
                    .into_iter()
                    .any(|var| text_of(var, source).trim_start_matches(['&', '$']) == name)
            }),
        _ => false,
    };
    if shared && !declared_here {
        return var_type(scope, function, name, depth + 1);
    }
    None
}

// --- PHP: what can follow `->` and `::` -------------------------------------------

/// The members a completion list offers after `->`, `?->` or `::`.
#[derive(Serialize, Clone, Debug, Default)]
pub struct PhpMembers {
    /// The receiver's class, fully qualified.
    pub class: String,
    /// `instance` after `->`, `static` after `Foo::`, `self::` and
    /// `static::`, `parent` after `parent::`.
    pub access: &'static str,
    pub members: Vec<Member>,
}

/// What is offered after the `->` or `::` that ends just before `start`
/// (bytes), the name being typed running to `end`. Methods, properties,
/// constants and cases of the receiver's class and everything it inherits,
/// by how it is reached (`->` instance, `::` static and constants, `parent::`
/// also the parent's instance methods) and by visibility from where the
/// caret is: private only inside the class that declares it (or uses the
/// trait that does), protected inside the hierarchy. `None` when the
/// receiver cannot be worked out.
pub fn php_members_at(
    root: &Path,
    path: &str,
    text: &str,
    start: usize,
    end: usize,
) -> Option<PhpMembers> {
    let (start, end) = (start.min(text.len()), end.min(text.len()).max(start));
    // A call in place of the half-typed name, so the tree has an expression
    // to hang the receiver on.
    let probe = format!("{}__gnc(){}", &text[..start], &text[end..]);
    let tree = parse_php(&probe)?;
    let imports = imports_in(tree.root_node(), &probe);
    let scope = Here {
        root,
        path,
        source: &probe,
        imports: &imports,
    };
    let name = tree
        .root_node()
        .named_descendant_for_byte_range(start, start)?;
    let call = name.parent()?;
    if call.child_by_field_name("name") != Some(name) {
        return None;
    }
    let (class, access) = match call.kind() {
        "member_call_expression" | "nullsafe_member_call_expression" => (
            php_type(&scope, call.child_by_field_name("object")?, 0)?,
            "instance",
        ),
        "scoped_call_expression" => {
            let target = call.child_by_field_name("scope")?;
            let access = if text_of(target, &probe).eq_ignore_ascii_case("parent") {
                "parent"
            } else {
                "static"
            };
            let class = match target.kind() {
                "variable_name" => php_type(&scope, target, 0)?,
                _ => scope_class(target, &probe, &imports)?,
            };
            (class, access)
        }
        _ => return None,
    };
    let here = scope.here();
    let decl = find_decl(root, &class, here)?;
    // Where the caret is, for what is visible from there.
    let inside = enclosing_class(name, &probe, &imports);
    let mut own: HashSet<String> = HashSet::new();
    let mut lineage: HashSet<String> = HashSet::new();
    if let Some(inside) = &inside {
        if let Some(decl) = find_decl(root, inside, here) {
            own_scope(root, &decl, here, &mut own, 0);
            walk(root, &decl, here, &mut HashSet::new(), 0, &mut |one| {
                lineage.insert(one.fqn.to_ascii_lowercase());
                false
            });
        }
    }
    let mut related = lineage.contains(&decl.fqn.to_ascii_lowercase());
    if !related {
        if let Some(inside) = &inside {
            let inside = inside.to_ascii_lowercase();
            walk(root, &decl, here, &mut HashSet::new(), 0, &mut |one| {
                related = one.fqn.eq_ignore_ascii_case(&inside);
                related
            });
        }
    }
    let mut members: Vec<Member> = Vec::new();
    let mut names: HashSet<(bool, String)> = HashSet::new();
    walk(root, &decl, here, &mut HashSet::new(), 0, &mut |one| {
        for member in &one.members {
            let method = member.kind == "method";
            let key = if method {
                member.name.to_ascii_lowercase()
            } else {
                member.name.clone()
            };
            if names.insert((method, key)) {
                members.push(member.clone());
            }
        }
        false
    });
    members.retain(|member| {
        let fits = match access {
            "instance" => {
                matches!(member.kind, "method" | "property")
                    && !member.is_static
                    && !member.name.starts_with("__")
            }
            "parent" => member.kind != "property" || member.is_static,
            _ => member.is_static && !(member.kind == "method" && member.name.starts_with("__")),
        };
        let visible = match member.visibility {
            "private" => own.contains(&member.class.to_ascii_lowercase()),
            "protected" => related,
            _ => true,
        };
        fits && visible
    });
    if access != "instance" {
        members.push(Member {
            name: "class".into(),
            kind: "constant",
            is_static: true,
            visibility: "public",
            line: decl.line,
            column: decl.column,
            class: decl.fqn.clone(),
            path: decl.path.clone(),
            doc: false,
            signature: Some("string".into()),
            summary: Some("The fully qualified class name".into()),
            returns: None,
        });
    }
    Some(PhpMembers {
        class: decl.fqn.clone(),
        access,
        members,
    })
}

/// A class and the traits it uses, theirs too: where its private members
/// can be seen from.
fn own_scope(
    root: &Path,
    decl: &ClassDecl,
    here: Option<(&str, &str)>,
    into: &mut HashSet<String>,
    depth: usize,
) {
    if depth > MAX_DEPTH || !into.insert(decl.fqn.to_ascii_lowercase()) {
        return;
    }
    for used in &decl.traits {
        if let Some(found) = find_decl(root, used, here) {
            own_scope(root, &found, here, into, depth + 1);
        }
    }
}

/// When a method's class cannot be worked out: the classes the file imports
/// that have a method of that name (facades followed), for a list to pick
/// from. At most twenty.
pub fn php_member_candidates(root: &Path, path: &str, text: &str, name: &str) -> Vec<Jump> {
    let imports = php_imports(text);
    let here = Some((path, text));
    let mut fqns: Vec<&String> = imports.classes.values().collect();
    fqns.sort();
    let mut out: Vec<Jump> = Vec::new();
    let mut seen: HashSet<(String, u32)> = HashSet::new();
    for fqn in fqns {
        let Some(decl) = find_decl(root, fqn, here) else {
            continue;
        };
        let found = find_member(root, &decl, here, name, Want::Method)
            .filter(|member| !member.doc)
            .or_else(|| {
                is_facade(root, &decl, here)
                    .then(|| through_facade(root, &decl, name).map(|(member, _)| member))
                    .flatten()
            });
        let Some(member) = found else { continue };
        if !seen.insert((member.path.clone(), member.line)) {
            continue;
        }
        out.push(Jump {
            text: line_of(root, &member.path, member.line),
            dependency: is_dependency(root, &member.path),
            path: member.path,
            line: member.line,
            column: member.column,
            kind: member.kind,
            note: None,
        });
        if out.len() >= 20 {
            break;
        }
    }
    out
}

// --- TypeScript and JavaScript --------------------------------------------------

pub(crate) fn parse_script(text: &str, ext: &str) -> Option<Tree> {
    let language = if matches!(ext, "tsx" | "jsx") {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    let mut parser = Parser::new();
    parser.set_language(&language.into()).ok()?;
    parser.parse(text, None)
}

pub(crate) fn ext_of(path: &str) -> String {
    path.rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

pub(crate) fn string_value(node: Node, source: &str) -> String {
    text_of(node, source)
        .trim_matches(|c| c == '\'' || c == '"' || c == '`')
        .to_string()
}

/// Names imported from packages: local name to (specifier, exported name;
/// `default` and `*` for those).
pub(crate) fn package_imports(root: Node, source: &str) -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    for statement in named_children(root) {
        if statement.kind() != "import_statement" {
            continue;
        }
        let Some(from) = statement.child_by_field_name("source") else {
            continue;
        };
        let spec = string_value(from, source);
        if spec.starts_with('.') || spec.starts_with('/') || spec.is_empty() {
            continue;
        }
        for clause in named_children(statement) {
            if clause.kind() != "import_clause" {
                continue;
            }
            for part in named_children(clause) {
                match part.kind() {
                    "identifier" => {
                        out.insert(
                            text_of(part, source).to_string(),
                            (spec.clone(), "default".into()),
                        );
                    }
                    "namespace_import" => {
                        if let Some(name) = named_children(part)
                            .into_iter()
                            .find(|n| n.kind() == "identifier")
                        {
                            out.insert(
                                text_of(name, source).to_string(),
                                (spec.clone(), "*".into()),
                            );
                        }
                    }
                    "named_imports" => {
                        for specifier in named_children(part) {
                            let Some(name) = specifier.child_by_field_name("name") else {
                                continue;
                            };
                            let local = specifier.child_by_field_name("alias").unwrap_or(name);
                            out.insert(
                                text_of(local, source).to_string(),
                                (spec.clone(), text_of(name, source).to_string()),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

fn script_jump(root: &Path, path: &str, text: &str, byte: usize, ext: &str) -> Option<Landing> {
    let tree = parse_script(text, ext)?;
    let imports = package_imports(tree.root_node(), text);
    if imports.is_empty() {
        return None;
    }
    let node = tree
        .root_node()
        .named_descendant_for_byte_range(byte, byte)?;
    if !matches!(
        node.kind(),
        "identifier" | "type_identifier" | "property_identifier" | "nested_type_identifier"
    ) {
        return None;
    }
    let name = text_of(node, text);
    // `ns.member` and `ns.Type` on a namespace import.
    let through = node.parent().filter(|parent| {
        matches!(
            parent.kind(),
            "member_expression" | "nested_type_identifier"
        ) && Some(node) != parent.named_child(0)
    });
    let (spec, export) = if let Some(parent) = through {
        let object = text_of(parent.named_child(0)?, text);
        let (spec, what) = imports.get(object)?;
        if what != "*" {
            return None;
        }
        (spec.clone(), name.to_string())
    } else {
        imports.get(name).cloned()?
    };
    let file = resolve_package(root, path, &spec)?;
    let (file, line, column) = if export == "*" {
        (file, 1, 0)
    } else {
        find_export(root, &file, &export, 0).unwrap_or((file, 1, 0))
    };
    Some((file, line, column, "export", None))
}

/// The file a bare import specifier means, from the importing file's folder
/// up to the work tree's root, the way Node and TypeScript look: the
/// package's `types`, then its `exports`, then `main`, a `.d.ts` beside a
/// `.js` preferred, and `@types/` when the package has no types of its own.
pub fn resolve_package(root: &Path, from: &str, spec: &str) -> Option<String> {
    let mut parts = spec.splitn(if spec.starts_with('@') { 3 } else { 2 }, '/');
    let package = if spec.starts_with('@') {
        format!("{}/{}", parts.next()?, parts.next()?)
    } else {
        parts.next()?.to_string()
    };
    let sub = parts
        .next()
        .filter(|one| !one.is_empty())
        .map(str::to_string);
    let mut dir = Path::new(from)
        .parent()
        .map(files::slashed)
        .unwrap_or_default();
    loop {
        let candidate = join(&dir, &format!("node_modules/{package}"));
        if is_dir(root, &candidate) {
            if let Some(found) = package_entry(root, &candidate, sub.as_deref()) {
                if found.ends_with(".d.ts")
                    || found.ends_with(".d.mts")
                    || found.ends_with(".d.cts")
                {
                    return Some(found);
                }
                let typed = types_package(root, &dir, &package, sub.as_deref());
                return Some(typed.unwrap_or(found));
            }
        }
        let types = types_package(root, &dir, &package, sub.as_deref());
        if types.is_some() {
            return types;
        }
        if dir.is_empty() {
            return None;
        }
        dir = dir
            .rsplit_once('/')
            .map(|(up, _)| up.to_string())
            .unwrap_or_default();
    }
}

/// `@types/a__b` for `@a/b`.
fn types_package(root: &Path, dir: &str, package: &str, sub: Option<&str>) -> Option<String> {
    let mangled = package.trim_start_matches('@').replace('/', "__");
    let candidate = join(dir, &format!("node_modules/@types/{mangled}"));
    if !is_dir(root, &candidate) {
        return None;
    }
    package_entry(root, &candidate, sub)
}

fn package_entry(root: &Path, dir: &str, sub: Option<&str>) -> Option<String> {
    let json = read(root, &format!("{dir}/package.json"))
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .unwrap_or(serde_json::Value::Null);
    let mut tries: Vec<String> = Vec::new();
    match sub {
        None => {
            for key in ["types", "typings"] {
                if let Some(one) = json[key].as_str() {
                    tries.push(one.to_string());
                }
            }
            let exports = &json["exports"];
            let dot = if exports.is_object() && exports.get(".").is_some() {
                &exports["."]
            } else {
                exports
            };
            if let Some(one) = condition(dot) {
                tries.push(one);
            }
            if let Some(one) = json["main"].as_str() {
                tries.push(one.to_string());
            }
            tries.push("index".into());
        }
        Some(sub) => {
            if let Some(one) = condition(&json["exports"][format!("./{sub}")]) {
                tries.push(one);
            }
            tries.push(sub.to_string());
        }
    }
    tries
        .into_iter()
        .filter_map(|one| normal(&join(dir, &one)))
        .find_map(|base| module_file(root, &base))
}

/// The file an `exports` entry points at, types first.
fn condition(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(one) => Some(one.clone()),
        serde_json::Value::Object(map) => ["types", "import", "default", "require", "node"]
            .iter()
            .find_map(|key| map.get(*key).and_then(condition)),
        serde_json::Value::Array(many) => many.iter().find_map(condition),
        _ => None,
    }
}

/// A module path as an import writes it, to a file that is there: the
/// declaration file beside a script first.
pub(crate) fn module_file(root: &Path, base: &str) -> Option<String> {
    let mut tries: Vec<String> = Vec::new();
    for (script, types) in [(".js", ".d.ts"), (".mjs", ".d.mts"), (".cjs", ".d.cts")] {
        if let Some(stem) = base.strip_suffix(script) {
            tries.push(format!("{stem}{types}"));
        }
    }
    tries.push(base.to_string());
    for ext in [".d.ts", ".ts", ".tsx", ".js", ".mjs", ".cjs"] {
        tries.push(format!("{base}{ext}"));
    }
    for index in ["index.d.ts", "index.ts", "index.js"] {
        tries.push(format!("{base}/{index}"));
    }
    tries.into_iter().find(|one| exists(root, one))
}

/// Where a file declares the name it exports as `name`, following one
/// `export … from` or `export * from` to another file.
pub(crate) fn find_export(
    root: &Path,
    path: &str,
    name: &str,
    depth: usize,
) -> Option<(String, u32, u32)> {
    let text = read(root, path)?;
    let ext = ext_of(path);
    let tree = parse_script(&text, &ext)?;
    let program = tree.root_node();
    let at = |node: Node| {
        (
            path.to_string(),
            node.start_position().row as u32 + 1,
            node.start_position().column as u32,
        )
    };
    let mut stars: Vec<String> = Vec::new();
    let mut follow: Vec<(String, String)> = Vec::new();
    for statement in named_children(program) {
        if statement.kind() != "export_statement" {
            continue;
        }
        let source = statement
            .child_by_field_name("source")
            .map(|one| string_value(one, &text));
        if let Some(declaration) = statement.child_by_field_name("declaration") {
            if let Some(found) = declared(declaration, &text, name) {
                return Some(at(found));
            }
            if name == "default" && text_of(statement, &text).contains("export default") {
                return Some(at(declaration));
            }
        }
        if name == "default" {
            if let Some(value) = statement.child_by_field_name("value") {
                let local = text_of(value, &text);
                return local_declaration(program, &text, local)
                    .map(at)
                    .or(Some(at(value)));
            }
        }
        let clause = named_children(statement)
            .into_iter()
            .find(|child| child.kind() == "export_clause");
        match (clause, source) {
            (Some(clause), source) => {
                for specifier in named_children(clause) {
                    let Some(local) = specifier.child_by_field_name("name") else {
                        continue;
                    };
                    let exported = specifier.child_by_field_name("alias").unwrap_or(local);
                    if text_of(exported, &text) != name {
                        continue;
                    }
                    let local = text_of(local, &text).to_string();
                    match source {
                        Some(ref from) => follow.push((from.clone(), local)),
                        None => {
                            return local_declaration(program, &text, &local)
                                .map(at)
                                .or(Some(at(specifier)));
                        }
                    }
                }
            }
            (None, Some(from)) => stars.push(from),
            _ => {}
        }
    }
    if depth >= 1 {
        return None;
    }
    let dir = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let relative = |from: &str| -> Option<String> {
        if !from.starts_with('.') {
            return None;
        }
        module_file(root, &normal(&join(dir, from))?)
    };
    for (from, local) in follow {
        if let Some(file) = relative(&from) {
            if let Some(found) = find_export(root, &file, &local, depth + 1) {
                return Some(found);
            }
        }
    }
    for from in stars {
        if let Some(file) = relative(&from) {
            if let Some(found) = find_export(root, &file, name, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

/// The name node of a declaration called `name`, looking through `declare`.
fn declared<'a>(declaration: Node<'a>, source: &str, name: &str) -> Option<Node<'a>> {
    match declaration.kind() {
        "lexical_declaration" | "variable_declaration" => named_children(declaration)
            .into_iter()
            .filter(|one| one.kind() == "variable_declarator")
            .filter_map(|one| one.child_by_field_name("name"))
            .find(|one| text_of(*one, source) == name),
        "ambient_declaration" => named_children(declaration)
            .into_iter()
            .find_map(|inner| declared(inner, source, name)),
        _ => declaration
            .child_by_field_name("name")
            .filter(|one| text_of(*one, source) == name),
    }
}

/// A declaration of `name` at the top of a file, exported or not.
fn local_declaration<'a>(program: Node<'a>, source: &str, name: &str) -> Option<Node<'a>> {
    named_children(program).into_iter().find_map(|statement| {
        let inner = if statement.kind() == "export_statement" {
            statement.child_by_field_name("declaration")?
        } else {
            statement
        };
        declared(inner, source, name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_composer_autoload_arrays() {
        let root = Path::new("/work/shop");
        let text = r#"<?php

// autoload_psr4.php @generated by Composer

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
    'Revolution\\Google\\Sheets\\' => array($vendorDir . '/revolution/laravel-google-sheets/src'),
    'App\\' => array($baseDir . '/app', $baseDir . '/src'),
    'Out\\' => array($baseDir . '/../elsewhere'),
);
"#;
        let read = php_array(text, root);
        assert_eq!(
            read[0],
            (
                "Revolution\\Google\\Sheets\\".to_string(),
                vec!["vendor/revolution/laravel-google-sheets/src".to_string()]
            )
        );
        assert_eq!(read[1].1, vec!["app".to_string(), "src".to_string()]);
        // Out of the work tree: dropped.
        assert!(read[2].1.is_empty());
    }

    #[test]
    fn qualifies_names_through_the_imports() {
        let imports = php_imports(
            "<?php\nnamespace App\\Jobs;\nuse Foo\\Bar as Baz, Qux\\Quux;\nuse Acme\\{One, Two as Deux, Sub\\Three};\nuse function Acme\\helper;\n",
        );
        assert_eq!(imports.namespace, "App\\Jobs");
        assert_eq!(imports.qualify("Baz"), "Foo\\Bar");
        assert_eq!(imports.qualify("Quux"), "Qux\\Quux");
        assert_eq!(imports.qualify("Deux"), "Acme\\Two");
        assert_eq!(imports.qualify("Three"), "Acme\\Sub\\Three");
        assert_eq!(imports.qualify("Local"), "App\\Jobs\\Local");
        assert_eq!(imports.qualify("\\Root\\Thing"), "Root\\Thing");
        assert_eq!(imports.qualify("Baz\\Inner"), "Foo\\Bar\\Inner");
        assert!(!imports.classes.contains_key("helper"));
    }

    #[test]
    fn reads_method_tags() {
        let tag = method_tag(" * @method static \\X\\Factory spreadsheet(string $id)").unwrap();
        assert_eq!(tag.name, "spreadsheet");
        assert!(tag.is_static);
        assert_eq!(tag.returns.as_deref(), Some("\\X\\Factory"));
        assert!(method_tag(" * @return void").is_none());
    }
}
