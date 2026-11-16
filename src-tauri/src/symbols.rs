//! Code navigation and unused code: what a name in a file is, where it is
//! defined, who uses it, and what nothing uses.
//!
//! Every file the work tree has in a language below is parsed with
//! tree-sitter, and each language's own tags query (the one GitHub's code
//! navigation uses) picks out the definitions and the references. That is
//! names, not types. For PHP and TypeScript, `receivers.rs` then walks the
//! same tree for the class each method call is made on, where the code says
//! it plainly, so a method's usages split in two: definite when the receiver
//! is its class or one in that hierarchy, possible when the receiver is
//! unknown. A call on another class is not counted. Methods the framework
//! calls (`handle` on a command, a controller action, `__construct`, a Tauri
//! command) are counted by the uses of their class, or of their name in
//! strings, instead.
//!
//! The index lives in memory, one per work tree. It is built on a thread of
//! its own when a repository opens and kept current a file at a time by the
//! watcher. Ignored files, files over 1 MB, and dependencies and build output
//! (`vendor`, `node_modules`, `dist`, …, see `generated`) are left out.
//!
//! Graying out unused code is where being wrong costs the most, so it is
//! careful on purpose:
//!
//! - A function or method is only marked when its name appears nowhere else in
//!   the repository's text, checked with the text search over every file git
//!   does not ignore, whatever its language. Strings count, so a route naming
//!   `'show'` or a config naming a class keeps it alive.
//! - Only private functions and methods, and local variables, are dimmed.
//!   Anything public or exported gets a lighter dotted hint: something outside
//!   the repository may call it.
//! - Entry points are never marked: constructors and magic methods, controller
//!   actions, `handle()`, tests and everything under a tests folder, Vue
//!   lifecycle hooks, `main`, anything carrying an attribute such as
//!   `#[test]` or `#[tauri::command]`, and trait implementations. A public
//!   method of a class that extends or implements something may be an
//!   override the framework calls, so that is left alone too.
//! - A local variable is dimmed when its name appears once in the function
//!   that declares it, and a function that could read its variables by name
//!   (`compact`, `extract`, `$$`, `eval`, an `include`) is left alone.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};

use serde::Serialize;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::files;
use crate::receivers::{self, ClassInfo};
use crate::search;
use crate::state::AppState;

/// Files bigger than this are not parsed: generated code, minified bundles.
pub const MAX_FILE: u64 = 1024 * 1024;

/// How many usages or definitions a popup is given at most.
const MAX_LOCATIONS: usize = 300;

// --- languages ---------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Lang {
    Php,
    Js,
    Ts,
    Tsx,
    Rust,
    Python,
    Go,
}

/// Which files can see each other's names. A PHP method is never the
/// definition a TypeScript call is looking for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    Php,
    Script,
    Rust,
    Python,
    Go,
}

impl Lang {
    fn family(self) -> Family {
        match self {
            Lang::Php => Family::Php,
            Lang::Js | Lang::Ts | Lang::Tsx => Family::Script,
            Lang::Rust => Family::Rust,
            Lang::Python => Family::Python,
            Lang::Go => Family::Go,
        }
    }
}

/// The language of a path, by its name. Blade templates are PHP in name only.
pub fn lang_of(path: &str) -> Option<Lang> {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if name.ends_with(".blade.php") || name.ends_with(".d.ts") {
        return None;
    }
    let ext = name.rsplit_once('.').map(|(_, ext)| ext)?;
    Some(match ext {
        // A `.phtml` template is PHP inside HTML, which the PHP grammar reads
        // as it is: what a template calls on its block counts as a usage.
        "php" | "phtml" => Lang::Php,
        "js" | "mjs" | "cjs" | "jsx" => Lang::Js,
        "ts" | "mts" | "cts" => Lang::Ts,
        "tsx" => Lang::Tsx,
        "vue" => Lang::Ts,
        "rs" => Lang::Rust,
        "py" => Lang::Python,
        "go" => Lang::Go,
        _ => return None,
    })
}

fn family_of(path: &str) -> Option<Family> {
    lang_of(path).map(Lang::family)
}

/// The text tree-sitter is given. For a `.vue` file that is the `<script>`
/// blocks with everything else blanked out, newlines kept, so every offset
/// in it is the same as in the file.
pub(crate) fn source_for(path: &str, text: &str) -> (Lang, String) {
    let lang = lang_of(path).unwrap_or(Lang::Ts);
    if !path.to_ascii_lowercase().ends_with(".vue") {
        return (lang, text.to_string());
    }
    let mut out: Vec<u8> = text
        .bytes()
        .map(|b| if b == b'\n' { b'\n' } else { b' ' })
        .collect();
    let lower = text.to_ascii_lowercase();
    let mut at = 0;
    let mut typed = false;
    while let Some(open) = lower[at..].find("<script") {
        let open = at + open;
        let Some(end_of_tag) = lower[open..].find('>') else {
            break;
        };
        let body = open + end_of_tag + 1;
        let tag = &lower[open..body];
        if tag.contains("lang=\"ts\"") || tag.contains("lang='ts'") || tag.contains("lang=\"tsx\"")
        {
            typed = true;
        }
        let close = lower[body..]
            .find("</script")
            .map(|found| body + found)
            .unwrap_or(text.len());
        out[body..close].copy_from_slice(&text.as_bytes()[body..close]);
        at = close;
    }
    // Blanking whole bytes can split a character; spaces in its place keep
    // the offsets and make it valid again.
    let blanked = String::from_utf8(out).unwrap_or_else(|e| {
        let mut bytes = e.into_bytes();
        for b in bytes.iter_mut() {
            if *b >= 0x80 {
                *b = b' ';
            }
        }
        String::from_utf8(bytes).unwrap_or_default()
    });
    (if typed { Lang::Ts } else { Lang::Js }, blanked)
}

const PHP_EXTRA: &str = r#"
(nullsafe_member_call_expression name: (name) @name) @reference.call
"#;

const RUST_EXTRA: &str = r#"
(call_expression function: (scoped_identifier name: (identifier) @name)) @reference.call
(call_expression function: (generic_function function: (identifier) @name)) @reference.call
(call_expression function: (generic_function function: (field_expression field: (field_identifier) @name))) @reference.call
(call_expression function: (generic_function function: (scoped_identifier name: (identifier) @name))) @reference.call
(macro_invocation macro: (scoped_identifier name: (identifier) @name)) @reference.call
"#;

struct Grammar {
    language: Language,
    query: Query,
    /// Per capture: `Some((is_definition, kind))` for the role captures.
    roles: Vec<Option<(bool, &'static str)>>,
    name: u32,
}

fn kind_of(suffix: &str) -> &'static str {
    match suffix {
        "function" => "function",
        "method" => "method",
        "class" => "class",
        "interface" => "interface",
        "module" => "module",
        "field" => "field",
        "constant" => "constant",
        "type" => "type",
        "macro" => "macro",
        "call" => "call",
        "implementation" => "implementation",
        _ => "other",
    }
}

fn make(language: Language, sources: &[&str]) -> Option<Grammar> {
    let source = sources.join("\n");
    let query = match Query::new(&language, &source) {
        Ok(query) => query,
        Err(error) => {
            eprintln!("symbols: a tags query did not compile: {error}");
            return None;
        }
    };
    let mut name = u32::MAX;
    let roles = query
        .capture_names()
        .iter()
        .enumerate()
        .map(|(at, capture)| {
            if *capture == "name" {
                name = at as u32;
            }
            if let Some(rest) = capture.strip_prefix("definition.") {
                Some((true, kind_of(rest)))
            } else {
                capture
                    .strip_prefix("reference.")
                    .map(|rest| (false, kind_of(rest)))
            }
        })
        .collect();
    Some(Grammar {
        language,
        query,
        roles,
        name,
    })
}

fn grammar(lang: Lang) -> Option<&'static Grammar> {
    static GRAMMARS: OnceLock<HashMap<Lang, Option<Grammar>>> = OnceLock::new();
    GRAMMARS
        .get_or_init(|| {
            let mut all = HashMap::new();
            all.insert(
                Lang::Php,
                make(
                    tree_sitter_php::LANGUAGE_PHP.into(),
                    &[tree_sitter_php::TAGS_QUERY, PHP_EXTRA],
                ),
            );
            all.insert(
                Lang::Js,
                make(
                    tree_sitter_javascript::LANGUAGE.into(),
                    &[tree_sitter_javascript::TAGS_QUERY],
                ),
            );
            all.insert(
                Lang::Ts,
                make(
                    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                    &[
                        tree_sitter_javascript::TAGS_QUERY,
                        tree_sitter_typescript::TAGS_QUERY,
                    ],
                ),
            );
            all.insert(
                Lang::Tsx,
                make(
                    tree_sitter_typescript::LANGUAGE_TSX.into(),
                    &[
                        tree_sitter_javascript::TAGS_QUERY,
                        tree_sitter_typescript::TAGS_QUERY,
                    ],
                ),
            );
            all.insert(
                Lang::Rust,
                make(
                    tree_sitter_rust::LANGUAGE.into(),
                    &[tree_sitter_rust::TAGS_QUERY, RUST_EXTRA],
                ),
            );
            all.insert(
                Lang::Python,
                make(
                    tree_sitter_python::LANGUAGE.into(),
                    &[tree_sitter_python::TAGS_QUERY],
                ),
            );
            all.insert(
                Lang::Go,
                make(
                    tree_sitter_go::LANGUAGE.into(),
                    &[tree_sitter_go::TAGS_QUERY],
                ),
            );
            all
        })
        .get(&lang)
        .and_then(Option::as_ref)
}

// --- positions ---------------------------------------------------------------

/// Byte offsets to lines and to UTF-16 offsets, which is how the editor
/// counts.
struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
    utf16: Vec<u32>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        let mut utf16 = vec![0u32];
        let mut units = 0u32;
        for (at, ch) in text.char_indices() {
            units += ch.len_utf16() as u32;
            if ch == '\n' {
                starts.push(at + 1);
                utf16.push(units);
            }
        }
        Lines {
            text,
            starts,
            utf16,
        }
    }

    /// `(line from 1, UTF-16 column in it, UTF-16 offset in the text)`.
    fn at(&self, byte: usize) -> (u32, u32, u32) {
        let line = self.starts.partition_point(|start| *start <= byte) - 1;
        let start = self.starts[line];
        let column = self
            .text
            .get(start..byte)
            .map(|part| part.encode_utf16().count() as u32)
            .unwrap_or(0);
        (line as u32 + 1, column, self.utf16[line] + column)
    }
}

fn byte_at_utf16(text: &str, offset: u32) -> usize {
    let mut units = 0u32;
    for (at, ch) in text.char_indices() {
        if units >= offset {
            return at;
        }
        units += ch.len_utf16() as u32;
    }
    text.len()
}

