//! Who a method call is made on, worked out from the tree the symbol index
//! already parsed. One walk over a PHP or TypeScript file finds, without
//! types and without looking at other files:
//!
//! - the class a call's receiver is: `$this->m()`, `self::m()`, `Foo::m()`,
//!   `(new Foo)->m()`, a variable assigned `new Foo`, `app(Foo::class)` or
//!   `Foo::make()`, a parameter, property or `@var` typed `Foo`; in
//!   TypeScript `this.m()`, `Foo.m()`, `new Foo().m()`, typed parameters,
//!   variables and fields. `use` and `import` aliases are undone.
//! - the class each method is defined in, and what every class extends,
//!   implements or uses, so a query can tell a call on a subclass from a
//!   call on a stranger.
//! - names the tags query leaves out: PHP class names (`Foo::class`,
//!   `new Foo`, type hints), route actions (`[Controller::class, 'show']`),
//!   artisan command names in strings and the command names TypeScript
//!   hands to Tauri's `invoke`.
//!
//! Anything it cannot work out stays unknown, and an unknown receiver makes
//! a usage "possible" rather than wrong.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tree_sitter::{Node, Tree};

use crate::symbols::Shape;

/// A class, interface or trait, and the names it builds on.
#[derive(Clone, Debug, Default)]
pub struct ClassInfo {
    pub name: Box<str>,
    /// What it extends, implements, or uses as a trait. Short names.
    pub supers: Vec<Box<str>>,
    /// An artisan command's name: its `$signature` up to the first space.
    pub signature: Option<Box<str>>,
}

/// A name the tags query does not capture, by byte offsets.
#[derive(Debug)]
pub struct Extra {
    pub name: String,
    /// `class` for a class name, `route` for a route's action, `command`
    /// for a command name in a string.
    pub kind: &'static str,
    pub shape: Shape,
    pub start: usize,
    pub end: usize,
}

#[derive(Default, Debug)]
pub struct Found {
    /// By the byte a name starts at: the class a call's receiver is, the
    /// class a class name means once aliases are undone, the class a
    /// method is defined in, or the module a plain call's function is
    /// imported from.
    pub via: HashMap<usize, Box<str>>,
    pub extra: Vec<Extra>,
    pub classes: Vec<ClassInfo>,
}

/// Names in a type position that are not classes.
const NOT_CLASSES: &[&str] = &[
    "mixed", "object", "callable", "iterable", "array", "null", "void", "never", "false", "true",
];

/// Calls whose first argument names an artisan command.
const COMMAND_CALLS: &[&str] = &[
    "call",
    "callSilent",
    "callSilently",
    "artisan",
    "command",
    "queue",
];

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

fn named_children(node: Node) -> impl Iterator<Item = Node> {
    (0..node.named_child_count()).filter_map(move |at| node.named_child(at))
}

/// The last part of a qualified name: `App\Models\User` is `User`.
fn short(name: &str) -> &str {
    let name = name.trim();
    name.rsplit(['\\', '.']).next().unwrap_or(name)
}

/// The command name in a signature: `export:analytics {--from=}` is
/// `export:analytics`.
fn command_name(signature: &str) -> Option<&str> {
    let name = signature
        .trim()
        .split(|c: char| c.is_whitespace() || c == '{')
        .next()
        .unwrap_or("");
    (!name.is_empty()).then_some(name)
}

struct ClassCtx {
    name: Box<str>,
    parent: Option<Box<str>>,
    props: HashMap<String, Box<str>>,
}

enum Frame {
    Class,
    Scope,
    This,
}

/// Variables by name: a class, or `None` once it held something else.
type Scope = HashMap<String, Option<Box<str>>>;

struct Walk<'s> {
    source: &'s str,
    /// An alias or a local name, and the class name it stands for.
    aliases: HashMap<String, String>,
    /// TypeScript: a local name and the module it is imported from.
    imports: HashMap<String, String>,
    classes: Vec<ClassCtx>,
    scopes: Vec<Scope>,
    /// TypeScript: what `this` is, as an index into `classes`.
    this: Vec<Option<usize>>,
    frames: Vec<(usize, Frame)>,
    found: Found,
}

impl<'s> Walk<'s> {
    fn new(source: &'s str) -> Self {
        Walk {
            source,
            aliases: HashMap::new(),
            imports: HashMap::new(),
            classes: Vec::new(),
            scopes: vec![Scope::new()],
            this: Vec::new(),
            frames: Vec::new(),
            found: Found::default(),
        }
    }

