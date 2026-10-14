//! A look over what is about to be committed, before it is.
//!
//! Some mistakes are cheap to make and dear to take back: a conflict marker
//! committed by accident breaks the build for everyone who pulls, and a key
//! that reaches a remote has to be rotated, not just deleted. Neither is hard
//! to spot in the staged diff, so it is looked at first.
//!
//! This warns and never refuses. Every check here can be wrong about a given
//! file — a test fixture full of fake keys, a markdown file explaining
//! conflict markers — and the person committing is the one who knows. Only
//! high-confidence patterns are looked for, so that the warning means
//! something when it does appear.

use serde::Serialize;

use crate::git_cmd;
use crate::state::AppState;

/// One thing worth a second look.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Finding {
    pub path: String,
    /// The line in the staged file, when the finding is about a line.
    pub line: Option<u32>,
    /// `conflict`, `secret`, `file` or `debug`.
    pub kind: String,
    /// What it is, said plainly.
    pub what: String,
    /// The line itself, shortened, with any secret in it masked.
    pub excerpt: Option<String>,
}

/// More than this and the list stops being read; the first few are enough to
/// make the point.
const MOST: usize = 40;

/// Looks over everything staged.
///
/// `debug` adds the leftovers of debugging — `console.log`, `dbg!` and the
/// like — which are worth a look in some codebases and noise in others.
pub fn scan(state: &AppState, debug: bool) -> Result<Vec<Finding>, String> {
    let root = state.path()?;
    let mut names = crate::work::plain_diff();
    names.extend(["--cached", "--name-only", "--diff-filter=d", "-z"]);
    let listed = git_cmd::run_checked(&root, &names)?;
    let paths: Vec<String> = listed
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect();

    let mut patch = crate::work::plain_diff();
    // No context: only the lines being added are being committed by this
    // commit, and the ones around them were judged when they went in.
    patch.extend(["--cached", "--unified=0", "--no-renames"]);
    let diff = git_cmd::run_checked(&root, &patch)?;

    let mut found = scan_paths(&paths);
    found.extend(scan_patch(&diff, debug));
    found.truncate(MOST);
    Ok(found)
}

/// Files that should almost never be in a repository at all.
pub fn scan_paths(paths: &[String]) -> Vec<Finding> {
    paths
        .iter()
        .filter_map(|path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            let what = sensitive_name(name)?;
            Some(Finding {
                path: path.clone(),
                line: None,
                kind: "file".to_string(),
                what: what.to_string(),
                excerpt: None,
            })
        })
        .collect()
}

fn sensitive_name(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    // The examples are what a repository is meant to hold instead.
    const EXAMPLES: [&str; 5] = ["example", "sample", "template", "dist", "defaults"];
    if lower == ".env"
        || (lower.starts_with(".env.") && !EXAMPLES.iter().any(|tail| lower.ends_with(tail)))
    {
        return Some("An environment file — these usually hold passwords and keys");
    }
    if matches!(
        lower.as_str(),
        "id_rsa" | "id_dsa" | "id_ecdsa" | "id_ed25519"
    ) {
        return Some("A private SSH key");
    }
    if lower.ends_with(".pem") || lower.ends_with(".p12") || lower.ends_with(".pfx") {
        return Some("A certificate or key file");
    }
    None
}

/// The added lines of a zero-context patch, looked over one by one.
pub fn scan_patch(diff: &str, debug: bool) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut path: Option<String> = None;
    let mut line_no: u32 = 0;
    // Inside a hunk the `+++` of a line that happens to start with `++` is
    // content, not a header; headers only come before the first `@@`.
    let mut in_hunk = false;

    for raw in diff.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.starts_with("diff --git ") {
            path = None;
            in_hunk = false;
            continue;
        }
        if !in_hunk {
            if let Some(name) = line.strip_prefix("+++ ") {
                path =
                    (name != "/dev/null").then(|| unquote(name.strip_prefix("b/").unwrap_or(name)));
                continue;
            }
        }
        if let Some(rest) = line.strip_prefix("@@ ") {
            in_hunk = true;
            line_no = new_start(rest).unwrap_or(1);
            continue;
        }
        let Some(added) = line.strip_prefix('+') else {
            continue;
        };
        if !in_hunk {
            continue;
        }
        let Some(file) = &path else {
            line_no += 1;
            continue;
        };
        for (kind, what, masked) in look_at(added, debug) {
            found.push(Finding {
                path: file.clone(),
                line: Some(line_no),
                kind: kind.to_string(),
                what: what.to_string(),
                excerpt: Some(masked),
            });
        }
        line_no += 1;
        if found.len() >= MOST {
            break;
        }
    }
    found
}

/// Where the new side of a hunk starts, out of `-a,b +c,d @@`.
fn new_start(header: &str) -> Option<u32> {
    let plus = header
        .split_whitespace()
        .find(|part| part.starts_with('+'))?;
    plus[1..].split(',').next()?.parse().ok()
}