// --- what one file holds -----------------------------------------------------

/// How a reference reaches its name: `foo()`, `$x->foo()`, `Foo::foo()`, or a
/// type. A method is looked for among methods, a function among functions.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    Plain,
    Member,
    Scoped,
    Type,
}

/// One name in a file.
#[derive(Clone, Debug, Serialize)]
pub struct Symbol {
    pub name: String,
    /// `function`, `method`, `class`… for a definition; `call`, `class`,
    /// `type`… for a reference.
    pub kind: &'static str,
    pub definition: bool,
    pub shape: Shape,
    pub line: u32,
    /// UTF-16, from the start of the line.
    pub column: u32,
    /// UTF-16 offsets of the name in the text.
    pub from: u32,
    pub to: u32,
    /// Public or exported. Only means something for a definition.
    pub public: bool,
    /// A definition something outside the code calls: never marked unused.
    pub entry: bool,
    /// UTF-16 offsets of the whole definition, for dimming all of it.
    #[serde(skip)]
    pub block: (u32, u32),
    /// For a method call, the class its receiver is; for a class name, the
    /// class it means once `use` aliases are undone; for a route action,
    /// its controller; for a method, the class it is in; for a plain call
    /// to an imported function, the module it comes from. PHP and
    /// TypeScript only, and only when the tree says so plainly.
    #[serde(skip)]
    pub via: Option<Box<str>>,
    /// For a definition: how the framework calls it, if it does.
    #[serde(skip)]
    pub hook: Hook,
}

/// A definition the framework calls, whose usages are the uses of what it
/// belongs to rather than calls by its name.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Hook {
    #[default]
    None,
    /// `handle` on a command, job, listener or middleware, `__invoke`,
    /// `__construct`, a lifecycle hook on a class: counted by the uses of
    /// its class, and for an artisan command by its name in strings.
    Class,
    /// A controller action: the uses of its controller, and the routes
    /// that name the action.
    Action,
    /// `#[tauri::command]`: the `invoke('name')` calls in the front end.
    Tauri,
    /// A lifecycle hook in an options object: nothing in the code calls it.
    Framework,
}

impl Symbol {
    fn callable(&self) -> bool {
        self.definition && (self.kind == "function" || self.kind == "method")
    }
}

fn parse(lang: Lang, source: &str) -> Option<Tree> {
    let grammar = grammar(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&grammar.language).ok()?;
    parser.parse(source, None)
}

fn text_of<'a>(node: Node, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

fn shape_of(kind: &str, name: Node) -> Shape {
    if matches!(kind, "class" | "implementation" | "type" | "interface") {
        return Shape::Type;
    }
    let Some(parent) = name.parent() else {
        return Shape::Plain;
    };
    let mut around = parent.kind();
    // `(qualified_name (name))` in PHP: the call is one further up.
    if around == "qualified_name" || around == "parenthesized_expression" {
        around = parent.parent().map(|p| p.kind()).unwrap_or(around);
    }
    if around.contains("member") || around == "field_expression" || around == "attribute" {
        Shape::Member
    } else if around.contains("scoped") || around == "selector_expression" {
        Shape::Scoped
    } else if around == "object_creation_expression" || around == "new_expression" {
        Shape::Type
    } else {
        Shape::Plain
    }
}

/// One file parsed: its tree, its names, and its classes.
struct Parsed {
    lang: Lang,
    source: String,
    tree: Option<Tree>,
    symbols: Vec<Symbol>,
    classes: Vec<ClassInfo>,
}

/// Every definition and reference the tags query finds in one file, with
/// what the receiver walk adds to them.
fn symbols_in(path: &str, text: &str) -> Parsed {
    let (lang, source) = source_for(path, text);
    let Some(tree) = parse(lang, &source) else {
        return Parsed {
            lang,
            source,
            tree: None,
            symbols: Vec::new(),
            classes: Vec::new(),
        };
    };
    let Some(grammar) = grammar(lang) else {
        return Parsed {
            lang,
            source,
            tree: Some(tree),
            symbols: Vec::new(),
            classes: Vec::new(),
        };
    };
    let lines = Lines::new(text);
    let mut found: Vec<Symbol> = Vec::new();
    let mut seen: HashMap<(usize, usize, bool), usize> = HashMap::new();
    let mut starts: Vec<usize> = Vec::new();
    let skipped_file = skipped(path);
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&grammar.query, tree.root_node(), source.as_bytes());
    while let Some(found_match) = matches.next() {
        let mut name: Option<Node> = None;
        let mut role: Option<(bool, &'static str, Node)> = None;
        for capture in found_match.captures {
            if capture.index == grammar.name {
                name = Some(capture.node);
            } else if let Some(Some((definition, kind))) = grammar.roles.get(capture.index as usize)
            {
                role = Some((*definition, kind, capture.node));
            }
        }
        let (Some(name), Some((definition, mut kind, node))) = (name, role) else {
            continue;
        };
        let word = text_of(name, &source).trim();
        let word = word.trim_start_matches('$');
        if word.is_empty() || word.len() > 200 || word.contains(char::is_whitespace) {
            continue;
        }
        if definition {
            kind = refine_kind(lang, node, kind);
        }
        let key = (name.start_byte(), name.end_byte(), definition);
        if let Some(&earlier) = seen.get(&key) {
            // Two patterns for one name: Rust's method pattern and its
            // function pattern both match a method. The method one is right.
            if kind == "method" {
                found[earlier].kind = kind;
            }
            continue;
        }
        let (line, column, from) = lines.at(name.start_byte());
        let to = lines.at(name.end_byte()).2;
        let (public, entry) = if definition {
            let (public, entry) = judge(lang, path, node, word, &source);
            (public, entry || skipped_file)
        } else {
            (false, false)
        };
        let hook = if definition {
            hook_of(lang, path, node, word, &source)
        } else {
            Hook::None
        };
        seen.insert(key, found.len());
        starts.push(name.start_byte());
        found.push(Symbol {
            name: word.to_string(),
            kind,
            definition,
            shape: if definition {
                Shape::Plain
            } else {
                shape_of(kind, name)
            },
            line,
            column,
            from,
            to,
            public,
            entry,
            block: (lines.at(node.start_byte()).2, lines.at(node.end_byte()).2),
            via: None,
            hook,
        });
    }
    drop(matches);
    let mut walked = match lang {
        Lang::Php => receivers::php(&tree, &source),
        Lang::Js | Lang::Ts | Lang::Tsx => receivers::script(&tree, &source),
        _ => receivers::Found::default(),
    };
    for (symbol, start) in found.iter_mut().zip(&starts) {
        symbol.via = walked.via.remove(start);
    }
    for extra in walked.extra {
        if seen.contains_key(&(extra.start, extra.end, false)) {
            continue;
        }
        seen.insert((extra.start, extra.end, false), found.len());
        let (line, column, from) = lines.at(extra.start);
        let to = lines.at(extra.end).2;
        found.push(Symbol {
            name: extra.name,
            kind: extra.kind,
            definition: false,
            shape: extra.shape,
            line,
            column,
            from,
            to,
            public: false,
            entry: false,
            block: (from, to),
            via: walked.via.remove(&extra.start),
            hook: Hook::None,
        });
    }
    Parsed {
        lang,
        source,
        tree: Some(tree),
        symbols: found,
        classes: walked.classes,
    }
}

/// Vue's lifecycle: the methods it calls on a component.
const VUE_LIFECYCLE: &[&str] = &[
    "setup",
    "data",
    "render",
    "beforeCreate",
    "created",
    "beforeMount",
    "mounted",
    "beforeUpdate",
    "updated",
    "beforeUnmount",
    "unmounted",
    "beforeDestroy",
    "destroyed",
    "activated",
    "deactivated",
    "errorCaptured",
    "renderTracked",
    "renderTriggered",
    "serverPrefetch",
];

/// Folders whose classes Laravel calls `handle` on.
const HANDLER_FOLDERS: &[&str] = &[
    "/Console/",
    "/Commands/",
    "/Jobs/",
    "/Listeners/",
    "/Middleware/",
    "/Controllers/",
    "/Http/",
    "/Events/",
    "/Notifications/",
    "/Mail/",
];

/// Whether, and how, the framework calls a definition.
fn hook_of(lang: Lang, path: &str, node: Node, name: &str, source: &str) -> Hook {
    match (lang, node.kind()) {
        (Lang::Php, "method_declaration") => {
            let Some(class) = node
                .parent()
                .and_then(|list| list.parent())
                .filter(|class| class.kind() == "class_declaration")
            else {
                return Hook::None;
            };
            if name == "__construct" || name == "__invoke" {
                return Hook::Class;
            }
            let inherits = has_child(class, &["base_clause", "class_interface_clause"]);
            let slashed = format!("/{path}");
            if name == "handle"
                && (inherits || HANDLER_FOLDERS.iter().any(|dir| slashed.contains(dir)))
            {
                return Hook::Class;
            }
            let class_name = class
                .child_by_field_name("name")
                .map(|n| text_of(n, source))
                .unwrap_or("");
            let visibility = child_text(node, "visibility_modifier", source).unwrap_or("public");
            let controller =
                class_name.ends_with("Controller") || slashed.contains("/Controllers/");
            if controller
                && visibility == "public"
                && !name.starts_with("__")
                && !has_child(node, &["static_modifier"])
            {
                return Hook::Action;
            }
            Hook::None
        }
        (Lang::Js | Lang::Ts | Lang::Tsx, "method_definition") if VUE_LIFECYCLE.contains(&name) => {
            if node.parent().is_some_and(|p| p.kind() == "class_body") {
                Hook::Class
            } else {
                Hook::Framework
            }
        }
        (Lang::Rust, "function_item") => {
            let tauri = rust_attributes(node, source)
                .iter()
                .any(|attribute| attribute.replace(' ', "").contains("tauri::command"));
            if tauri {
                Hook::Tauri
            } else {
                Hook::None
            }
        }
        _ => Hook::None,
    }
}

