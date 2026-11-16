//! Autocomplete in the editor: what can follow `->`, `::` and `.`, project
//! names for a word being typed, and the `use` line a PHP class needs.
//!
//! Nothing here walks the repository. Project names come from the symbol
//! index (`symbols.rs`), which is kept up to date by the watcher; a class's
//! members come from parsing the one file it is in, and its parents', found
//! through Composer's maps, the file's imports or the index (`deps.rs`), and
//! kept parsed until the file changes. Types are only what the code says:
//! a declared or documented type, `new Foo`, a return type. A receiver whose
//! class that does not settle gets no list rather than a guess.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tree_sitter::{Node, Tree};

use crate::deps;
use crate::symbols;

/// How far up `extends` a script class's members are looked for.
const MAX_DEPTH: usize = 10;
/// How many steps of a chain are followed.
const MAX_CHAIN: usize = 8;

fn text_of<'a>(node: Node, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

fn named_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn utf16_at(text: &str, byte: usize) -> u32 {
    text[..byte.min(text.len())]
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum()
}

// --- what follows `->`, `::` and `.` ---------------------------------------------

/// One member in the list.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MemberItem {
    pub name: String,
    /// `method`, `property`, `constant` or `case`.
    pub kind: &'static str,
    #[serde(rename = "static")]
    pub is_static: bool,
    pub visibility: &'static str,
    /// `(int $id): User`, or a property's type.
    pub signature: Option<String>,
    /// The first line of its docblock.
    pub summary: Option<String>,
    /// The class that declares it, by its short name.
    pub class: String,
    /// Declared in a parent or a trait, not in the receiver's class.
    pub inherited: bool,
}

/// What `complete_members` answers.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct MemberList {
    /// The receiver's class, by its short name; `None` when there is no
    /// receiver before the caret or it could not be worked out.
    pub receiver: Option<String>,
    /// `instance`, `static` or `parent`.
    pub access: Option<&'static str>,
    /// UTF-16 offset where the name being typed starts.
    pub from: u32,
    pub items: Vec<MemberItem>,
}

fn short(name: &str) -> &str {
    name.rsplit(['\\', '.']).next().unwrap_or(name)
}

/// The members that can follow the `->`, `?->` or `::` (PHP), or `.` and
/// `?.` (TypeScript, JavaScript) before the caret at `offset` (UTF-16, in
/// `text`, the editor's copy).
pub fn complete_members(root: &Path, path: &str, offset: u32, text: &str) -> MemberList {
    let byte = deps::byte_at_utf16(text, offset);
    let php = symbols::lang_of(path) == Some(symbols::Lang::Php);
    let script = symbols::lang_of(path).is_some_and(|_| !php);
    if !php && !script {
        return MemberList::default();
    }
    // The name being typed, wherever the caret is in it.
    let mut start = byte;
    while let Some(c) = text[..start].chars().next_back() {
        if !(c.is_alphanumeric() || c == '_' || (!php && c == '$')) {
            break;
        }
        start -= c.len_utf8();
    }
    let mut end = byte;
    while let Some(c) = text[end..].chars().next() {
        if !(c.is_alphanumeric() || c == '_' || (!php && c == '$')) {
            break;
        }
        end += c.len_utf8();
    }
    // `Foo::$static`: the `$` belongs to the name.
    if php && text[..start].ends_with("::$") {
        start -= 1;
    }
    let before = &text[..start];
    let from = utf16_at(text, start);
    let empty = MemberList {
        from,
        ..MemberList::default()
    };
    if php {
        if !(before.ends_with("->") || before.ends_with("::")) {
            return empty;
        }
        let Some(found) = deps::php_members_at(root, path, text, start, end) else {
            return empty;
        };
        let class = found.class.to_ascii_lowercase();
        let items = found
            .members
            .into_iter()
            .map(|member| MemberItem {
                inherited: member.class.to_ascii_lowercase() != class,
                class: short(&member.class).to_string(),
                name: if member.kind == "property" && member.is_static {
                    format!("${}", member.name)
                } else {
                    member.name
                },
                kind: member.kind,
                is_static: member.is_static,
                visibility: member.visibility,
                signature: member.signature,
                summary: member.summary,
            })
            .collect();
        return MemberList {
            receiver: Some(short(&found.class).to_string()),
            access: Some(found.access),
            from,
            items,
        };
    }
    if !before.ends_with('.') || before.ends_with("..") {
        return empty;
    }
    match script_members_at(root, path, text, start, end) {
        Some((class, access, items)) => MemberList {
            receiver: Some(class),
            access: Some(access),
            from,
            items,
        },
        None => empty,
    }
}

