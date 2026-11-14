//! What `.editorconfig` files say about indenting one file.
//!
//! Only the indentation keys are read: `indent_style`, `indent_size` and
//! `tab_width`. Files are looked for from the file's folder up to the work
//! tree's root, stopping early at one that says `root = true`. A nearer file
//! wins over a farther one, and within a file a later section wins over an
//! earlier one, as the spec has it. Section globs are matched with globset:
//! a glob with no `/` matches the file name in any folder, one with a `/` is
//! relative to the folder the `.editorconfig` is in.

use std::path::Path;

use globset::GlobBuilder;
use serde::Serialize;

use crate::files;
use crate::state::AppState;

/// The indentation `.editorconfig` asks for; every field is optional.
#[derive(Serialize, Default, Debug, Clone, PartialEq)]
pub struct Indent {
    /// `space` or `tab`.
    pub style: Option<String>,
    /// Columns per level.
    pub size: Option<u32>,
    /// Columns a tab is drawn as.
    pub tab_width: Option<u32>,
}

/// One `[glob]` section and its keys, lower-cased.
#[derive(Debug, Default, PartialEq)]
pub struct Section {
    pub glob: String,
    pub props: Vec<(String, String)>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub root: bool,
    pub sections: Vec<Section>,
}

/// Reads an `.editorconfig`: `[sections]`, `key = value`, `#` and `;`
/// comments. Keys and the values that matter here are case-insensitive.
pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            parsed.sections.push(Section {
                glob: line[1..line.len() - 1].trim().to_string(),
                props: Vec::new(),
            });
            continue;
        }
        let Some((key, value)) = line.split_once('=').or_else(|| line.split_once(':')) else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim().to_lowercase();
        match parsed.sections.last_mut() {
            Some(section) => section.props.push((key, value)),
            None if key == "root" => parsed.root = value == "true",
            None => {}
        }
    }
    parsed
}

/// Whether a section's glob takes `relative`, a path with `/` from the
/// folder the `.editorconfig` is in.
pub fn matches(glob: &str, relative: &str) -> bool {
    let pattern = if glob.contains('/') {
        glob.trim_start_matches('/').to_string()
    } else {
        format!("**/{glob}")
    };
    GlobBuilder::new(&pattern)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map(|glob| glob.compile_matcher().is_match(relative))
        .unwrap_or(false)
}

pub fn indent_for(state: &AppState, relative: &str) -> Result<Indent, String> {
    indent_for_at(&state.path()?, relative)
}

pub fn indent_for_at(root: &Path, relative: &str) -> Result<Indent, String> {
    // The same guard as every other path from the window.
    let real = files::resolve(root, relative, false)?;
    let root = root
        .canonicalize()
        .map_err(|e| format!("Could not read the work tree: {e}"))?;
    // Nearest first, up to the root or a file that says it is the root.
    let mut found: Vec<(std::path::PathBuf, Parsed)> = Vec::new();
    let mut dir = real.parent();
    while let Some(here) = dir {
        if !here.starts_with(&root) {
            break;
        }
        if let Ok(text) = std::fs::read_to_string(here.join(".editorconfig")) {
            let parsed = parse(&text);
            let stop = parsed.root;
            found.push((here.to_path_buf(), parsed));
            if stop {
                break;
            }
        }
        if here == root {
            break;
        }
        dir = here.parent();
    }
    let mut props: Vec<(String, String)> = Vec::new();
    for (dir, parsed) in found.iter().rev() {
        let Ok(inside) = real.strip_prefix(dir) else {
            continue;
        };
        let inside = files::slashed(inside);
        for section in &parsed.sections {
            if matches(&section.glob, &inside) {
                props.extend(section.props.iter().cloned());
            }
        }
    }
    let last = |key: &str| {
        props
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let style = last("indent_style")
        .filter(|v| *v == "space" || *v == "tab")
        .map(str::to_string);
    let tab_width = last("tab_width").and_then(|v| v.parse::<u32>().ok());
    let size = match last("indent_size") {
        Some("tab") => tab_width,
        Some(v) => v.parse::<u32>().ok(),
        None => None,
    };
    // The spec's defaults: a tab is as wide as a level, and a level of tabs
    // is one tab wide.
    let tab_width = tab_width.or(size);
    let size = size.or(if style.as_deref() == Some("tab") {
        tab_width
    } else {
        None
    });
    Ok(Indent {
        style,
        size: size.filter(|n| (1..=16).contains(n)),
        tab_width: tab_width.filter(|n| (1..=16).contains(n)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sections_keys_and_the_root_flag() {
        let parsed = parse(
            "root = true\n# comment\n[*]\nindent_style = space\n; other\n[*.{js,ts}]\nIndent_Size = 2\n",
        );
        assert!(parsed.root);
        assert_eq!(parsed.sections.len(), 2);
        assert_eq!(parsed.sections[1].glob, "*.{js,ts}");
        assert_eq!(
            parsed.sections[1].props,
            vec![("indent_size".to_string(), "2".to_string())]
        );
    }

    #[test]
    fn globs_without_a_slash_match_the_name_anywhere() {
        assert!(matches("*", "app/Http/Kernel.php"));
        assert!(matches("*.php", "app/Http/Kernel.php"));
        assert!(matches("*.{js,ts}", "src/main.ts"));
        assert!(!matches("*.{js,ts}", "src/main.tsx"));
        assert!(matches("Makefile", "build/Makefile"));
        assert!(matches("src/**.ts", "src/a/b.ts") || matches("src/**/*.ts", "src/a/b.ts"));
        assert!(matches("/lib/*.rs", "lib/main.rs"));
        assert!(!matches("lib/*.rs", "lib/deep/main.rs"));
        assert!(matches("[Mm]akefile", "Makefile"));
    }
}