/// The tags queries call PHP methods functions and Python methods functions.
fn refine_kind(lang: Lang, node: Node, kind: &'static str) -> &'static str {
    match (lang, node.kind()) {
        (Lang::Php, "method_declaration") => "method",
        (Lang::Python, "function_definition") => {
            let in_class = node
                .parent()
                .and_then(|block| block.parent())
                .is_some_and(|class| class.kind() == "class_definition");
            if in_class {
                "method"
            } else {
                kind
            }
        }
        _ => kind,
    }
}

/// Files that are entry points from top to bottom: tests, fixtures, examples,
/// build scripts.
fn skipped(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let parts: Vec<&str> = lower.split('/').collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    if dirs.iter().any(|dir| {
        matches!(
            *dir,
            "tests" | "test" | "__tests__" | "spec" | "benches" | "examples" | "fixtures" | "e2e"
        )
    }) {
        return true;
    }
    let name = parts.last().copied().unwrap_or("");
    name.ends_with("test.php")
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.ends_with("_test.go")
        || name.ends_with("_test.py")
        || name.starts_with("test_")
        || name == "conftest.py"
        || name == "build.rs"
}

fn has_child(node: Node, kinds: &[&str]) -> bool {
    (0..node.child_count()).any(|at| {
        node.child(at)
            .is_some_and(|child| kinds.contains(&child.kind()))
    })
}

fn child_text<'a>(node: Node, kind: &str, source: &'a str) -> Option<&'a str> {
    (0..node.child_count())
        .filter_map(|at| node.child(at))
        .find(|child| child.kind() == kind)
        .map(|child| text_of(child, source))
}

/// `(public, entry)` for one definition.
fn judge(lang: Lang, path: &str, node: Node, name: &str, source: &str) -> (bool, bool) {
    match lang {
        Lang::Php => judge_php(path, node, name, source),
        Lang::Js | Lang::Ts | Lang::Tsx => judge_script(node, name, source),
        Lang::Rust => judge_rust(node, name, source),
        Lang::Python => judge_python(node, name),
        Lang::Go => judge_go(node, name),
    }
}

const PHP_HOOKS: &[&str] = &[
    "handle",
    "boot",
    "register",
    "up",
    "down",
    "run",
    "definition",
    "rules",
    "authorize",
    "messages",
    "attributes",
    "render",
    "report",
    "toArray",
    "toMail",
    "toDatabase",
    "toBroadcast",
    "via",
    "broadcastOn",
    "casts",
    "schedule",
    "commands",
    "configure",
    "setUp",
    "tearDown",
    "invoke",
];

fn judge_php(path: &str, node: Node, name: &str, source: &str) -> (bool, bool) {
    match node.kind() {
        "method_declaration" => {
            let visibility = child_text(node, "visibility_modifier", source).unwrap_or("public");
            let public = visibility != "private";
            let class = node.parent().and_then(|list| list.parent());
            let class_kind = class.map(|c| c.kind()).unwrap_or("");
            let class_name = class
                .and_then(|c| c.child_by_field_name("name"))
                .map(|n| text_of(n, source))
                .unwrap_or("");
            let inherits =
                class.is_some_and(|c| has_child(c, &["base_clause", "class_interface_clause"]));
            let lower = name.to_ascii_lowercase();
            let entry = name.starts_with("__")
                || PHP_HOOKS.contains(&name)
                || lower.starts_with("test")
                || lower.starts_with("boot")
                || lower.starts_with("initialize")
                || (name.starts_with("scope") && name.len() > 5)
                || (name.starts_with("get") && name.ends_with("Attribute"))
                || (name.starts_with("set") && name.ends_with("Attribute"))
                || has_child(node, &["attribute_list"])
                || visibility == "protected"
                || class_kind == "interface_declaration"
                || class_kind == "trait_declaration"
                || has_child(node, &["abstract_modifier"])
                || (public
                    && (inherits
                        || class_name.ends_with("Controller")
                        || path.contains("/Controllers/")));
            (public, entry)
        }
        "function_definition" => (true, has_child(node, &["attribute_list"])),
        _ => (true, true),
    }
}

const VUE_HOOKS: &[&str] = &[
    "setup",
    "data",
    "render",
    "props",
    "emits",
    "computed",
    "watch",
    "methods",
    "components",
    "beforeCreate",
    "created",
    "beforeMount",
    "mounted",
    "beforeUpdate",
    "updated",
    "beforeUnmount",
    "unmounted",
    "beforeDestroy",
    "destroyed",
    "activated",
    "deactivated",
    "errorCaptured",
    "renderTracked",
    "renderTriggered",
    "serverPrefetch",
    "onMounted",
    "onUnmounted",
    "onBeforeMount",
    "onBeforeUnmount",
    "onUpdated",
    "onBeforeUpdate",
    "onActivated",
    "onDeactivated",
    "onErrorCaptured",
    "defineProps",
    "defineEmits",
    "defineExpose",
    "defineModel",
    "defineSlots",
    "defineOptions",
    "constructor",
    "default",
];

fn judge_script(node: Node, name: &str, source: &str) -> (bool, bool) {
    let hook = VUE_HOOKS.contains(&name);
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            let parent = node.parent().map(|p| p.kind()).unwrap_or("");
            (parent == "export_statement", hook)
        }
        "variable_declarator" => {
            let declaration = node.parent();
            let outer = declaration
                .and_then(|d| d.parent())
                .map(|p| p.kind())
                .unwrap_or("");
            (outer == "export_statement", hook)
        }
        "method_definition" => {
            let body = node.parent();
            if body.map(|b| b.kind()) != Some("class_body") {
                // A method in an object literal: whoever receives the object
                // calls it.
                return (true, true);
            }
            let class = body.and_then(|b| b.parent());
            let private = child_text(node, "accessibility_modifier", source)
                .is_some_and(|access| access == "private")
                || node
                    .child_by_field_name("name")
                    .is_some_and(|n| n.kind() == "private_property_identifier");
            let protected = child_text(node, "accessibility_modifier", source)
                .is_some_and(|access| access == "protected");
            let inherits = class.is_some_and(|c| has_child(c, &["class_heritage"]));
            let entry = hook
                || protected
                || has_child(node, &["override_modifier", "decorator", "get", "set"])
                || (!private && inherits);
            (!private, entry)
        }
        // Named function expressions, assignments, object pairs, signatures:
        // nothing a reader would call dead.
        _ => (true, true),
    }
}

/// Attributes that mean nothing about who calls a function.
const QUIET_ATTRIBUTES: &[&str] = &[
    "#[doc",
    "#[inline",
    "#[must_use",
    "#[allow(clippy",
    "#[deprecated",
    "#[track_caller",
    "#[cold",
    "#[cfg(target",
    "#[cfg(windows",
    "#[cfg(unix",
    "#[cfg(not(target",
    "#[cfg(feature",
    "#[cfg(debug_assertions",
    "#[cfg(not(debug_assertions",
];

fn rust_attributes<'a>(node: Node, source: &'a str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut at = node.prev_sibling();
    while let Some(sibling) = at {
        match sibling.kind() {
            "attribute_item" => out.push(text_of(sibling, source)),
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        at = sibling.prev_sibling();
    }
    out
}

fn loud(attributes: &[&str]) -> bool {
    attributes.iter().any(|attribute| {
        let squeezed: String = attribute.chars().filter(|c| !c.is_whitespace()).collect();
        !QUIET_ATTRIBUTES
            .iter()
            .any(|quiet| squeezed.starts_with(quiet))
    })
}

fn judge_rust(node: Node, name: &str, source: &str) -> (bool, bool) {
    if node.kind() != "function_item" {
        return (true, true);
    }
    let public = has_child(node, &["visibility_modifier"]);
    let mut entry = name == "main" || loud(&rust_attributes(node, source));
    // Inside a trait, a trait impl, an extern block, or a module only built
    // for tests.
    let mut up = node.parent();
    while let Some(parent) = up {
        match parent.kind() {
            "impl_item" if parent.child_by_field_name("trait").is_some() => entry = true,
            "trait_item" | "foreign_mod_item" => entry = true,
            "mod_item"
                if rust_attributes(parent, source)
                    .iter()
                    .any(|attribute| attribute.contains("test")) =>
            {
                entry = true
            }
            _ => {}
        }
        up = parent.parent();
    }
    (public, entry)
}

fn judge_python(node: Node, name: &str) -> (bool, bool) {
    if node.kind() != "function_definition" {
        return (true, true);
    }
    let dunder = name.starts_with("__") && name.ends_with("__");
    let private = name.starts_with('_') && !dunder;
    let decorated = node
        .parent()
        .is_some_and(|p| p.kind() == "decorated_definition");
    let class = node
        .parent()
        .and_then(|block| block.parent())
        .filter(|class| class.kind() == "class_definition");
    let inherits = class.is_some_and(|c| c.child_by_field_name("superclasses").is_some());
    let entry =
        dunder || decorated || name.starts_with("test") || name == "main" || (!private && inherits);
    (!private, entry)
}

fn judge_go(node: Node, name: &str) -> (bool, bool) {
    let public = name.chars().next().is_some_and(char::is_uppercase);
    let entry = match node.kind() {
        "function_declaration" => {
            name == "main"
                || name == "init"
                || ["Test", "Benchmark", "Example", "Fuzz"]
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
        }
        // A method can satisfy an interface nobody names.
        "method_declaration" => public,
        _ => true,
    };
    (public, entry)
}

// --- the index ---------------------------------------------------------------

/// What one file holds: its family, every name in it, and its classes.
struct FileSymbols {
    family: Family,
    symbols: Vec<Symbol>,
    classes: Vec<ClassInfo>,
}

/// A file's size and time when it was read, to tell whether it changed while
/// nobody was watching.
type Stamp = (u64, Option<std::time::SystemTime>);

#[derive(Default)]
struct Index {
    files: HashMap<String, FileSymbols>,
    stamps: HashMap<String, Stamp>,
}

#[derive(Default)]
struct Shared {
    index: RwLock<Index>,
    ready: Mutex<bool>,
    done: Condvar,
    /// A catch-up pass is running; another start leaves it to finish.
    refreshing: std::sync::atomic::AtomicBool,
    /// The newest `unused_in` request; older ones stop between files.
    newest: std::sync::atomic::AtomicU64,
    /// Files changed while the first build was still reading: it may have
    /// read them before the change, so it looks again when it is done.
    missed: std::sync::atomic::AtomicBool,
}

fn indexes() -> &'static Mutex<HashMap<PathBuf, Arc<Shared>>> {
    static ALL: OnceLock<Mutex<HashMap<PathBuf, Arc<Shared>>>> = OnceLock::new();
    ALL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key(root: &Path) -> PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
}

fn stamp_of(root: &Path, path: &str) -> Option<Stamp> {
    let meta = std::fs::metadata(root.join(path)).ok()?;
    Some((meta.len(), meta.modified().ok()))
}

/// One file as the index has it: its names, or none when it is left out
/// (over 1 MB, not UTF-8), and its stamp whenever it is on disk at all, so a
/// file left out is not read again until it changes.
type Indexed = (Option<FileSymbols>, Option<Stamp>);