    fn text(&self, node: Node) -> &'s str {
        text(node, self.source)
    }

    fn push(&mut self, node: Node, frame: Frame) {
        self.frames.push((node.id(), frame));
    }

    fn leave(&mut self, node: Node) {
        while self.frames.last().is_some_and(|(id, _)| *id == node.id()) {
            match self.frames.pop().map(|(_, frame)| frame) {
                Some(Frame::Class) => {
                    self.classes.pop();
                }
                Some(Frame::Scope) => {
                    self.scopes.pop();
                }
                Some(Frame::This) => {
                    self.this.pop();
                }
                None => {}
            }
        }
    }

    fn scope(&mut self) -> &mut Scope {
        if self.scopes.is_empty() {
            self.scopes.push(Scope::new());
        }
        let last = self.scopes.len() - 1;
        &mut self.scopes[last]
    }

    fn set_var(&mut self, name: &str, class: Option<Box<str>>) {
        if name.is_empty() {
            return;
        }
        self.scope().insert(name.to_string(), class);
    }

    fn alias(&self, name: &str) -> Box<str> {
        self.alias_of(name).into()
    }

    fn alias_of<'a>(&'a self, name: &'a str) -> &'a str {
        self.aliases.get(name).map(String::as_str).unwrap_or(name)
    }

    fn resolve(&mut self, at: usize, class: Box<str>) {
        if !class.is_empty() {
            self.found.via.insert(at, class);
        }
    }

    fn extra(&mut self, node: Node, name: &str, kind: &'static str, shape: Shape) {
        if name.is_empty() || name.len() > 200 {
            return;
        }
        self.found.extra.push(Extra {
            name: name.to_string(),
            kind,
            shape,
            start: node.start_byte(),
            end: node.end_byte(),
        });
    }

    /// Walks the tree without recursion: a long chain of calls or a big
    /// array nests deep. `enter` sees only the kinds of node it handles:
    /// most of a tree is punctuation and names.
    fn run(&mut self, tree: &Tree, kinds: &[&str], enter: fn(&mut Self, Node)) {
        let wanted = wanted_kinds(tree, kinds);
        let mut cursor = tree.walk();
        loop {
            let node = cursor.node();
            if wanted
                .get(node.kind_id() as usize)
                .copied()
                .unwrap_or(false)
            {
                enter(self, node);
            }
            if cursor.goto_first_child() {
                continue;
            }
            loop {
                self.leave(cursor.node());
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    return;
                }
            }
        }
    }
}

/// Which node kinds, by id, are in `kinds`, for one grammar. Worked out once
/// per grammar: comparing every node's kind by name is most of a walk.
fn wanted_kinds(tree: &Tree, kinds: &[&str]) -> Arc<Vec<bool>> {
    type Tables = Mutex<HashMap<(Option<&'static str>, usize, usize), Arc<Vec<bool>>>>;
    static TABLES: OnceLock<Tables> = OnceLock::new();
    let language = tree.language();
    let count = language.node_kind_count();
    let key = (language.name(), count, kinds.as_ptr() as usize);
    let build = || {
        Arc::new(
            (0..count as u16)
                .map(|id| {
                    language.node_kind_is_named(id)
                        && language
                            .node_kind_for_id(id)
                            .is_some_and(|kind| kinds.contains(&kind))
                })
                .collect::<Vec<bool>>(),
        )
    };
    let Ok(mut tables) = TABLES.get_or_init(Default::default).lock() else {
        return build();
    };
    tables.entry(key).or_insert_with(build).clone()
}

// --- PHP ---------------------------------------------------------------------

/// The PHP nodes the walk acts on.
const PHP_KINDS: &[&str] = &[
    "namespace_use_declaration",
    "class_declaration",
    "trait_declaration",
    "interface_declaration",
    "enum_declaration",
    "anonymous_class",
    "method_declaration",
    "function_definition",
    "anonymous_function",
    "arrow_function",
    "comment",
    "assignment_expression",
    "member_call_expression",
    "nullsafe_member_call_expression",
    "scoped_call_expression",
    "function_call_expression",
    "class_constant_access_expression",
    "scoped_property_access_expression",
    "object_creation_expression",
    "named_type",
    "binary_expression",
];

/// What one PHP file says about its calls and classes.
pub fn php(tree: &Tree, source: &str) -> Found {
    let mut walk = Walk::new(source);
    walk.run(tree, PHP_KINDS, php_enter);
    walk.found
}

/// The text of a plain string literal, without its quotes. Not one with
/// variables in it.
fn php_string<'t>(node: Node<'t>, source: &str) -> Option<(Node<'t>, String)> {
    if !matches!(node.kind(), "string" | "encapsed_string") || node.named_child_count() != 1 {
        return None;
    }
    let one = node.named_child(0)?;
    matches!(one.kind(), "string_content" | "string_value")
        .then(|| (one, text(one, source).to_string()))
}

/// The first argument of a call, when it is a plain string.
fn php_first_string<'t>(call: Node<'t>, source: &str) -> Option<(Node<'t>, String)> {
    let arguments = call.child_by_field_name("arguments")?;
    let first = named_children(arguments).find(|n| n.kind() == "argument")?;
    let value = named_children(first).last()?;
    php_string(value, source)
}

/// `X` in `X::class`, when the expression is one.
fn class_constant<'t>(node: Node<'t>, source: &str) -> Option<Node<'t>> {
    if node.kind() != "class_constant_access_expression" || node.named_child_count() != 2 {
        return None;
    }
    let scope = node.named_child(0)?;
    let constant = node.named_child(1)?;
    (text(constant, source) == "class" && matches!(scope.kind(), "name" | "qualified_name"))
        .then_some(scope)
}