/// Undoes git's quoting of an unusual path: `"a\303\244.txt"` and the like.
fn unquote(name: &str) -> String {
    let Some(inner) = name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) else {
        return name.to_string();
    };
    let inner = inner.strip_prefix("b/").unwrap_or(inner);
    let mut bytes = Vec::new();
    let mut chars = inner.bytes().peekable();
    while let Some(byte) = chars.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        match chars.next() {
            Some(b'n') => bytes.push(b'\n'),
            Some(b't') => bytes.push(b'\t'),
            Some(digit @ b'0'..=b'7') => {
                let mut value = u32::from(digit - b'0');
                for _ in 0..2 {
                    if let Some(next @ b'0'..=b'7') = chars.peek().copied() {
                        value = value * 8 + u32::from(next - b'0');
                        chars.next();
                    }
                }
                bytes.push(value as u8);
            }
            Some(other) => bytes.push(other),
            None => {}
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Everything wrong with one added line: kind, what, and the line as shown.
fn look_at(line: &str, debug: bool) -> Vec<(&'static str, &'static str, String)> {
    let mut out = Vec::new();
    if line.starts_with("<<<<<<<") || line.starts_with(">>>>>>>") || line.starts_with("|||||||") {
        out.push(("conflict", "A conflict marker", shorten(line)));
    }
    if let Some((what, at, len)) = secret_in(line) {
        out.push(("secret", what, masked(line, at, len)));
    }
    if debug {
        if let Some(what) = leftover_in(line) {
            out.push(("debug", what, shorten(line)));
        }
    }
    out
}

fn shorten(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() > 120 {
        let cut: String = trimmed.chars().take(119).collect();
        format!("{cut}…")
    } else {
        trimmed.to_string()
    }
}

/// The line with all but the first four characters of the secret hidden, so
/// the warning itself does not put the key on screen in full.
fn masked(line: &str, at: usize, len: usize) -> String {
    let secret = &line[at..at + len];
    let shown: String = secret.chars().take(4).collect();
    shorten(&format!("{}{shown}…{}", &line[..at], &line[at + len..]))
}

/// A secret in the line, as what it is and where it sits (byte offsets).
fn secret_in(line: &str) -> Option<(&'static str, usize, usize)> {
    if line.contains("-----BEGIN")
        && (line.contains("PRIVATE KEY-----") || line.contains("PRIVATE KEY BLOCK-----"))
    {
        let at = line.find("-----BEGIN")?;
        return Some(("The start of a private key", at, line.len() - at));
    }
    for prefix in ["AKIA", "ASIA"] {
        if let Some(at) = token(line, prefix, 16, 16, |c| {
            c.is_ascii_uppercase() || c.is_ascii_digit()
        }) {
            return Some(("An AWS access key", at, 20));
        }
    }
    for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        if let Some(at) = token(line, prefix, 36, 255, |c| c.is_ascii_alphanumeric()) {
            let len = run_len(line, at, prefix.len(), |c| c.is_ascii_alphanumeric());
            return Some(("A GitHub token", at, len));
        }
    }
    if let Some(at) = token(line, "github_pat_", 22, 255, |c| {
        c.is_ascii_alphanumeric() || c == '_'
    }) {
        let len = run_len(line, at, 11, |c| c.is_ascii_alphanumeric() || c == '_');
        return Some(("A GitHub token", at, len));
    }
    if let Some(at) = token(line, "glpat-", 20, 255, |c| {
        c.is_ascii_alphanumeric() || c == '_' || c == '-'
    }) {
        let len = run_len(line, at, 6, |c| {
            c.is_ascii_alphanumeric() || c == '_' || c == '-'
        });
        return Some(("A GitLab token", at, len));
    }
    for prefix in ["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-"] {
        if let Some(at) = token(line, prefix, 10, 255, |c| {
            c.is_ascii_alphanumeric() || c == '-'
        }) {
            let len = run_len(line, at, prefix.len(), |c| {
                c.is_ascii_alphanumeric() || c == '-'
            });
            return Some(("A Slack token", at, len));
        }
    }
    for prefix in ["sk_live_", "rk_live_"] {
        if let Some(at) = token(line, prefix, 16, 255, |c| c.is_ascii_alphanumeric()) {
            let len = run_len(line, at, prefix.len(), |c| c.is_ascii_alphanumeric());
            return Some(("A live Stripe key", at, len));
        }
    }
    if let Some(at) = token(line, "AIza", 35, 35, |c| {
        c.is_ascii_alphanumeric() || c == '_' || c == '-'
    }) {
        return Some(("A Google API key", at, 39));
    }
    None
}

/// Where `prefix` starts a token of `min..=max` more characters that `body`
/// accepts, standing on its own rather than inside a longer word.
fn token(
    line: &str,
    prefix: &str,
    min: usize,
    max: usize,
    body: impl Fn(char) -> bool,
) -> Option<usize> {
    let mut from = 0;
    while let Some(found) = line[from..].find(prefix) {
        let at = from + found;
        from = at + prefix.len();
        let before = line[..at].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let len = run_len(line, at, prefix.len(), &body) - prefix.len();
        if len >= min && len <= max {
            return Some(at);
        }
    }
    None
}