/// One file read and parsed for the index.
fn index_file(root: &Path, path: &str) -> Indexed {
    let Some(lang) = lang_of(path) else {
        return (None, None);
    };
    let full = root.join(path);
    let Ok(meta) = std::fs::metadata(&full) else {
        return (None, None);
    };
    let stamp = Some((meta.len(), meta.modified().ok()));
    if !meta.is_file() || meta.len() > MAX_FILE {
        return (None, stamp);
    }
    let Some(text) = std::fs::read(&full)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
    else {
        return (None, stamp);
    };
    let parsed = symbols_in(path, &text);
    (
        Some(FileSymbols {
            family: lang.family(),
            symbols: parsed.symbols,
            classes: parsed.classes,
        }),
        stamp,
    )
}

/// Parses files on as many threads as there are cores, up to eight. Each
/// thread takes the next file when it is done with one, so a slow core or a
/// big file holds up only itself.
fn index_many(root: &Path, paths: &[String]) -> Vec<(String, Indexed)> {
    use std::sync::atomic::AtomicUsize;
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8)
        .min(paths.len().max(1));
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                let next = &next;
                scope.spawn(move || {
                    let mut done = Vec::new();
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = paths.get(at) else { break };
                        done.push((path.clone(), index_file(root, path)));
                    }
                    done
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    })
}

/// Code the project did not write: dependencies, build output, compiled
/// templates. Left out of the index even when git does not ignore it, so a
/// vendored `handle()` is never counted as a usage of the project's own.
pub fn generated(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    dirs.iter().enumerate().any(|(at, dir)| {
        let next = dirs.get(at + 1).copied();
        match *dir {
            "vendor" | "node_modules" | ".output" | ".nuxt" | "dist" => true,
            // Laravel's: at the top, or wherever its compiled views are.
            "storage" => at == 0 || next == Some("framework"),
            "bootstrap" => next == Some("cache"),
            "public" => next == Some("build"),
            _ => false,
        }
    })
}