// --- project names -----------------------------------------------------------------

/// Project names for the word being typed: see [`symbols::complete_symbols`].
pub fn complete_symbols(
    root: &Path,
    prefix: &str,
    path: &str,
    limit: usize,
    changed: &[String],
) -> Vec<symbols::Completion> {
    // Asked for over IPC: never more than a list can show.
    symbols::complete_symbols(root, prefix, path, limit.min(200), changed)
}

// --- the `use` line a PHP class needs ------------------------------------------------

/// Text to insert at a UTF-16 offset.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Insert {
    pub offset: u32,
    pub text: String,
}

/// The `use` line a PHP file needs for the class `name` declared in
/// `class_path`, and where it goes: in sorted order among the `use` lines
/// already there, or after the `namespace` line, or after `<?php` when the
/// file has neither. `None` when nothing is needed or it would not be safe:
/// the class is in the global namespace or the file's own, it is imported
/// already, another import or the file's own class has the name, or the
/// file has `namespace X { }` blocks or more than one namespace.
pub fn complete_import(
    root: &Path,
    path: &str,
    text: &str,
    class_path: &str,
    name: &str,
) -> Option<Insert> {
    if !path.to_ascii_lowercase().ends_with(".php") || !class_path.ends_with(".php") {
        return None;
    }
    let declared = if class_path == path {
        text.to_string()
    } else {
        deps::read(root, class_path)?
    };
    let namespace = deps::php_imports(&declared).namespace;
    if namespace.is_empty() {
        return None;
    }
    let fqn = format!("{namespace}\\{name}");
    let imports = deps::php_imports(text);
    if imports.namespace.eq_ignore_ascii_case(&namespace) {
        return None;
    }
    if imports.classes.iter().any(|(alias, imported)| {
        imported.eq_ignore_ascii_case(&fqn) || alias.eq_ignore_ascii_case(name)
    }) {
        return None;
    }
    let tree = deps::parse_php(text)?;
    let program = tree.root_node();
    // The editor's text has `\n` only; a caller with the file's own text
    // gets the line ending it already uses.
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let line = format!("use {fqn};");
    let mut uses: Vec<Node> = Vec::new();
    let mut namespace_node: Option<Node> = None;
    let mut opening: Option<Node> = None;
    let mut declare: Option<Node> = None;
    for statement in named_children(program) {
        match statement.kind() {
            "php_tag" => opening = opening.or(Some(statement)),
            "declare_statement" => declare = declare.or(Some(statement)),
            "namespace_definition" => {
                // `namespace X { }` blocks, or a second `namespace Y;`: which
                // one the name was typed in is not known here, and a line in
                // the wrong one imports nothing.
                if statement.child_by_field_name("body").is_some() || namespace_node.is_some() {
                    return None;
                }
                namespace_node = Some(statement);
            }
            "namespace_use_declaration" => {
                let after = text_of(statement, text)
                    .trim_start()
                    .trim_start_matches("use")
                    .trim_start();
                if !after.starts_with("function ") && !after.starts_with("const ") {
                    uses.push(statement);
                }
            }
            // The file's own class of that name: a `use` beside it is a
            // fatal "name is already in use".
            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration" => {
                let own = statement
                    .child_by_field_name("name")
                    .map(|node| text_of(node, text))
                    .unwrap_or("");
                if own.eq_ignore_ascii_case(name) {
                    return None;
                }
            }
            _ => {}
        }
    }
    let key = |written: &str| {
        written
            .trim()
            .trim_start_matches("use")
            .trim()
            .trim_start_matches('\\')
            .trim_end_matches(';')
            .to_ascii_lowercase()
    };
    let wanted = key(&line);
    if !uses.is_empty() {
        // Before the first line that sorts after it, else after the last.
        if let Some(after) = uses.iter().find(|one| key(text_of(**one, text)) > wanted) {
            let line_start = text[..after.start_byte()]
                .rfind('\n')
                .map(|at| at + 1)
                .unwrap_or(0);
            let lead = &text[line_start..after.start_byte()];
            // Something else is on that line before it (`<?php use A;`): the
            // new line goes right before the statement, not before the tag.
            if !lead.trim().is_empty() {
                return Some(Insert {
                    offset: utf16_at(text, after.start_byte()),
                    text: format!("{line}{eol}"),
                });
            }
            return Some(Insert {
                offset: utf16_at(text, line_start),
                text: format!("{lead}{line}{eol}"),
            });
        }
        let last = uses.last()?;
        return Some(Insert {
            offset: utf16_at(text, last.end_byte()),
            text: format!("{eol}{line}"),
        });
    }
    let anchor = namespace_node.or(declare).or(opening)?;
    // `<?=` and the like open an expression, not a place for statements.
    if anchor.kind() == "php_tag" && !text_of(anchor, text).eq_ignore_ascii_case("<?php") {
        return None;
    }
    Some(Insert {
        offset: utf16_at(text, anchor.end_byte()),
        text: format!("{eol}{eol}{line}"),
    })
}