/// How long the token at `at` runs: the prefix and every accepted character
/// after it.
fn run_len(line: &str, at: usize, prefix: usize, body: impl Fn(char) -> bool) -> usize {
    prefix
        + line[at + prefix..]
            .chars()
            .take_while(|c| body(*c))
            .map(char::len_utf8)
            .sum::<usize>()
}

/// A debugging call left in, by name.
fn leftover_in(line: &str) -> Option<&'static str> {
    let code = line.trim_start();
    // A comment that mentions one is not one.
    if code.starts_with("//") || code.starts_with('#') || code.starts_with('*') {
        return None;
    }
    let calls: [(&str, &str); 4] = [
        ("console.log(", "A console.log call"),
        ("dbg!(", "A dbg! call"),
        ("var_dump(", "A var_dump call"),
        ("dd(", "A dd() call"),
    ];
    for (call, what) in calls {
        let mut from = 0;
        while let Some(found) = line[from..].find(call) {
            let at = from + found;
            from = at + call.len();
            // `add(`, `$this->odd(` and `x.dd(` are other functions entirely.
            let before = line[..at].chars().next_back();
            let joined = before.is_some_and(|c| {
                c.is_alphanumeric()
                    || matches!(c, '_' | '$' | '>' | ':')
                    || (c == '.' && call != "console.log(")
            });
            if !joined {
                return Some(what);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(path: &str, added: &[&str]) -> String {
        let mut text = format!(
            "diff --git a/{path} b/{path}\nindex 1..2 100644\n--- a/{path}\n+++ b/{path}\n@@ -3,0 +4,{} @@\n",
            added.len()
        );
        for line in added {
            text.push('+');
            text.push_str(line);
            text.push('\n');
        }
        text
    }

    #[test]
    fn finds_a_conflict_marker_on_the_line_it_is_on() {
        let found = scan_patch(&patch("a.txt", &["fine", "<<<<<<< HEAD", "x"]), false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, "conflict");
        assert_eq!(found[0].line, Some(5));
    }

    #[test]
    fn a_line_of_equals_signs_alone_is_left_alone() {
        // A markdown heading underline looks exactly like the middle marker.
        assert!(scan_patch(&patch("README.md", &["Title", "======="]), false).is_empty());
    }

    #[test]
    fn finds_keys_and_masks_them() {
        let aws = "aws_key = \"AKIAIOSFODNN7EXAMPLE\"";
        let found = scan_patch(&patch("config.py", &[aws]), false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, "secret");
        let shown = found[0].excerpt.as_deref().unwrap();
        assert!(shown.contains("AKIA…"), "{shown}");
        assert!(!shown.contains("IOSFODNN7EXAMPLE"));

        let github = format!("token: ghp_{}", "a".repeat(36));
        assert_eq!(
            scan_patch(&patch("ci.yml", &[&github]), false)[0].what,
            "A GitHub token"
        );

        let key = "-----BEGIN OPENSSH PRIVATE KEY-----";
        assert_eq!(scan_patch(&patch("k", &[key]), false)[0].kind, "secret");
    }

    #[test]
    fn a_word_that_only_contains_a_prefix_is_not_a_key() {
        // Too short, and inside a longer word.
        let lines = [
            "let ghp_short = 1",
            "XAKIAIOSFODNN7EXAMPLE",
            "AKIA_not_a_key",
        ];
        assert!(scan_patch(&patch("a.rs", &lines), false).is_empty());
    }

    #[test]
    fn leftovers_are_only_looked_for_when_asked() {
        let lines = ["  console.log(x)", "dbg!(y);", "dd($z);", "var_dump($w);"];
        assert!(scan_patch(&patch("a.js", &lines), false).is_empty());
        assert_eq!(scan_patch(&patch("a.js", &lines), true).len(), 4);
    }

    #[test]
    fn functions_that_merely_end_the_same_way_are_not_leftovers() {
        let lines = [
            "add(1, 2)",
            "$this->dd(1)",
            "list.add(x)",
            "// console.log(old)",
        ];
        assert!(scan_patch(&patch("a.php", &lines), true).is_empty());
    }

    #[test]
    fn names_files_that_should_not_be_committed() {
        let paths: Vec<String> = [
            ".env",
            "app/.env.local",
            ".env.example",
            "keys/id_ed25519",
            "keys/id_ed25519.pub",
            "certs/server.pem",
            "src/main.rs",
        ]
        .iter()
        .map(|p| p.to_string())
        .collect();
        let found: Vec<String> = scan_paths(&paths).into_iter().map(|f| f.path).collect();
        assert_eq!(
            found,
            vec![
                ".env",
                "app/.env.local",
                "keys/id_ed25519",
                "certs/server.pem"
            ]
        );
    }

    #[test]
    fn a_crlf_line_is_read_without_its_carriage_return() {
        let text = patch("a.txt", &["<<<<<<< ours\r"]);
        assert_eq!(scan_patch(&text, false).len(), 1);
    }

    #[test]
    fn reads_a_quoted_path() {
        assert_eq!(unquote("\"b/a\\303\\244.txt\""), "aä.txt");
        assert_eq!(unquote("plain.txt"), "plain.txt");
    }
}