fn named_paths(root: &Path) -> Vec<String> {
    files::all_files(root)
        .map(|all| {
            all.iter()
                .filter(|path| lang_of(path).is_some() && !generated(path))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn build_into(root: &Path, shared: &Shared) {
    let root = key(root);
    let paths = named_paths(&root);
    let built = index_many(&root, &paths);
    if let Ok(mut index) = shared.index.write() {
        index.files.clear();
        index.stamps.clear();
        apply(&mut index, built, None);
    }
    if let Ok(mut ready) = shared.ready.lock() {
        *ready = true;
    }
    shared.done.notify_all();
    // A save that landed while this was reading was not heard by anyone.
    while shared.missed.swap(false, Ordering::SeqCst) {
        catch_up(&root, shared);
    }
}

/// Brings an index up to date with the disk: files added, gone, or changed
/// since it was last looked at. For a project coming back to the front, whose
/// files nobody watched while it was behind.
fn catch_up(root: &Path, shared: &Shared) {
    files::forget(root);
    let paths = named_paths(root);
    let known: HashMap<String, Stamp> = match shared.index.read() {
        Ok(index) => index.stamps.clone(),
        Err(_) => return,
    };
    let here: HashSet<&String> = paths.iter().collect();
    let gone: Vec<String> = known
        .keys()
        .filter(|path| !here.contains(path))
        .cloned()
        .collect();
    let changed: Vec<String> = paths
        .iter()
        .filter(|path| known.get(*path) != stamp_of(root, path).as_ref())
        .cloned()
        .collect();
    if gone.is_empty() && changed.is_empty() {
        return;
    }
    let mut updates = index_many(root, &changed);
    updates.extend(gone.into_iter().map(|path| (path, (None, None))));
    if let Ok(mut index) = shared.index.write() {
        apply(&mut index, updates, Some(&known));
    }
}

/// Writes reads into the index. With `since`, the stamps a slow pass started
/// from: a file whose stamp moved meanwhile was read again by the watcher,
/// and that newer read stays.
fn apply(
    index: &mut Index,
    updates: Vec<(String, Indexed)>,
    since: Option<&HashMap<String, Stamp>>,
) {
    for (path, (symbols, stamp)) in updates {
        if let Some(since) = since {
            if index.stamps.get(&path) != since.get(&path) {
                continue;
            }
        } else if let (Some((_, Some(read))), Some((_, Some(held)))) =
            (&stamp, index.stamps.get(&path))
        {
            // Two bursts read the same file on two threads: the older read
            // must not be the one that stays.
            if read < held {
                continue;
            }
        }
        match symbols {
            Some(symbols) => {
                index.files.insert(path.clone(), symbols);
            }
            None => {
                index.files.remove(&path);
            }
        }
        match stamp {
            Some(stamp) => {
                index.stamps.insert(path, stamp);
            }
            None => {
                index.stamps.remove(&path);
            }
        }
    }
}

/// Starts building the index for a work tree on a thread of its own. One
/// that exists already is brought up to date instead, since nothing watched
/// it while its project was not the one open.
pub fn start(root: PathBuf) {
    let root = key(&root);
    let (shared, fresh) = {
        let Ok(mut all) = indexes().lock() else {
            return;
        };
        match all.get(&root) {
            Some(shared) => (shared.clone(), false),
            None => {
                let shared = Arc::new(Shared::default());
                all.insert(root.clone(), shared.clone());
                (shared, true)
            }
        }
    };
    if fresh {
        std::thread::spawn(move || build_into(&root, &shared));
        return;
    }
    if !shared.ready.lock().map(|ready| *ready).unwrap_or(false)
        || shared.refreshing.swap(true, Ordering::SeqCst)
    {
        return;
    }
    std::thread::spawn(move || {
        catch_up(&root, &shared);
        shared.refreshing.store(false, Ordering::SeqCst);
    });
}

/// The index for a work tree, built here if nobody started it, and waited for
/// if it is still being built.
fn ready(root: &Path) -> Result<Arc<Shared>, String> {
    let root = key(root);
    let (shared, mine) = {
        let mut all = indexes().lock().map_err(|e| e.to_string())?;
        match all.get(&root) {
            Some(shared) => (shared.clone(), false),
            None => {
                let shared = Arc::new(Shared::default());
                all.insert(root.clone(), shared.clone());
                (shared, true)
            }
        }
    };
    if mine {
        build_into(&root, &shared);
    }
    let mut done = shared.ready.lock().map_err(|e| e.to_string())?;
    while !*done {
        done = shared.done.wait(done).map_err(|e| e.to_string())?;
    }
    drop(done);
    Ok(shared)
}

/// Reads the files the watcher saw change into the index again. Paths are
/// absolute, as the watcher has them. Files git ignores, or that are gone,
/// leave the index.
pub fn touched(root: &Path, paths: &HashSet<PathBuf>) {
    let given = root.to_path_buf();
    let root = key(root);
    let shared = match indexes().lock() {
        Ok(all) => all.get(&root).cloned(),
        Err(_) => None,
    };
    let Some(shared) = shared else { return };
    if !shared.ready.lock().map(|ready| *ready).unwrap_or(false) {
        // Still being built: said first and checked again after, so the
        // build either sees the note or has already finished.
        shared.missed.store(true, Ordering::SeqCst);
        if !shared.ready.lock().map(|ready| *ready).unwrap_or(false) {
            return;
        }
    }
    let mut relatives: Vec<String> = Vec::new();
    for path in paths {
        // A deleted file cannot be resolved, but its folder can: events come
        // with `/var` where the root resolved to `/private/var`.
        let real = path.canonicalize().unwrap_or_else(|_| {
            match (
                path.parent().and_then(|dir| dir.canonicalize().ok()),
                path.file_name(),
            ) {
                (Some(dir), Some(name)) => dir.join(name),
                _ => path.clone(),
            }
        });
        let Ok(inner) = real
            .strip_prefix(&root)
            .or_else(|_| path.strip_prefix(&root))
            .or_else(|_| path.strip_prefix(&given))
        else {
            continue;
        };
        let relative = inner
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if lang_of(&relative).is_some() && !generated(&relative) {
            relatives.push(relative);
        }
    }
    // Most bursts are the index, a lock file, a log: nothing to read.
    if relatives.is_empty() {
        return;
    }
    let listed = match files::all_files(&root) {
        Ok(all) => all,
        Err(_) => return,
    };
    let updates: Vec<(String, Indexed)> = relatives
        .into_iter()
        .map(|relative| {
            let found = if listed.binary_search(&relative).is_ok() {
                index_file(&root, &relative)
            } else {
                (None, None)
            };
            (relative, found)
        })
        .collect();
    let written = shared.index.write();
    if let Ok(mut index) = written {
        apply(&mut index, updates, None);
    }
}

/// Drops the index for a work tree: its project was closed.
pub fn forget(root: &Path) {
    if let Ok(mut all) = indexes().lock() {
        all.remove(&key(root));
    }
}

// --- autocomplete --------------------------------------------------------------

/// The index for a work tree if it is built, without waiting for it. What
/// completion asks between two keystrokes cannot wait for a whole build.
fn ready_now(root: &Path) -> Option<Arc<Shared>> {
    let shared = indexes().lock().ok()?.get(&key(root)).cloned()?;
    let built = shared.ready.lock().map(|ready| *ready).unwrap_or(false);
    built.then_some(shared)
}

/// The files that declare a class, interface, trait or enum called `name`
/// (case aside), in PHP files or in scripts. Empty while the index is being
/// built.
pub fn class_paths(root: &Path, name: &str, php: bool) -> Vec<String> {
    let family = if php { Family::Php } else { Family::Script };
    let Some(shared) = ready_now(root) else {
        return Vec::new();
    };
    let Ok(index) = shared.index.read() else {
        return Vec::new();
    };
    let mut found: Vec<String> = index
        .files
        .iter()
        .filter(|(_, held)| held.family == family)
        .filter(|(_, held)| {
            held.classes
                .iter()
                .any(|class| class.name.eq_ignore_ascii_case(name))
        })
        .map(|(path, _)| path.clone())
        .collect();
    found.sort();
    found
}

/// A project name the completion list offers.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Completion {
    pub name: String,
    /// `class`, `interface`, `trait`, `enum`, `function` or `constant`.
    pub kind: &'static str,
    /// The file it is defined in.
    pub path: String,
}

/// The kinds of definition a name typed in code can mean.
const COMPLETABLE: &[&str] = &["class", "interface", "function", "constant"];

/// Class, interface, trait, enum, function and constant names from the
/// index that start with `prefix` (case aside), then those whose capitals
/// it is, then fuzzy matches. Within each: the file being edited first, then
/// its folder, then the files in `changed`, then the rest; shorter names
/// first after that. Only names in files of `path`'s language family.
/// Nothing while the index is still being built.
pub fn complete_symbols(
    root: &Path,
    prefix: &str,
    path: &str,
    limit: usize,
    changed: &[String],
) -> Vec<Completion> {
    use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
    use nucleo_matcher::{Config, Matcher, Utf32Str};
    let prefix = prefix.trim();
    let Some(family) = family_of(path) else {
        return Vec::new();
    };
    if prefix.is_empty() {
        return Vec::new();
    }
    let Some(shared) = ready_now(root) else {
        return Vec::new();
    };
    let Ok(index) = shared.index.read() else {
        return Vec::new();
    };
    let lower = prefix.to_lowercase();
    let folder = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let changed: HashSet<&str> = changed.iter().map(String::as_str).collect();
    let capitals = files::is_initials(prefix);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(prefix, CaseMatching::Smart, Normalization::Smart);
    let mut buffer = Vec::new();
    let mut scored: Vec<((u8, u8, usize, u32), Completion)> = Vec::new();
    for (file, held) in index.files.iter() {
        if held.family != family {
            continue;
        }
        let place = if file == path {
            0
        } else if file.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("") == folder {
            1
        } else if changed.contains(file.as_str()) {
            2
        } else {
            3
        };
        let mut names: Vec<(&str, &'static str)> = held
            .classes
            .iter()
            .filter(|class| !class.name.is_empty())
            .map(|class| {
                (
                    &*class.name,
                    if class.kind.is_empty() {
                        "class"
                    } else {
                        class.kind
                    },
                )
            })
            .collect();
        for symbol in &held.symbols {
            if symbol.definition
                && COMPLETABLE.contains(&symbol.kind)
                && !names.iter().any(|(name, _)| *name == symbol.name)
            {
                names.push((&symbol.name, symbol.kind));
            }
        }
        for (name, kind) in names {
            let tier = if name.to_lowercase().starts_with(&lower) {
                0
            } else if capitals && files::initials(name, prefix).is_some() {
                1
            } else {
                match pattern.score(Utf32Str::new(name, &mut buffer), &mut matcher) {
                    Some(score) => {
                        scored.push((
                            (2, place, name.len(), u32::MAX - score),
                            Completion {
                                name: name.to_string(),
                                kind,
                                path: file.clone(),
                            },
                        ));
                        continue;
                    }
                    None => continue,
                }
            };
            scored.push((
                (tier, place, name.len(), 0),
                Completion {
                    name: name.to_string(),
                    kind,
                    path: file.clone(),
                },
            ));
        }
    }
    scored.sort_by(|(a, one), (b, other)| {
        a.cmp(b)
            .then_with(|| one.name.cmp(&other.name))
            .then_with(|| one.path.cmp(&other.path))
    });
    let mut seen: HashSet<(String, String)> = HashSet::new();
    scored
        .into_iter()
        .map(|(_, found)| found)
        .filter(|found| seen.insert((found.name.clone(), found.path.clone())))
        .take(limit.max(1))
        .collect()
}

// --- commands ----------------------------------------------------------------

/// What is under the caret or the mouse.
#[derive(Serialize, Clone, Debug)]
pub struct SymbolAt {
    pub name: String,
    pub kind: &'static str,
    pub definition: bool,
    pub shape: Shape,
    pub line: u32,
    pub from: u32,
    pub to: u32,
}

/// One place in the repository a popup can list and open.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Location {
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub kind: &'static str,
    /// The line, trimmed, for the list.
    pub text: String,
}

fn current_text(root: &Path, path: &str, text: Option<&str>) -> Result<String, String> {
    if let Some(text) = text {
        return Ok(text.to_string());
    }
    let full = files::resolve(root, path, true)?;
    let bytes = std::fs::read(&full).map_err(|e| format!("Could not read {path}: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The name at `offset` (UTF-16, in `text` or in the file on disk). A name the
/// tags query does not know, such as a function passed as a value, still
/// reads as a reference to whatever it names.
pub fn symbol_at(
    state: &AppState,
    path: &str,
    offset: u32,
    text: Option<&str>,
) -> Result<Option<SymbolAt>, String> {
    let root = state.path()?;
    if lang_of(path).is_none() {
        return Ok(None);
    }
    let text = current_text(&root, path, text)?;
    let Parsed {
        source,
        tree,
        symbols,
        ..
    } = symbols_in(path, &text);
    if let Some(symbol) = symbols
        .iter()
        .filter(|s| s.from <= offset && offset <= s.to && s.kind != "command")
        .min_by_key(|s| (s.to - s.from, !s.definition))
    {
        return Ok(Some(SymbolAt {
            name: symbol.name.clone(),
            kind: symbol.kind,
            definition: symbol.definition,
            shape: symbol.shape,
            line: symbol.line,
            from: symbol.from,
            to: symbol.to,
        }));
    }
    let Some(tree) = tree else { return Ok(None) };
    let byte = byte_at_utf16(&text, offset);
    let Some(node) = tree.root_node().named_descendant_for_byte_range(byte, byte) else {
        return Ok(None);
    };
    let kind = node.kind();
    let named = kind == "name"
        || kind.ends_with("identifier") && kind != "type_identifier"
        || kind == "type_identifier";
    if !named {
        return Ok(None);
    }
    let name = text_of(node, &source).trim_start_matches('$').to_string();
    if name.is_empty() {
        return Ok(None);
    }
    let lines = Lines::new(&text);
    let (line, _, from) = lines.at(node.start_byte());
    let to = lines.at(node.end_byte()).2;
    let shape = if kind == "type_identifier" {
        Shape::Type
    } else {
        shape_of("call", node)
    };
    Ok(Some(SymbolAt {
        name,
        kind: "call",
        definition: false,
        shape,
        line,
        from,
        to,
    }))
}

fn line_text(
    root: &Path,
    path: &str,
    line: u32,
    cache: &mut HashMap<String, Vec<String>>,
) -> String {
    let lines = cache.entry(path.to_string()).or_insert_with(|| {
        std::fs::read(root.join(path))
            .map(|bytes| {
                String::from_utf8_lossy(&bytes)
                    .lines()
                    .map(|line| line.trim_end_matches('\r').to_string())
                    .collect()
            })
            .unwrap_or_default()
    });
    let text = lines
        .get(line.saturating_sub(1) as usize)
        .map(|line| line.trim())
        .unwrap_or("");
    text.chars().take(200).collect()
}

fn fits(shape: Option<Shape>, kind: &str) -> bool {
    match shape {
        None => true,
        Some(Shape::Type) => !matches!(kind, "function" | "method"),
        Some(Shape::Member) => kind == "method",
        Some(Shape::Plain) => matches!(kind, "function" | "macro" | "class"),
        Some(Shape::Scoped) => matches!(kind, "function" | "method" | "class"),
    }
}

/// Where a name is defined, in files of the same family as `from`. `shape`
/// narrows to the definitions a reference of that shape can mean, unless that
/// leaves none.
pub fn definitions(
    state: &AppState,
    name: &str,
    from: Option<&str>,
    shape: Option<Shape>,
) -> Result<Vec<Location>, String> {
    let root = key(&state.path()?);
    let shared = ready(&root)?;
    let family = from.and_then(family_of);
    let index = shared.index.read().map_err(|e| e.to_string())?;
    let mut all: Vec<(String, &Symbol)> = Vec::new();
    for (path, held) in index.files.iter() {
        if family.is_some_and(|family| family != held.family) {
            continue;
        }
        for symbol in &held.symbols {
            if symbol.definition && symbol.name == name {
                all.push((path.clone(), symbol));
            }
        }
    }
    let narrowed: Vec<_> = all
        .iter()
        .filter(|(_, s)| fits(shape, s.kind))
        .cloned()
        .collect();
    let chosen = if narrowed.is_empty() { all } else { narrowed };
    let mut cache = HashMap::new();
    let mut out: Vec<Location> = chosen
        .into_iter()
        .take(MAX_LOCATIONS)
        .map(|(path, symbol)| Location {
            line: symbol.line,
            column: symbol.column,
            kind: symbol.kind,
            text: line_text(&root, &path, symbol.line, &mut cache),
            path,
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    Ok(out)
}

/// A definition the search window's Symbols mode found.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FoundSymbol {
    pub name: String,
    pub kind: &'static str,
    pub path: String,
    pub line: u32,
    pub column: u32,
    /// Which characters of the name matched, for marking them.
    pub indices: Vec<u32>,
}

/// The kinds a person looks a definition up by: the things with a name of
/// their own, not fields, constants or every `impl` block.
const FINDABLE: &[&str] = &[
    "class",
    "interface",
    "function",
    "method",
    "module",
    "type",
    "macro",
];

/// Class, function and method names that fuzzily match `query`, best first.
/// Capitals (`MECC`) match the capitals of a name and rank above the rest.
pub fn find_symbols(
    state: &AppState,
    query: &str,
    limit: usize,
) -> Result<Vec<FoundSymbol>, String> {
    find_symbols_at(&state.path()?, query, limit)
}

pub fn find_symbols_at(root: &Path, query: &str, limit: usize) -> Result<Vec<FoundSymbol>, String> {
    use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
    use nucleo_matcher::{Config, Matcher, Utf32Str};
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let root = key(root);
    let shared = ready(&root)?;
    let index = shared.index.read().map_err(|e| e.to_string())?;
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let capitals = files::is_initials(query);
    let mut buffer = Vec::new();
    let mut scored: Vec<(u32, FoundSymbol)> = Vec::new();
    for (path, held) in index.files.iter() {
        for symbol in &held.symbols {
            if !symbol.definition || !FINDABLE.contains(&symbol.kind) {
                continue;
            }
            let shaped = if capitals {
                files::initials(&symbol.name, query)
            } else {
                None
            };
            let fuzzy = pattern.score(Utf32Str::new(&symbol.name, &mut buffer), &mut matcher);
            let score = match (&shaped, fuzzy) {
                (Some((tier, _)), fuzzy) => tier * 100_000 + fuzzy.unwrap_or(0).min(99_999),
                (None, Some(score)) => score.min(99_999),
                (None, None) => continue,
            };
            let indices = match shaped {
                Some((_, indices)) => indices,
                None => {
                    let mut indices = Vec::new();
                    pattern.indices(
                        Utf32Str::new(&symbol.name, &mut buffer),
                        &mut matcher,
                        &mut indices,
                    );
                    indices.sort_unstable();
                    indices.dedup();
                    indices
                }
            };
            scored.push((
                score,
                FoundSymbol {
                    name: symbol.name.clone(),
                    kind: symbol.kind,
                    path: path.clone(),
                    line: symbol.line,
                    column: symbol.column,
                    indices,
                },
            ));
        }
    }
    // Best first; between equals the shorter name, then a type before what is
    // in it, then by place so the order never jumps about.
    let rank = |kind: &str| match kind {
        "class" | "interface" | "type" | "module" => 0,
        "function" | "macro" => 1,
        _ => 2,
    };
    scored.sort_by(|(a_score, a), (b_score, b)| {
        b_score
            .cmp(a_score)
            .then_with(|| a.name.len().cmp(&b.name.len()))
            .then_with(|| rank(a.kind).cmp(&rank(b.kind)))
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
    });
    scored.truncate(limit.max(1));
    Ok(scored.into_iter().map(|(_, found)| found).collect())
}

/// How sure a usage is: the receiver is the class or one in its hierarchy,
/// or the name matches and the receiver could not be worked out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verdict {
    Definite,
    Possible,
}

/// What extends, implements or uses what, across one family.
struct Graph<'a> {
    up: HashMap<&'a str, Vec<&'a str>>,
    down: HashMap<&'a str, Vec<&'a str>>,
}

impl<'a> Graph<'a> {
    fn new(lists: impl Iterator<Item = &'a [ClassInfo]>) -> Self {
        let mut graph = Graph {
            up: HashMap::new(),
            down: HashMap::new(),
        };
        for class in lists.flatten() {
            for parent in &class.supers {
                graph.up.entry(&class.name).or_default().push(parent);
                graph.down.entry(parent).or_default().push(&class.name);
            }
        }
        graph
    }

    /// A class with everything above and below it: the classes a call can
    /// be made on and still reach a method defined in it.
    fn related(&self, class: &'a str) -> HashSet<&'a str> {
        let mut out = HashSet::from([class]);
        for map in [&self.up, &self.down] {
            let mut stack = vec![class];
            let mut seen = HashSet::from([class]);
            while let Some(at) = stack.pop() {
                for next in map.get(at).into_iter().flatten() {
                    if seen.insert(*next) {
                        out.insert(*next);
                        stack.push(*next);
                    }
                }
            }
        }
        out
    }
}

/// The name a reference is counted under: a class name under the class it
/// means once aliases are undone, anything else under its own.
fn counted_as(symbol: &Symbol) -> &str {
    if symbol.shape == Shape::Type && !symbol.definition {
        symbol.via.as_deref().unwrap_or(&symbol.name)
    } else {
        &symbol.name
    }
}

/// The references one request looks at, by the name they count under.
#[derive(Default)]
struct Refs<'a> {
    by_name: HashMap<&'a str, Vec<(&'a str, &'a Symbol)>>,
    /// Files that define a function of a wanted name themselves.
    defines: HashSet<(&'a str, &'a str)>,
}

impl<'a> Refs<'a> {
    fn gather(files: &[(&'a str, &'a [Symbol])], wanted: &HashSet<&str>) -> Self {
        let mut refs = Refs::default();
        for (path, symbols) in files {
            for symbol in symbols.iter() {
                let name = counted_as(symbol);
                if !wanted.contains(name) {
                    continue;
                }
                if symbol.definition {
                    if symbol.kind == "function" {
                        refs.defines.insert((path, &symbol.name));
                    }
                    continue;
                }
                refs.by_name.entry(name).or_default().push((path, symbol));
            }
        }
        refs
    }

    fn named(&self, name: &str) -> impl Iterator<Item = &(&'a str, &'a Symbol)> {
        self.by_name.get(name).into_iter().flatten()
    }
}

/// A path without its extension, when the extension is a language's.
fn without_ext(path: &str) -> &str {
    if lang_of(path).is_some() {
        path.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(path)
    } else {
        path
    }
}

/// Whether an import names the file a function is defined in: `./money`
/// from next to it, or anything ending in its name. Only a plain mismatch
/// says no.
fn module_matches(module: &str, from: &str, def_path: &str) -> bool {
    let def = without_ext(def_path);
    let def_dir = def.strip_suffix("/index");
    if module.starts_with("./") || module.starts_with("../") {
        let mut parts: Vec<&str> = from.split('/').collect();
        parts.pop();
        for segment in module.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        let joined = parts.join("/");
        let target = without_ext(&joined);
        target == def || def_dir == Some(target)
    } else {
        let last = without_ext(module.rsplit('/').next().unwrap_or(module));
        let stem = match def_dir {
            Some(dir) => dir.rsplit('/').next().unwrap_or(dir),
            None => def.rsplit('/').next().unwrap_or(def),
        };
        last == stem
    }
}

/// What a definition's usages are, judged.
struct Judged<'a> {
    /// The framework calls it: the usages are the uses of what it belongs to.
    framework: bool,
    /// The class whose uses are counted, for "2 uses of Foo".
    owner: Option<&'a str>,
    found: Vec<(&'a str, &'a Symbol, Verdict)>,
}

/// Everything a usage question about definitions in one file needs from the
/// index, gathered in one pass.
struct Request<'a> {
    path: &'a str,
    family: Family,
    fresh: &'a Parsed,
    graph: Graph<'a>,
    refs: Refs<'a>,
    /// TypeScript's `invoke('name')` calls, for Tauri commands.
    invokes: Refs<'a>,
}

impl<'a> Request<'a> {
    fn new(
        index: &'a Index,
        path: &'a str,
        family: Family,
        fresh: &'a Parsed,
        targets: &[&'a Symbol],
    ) -> Self {
        let mut files: Vec<(&str, &[Symbol])> = index
            .files
            .iter()
            .filter(|(other, held)| held.family == family && other.as_str() != path)
            .map(|(other, held)| (other.as_str(), held.symbols.as_slice()))
            .collect();
        files.push((path, fresh.symbols.as_slice()));
        let graph = Graph::new(
            index
                .files
                .iter()
                .filter(|(other, held)| held.family == family && other.as_str() != path)
                .map(|(_, held)| held.classes.as_slice())
                .chain(std::iter::once(fresh.classes.as_slice())),
        );
        let mut wanted: HashSet<&str> = HashSet::new();
        let mut tauri: HashSet<&str> = HashSet::new();
        for target in targets {
            wanted.insert(&target.name);
            if let Some(owner) = owner_of(target) {
                wanted.insert(owner);
                if let Some(signature) = signature_of(fresh, owner) {
                    wanted.insert(signature);
                }
            }
            if target.hook == Hook::Tauri {
                tauri.insert(&target.name);
            }
        }
        let refs = Refs::gather(&files, &wanted);
        let invokes = if tauri.is_empty() {
            Refs::default()
        } else {
            let scripts: Vec<(&str, &[Symbol])> = index
                .files
                .iter()
                .filter(|(_, held)| held.family == Family::Script)
                .map(|(other, held)| (other.as_str(), held.symbols.as_slice()))
                .collect();
            Refs::gather(&scripts, &tauri)
        };
        Request {
            path,
            family,
            fresh,
            graph,
            refs,
            invokes,
        }
    }

    fn judge(&self, def: &'a Symbol) -> Judged<'a> {
        let owner = owner_of(def);
        let found = match (def.hook, owner) {
            (Hook::Framework, _) => Vec::new(),
            (Hook::Class | Hook::Action, Some(owner)) => {
                return Judged {
                    framework: true,
                    owner: (def.hook == Hook::Class).then_some(owner),
                    found: self.class_uses(def, owner),
                };
            }
            (Hook::Tauri, _) => {
                let mut found = self.plain(def);
                found.extend(
                    self.invokes
                        .named(&def.name)
                        .filter(|(_, s)| s.kind == "command")
                        .map(|(path, s)| (*path, *s, Verdict::Definite)),
                );
                found
            }
            _ => match def.kind {
                "method" => self.method(def, owner),
                "function" => self.plain(def),
                kind => self
                    .refs
                    .named(&def.name)
                    .filter(|(_, s)| {
                        !matches!(s.kind, "command" | "route")
                            && used_as(Some(kind), s.shape, self.family)
                    })
                    .map(|(path, s)| (*path, *s, Verdict::Definite))
                    .collect(),
            },
        };
        Judged {
            framework: def.hook != Hook::None,
            owner: None,
            found,
        }
    }

    /// A method's calls: on its class or one in its hierarchy, definite; on
    /// another class, not counted; on something unknown, possible.
    fn method(&self, def: &Symbol, owner: Option<&'a str>) -> Vec<(&'a str, &'a Symbol, Verdict)> {
        let precise = matches!(self.family, Family::Php | Family::Script);
        let related = owner
            .map(|owner| self.graph.related(owner))
            .unwrap_or_default();
        self.refs
            .named(&def.name)
            .filter(|(_, s)| s.kind != "command" && used_as(Some("method"), s.shape, self.family))
            .filter_map(|(path, s)| {
                let verdict = if !precise {
                    Verdict::Definite
                } else {
                    match s.via.as_deref() {
                        Some(class) if related.contains(class) => Verdict::Definite,
                        Some(_) => return None,
                        None => Verdict::Possible,
                    }
                };
                Some((*path, *s, verdict))
            })
            .collect()
    }

    /// A plain function's calls, by name, less the files that define a
    /// function of that name themselves and the calls imported from
    /// somewhere else.
    fn plain(&self, def: &Symbol) -> Vec<(&'a str, &'a Symbol, Verdict)> {
        self.refs
            .named(&def.name)
            .filter(|(path, s)| {
                if matches!(s.kind, "command" | "route")
                    || !used_as(Some("function"), s.shape, self.family)
                {
                    return false;
                }
                if *path != self.path && self.refs.defines.contains(&(*path, def.name.as_str())) {
                    return false;
                }
                match (self.family, s.shape, s.via.as_deref()) {
                    (Family::Script, Shape::Plain, Some(module)) => {
                        module_matches(module, path, self.path)
                    }
                    _ => true,
                }
            })
            .map(|(path, s)| (*path, *s, Verdict::Definite))
            .collect()
    }

    /// A method the framework calls: the uses of its class, the routes that
    /// name it, the calls on its class from elsewhere, and an artisan
    /// command's name in strings. One per line.
    fn class_uses(&self, def: &Symbol, owner: &'a str) -> Vec<(&'a str, &'a Symbol, Verdict)> {
        let related = self.graph.related(owner);
        let block = self
            .fresh
            .symbols
            .iter()
            .find(|s| s.definition && s.name == owner && s.kind == "class")
            .map(|s| s.block);
        let inside = |path: &str, s: &Symbol| {
            path == self.path && block.is_some_and(|(from, to)| from <= s.from && s.from <= to)
        };
        let mut found: Vec<(&'a str, &'a Symbol, Verdict)> = Vec::new();
        for (path, s) in self.refs.named(owner) {
            if s.shape == Shape::Type && !matches!(s.kind, "command" | "route") && !inside(path, s)
            {
                found.push((path, s, Verdict::Definite));
            }
        }
        for (path, s) in self.refs.named(&def.name) {
            let call = s.kind == "route"
                || (s.kind != "command" && used_as(Some("method"), s.shape, self.family));
            let on_it = s
                .via
                .as_deref()
                .is_some_and(|class| related.contains(class));
            if call && on_it && !inside(path, s) {
                found.push((path, s, Verdict::Definite));
            }
        }
        if let Some(signature) = signature_of(self.fresh, owner) {
            for (path, s) in self.refs.named(signature) {
                if s.kind == "command" {
                    found.push((path, s, Verdict::Definite));
                }
            }
        }
        found.sort_by(|a, b| a.0.cmp(b.0).then(a.1.line.cmp(&b.1.line)));
        found.dedup_by(|a, b| a.0 == b.0 && a.1.line == b.1.line);
        found
    }
}

/// The class a method is defined in.
fn owner_of(def: &Symbol) -> Option<&str> {
    (def.definition && def.kind == "method")
        .then_some(def.via.as_deref())
        .flatten()
}

fn signature_of<'a>(parsed: &'a Parsed, class: &str) -> Option<&'a str> {
    parsed
        .classes
        .iter()
        .find(|c| &*c.name == class)
        .and_then(|c| c.signature.as_deref())
}

/// A definition's usages, for the popup.
#[derive(Serialize, Clone, Debug, Default)]
pub struct UsageList {
    pub definite: Vec<Location>,
    /// The name matches and the receiver could not be worked out.
    pub possible: Vec<Location>,
    /// The framework calls it; the usages are the uses of its class.
    pub framework: bool,
    /// The class those uses are of.
    pub owner: Option<String>,
    /// Config files a framework reads that name the class (Magento's XML).
    pub config: Vec<Location>,
    /// What those are called in the popup: `Magento config`.
    pub config_label: Option<String>,
}

/// The fully qualified name of a PHP class defined in `text` as `name`.
fn php_fqn(text: &str, name: &str) -> String {
    let space = crate::deps::php_imports(text).namespace;
    if space.is_empty() {
        name.to_string()
    } else {
        format!("{space}\\{name}")
    }
}

/// The framework config that names a PHP class, as usages.
fn config_references(
    state: &AppState,
    path: &str,
    text: &str,
    name: &str,
) -> Option<(String, Vec<Location>)> {
    if lang_of(path) != Some(Lang::Php) {
        return None;
    }
    let (label, found) = crate::frameworks::extra_references(state, &php_fqn(text, name))?;
    let found = found
        .into_iter()
        .map(|one| Location {
            path: one.path,
            line: one.line,
            column: one.column,
            kind: "config",
            text: one.text,
        })
        .collect();
    Some((label.to_string(), found))
}

/// How many places in framework config name a PHP class.
fn config_reference_count(state: &AppState, path: &str, text: &str, name: &str) -> usize {
    if lang_of(path) != Some(Lang::Php) {
        return 0;
    }
    crate::frameworks::extra_reference_count(state, &php_fqn(text, name)).unwrap_or(0)
}

/// Every usage of a definition, in files of the same family as `from`.
/// `kind` and `line` pick the definition in `from`; its class decides which
/// calls are its. Without them, every reference to the name, as written.
pub fn usages(
    state: &AppState,
    name: &str,
    from: Option<&str>,
    kind: Option<&str>,
    line: Option<u32>,
) -> Result<UsageList, String> {
    let root = key(&state.path()?);
    let shared = ready(&root)?;
    let family = from.and_then(family_of);
    let parsed = match (from, family) {
        (Some(from), Some(_)) => current_text(&root, from, None)
            .ok()
            .map(|text| symbols_in(from, &text)),
        _ => None,
    };
    let index = shared.index.read().map_err(|e| e.to_string())?;
    let mut cache = HashMap::new();
    let mut place = |path: &str, symbol: &Symbol| Location {
        path: path.to_string(),
        line: symbol.line,
        column: symbol.column,
        kind: symbol.kind,
        text: line_text(&root, path, symbol.line, &mut cache),
    };
    let def = parsed.as_ref().and_then(|parsed| {
        parsed
            .symbols
            .iter()
            .filter(|s| s.definition && s.name == name && kind.map_or(true, |kind| s.kind == kind))
            .min_by_key(|s| line.map(|line| s.line.abs_diff(line)).unwrap_or(0))
    });
    if let (Some(def), Some(parsed), Some(from), Some(family)) = (def, &parsed, from, family) {
        let request = Request::new(&index, from, family, parsed, &[def]);
        let mut judged = request.judge(def);
        judged
            .found
            .sort_by(|a, b| (a.0, a.1.line, a.1.column).cmp(&(b.0, b.1.line, b.1.column)));
        let mut list = UsageList {
            framework: judged.framework,
            owner: judged.owner.map(str::to_string),
            ..UsageList::default()
        };
        for (path, symbol, verdict) in judged.found {
            let into = match verdict {
                Verdict::Definite => &mut list.definite,
                Verdict::Possible => &mut list.possible,
            };
            if into.len() < MAX_LOCATIONS {
                into.push(place(path, symbol));
            }
        }
        if matches!(def.kind, "class" | "interface") {
            let text = current_text(&root, from, None).unwrap_or_default();
            if let Some((label, found)) = config_references(state, from, &text, &def.name) {
                list.config = found;
                list.config_label = Some(label);
            }
        }
        return Ok(list);
    }
    let mut list = UsageList::default();
    let mut paths: Vec<&String> = index.files.keys().collect();
    paths.sort();
    for path in paths {
        let held = &index.files[path];
        if family.is_some_and(|family| family != held.family) {
            continue;
        }
        for symbol in &held.symbols {
            if symbol.definition
                || symbol.name != name
                || symbol.kind == "command"
                || !used_as(kind, symbol.shape, held.family)
            {
                continue;
            }
            if list.definite.len() >= MAX_LOCATIONS {
                return Ok(list);
            }
            list.definite.push(place(path, symbol));
        }
    }
    Ok(list)
}

fn used_as(kind: Option<&str>, shape: Shape, family: Family) -> bool {
    match kind {
        Some("method") => matches!(shape, Shape::Member | Shape::Scoped),
        // A function reached through its module is written like a method
        // call: `money.format_money()`, `utils.formatDate()`, `fmt.Println()`.
        // Counting a method of the same name as well is the safer mistake.
        Some("function") if matches!(family, Family::Script | Family::Python | Family::Go) => {
            matches!(shape, Shape::Plain | Shape::Scoped | Shape::Member)
        }
        Some("function") => matches!(shape, Shape::Plain | Shape::Scoped),
        Some("class" | "interface" | "type") => {
            matches!(shape, Shape::Type | Shape::Scoped | Shape::Plain)
        }
        _ => true,
    }
}

/// A usage count over one definition.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Lens {
    pub name: String,
    pub kind: &'static str,
    pub line: u32,
    pub from: u32,
    /// Definite usages.
    pub count: usize,
    /// Calls by the name on something whose class is not known.
    pub possible: usize,
    /// The framework calls it; `count` is the uses of what it belongs to.
    pub framework: bool,
    /// The class whose uses `count` is.
    pub owner: Option<String>,
    /// Places in a framework's config that name the class.
    pub config: usize,
}

/// How many usages each function, method and class in a file has, counted
/// in the index for other files and in `text` for this one.
pub fn usage_counts(state: &AppState, path: &str, text: Option<&str>) -> Result<Vec<Lens>, String> {
    let root = key(&state.path()?);
    let Some(family) = family_of(path) else {
        return Ok(Vec::new());
    };
    let text = current_text(&root, path, text)?;
    let parsed = symbols_in(path, &text);
    let shared = ready(&root)?;
    let index = shared.index.read().map_err(|e| e.to_string())?;
    let targets: Vec<&Symbol> = parsed
        .symbols
        .iter()
        .filter(|s| s.definition && matches!(s.kind, "function" | "method" | "class" | "interface"))
        .collect();
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let request = Request::new(&index, path, family, &parsed, &targets);
    let mut lenses = Vec::new();
    for def in targets {
        let judged = request.judge(def);
        let count = judged
            .found
            .iter()
            .filter(|(_, _, verdict)| *verdict == Verdict::Definite)
            .count();
        let possible = judged.found.len() - count;
        let config = if matches!(def.kind, "class" | "interface") {
            config_reference_count(state, path, &text, &def.name)
        } else {
            0
        };
        if judged.framework || count + possible + config > 0 {
            lenses.push(Lens {
                name: def.name.clone(),
                kind: def.kind,
                line: def.line,
                from: def.from,
                count,
                possible,
                framework: judged.framework,
                owner: judged.owner.map(str::to_string),
                config,
            });
        }
    }
    Ok(lenses)
}

/// One thing nothing uses.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Mark {
    pub name: String,
    /// `function`, `method` or `variable`.
    pub kind: &'static str,
    /// `dim` for private code and locals, `hint` for public code.
    pub level: &'static str,
    pub line: u32,
    /// The name, UTF-16.
    pub from: u32,
    pub to: u32,
    /// The whole definition, UTF-16: what `dim` grays out.
    pub block_from: u32,
    pub block_to: u32,
    /// Added on this branch, or not committed yet.
    pub new: bool,
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Whole-word occurrences of `word` in `text`. A word starting with `$` (a PHP
/// variable) may follow anything but another `$`.
fn count_words(text: &str, word: &str, case: bool) -> usize {
    if word.is_empty() {
        return 0;
    }
    let lowered;
    let (hay, needle): (&str, String) = if case {
        (text, word.to_string())
    } else {
        lowered = text.to_lowercase();
        (&lowered, word.to_lowercase())
    };
    let variable = needle.starts_with('$');
    let mut count = 0;
    let mut at = 0;
    while let Some(found) = hay[at..].find(&needle) {
        let start = at + found;
        let end = start + needle.len();
        let before = hay[..start].chars().next_back();
        let after = hay[end..].chars().next();
        let clean_before = if variable {
            before != Some('$')
        } else {
            !before.is_some_and(ident_char)
        };
        let clean_after = !after.is_some_and(|c| c.is_alphanumeric() || c == '_');
        if clean_before && clean_after {
            count += 1;
        }
        at = end;
    }
    count
}

/// Locals nothing reads: declared, and the name never appears again in the
/// function that declares it.
fn unused_locals(lang: Lang, tree: &Tree, source: &str, text: &str, lines: &Lines) -> Vec<Mark> {
    let functions: &[&str] = match lang {
        Lang::Php => &[
            "function_definition",
            "method_declaration",
            "anonymous_function",
            "arrow_function",
        ],
        Lang::Js | Lang::Ts | Lang::Tsx => &[
            "function_declaration",
            "function_expression",
            "arrow_function",
            "method_definition",
            "generator_function",
            "generator_function_declaration",
        ],
        Lang::Rust => &["function_item", "closure_expression"],
        Lang::Python => &["function_definition"],
        Lang::Go => return Vec::new(),
    };
    let risky: &[&str] = match lang {
        Lang::Php => &[
            "compact(",
            "extract(",
            "$$",
            "get_defined_vars",
            "include",
            "require",
            "eval(",
            "${",
        ],
        Lang::Js | Lang::Ts | Lang::Tsx => &["eval(", "arguments"],
        Lang::Python => &["locals(", "vars(", "eval(", "exec(", "global ", "nonlocal "],
        _ => &[],
    };
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        for at in 0..node.child_count() {
            if let Some(child) = node.child(at) {
                stack.push(child);
            }
        }
        let Some((name_node, case)) = local_declaration(lang, node) else {
            continue;
        };
        let name = text_of(name_node, source);
        if name.is_empty() || name == "$this" || name.starts_with('_') || name.starts_with("$_") {
            continue;
        }
        let mut scope = node.parent();
        while let Some(parent) = scope {
            if functions.contains(&parent.kind()) {
                break;
            }
            scope = parent.parent();
        }
        let Some(scope) = scope else { continue };
        // The function as written, template and all: blanked parts of a
        // `.vue` file are never inside a function.
        let body = text.get(scope.byte_range()).unwrap_or("");
        if risky.iter().any(|word| body.contains(word)) {
            continue;
        }
        if count_words(body, name, case) != 1 {
            continue;
        }
        let (line, _, from) = lines.at(name_node.start_byte());
        let to = lines.at(name_node.end_byte()).2;
        out.push(Mark {
            name: name.to_string(),
            kind: "variable",
            level: "dim",
            line,
            from,
            to,
            block_from: from,
            block_to: to,
            new: false,
        });
    }
    out.sort_by_key(|mark| mark.from);
    out.dedup_by_key(|mark| mark.from);
    out
}

/// The name a statement declares as a local, if it is one: `$x = …;`,
/// `const x = …`, `let x = …;`, `x = …`.
fn local_declaration(lang: Lang, node: Node) -> Option<(Node, bool)> {
    match lang {
        Lang::Php => {
            if node.kind() != "assignment_expression"
                || node.parent().map(|p| p.kind()) != Some("expression_statement")
            {
                return None;
            }
            let left = node.child_by_field_name("left")?;
            (left.kind() == "variable_name").then_some((left, true))
        }
        Lang::Js | Lang::Ts | Lang::Tsx => {
            if node.kind() != "variable_declarator" {
                return None;
            }
            let declaration = node.parent()?;
            if !matches!(
                declaration.kind(),
                "lexical_declaration" | "variable_declaration"
            ) {
                return None;
            }
            // Only inside a function body: module-level names are what a
            // template or another module reads.
            let outer = declaration.parent()?;
            if matches!(outer.kind(), "program" | "export_statement") {
                return None;
            }
            let name = node.child_by_field_name("name")?;
            (name.kind() == "identifier").then_some((name, true))
        }
        Lang::Rust => {
            if node.kind() != "let_declaration" {
                return None;
            }
            let pattern = node.child_by_field_name("pattern")?;
            (pattern.kind() == "identifier").then_some((pattern, true))
        }
        Lang::Python => {
            if node.kind() != "assignment"
                || node.parent().map(|p| p.kind()) != Some("expression_statement")
            {
                return None;
            }
            let left = node.child_by_field_name("left")?;
            (left.kind() == "identifier").then_some((left, true))
        }
        Lang::Go => None,
    }
}

/// Whether a file calls methods by a name it builds: the forms each language
/// has for it. Being wrong this way only costs a dimming.
fn calls_by_built_name(lang: Lang, text: &str) -> bool {
    let forms: &[&str] = match lang {
        Lang::Php => &[
            "->{",
            "->$",
            "::{",
            "::$",
            "call_user_func",
            "method_exists",
            "Reflection",
        ],
        Lang::Python => &["getattr(", "__getattribute__", "methodcaller("],
        Lang::Rust | Lang::Go => &[],
        // JavaScript and TypeScript, in a file of their own or in a `.vue`.
        _ => &["this[", "self[", "Reflect."],
    };
    forms.iter().any(|form| text.contains(form))
}

/// What to gray out in a file, and what to hint at. See the top of this file
/// for how careful that is.
///
/// `request` orders the calls for a work tree: one overtaken by a newer
/// request stops between files and answers with nothing.
pub fn unused_in(
    state: &AppState,
    path: &str,
    text: Option<&str>,
    request: Option<u64>,
) -> Result<Vec<Mark>, String> {
    let root = key(&state.path()?);
    if lang_of(path).is_none() || skipped(path) {
        return Ok(Vec::new());
    }
    let text = current_text(&root, path, text)?;
    let Parsed {
        lang,
        source,
        tree,
        symbols,
        ..
    } = symbols_in(path, &text);
    let Some(tree) = tree else {
        return Ok(Vec::new());
    };
    let lines = Lines::new(&text);
    let case = lang != Lang::Php;
    let mut marks = Vec::new();

    // A method reached through a name put together at run time
    // (`$this->{"handle{$type}"}()`, `getattr(self, f"on_{evt}")`,
    // `this[`on${evt}`]()`) is spelled nowhere in full, so the text search
    // cannot vouch for it. In a file that calls that way, no method is marked.
    let dynamic = calls_by_built_name(lang, &text);
    let candidates: Vec<&Symbol> = symbols
        .iter()
        .filter(|s| s.callable() && !s.entry && !s.name.contains('$'))
        .filter(|s| !(dynamic && s.kind == "method"))
        .filter(|s| count_words(&text, &s.name, case) == 1)
        .collect();
    if !candidates.is_empty() {
        let shared = ready(&root)?;
        if let Some(request) = request {
            shared.newest.fetch_max(request, Ordering::SeqCst);
        }
        let superseded =
            || request.is_some_and(|request| request < shared.newest.load(Ordering::SeqCst));
        let family = lang.family();
        // The index only while it is read: the text search below takes a
        // while, and the watcher needs to write in the meantime.
        let survivors: Vec<&Symbol> = {
            let index = shared.index.read().map_err(|e| e.to_string())?;
            candidates
                .into_iter()
                .filter(|symbol| {
                    !index.files.iter().any(|(other, held)| {
                        other != path
                            && held.family == family
                            && held.symbols.iter().any(|s| s.name == symbol.name)
                    })
                })
                .collect()
        };
        if !survivors.is_empty() {
            // The last word: the text search, over every file git does not
            // ignore, in any language. All the names in one pass.
            let names: Vec<String> = survivors.iter().map(|s| s.name.clone()).collect();
            let Some(seen) = search::words_found(&root, &names, case, Some(path), &superseded)?
            else {
                return Ok(Vec::new());
            };
            for symbol in survivors {
                let name = if case {
                    symbol.name.clone()
                } else {
                    symbol.name.to_lowercase()
                };
                if seen.contains(&name) {
                    continue;
                }
                marks.push(Mark {
                    name: symbol.name.clone(),
                    kind: symbol.kind,
                    level: if symbol.public { "hint" } else { "dim" },
                    line: symbol.line,
                    from: symbol.from,
                    to: symbol.to,
                    block_from: symbol.block.0,
                    block_to: symbol.block.1,
                    new: false,
                });
            }
        }
    }
    // A class a framework's config names is made by the framework, and so
    // are calls to its public methods (a Magento plugin's `afterGetName`,
    // an observer's method): not unused for want of a caller in the code.
    if lang == Lang::Php && marks.iter().any(|mark| mark.level == "hint") {
        let wired = symbols
            .iter()
            .filter(|s| s.definition && s.kind == "class")
            .any(|class| config_reference_count(state, path, &text, &class.name) > 0);
        if wired {
            marks.retain(|mark| mark.level != "hint");
        }
    }
    marks.extend(unused_locals(lang, &tree, &source, &text, &lines));

    if !marks.is_empty() {
        let added = crate::diff::branch_added_lines(state, path).unwrap_or_default();
        for mark in marks.iter_mut() {
            mark.new = added.contains(&mark.line);
        }
    }
    marks.sort_by_key(|mark| mark.from);
    Ok(marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_grammar_and_its_query_load() {
        for lang in [
            Lang::Php,
            Lang::Js,
            Lang::Ts,
            Lang::Tsx,
            Lang::Rust,
            Lang::Python,
            Lang::Go,
        ] {
            assert!(grammar(lang).is_some(), "{lang:?}");
        }
    }

    #[test]
    fn counts_whole_words_only() {
        assert_eq!(
            count_words("total totals subtotal total()", "total", true),
            2
        );
        assert_eq!(count_words("$net = 1; $network; $$net", "$net", true), 1);
        assert_eq!(count_words("Total TOTAL", "total", false), 2);
    }

    #[test]
    fn a_vue_file_keeps_its_offsets() {
        let text = "<template><b @click=\"go\">é</b></template>\n<script setup lang=\"ts\">\nfunction go() {}\n</script>\n";
        let (lang, source) = source_for("A.vue", text);
        assert_eq!(lang, Lang::Ts);
        assert_eq!(source.len(), text.len());
        assert!(source.contains("function go() {}"));
        assert!(!source.contains("template"));
    }

    #[test]
    fn a_file_left_out_still_has_a_stamp_so_it_is_not_read_again() {
        let dir = std::env::temp_dir().join(format!("gitnoob-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("latin1.ts"), b"const caf\xe9 = 1\n").unwrap();
        let (symbols, stamp) = index_file(&dir, "latin1.ts");
        assert!(symbols.is_none());
        assert!(stamp.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_slow_pass_never_overwrites_a_newer_read() {
        let old: Stamp = (1, None);
        let newer: Stamp = (2, None);
        let mut index = Index::default();
        index.stamps.insert("a.ts".into(), newer);
        index.files.insert(
            "a.ts".into(),
            FileSymbols {
                family: Family::Script,
                symbols: Vec::new(),
                classes: Vec::new(),
            },
        );
        let since = HashMap::from([("a.ts".to_string(), old)]);
        // The pass read `a.ts` before the watcher's newer read landed.
        apply(
            &mut index,
            vec![("a.ts".into(), (None, Some((3, None))))],
            Some(&since),
        );
        assert_eq!(index.stamps.get("a.ts"), Some(&newer));
        assert!(index.files.contains_key("a.ts"));
    }

    #[test]
    fn offsets_are_utf16() {
        let lines = Lines::new("é😀x\ny");
        assert_eq!(lines.at(6), (1, 3, 3));
        assert_eq!(lines.at(8), (2, 0, 5));
        assert_eq!(byte_at_utf16("é😀x", 3), 6);
    }
}