/// The `name` node of a class name: itself, or the last part of a
/// qualified one.
fn last_name(node: Node<'_>) -> Node<'_> {
    if node.kind() == "qualified_name" {
        named_children(node)
            .filter(|n| n.kind() == "name")
            .last()
            .unwrap_or(node)
    } else {
        node
    }
}

fn php_type_comment(comment: &str) -> Option<(&str, Option<&str>)> {
    let rest = comment.split("@var").nth(1)?;
    let mut words = rest.split_whitespace();
    let ty = words.next()?;
    let var = words
        .next()
        .and_then(|word| word.strip_prefix('$'))
        .map(|word| word.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_'));
    Some((ty, var))
}

impl Walk<'_> {
    fn this_class(&self) -> Option<&ClassCtx> {
        self.classes.last().filter(|class| !class.name.is_empty())
    }

    /// A class name as written: `self`, `parent`, a name, a qualified name,
    /// a type, or a nullable type.
    fn php_class(&self, node: Node) -> Option<Box<str>> {
        self.php_class_name(node).map(Box::from)
    }

    /// `php_class` without making a copy of the name.
    fn php_class_name(&self, node: Node) -> Option<&str> {
        match node.kind() {
            "name" | "qualified_name" | "relative_scope" => {
                let written = self.text(node).trim();
                let is = |word: &str| written.eq_ignore_ascii_case(word);
                if is("self") || is("static") {
                    self.this_class().map(|c| &*c.name)
                } else if is("parent") {
                    self.this_class().and_then(|c| c.parent.as_deref())
                } else if NOT_CLASSES.iter().any(|word| is(word)) {
                    None
                } else if node.kind() == "qualified_name" {
                    Some(short(written))
                } else {
                    Some(self.alias_of(written))
                }
            }
            "named_type" | "optional_type" => {
                node.named_child(0).and_then(|n| self.php_class_name(n))
            }
            "union_type" => {
                let mut real = named_children(node)
                    .filter(|n| !(n.kind() == "primitive_type" && self.text(*n) == "null"));
                match (real.next(), real.next()) {
                    (Some(one), None) => self.php_class_name(one),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// A type written in a docblock: `Foo`, `\App\Foo`, `?Foo`, `Foo|null`.
    fn php_doc_class(&self, written: &str) -> Option<Box<str>> {
        let written = written.trim_start_matches('?');
        let parts: Vec<&str> = written
            .split('|')
            .filter(|part| !part.eq_ignore_ascii_case("null"))
            .collect();
        let [one] = parts.as_slice() else {
            return None;
        };
        let first = one.chars().next()?;
        if (!first.is_alphabetic() && first != '\\') || one.contains(['<', '[', '(', '{']) {
            return None;
        }
        if one.contains('\\') {
            Some(short(one).into())
        } else if first.is_lowercase() {
            None
        } else {
            Some(self.alias(one))
        }
    }

    fn php_var(&self, name: &str) -> Option<Box<str>> {
        self.scopes.last()?.get(name).cloned().flatten()
    }

    /// What an expression holds, when it plainly holds an instance of one
    /// class.
    fn php_value(&self, node: Node) -> Option<Box<str>> {
        match node.kind() {
            "variable_name" => {
                let name = self.text(node).trim_start_matches('$');
                if name == "this" {
                    self.this_class().map(|c| c.name.clone())
                } else {
                    self.php_var(name)
                }
            }
            "member_access_expression" | "nullsafe_member_access_expression" => {
                let object = node.child_by_field_name("object")?;
                let name = node.child_by_field_name("name")?;
                if object.kind() != "variable_name" || self.text(object) != "$this" {
                    return None;
                }
                self.this_class()?.props.get(self.text(name)).cloned()
            }
            "object_creation_expression" => named_children(node)
                .find(|n| matches!(n.kind(), "name" | "qualified_name"))
                .and_then(|n| self.php_class(n)),
            "parenthesized_expression" => named_children(node)
                .next()
                .and_then(|inner| self.php_value(inner)),
            "function_call_expression" => {
                let function = node.child_by_field_name("function")?;
                if !matches!(self.text(function), "app" | "resolve") {
                    return None;
                }
                let arguments = node.child_by_field_name("arguments")?;
                let first = named_children(arguments).find(|n| n.kind() == "argument")?;
                let value = named_children(first).last()?;
                class_constant(value, self.source).and_then(|scope| self.php_class(scope))
            }
            "scoped_call_expression" => {
                let name = node.child_by_field_name("name")?;
                if self.text(name) != "make" {
                    return None;
                }
                self.php_class(node.child_by_field_name("scope")?)
            }
            _ => None,
        }
    }

    /// A class name in the code: counted as a use of the class, and
    /// resolved through `use` aliases.
    fn php_class_ref(&mut self, node: Node) {
        if !matches!(node.kind(), "name" | "qualified_name") {
            return;
        }
        let text = self.text(node);
        if ["self", "static", "parent"]
            .iter()
            .any(|word| text.eq_ignore_ascii_case(word))
        {
            return;
        }
        let name = last_name(node);
        let written = self.text(name);
        let aliased = match self.php_class_name(node) {
            None => return,
            Some(class) => (class != written).then(|| Box::<str>::from(class)),
        };
        if let Some(class) = aliased {
            self.resolve(name.start_byte(), class);
        }
        self.extra(name, written, "class", Shape::Type);
    }

    fn php_uses(&mut self, node: Node) {
        // `use function …` and `use const …` bring in no classes.
        if (0..node.child_count())
            .filter_map(|at| node.child(at))
            .any(|child| matches!(child.kind(), "function" | "const"))
        {
            return;
        }
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            for child in named_children(at) {
                if child.kind() == "namespace_use_clause" {
                    let target = named_children(child)
                        .find(|n| matches!(n.kind(), "name" | "qualified_name"));
                    let alias = child.child_by_field_name("alias");
                    if let (Some(target), Some(alias)) = (target, alias) {
                        if target.id() != alias.id() {
                            let real = short(self.text(target)).to_string();
                            self.aliases.insert(self.text(alias).to_string(), real);
                        }
                    }
                } else if child.kind() == "namespace_use_group" {
                    stack.push(child);
                }
            }
        }
    }

    fn php_class_enter(&mut self, node: Node) {
        let name: Box<str> = node
            .child_by_field_name("name")
            .map(|n| self.text(n))
            .unwrap_or("")
            .into();
        let mut supers: Vec<Box<str>> = Vec::new();
        let mut parent = None;
        for child in named_children(node) {
            match child.kind() {
                "base_clause" => {
                    for base in named_children(child) {
                        if let Some(class) = self.php_class(base) {
                            if node.kind() == "class_declaration" && parent.is_none() {
                                parent = Some(class.clone());
                            }
                            supers.push(class);
                            self.php_class_ref(base);
                        }
                    }
                }
                "class_interface_clause" => {
                    for base in named_children(child) {
                        if let Some(class) = self.php_class(base) {
                            supers.push(class);
                        }
                    }
                }
                _ => {}
            }
        }
        // Properties first: a method may use one declared below it, or one
        // the constructor promotes.
        let mut props: HashMap<String, Box<str>> = HashMap::new();
        let mut signature: Option<Box<str>> = None;
        let mut named: Option<Box<str>> = None;
        if let Some(body) = node.child_by_field_name("body") {
            let members: Vec<Node> = named_children(body).collect();
            for (at, member) in members.iter().enumerate() {
                match member.kind() {
                    "use_declaration" => {
                        for used in named_children(*member) {
                            if let Some(class) = self.php_class(used) {
                                supers.push(class);
                            }
                        }
                    }
                    "property_declaration" => {
                        let typed = member
                            .child_by_field_name("type")
                            .and_then(|t| self.php_class(t))
                            .or_else(|| {
                                let before = at.checked_sub(1).map(|b| members[b])?;
                                if before.kind() != "comment" {
                                    return None;
                                }
                                let (ty, _) = php_type_comment(self.text(before))?;
                                self.php_doc_class(ty)
                            });
                        for element in
                            named_children(*member).filter(|n| n.kind() == "property_element")
                        {
                            let Some(var) = element.child_by_field_name("name") else {
                                continue;
                            };
                            let prop = self.text(var).trim_start_matches('$');
                            if let Some(class) = &typed {
                                props.insert(prop.to_string(), class.clone());
                            }
                            let value = element
                                .child_by_field_name("default_value")
                                .and_then(|v| php_string(v, self.source))
                                .and_then(|(_, value)| command_name(&value).map(Box::from));
                            match prop {
                                "signature" => signature = value,
                                "name" => named = value,
                                _ => {}
                            }
                        }
                    }
                    "method_declaration" => {
                        let is_constructor = member
                            .child_by_field_name("name")
                            .is_some_and(|n| self.text(n) == "__construct");
                        let parameters = member.child_by_field_name("parameters");
                        if let (true, Some(parameters)) = (is_constructor, parameters) {
                            for parameter in named_children(parameters)
                                .filter(|p| p.kind() == "property_promotion_parameter")
                            {
                                let class = parameter
                                    .child_by_field_name("type")
                                    .and_then(|t| self.php_class(t));
                                let var = parameter.child_by_field_name("name");
                                if let (Some(class), Some(var)) = (class, var) {
                                    let prop = self.text(var).trim_start_matches('$');
                                    props.insert(prop.to_string(), class);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // `#[AsCommand(name: 'export:analytics')]`
        if signature.is_none() {
            signature = named_children(node)
                .filter(|n| n.kind() == "attribute_list")
                .find(|n| self.text(*n).contains("AsCommand"))
                .and_then(|list| {
                    let mut stack = vec![list];
                    while let Some(at) = stack.pop() {
                        if let Some((_, value)) = php_string(at, self.source) {
                            return command_name(&value).map(Box::from);
                        }
                        stack.extend(named_children(at).collect::<Vec<_>>().into_iter().rev());
                    }
                    None
                });
        }
        if signature.is_none() && supers.iter().any(|s| &**s == "Command") {
            signature = named;
        }
        if !name.is_empty() {
            self.found.classes.push(ClassInfo {
                name: name.clone(),
                supers,
                signature,
            });
        }
        self.classes.push(ClassCtx {
            name,
            parent,
            props,
        });
        self.push(node, Frame::Class);
    }

    fn php_parameters(&mut self, function: Node) {
        let Some(parameters) = function.child_by_field_name("parameters") else {
            return;
        };
        for parameter in named_children(parameters) {
            if !matches!(
                parameter.kind(),
                "simple_parameter" | "property_promotion_parameter" | "variadic_parameter"
            ) {
                continue;
            }
            let Some(var) = parameter.child_by_field_name("name") else {
                continue;
            };
            let class = if parameter.kind() == "variadic_parameter" {
                None
            } else {
                parameter
                    .child_by_field_name("type")
                    .and_then(|t| self.php_class(t))
            };
            let name = self.text(var).trim_start_matches('$');
            self.set_var(name, class);
        }
    }

    fn php_command_string(&mut self, call: Node) {
        let Some(name) = call
            .child_by_field_name("name")
            .or_else(|| call.child_by_field_name("function"))
        else {
            return;
        };
        if !COMMAND_CALLS.contains(&self.text(name)) {
            return;
        }
        if let Some((node, value)) = php_first_string(call, self.source) {
            if let Some(command) = command_name(&value) {
                let start = node.start_byte();
                let end = start + command.len();
                self.found.extra.push(Extra {
                    name: command.to_string(),
                    kind: "command",
                    shape: Shape::Plain,
                    start,
                    end,
                });
            }
        }
    }

    /// `[Controller::class, 'action']`: a use of that one action.
    fn php_route(&mut self, node: Node, scope: Node) -> bool {
        let Some(element) = node
            .parent()
            .filter(|p| p.kind() == "array_element_initializer")
        else {
            return false;
        };
        let Some(array) = element
            .parent()
            .filter(|p| p.kind() == "array_creation_expression")
        else {
            return false;
        };
        let elements: Vec<Node> = named_children(array)
            .filter(|n| n.kind() == "array_element_initializer")
            .collect();
        let [first, second] = elements.as_slice() else {
            return false;
        };
        if first.id() != element.id() {
            return false;
        }
        let Some((at, action)) = named_children(*second)
            .last()
            .and_then(|value| php_string(value, self.source))
        else {
            return false;
        };
        if action.is_empty() || !action.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return false;
        }
        let Some(class) = self.php_class(scope) else {
            return false;
        };
        self.resolve(at.start_byte(), class);
        self.extra(at, &action, "route", Shape::Member);
        true
    }
}

fn php_enter(walk: &mut Walk, node: Node) {
    match node.kind() {
        "namespace_use_declaration" => walk.php_uses(node),
        "class_declaration"
        | "trait_declaration"
        | "interface_declaration"
        | "enum_declaration" => walk.php_class_enter(node),
        "anonymous_class" => {
            walk.classes.push(ClassCtx {
                name: "".into(),
                parent: None,
                props: HashMap::new(),
            });
            walk.push(node, Frame::Class);
        }
        "method_declaration" | "function_definition" => {
            if node.kind() == "method_declaration" {
                let owner = walk.this_class().map(|c| c.name.clone());
                if let (Some(owner), Some(name)) = (owner, node.child_by_field_name("name")) {
                    walk.resolve(name.start_byte(), owner);
                }
            }
            walk.scopes.push(Scope::new());
            walk.push(node, Frame::Scope);
            walk.php_parameters(node);
        }
        "anonymous_function" => {
            // A closure sees only what it `use`s.
            let mut scope = Scope::new();
            if let Some(clause) =
                named_children(node).find(|n| n.kind() == "anonymous_function_use_clause")
            {
                for var in named_children(clause) {
                    let name = walk.text(var).trim_start_matches(['&', '$']);
                    if let Some(class) = walk.scopes.last().and_then(|s| s.get(name)) {
                        scope.insert(name.to_string(), class.clone());
                    }
                }
            }
            walk.scopes.push(scope);
            walk.push(node, Frame::Scope);
            walk.php_parameters(node);
        }
        "arrow_function" => {
            let scope = walk.scopes.last().cloned().unwrap_or_default();
            walk.scopes.push(scope);
            walk.push(node, Frame::Scope);
            walk.php_parameters(node);
        }
        "comment" => {
            let comment = walk.text(node);
            if let Some((ty, Some(var))) = php_type_comment(comment) {
                let class = walk.php_doc_class(ty);
                if class.is_some() {
                    walk.set_var(var, class);
                }
            }
        }
        "assignment_expression" => {
            let Some(left) = node.child_by_field_name("left") else {
                return;
            };
            if left.kind() != "variable_name" {
                return;
            }
            let name = walk.text(left).trim_start_matches('$');
            // `/** @var Foo $x */` or `/** @var Foo */` on the line above.
            let documented = node
                .parent()
                .filter(|p| p.kind() == "expression_statement")
                .and_then(|statement| statement.prev_named_sibling())
                .filter(|before| before.kind() == "comment")
                .and_then(|before| php_type_comment(walk.text(before)))
                .filter(|(_, var)| var.map_or(true, |var| var == name))
                .and_then(|(ty, _)| walk.php_doc_class(ty));
            let class = documented.or_else(|| {
                node.child_by_field_name("right")
                    .and_then(|right| walk.php_value(right))
            });
            walk.set_var(name, class);
        }
        "member_call_expression" | "nullsafe_member_call_expression" => {
            let receiver = node
                .child_by_field_name("object")
                .and_then(|object| walk.php_value(object));
            if let (Some(receiver), Some(name)) = (receiver, node.child_by_field_name("name")) {
                walk.resolve(name.start_byte(), receiver);
            }
            walk.php_command_string(node);
        }
        "scoped_call_expression" => {
            if let Some(scope) = node.child_by_field_name("scope") {
                let receiver = walk.php_class(scope);
                if let (Some(receiver), Some(name)) = (receiver, node.child_by_field_name("name")) {
                    walk.resolve(name.start_byte(), receiver);
                }
                walk.php_class_ref(scope);
            }
            walk.php_command_string(node);
        }
        "function_call_expression" => walk.php_command_string(node),
        "class_constant_access_expression" => {
            let Some(scope) = named_children(node).next() else {
                return;
            };
            if class_constant(node, walk.source).is_some() && walk.php_route(node, scope) {
                return;
            }
            walk.php_class_ref(scope);
        }
        "scoped_property_access_expression" => {
            if let Some(scope) = node.child_by_field_name("scope") {
                walk.php_class_ref(scope);
            }
        }
        "object_creation_expression" => {
            if let Some(class) =
                named_children(node).find(|n| matches!(n.kind(), "name" | "qualified_name"))
            {
                walk.php_class_ref(class);
            }
        }
        "named_type" => {
            if let Some(class) = named_children(node).next() {
                walk.php_class_ref(class);
            }
        }
        "binary_expression" => {
            let instanceof = node
                .child_by_field_name("operator")
                .is_some_and(|op| walk.text(op) == "instanceof");
            if let (true, Some(right)) = (instanceof, node.child_by_field_name("right")) {
                walk.php_class_ref(right);
            }
        }
        _ => {}
    }
}

// --- TypeScript and JavaScript -----------------------------------------------

/// The TypeScript and JavaScript nodes the walk acts on.
const SCRIPT_KINDS: &[&str] = &[
    "import_statement",
    "class_declaration",
    "class",
    "abstract_class_declaration",
    "interface_declaration",
    "method_definition",
    "function_declaration",
    "function_expression",
    "function",
    "generator_function_declaration",
    "generator_function",
    "arrow_function",
    "variable_declarator",
    "call_expression",
];

/// What one TypeScript or JavaScript file says about its calls and classes.
pub fn script(tree: &Tree, source: &str) -> Found {
    let mut walk = Walk::new(source);
    walk.run(tree, SCRIPT_KINDS, script_enter);
    walk.found
}

fn script_string<'t>(node: Node<'t>, source: &str) -> Option<(Node<'t>, String)> {
    if node.kind() != "string" || node.named_child_count() != 1 {
        return None;
    }
    let one = node.named_child(0)?;
    (one.kind() == "string_fragment").then(|| (one, text(one, source).to_string()))
}

impl Walk<'_> {
    fn script_this(&self) -> Option<&ClassCtx> {
        let at = (*self.this.last()?)?;
        self.classes.get(at).filter(|class| !class.name.is_empty())
    }

    /// A type, when it is one class: `Foo`, `Foo<T>`, `ns.Foo`, `Foo | null`.
    fn script_type(&self, node: Node) -> Option<Box<str>> {
        match node.kind() {
            "type_annotation" => named_children(node)
                .next()
                .and_then(|n| self.script_type(n)),
            "type_identifier" | "identifier" => Some(self.alias(self.text(node))),
            "generic_type" => self.script_type(node.child_by_field_name("name")?),
            "nested_type_identifier" => {
                named_children(node).last().map(|n| Box::from(self.text(n)))
            }
            "union_type" => {
                let real: Vec<Node> = named_children(node)
                    .filter(|n| {
                        !matches!(n.kind(), "literal_type" | "predefined_type")
                            || !matches!(self.text(*n), "null" | "undefined")
                    })
                    .collect();
                match real.as_slice() {
                    [one] => self.script_type(*one),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn script_var(&self, name: &str) -> Option<Option<Box<str>>> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).cloned())
    }

    fn script_value(&self, node: Node) -> Option<Box<str>> {
        match node.kind() {
            "this" => self.script_this().map(|c| c.name.clone()),
            "super" => self.script_this().and_then(|c| c.parent.clone()),
            "identifier" => {
                let name = self.text(node);
                match self.script_var(name) {
                    Some(class) => class,
                    None if name.starts_with(char::is_uppercase) => Some(self.alias(name)),
                    None => None,
                }
            }
            "member_expression" => {
                let object = node.child_by_field_name("object")?;
                let property = node.child_by_field_name("property")?;
                if object.kind() != "this" {
                    return None;
                }
                self.script_this()?.props.get(self.text(property)).cloned()
            }
            "new_expression" => {
                let constructor = node.child_by_field_name("constructor")?;
                (constructor.kind() == "identifier").then(|| self.alias(self.text(constructor)))
            }
            "parenthesized_expression" | "non_null_expression" => named_children(node)
                .next()
                .and_then(|inner| self.script_value(inner)),
            "as_expression" => named_children(node)
                .nth(1)
                .and_then(|ty| self.script_type(ty)),
            _ => None,
        }
    }

    fn script_imports(&mut self, node: Node) {
        let Some(source) = node
            .child_by_field_name("source")
            .and_then(|s| script_string(s, self.source))
            .map(|(_, value)| value)
        else {
            return;
        };
        let Some(clause) = named_children(node).find(|n| n.kind() == "import_clause") else {
            return;
        };
        for part in named_children(clause) {
            match part.kind() {
                "identifier" => {
                    self.imports
                        .insert(self.text(part).to_string(), source.clone());
                }
                "named_imports" => {
                    for specifier in named_children(part).filter(|n| n.kind() == "import_specifier")
                    {
                        let Some(name) = specifier.child_by_field_name("name") else {
                            continue;
                        };
                        let name = self.text(name).to_string();
                        let local = specifier
                            .child_by_field_name("alias")
                            .map(|alias| self.text(alias).to_string())
                            .unwrap_or_else(|| name.clone());
                        if local != name {
                            self.aliases.insert(local.clone(), name);
                        }
                        self.imports.insert(local, source.clone());
                    }
                }
                _ => {}
            }
        }
    }

    fn script_class_enter(&mut self, node: Node) {
        let name: Box<str> = node
            .child_by_field_name("name")
            .map(|n| self.text(n))
            .unwrap_or("")
            .into();
        let mut supers: Vec<Box<str>> = Vec::new();
        let mut parent = None;
        if let Some(heritage) = named_children(node).find(|n| n.kind() == "class_heritage") {
            for clause in named_children(heritage) {
                match clause.kind() {
                    "extends_clause" => {
                        for value in named_children(clause) {
                            let class = match value.kind() {
                                "identifier" => Some(self.alias(self.text(value))),
                                "member_expression" => value
                                    .child_by_field_name("property")
                                    .map(|p| Box::from(self.text(p))),
                                _ => None,
                            };
                            if let Some(class) = class {
                                parent.get_or_insert_with(|| class.clone());
                                supers.push(class);
                            }
                        }
                    }
                    "implements_clause" => {
                        supers.extend(named_children(clause).filter_map(|t| self.script_type(t)));
                    }
                    _ => {}
                }
            }
        }
        let mut props: HashMap<String, Box<str>> = HashMap::new();
        if let Some(body) = node.child_by_field_name("body") {
            for member in named_children(body) {
                match member.kind() {
                    "public_field_definition" => {
                        let field = member.child_by_field_name("name");
                        let class = member
                            .child_by_field_name("type")
                            .and_then(|t| self.script_type(t));
                        if let (Some(field), Some(class)) = (field, class) {
                            props.insert(self.text(field).to_string(), class);
                        }
                    }
                    "method_definition" => {
                        let constructor = member
                            .child_by_field_name("name")
                            .is_some_and(|n| self.text(n) == "constructor");
                        let parameters = member.child_by_field_name("parameters");
                        if let (true, Some(parameters)) = (constructor, parameters) {
                            for parameter in named_children(parameters) {
                                let promoted = named_children(parameter).any(|n| {
                                    matches!(n.kind(), "accessibility_modifier" | "readonly")
                                }) || (0..parameter.child_count())
                                    .filter_map(|at| parameter.child(at))
                                    .any(|n| n.kind() == "readonly");
                                if !promoted {
                                    continue;
                                }
                                let field = parameter.child_by_field_name("pattern");
                                let class = parameter
                                    .child_by_field_name("type")
                                    .and_then(|t| self.script_type(t));
                                if let (Some(field), Some(class)) = (field, class) {
                                    props.insert(self.text(field).to_string(), class);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if !name.is_empty() {
            self.found.classes.push(ClassInfo {
                name: name.clone(),
                supers,
                signature: None,
            });
        }
        self.classes.push(ClassCtx {
            name,
            parent,
            props,
        });
        self.push(node, Frame::Class);
        self.this.push(Some(self.classes.len() - 1));
        self.push(node, Frame::This);
    }

    fn script_parameters(&mut self, function: Node) {
        let Some(parameters) = function.child_by_field_name("parameters") else {
            return;
        };
        for parameter in named_children(parameters) {
            if !matches!(
                parameter.kind(),
                "required_parameter" | "optional_parameter"
            ) {
                continue;
            }
            let Some(pattern) = parameter
                .child_by_field_name("pattern")
                .filter(|p| p.kind() == "identifier")
            else {
                continue;
            };
            let class = parameter
                .child_by_field_name("type")
                .and_then(|t| self.script_type(t));
            let name = self.text(pattern);
            self.set_var(name, class);
        }
    }

    fn script_function(&mut self, node: Node, this: Option<Option<usize>>) {
        if let Some(this) = this {
            self.this.push(this);
            self.push(node, Frame::This);
        }
        self.scopes.push(Scope::new());
        self.push(node, Frame::Scope);
        self.script_parameters(node);
    }
}

fn script_enter(walk: &mut Walk, node: Node) {
    match node.kind() {
        "import_statement" => walk.script_imports(node),
        "class_declaration" | "class" | "abstract_class_declaration" => {
            walk.script_class_enter(node)
        }
        "interface_declaration" => {
            let Some(name) = node.child_by_field_name("name") else {
                return;
            };
            let supers = named_children(node)
                .filter(|n| n.kind() == "extends_type_clause")
                .flat_map(named_children)
                .filter_map(|t| walk.script_type(t))
                .collect();
            let name = walk.text(name).into();
            walk.found.classes.push(ClassInfo {
                name,
                supers,
                signature: None,
            });
        }
        "method_definition" => {
            let in_class = node.parent().is_some_and(|p| p.kind() == "class_body");
            if in_class {
                let owner = walk.script_this().map(|c| c.name.clone());
                if let (Some(owner), Some(name)) = (owner, node.child_by_field_name("name")) {
                    walk.resolve(name.start_byte(), owner);
                }
                walk.script_function(node, None);
            } else {
                walk.script_function(node, Some(None));
            }
        }
        "function_declaration"
        | "function_expression"
        | "function"
        | "generator_function_declaration"
        | "generator_function" => walk.script_function(node, Some(None)),
        "arrow_function" => {
            walk.scopes.push(Scope::new());
            walk.push(node, Frame::Scope);
            walk.script_parameters(node);
        }
        "variable_declarator" => {
            let Some(name) = node
                .child_by_field_name("name")
                .filter(|n| n.kind() == "identifier")
            else {
                return;
            };
            let class = node
                .child_by_field_name("type")
                .and_then(|t| walk.script_type(t))
                .or_else(|| {
                    node.child_by_field_name("value")
                        .and_then(|value| walk.script_value(value))
                });
            let name = walk.text(name);
            walk.set_var(name, class);
        }
        "call_expression" => {
            let Some(function) = node.child_by_field_name("function") else {
                return;
            };
            match function.kind() {
                "member_expression" => {
                    let receiver = function
                        .child_by_field_name("object")
                        .and_then(|object| walk.script_value(object));
                    let property = function.child_by_field_name("property");
                    if let (Some(receiver), Some(property)) = (receiver, property) {
                        walk.resolve(property.start_byte(), receiver);
                    }
                }
                "identifier" => {
                    let name = walk.text(function);
                    if let Some(module) = walk.imports.get(name) {
                        let module: Box<str> = module.as_str().into();
                        walk.resolve(function.start_byte(), module);
                    }
                    // `invoke('usage_counts', …)`, `invokeAt(repo, 'usages', …)`:
                    // a Tauri command by name.
                    if name.to_ascii_lowercase().contains("invoke") {
                        let found = node.child_by_field_name("arguments").and_then(|arguments| {
                            named_children(arguments)
                                .take(2)
                                .find_map(|a| script_string(a, walk.source))
                        });
                        if let Some((at, command)) = found {
                            walk.extra(at, &command, "command", Shape::Plain);
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_php(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    fn parse_ts(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    /// The class each `m(` call in `source` resolves to, in order.
    fn receivers(found: &Found, source: &str) -> Vec<Option<String>> {
        source
            .match_indices("m(")
            .filter(|(at, _)| {
                let before = source[..*at].chars().next_back();
                matches!(before, Some('>' | ':' | '.'))
            })
            .map(|(at, _)| found.via.get(&at).map(|c| c.to_string()))
            .collect()
    }

    #[test]
    fn php_receivers_resolve_from_the_tree() {
        let source = r#"<?php
use App\Services\Exporter;
use App\Services\Mailer as Post;
class Report extends Base
{
    /** @var Exporter */
    protected $legacy;
    public function __construct(private Post $post) {}
    public function run(Exporter $typed, $loose)
    {
        $this->m();
        self::m();
        parent::m();
        Exporter::m();
        (new Exporter)->m();
        $made = Post::make();
        $made->m();
        $bound = app(Exporter::class);
        $bound->m();
        $this->post->m();
        $this->legacy->m();
        $typed->m();
        $loose->m();
        /** @var Post $doc */
        $doc = something();
        $doc->m();
        $f = function () use ($typed) { $typed->m(); };
    }
}
"#;
        let found = php(&parse_php(source), source);
        let r = |s: &str| Some(s.to_string());
        assert_eq!(
            receivers(&found, source),
            vec![
                r("Report"),
                r("Report"),
                r("Base"),
                r("Exporter"),
                r("Exporter"),
                r("Mailer"),
                r("Exporter"),
                r("Mailer"),
                r("Exporter"),
                r("Exporter"),
                None,
                r("Mailer"),
                r("Exporter"),
            ]
        );
        let report = found.classes.iter().find(|c| &*c.name == "Report").unwrap();
        assert_eq!(report.supers, vec![Box::from("Base")]);
    }

    #[test]
    fn php_finds_routes_command_names_and_signatures() {
        let source = r#"<?php
use App\Http\Controllers\PhotoController as Photos;
Route::get('/p', [Photos::class, 'show']);
Artisan::call('export:analytics --from=1');
$schedule->command('export:analytics')->daily();
class ExportAnalyticsCommand extends Command
{
    protected $signature = 'export:analytics
        {--from=}';
}
"#;
        let found = php(&parse_php(source), source);
        let route = found.extra.iter().find(|e| e.kind == "route").unwrap();
        assert_eq!(route.name, "show");
        assert_eq!(
            found.via.get(&route.start).map(|c| &**c),
            Some("PhotoController")
        );
        let commands: Vec<&str> = found
            .extra
            .iter()
            .filter(|e| e.kind == "command")
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(commands, vec!["export:analytics", "export:analytics"]);
        assert_eq!(
            found.classes[0].signature.as_deref(),
            Some("export:analytics")
        );
        // The class in a route pair is the route's, not a use of its own.
        assert!(!found
            .extra
            .iter()
            .any(|e| e.kind == "class" && e.name == "Photos"));
    }

    #[test]
    fn script_receivers_resolve_from_the_tree() {
        let source = r#"
import { Foo, Bar as Baz } from './foo'
import { helper } from '../lib/helpers'
class Runner extends Base {
  private svc: Foo
  constructor(private readonly other: Baz) { super() }
  run(p: Foo, loose) {
    this.m()
    this.svc.m()
    this.other.m()
    p.m()
    Foo.m()
    new Foo().m()
    const a = new Baz()
    a.m()
    loose.m()
    super.m()
    helper()
    invoke<number>('usage_counts', {})
  }
}
"#;
        let found = script(&parse_ts(source), source);
        let r = |s: &str| Some(s.to_string());
        assert_eq!(
            receivers(&found, source),
            vec![
                r("Runner"),
                r("Foo"),
                r("Bar"),
                r("Foo"),
                r("Foo"),
                r("Foo"),
                r("Bar"),
                None,
                r("Base"),
            ]
        );
        let helper = source.find("helper()").unwrap();
        assert_eq!(found.via.get(&helper).map(|m| &**m), Some("../lib/helpers"));
        assert!(found
            .extra
            .iter()
            .any(|e| e.kind == "command" && e.name == "usage_counts"));
    }
}