// --- TypeScript and JavaScript: classes and their members --------------------------

#[derive(Clone, Debug)]
struct ScriptMember {
    name: String,
    /// `method` or `property`.
    kind: &'static str,
    is_static: bool,
    visibility: &'static str,
    signature: Option<String>,
    summary: Option<String>,
    /// The type it returns or holds, as written: `Foo`, or `this`.
    returns: Option<String>,
}

#[derive(Clone, Debug)]
struct ScriptClass {
    name: String,
    /// What it extends, as written.
    extends: Vec<String>,
    members: Vec<ScriptMember>,
    /// Where its body starts and ends, in bytes.
    span: (usize, usize),
}

#[derive(Debug, Default)]
struct ScriptFile {
    classes: Vec<ScriptClass>,
    /// Local name to (specifier, exported name).
    imports: HashMap<String, (String, String)>,
}

type ParsedScripts = HashMap<String, (u64, Arc<ScriptFile>)>;

fn parsed_scripts() -> &'static Mutex<ParsedScripts> {
    static PARSED: OnceLock<Mutex<ParsedScripts>> = OnceLock::new();
    PARSED.get_or_init(|| Mutex::new(HashMap::new()))
}

fn script_tree(path: &str, text: &str) -> Option<(String, Tree)> {
    let (_, source) = symbols::source_for(path, text);
    let ext = deps::ext_of(path);
    let ext = if ext == "vue" { "ts".to_string() } else { ext };
    let tree = deps::parse_script(&source, &ext)?;
    Some((source, tree))
}

/// A script file's classes, interfaces and imports, kept by path until its
/// text changes.
fn script_file(path: &str, text: &str) -> Arc<ScriptFile> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    let hash = hasher.finish();
    if let Ok(cache) = parsed_scripts().lock() {
        if let Some((seen, file)) = cache.get(path) {
            if *seen == hash {
                return file.clone();
            }
        }
    }
    let file = Arc::new(match script_tree(path, text) {
        Some((source, tree)) => read_script(&tree, &source),
        None => ScriptFile::default(),
    });
    if let Ok(mut cache) = parsed_scripts().lock() {
        if cache.len() > 400 {
            cache.clear();
        }
        cache.insert(path.to_string(), (hash, file.clone()));
    }
    file
}

fn read_script(tree: &Tree, source: &str) -> ScriptFile {
    let root = tree.root_node();
    let mut file = ScriptFile {
        imports: script_imports(root, source),
        ..ScriptFile::default()
    };
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "class_declaration" | "abstract_class_declaration" | "interface_declaration" => {
                if let Some(class) = script_class(node, source) {
                    file.classes.push(class);
                }
            }
            "program"
            | "export_statement"
            | "ambient_declaration"
            | "statement_block"
            | "module"
            | "internal_module" => stack.extend(named_children(node)),
            _ => {}
        }
    }
    file
}

/// Every import, relative ones too: local name to (specifier, exported name).
fn script_imports(root: Node, source: &str) -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    for statement in named_children(root) {
        if statement.kind() != "import_statement" {
            continue;
        }
        let Some(from) = statement.child_by_field_name("source") else {
            continue;
        };
        let spec = deps::string_value(from, source);
        for clause in named_children(statement) {
            if clause.kind() != "import_clause" {
                continue;
            }
            for part in named_children(clause) {
                match part.kind() {
                    "identifier" => {
                        out.insert(
                            text_of(part, source).to_string(),
                            (spec.clone(), "default".to_string()),
                        );
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

fn squash(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() > 120 {
        let cut: String = joined.chars().take(119).collect();
        format!("{cut}…")
    } else {
        joined
    }
}

/// The class a type annotation names: `Foo`, `Foo<T>`, `ns.Foo`, the first
/// of `Foo | null`, `this`. `None` for anything else.
fn type_name(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "type_annotation" | "parenthesized_type" => named_children(node)
            .into_iter()
            .find_map(|inner| type_name(inner, source)),
        "type_identifier" | "identifier" => Some(text_of(node, source).to_string()),
        "this_type" | "this" => Some("this".to_string()),
        "generic_type" => type_name(node.child_by_field_name("name")?, source),
        "nested_type_identifier" => named_children(node)
            .last()
            .map(|one| text_of(*one, source).to_string()),
        "union_type" => named_children(node).into_iter().find_map(|one| {
            let written = text_of(one, source).trim();
            if matches!(written, "null" | "undefined") {
                None
            } else {
                type_name(one, source)
            }
        }),
        _ => None,
    }
}

/// The comment right above a member, its first line.
fn summary_of(node: Node, source: &str) -> Option<String> {
    let before = node.prev_named_sibling()?;
    let text = text_of(before, source);
    if before.kind() != "comment" || !text.starts_with("/**") {
        return None;
    }
    text.lines()
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

fn script_class(node: Node, source: &str) -> Option<ScriptClass> {
    let name = text_of(node.child_by_field_name("name")?, source).to_string();
    let body = node.child_by_field_name("body")?;
    let mut extends = Vec::new();
    for child in named_children(node) {
        match child.kind() {
            "class_heritage" => {
                for clause in named_children(child) {
                    if clause.kind() != "extends_clause" {
                        continue;
                    }
                    for value in named_children(clause) {
                        match value.kind() {
                            "identifier" | "type_identifier" => {
                                extends.push(text_of(value, source).to_string())
                            }
                            "member_expression" => {
                                if let Some(property) = value.child_by_field_name("property") {
                                    extends.push(text_of(property, source).to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            "extends_type_clause" => {
                extends.extend(
                    named_children(child)
                        .into_iter()
                        .filter_map(|one| type_name(one, source)),
                );
            }
            _ => {}
        }
    }
    let mut members = Vec::new();
    for item in named_children(body) {
        let kind = match item.kind() {
            "method_definition" | "method_signature" | "abstract_method_signature" => "method",
            "public_field_definition" | "property_signature" => "property",
            _ => continue,
        };
        let Some(name) = item.child_by_field_name("name") else {
            continue;
        };
        let written = text_of(name, source).to_string();
        let mut is_static = false;
        let mut visibility = if written.starts_with('#') {
            "private"
        } else {
            "public"
        };
        for at in 0..item.child_count() {
            let Some(child) = item.child(at) else {
                continue;
            };
            match child.kind() {
                "static" => is_static = true,
                "accessibility_modifier" => {
                    visibility = match text_of(child, source) {
                        "private" => "private",
                        "protected" => "protected",
                        _ => "public",
                    }
                }
                _ => {}
            }
        }
        let (signature, returns) = if kind == "method" {
            let params = item
                .child_by_field_name("parameters")
                .map(|one| squash(text_of(one, source)))
                .unwrap_or_else(|| "()".into());
            let returned = item.child_by_field_name("return_type");
            let signature = match returned {
                Some(ty) => format!("{params}{}", squash(text_of(ty, source))),
                None => params,
            };
            (
                Some(signature),
                returned.and_then(|ty| type_name(ty, source)),
            )
        } else {
            let ty = item.child_by_field_name("type");
            (
                ty.map(|ty| squash(text_of(ty, source).trim_start_matches(':').trim())),
                ty.and_then(|ty| type_name(ty, source)),
            )
        };
        members.push(ScriptMember {
            name: written,
            kind,
            is_static,
            visibility,
            signature,
            summary: summary_of(item, source),
            returns,
        });
    }
    Some(ScriptClass {
        name,
        extends,
        members,
        span: (node.start_byte(), node.end_byte()),
    })
}

/// A class found: the file it is in and the class.
#[derive(Clone)]
struct Found {
    path: String,
    class: ScriptClass,
}

impl Found {
    fn key(&self) -> (String, String) {
        (self.path.clone(), self.class.name.clone())
    }
}

/// The scripts being looked at, read once per request.
struct Scripts<'a> {
    root: &'a Path,
    /// The editor's file and text, which may be ahead of the disk.
    path: &'a str,
    text: &'a str,
}

impl Scripts<'_> {
    fn file(&self, path: &str) -> Option<Arc<ScriptFile>> {
        if path == self.path {
            return Some(script_file(path, self.text));
        }
        let text = deps::read(self.root, path)?;
        Some(script_file(path, &text))
    }

    /// The class `name` means in the file at `from`: declared there,
    /// imported (relative path or package), or the only one of that name
    /// the index knows.
    fn class(&self, from: &str, name: &str) -> Option<Found> {
        let file = self.file(from)?;
        if let Some(class) = file.classes.iter().find(|class| class.name == name) {
            return Some(Found {
                path: from.to_string(),
                class: class.clone(),
            });
        }
        if let Some((spec, exported)) = file.imports.get(name) {
            let target = if spec.starts_with('.') {
                let dir = from.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
                deps::module_file(self.root, &deps::normal(&deps::join(dir, spec))?)
            } else {
                deps::resolve_package(self.root, from, spec)
            }?;
            let (path, line, _) = deps::find_export(self.root, &target, exported, 0).unwrap_or((
                target.clone(),
                0,
                0,
            ));
            let file = self.file(&path)?;
            let text = deps::read(self.root, &path).unwrap_or_default();
            let class = file
                .classes
                .iter()
                .find(|class| &class.name == exported)
                .or_else(|| {
                    file.classes
                        .iter()
                        .find(|class| line > 0 && utf16_line(&text, class.span.0) == line)
                })?
                .clone();
            return Some(Found { path, class });
        }
        let paths = symbols::class_paths(self.root, name, false);
        let [only] = paths.as_slice() else {
            return None;
        };
        let file = self.file(only)?;
        let class = file
            .classes
            .iter()
            .find(|class| class.name == name)?
            .clone();
        Some(Found {
            path: only.clone(),
            class,
        })
    }

    /// A class and what it extends, nearest first, ten levels at most.
    fn lineage(&self, start: &Found) -> Vec<Found> {
        let mut out: Vec<Found> = Vec::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut queue = vec![(start.clone(), 0)];
        while let Some((found, depth)) = queue.pop() {
            if depth > MAX_DEPTH || !seen.insert(found.key()) {
                continue;
            }
            for parent in found.class.extends.iter().rev() {
                if let Some(next) = self.class(&found.path, parent) {
                    queue.push((next, depth + 1));
                }
            }
            out.push(found);
        }
        out
    }

    fn member(&self, class: &Found, name: &str) -> Option<(ScriptMember, Found)> {
        self.lineage(class).into_iter().find_map(|found| {
            found
                .class
                .members
                .iter()
                .find(|member| member.name == name)
                .cloned()
                .map(|member| (member, found))
        })
    }

    /// What a member gives back, as a class: `this` keeps `class`.
    fn returns(&self, class: &Found, name: &str) -> Option<Found> {
        let (member, owner) = self.member(class, name)?;
        let returns = member.returns?;
        if returns == "this" {
            return Some(class.clone());
        }
        self.class(&owner.path, &returns)
    }
}

fn utf16_line(text: &str, byte: usize) -> u32 {
    text[..byte.min(text.len())].matches('\n').count() as u32 + 1
}

/// The class around a node, by name.
fn enclosing_class_name(node: Node, source: &str) -> Option<String> {
    let mut at = node.parent();
    while let Some(here) = at {
        if matches!(
            here.kind(),
            "class_declaration" | "abstract_class_declaration" | "class"
        ) {
            return here
                .child_by_field_name("name")
                .map(|name| text_of(name, source).to_string());
        }
        at = here.parent();
    }
    None
}

/// What a script expression holds: a class, and whether it is the class
/// itself (`Foo.`) rather than an instance.
fn script_type(scripts: &Scripts, source: &str, node: Node, depth: usize) -> Option<(Found, bool)> {
    if depth > MAX_CHAIN {
        return None;
    }
    match node.kind() {
        "this" => {
            let name = enclosing_class_name(node, source)?;
            Some((scripts.class(scripts.path, &name)?, false))
        }
        "identifier" => {
            let name = text_of(node, source);
            if let Some(found) = variable_type(scripts, source, node, name, depth) {
                return found.map(|class| (class, false));
            }
            if name.starts_with(char::is_uppercase) {
                return Some((scripts.class(scripts.path, name)?, true));
            }
            None
        }
        "member_expression" => {
            let (object, _) = script_type(
                scripts,
                source,
                node.child_by_field_name("object")?,
                depth + 1,
            )?;
            let property = node.child_by_field_name("property")?;
            Some((scripts.returns(&object, text_of(property, source))?, false))
        }
        "call_expression" => {
            let function = node.child_by_field_name("function")?;
            if function.kind() != "member_expression" {
                return None;
            }
            let (object, _) = script_type(
                scripts,
                source,
                function.child_by_field_name("object")?,
                depth + 1,
            )?;
            let property = function.child_by_field_name("property")?;
            Some((scripts.returns(&object, text_of(property, source))?, false))
        }
        "new_expression" => {
            let constructor = node.child_by_field_name("constructor")?;
            (constructor.kind() == "identifier")
                .then(|| scripts.class(scripts.path, text_of(constructor, source)))
                .flatten()
                .map(|class| (class, false))
        }
        "parenthesized_expression" | "non_null_expression" => named_children(node)
            .into_iter()
            .next()
            .and_then(|inner| script_type(scripts, source, inner, depth + 1)),
        "as_expression" => {
            let ty = named_children(node).into_iter().nth(1)?;
            let name = type_name(ty, source)?;
            Some((scripts.class(scripts.path, &name)?, false))
        }
        _ => None,
    }
}

/// A variable's class where it is used: `Some(None)` when it is a variable
/// whose class cannot be worked out, `None` when no variable of that name
/// is in sight (it may be a class).
fn variable_type(
    scripts: &Scripts,
    source: &str,
    used: Node,
    name: &str,
    depth: usize,
) -> Option<Option<Found>> {
    let before = used.start_byte();
    let mut at = used.parent();
    while let Some(scope) = at {
        // Parameters of the functions around it.
        if let Some(parameters) = scope.child_by_field_name("parameters") {
            for parameter in named_children(parameters) {
                let pattern = parameter.child_by_field_name("pattern");
                if !pattern.is_some_and(|p| text_of(p, source) == name) {
                    continue;
                }
                let class = parameter
                    .child_by_field_name("type")
                    .and_then(|ty| type_name(ty, source))
                    .and_then(|ty| scripts.class(scripts.path, &ty));
                return Some(class);
            }
        }
        // Declarations in the blocks around it, the nearest before the use.
        if matches!(scope.kind(), "statement_block" | "program") {
            let mut last: Option<Node> = None;
            for statement in named_children(scope) {
                if statement.start_byte() >= before {
                    break;
                }
                if !matches!(
                    statement.kind(),
                    "lexical_declaration" | "variable_declaration"
                ) {
                    continue;
                }
                for declarator in named_children(statement) {
                    let is_it = declarator.kind() == "variable_declarator"
                        && declarator
                            .child_by_field_name("name")
                            .is_some_and(|n| text_of(n, source) == name);
                    if is_it {
                        last = Some(declarator);
                    }
                }
            }
            if let Some(declarator) = last {
                if let Some(ty) = declarator
                    .child_by_field_name("type")
                    .and_then(|ty| type_name(ty, source))
                {
                    return Some(scripts.class(scripts.path, &ty));
                }
                return Some(
                    declarator
                        .child_by_field_name("value")
                        .filter(|value| value.end_byte() <= before)
                        .and_then(|value| script_type(scripts, source, value, depth + 1))
                        .map(|(class, _)| class),
                );
            }
        }
        at = scope.parent();
    }
    None
}

/// The members after the `.` before `start` in a script.
fn script_members_at(
    root: &Path,
    path: &str,
    text: &str,
    start: usize,
    end: usize,
) -> Option<(String, &'static str, Vec<MemberItem>)> {
    let probe = format!("{}__gnc{}", &text[..start], &text[end.max(start)..]);
    let (source, tree) = script_tree(path, &probe)?;
    let scripts = Scripts {
        root,
        path,
        text: &probe,
    };
    let name = tree
        .root_node()
        .named_descendant_for_byte_range(start, start)?;
    let access = name.parent().filter(|p| p.kind() == "member_expression")?;
    if access.child_by_field_name("property") != Some(name) {
        return None;
    }
    let object = access.child_by_field_name("object")?;
    let (class, is_class) = script_type(&scripts, &source, object, 0)?;
    let inside = enclosing_class_name(name, &source).and_then(|own| scripts.class(path, &own));
    let lineage = scripts.lineage(&class);
    let related = inside.as_ref().is_some_and(|inside| {
        lineage.iter().any(|one| one.key() == inside.key())
            || scripts
                .lineage(inside)
                .iter()
                .any(|one| one.key() == class.key())
    });
    let mut items: Vec<MemberItem> = Vec::new();
    let mut names: HashSet<String> = HashSet::new();
    for found in &lineage {
        for member in &found.class.members {
            if member.is_static != is_class || member.name == "constructor" {
                continue;
            }
            let visible = match member.visibility {
                "private" => inside
                    .as_ref()
                    .is_some_and(|inside| inside.key() == found.key()),
                "protected" => related,
                _ => true,
            };
            if !visible || !names.insert(member.name.clone()) {
                continue;
            }
            items.push(MemberItem {
                name: member.name.clone(),
                kind: member.kind,
                is_static: member.is_static,
                visibility: member.visibility,
                signature: member.signature.clone(),
                summary: member.summary.clone(),
                class: found.class.name.clone(),
                inherited: found.key() != class.key(),
            });
        }
    }
    let access = if is_class { "static" } else { "instance" };
    Some((class.class.name.clone(), access, items))
}

/// ⌘-click on a member called on a script expression whose class the code
/// says: `a.b().c()` through the return types the methods declare. The
/// place it is declared, as (path, line, column).
pub fn script_member_definition(
    root: &Path,
    path: &str,
    text: &str,
    byte: usize,
) -> Option<(String, u32, u32)> {
    let (source, tree) = script_tree(path, text)?;
    let scripts = Scripts { root, path, text };
    let node = tree
        .root_node()
        .named_descendant_for_byte_range(byte, byte)?;
    if node.kind() != "property_identifier" {
        return None;
    }
    let access = node.parent().filter(|p| p.kind() == "member_expression")?;
    if access.child_by_field_name("property") != Some(node) {
        return None;
    }
    let (class, _) = script_type(&scripts, &source, access.child_by_field_name("object")?, 0)?;
    let name = text_of(node, &source);
    let (_, owner) = scripts.member(&class, name)?;
    let owner_text = if owner.path == path {
        text.to_string()
    } else {
        deps::read(root, &owner.path)?
    };
    let (owner_source, owner_tree) = script_tree(&owner.path, &owner_text)?;
    // The member's own name node, inside the class's span.
    let mut stack = vec![owner_tree.root_node()];
    while let Some(at) = stack.pop() {
        if at.end_byte() < owner.class.span.0 || at.start_byte() > owner.class.span.1 {
            continue;
        }
        if matches!(
            at.kind(),
            "method_definition"
                | "method_signature"
                | "abstract_method_signature"
                | "public_field_definition"
                | "property_signature"
        ) {
            if let Some(found) = at
                .child_by_field_name("name")
                .filter(|n| text_of(*n, &owner_source) == name)
            {
                let position = found.start_position();
                return Some((
                    owner.path.clone(),
                    position.row as u32 + 1,
                    position.column as u32,
                ));
            }
        }
        stack.extend(named_children(at));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_script_classes_and_their_types() {
        let file = script_file(
            "x.ts",
            "export class Api extends Base {\n  private token: string\n  /** Sends it. */\n  static make(): Api { return new Api() }\n  get(url: string): Request | null { return null }\n  self(): this { return this }\n}\n",
        );
        let api = &file.classes[0];
        assert_eq!(api.extends, ["Base"]);
        let names: Vec<(&str, &str, bool, &str)> = api
            .members
            .iter()
            .map(|m| (m.name.as_str(), m.kind, m.is_static, m.visibility))
            .collect();
        assert_eq!(
            names,
            [
                ("token", "property", false, "private"),
                ("make", "method", true, "public"),
                ("get", "method", false, "public"),
                ("self", "method", false, "public"),
            ]
        );
        assert_eq!(api.members[1].summary.as_deref(), Some("Sends it."));
        assert_eq!(api.members[2].returns.as_deref(), Some("Request"));
        assert_eq!(api.members[3].returns.as_deref(), Some("this"));
        assert_eq!(
            api.members[2].signature.as_deref(),
            Some("(url: string): Request | null")
        );
    }
}
