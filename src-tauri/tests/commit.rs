//! The working tree to commit loop, against real repositories: staging hunks
//! and lines whatever the file's line endings and the user's diff settings,
//! unstaging before the first commit, hooks that stop a commit, and the look
//! over the staged diff before one.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use gitnoob_lib::state::AppState;
use gitnoob_lib::work::{self, HunkAction, Lines};
use gitnoob_lib::{diff, guard, refs};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A throwaway repository, removed when the test ends.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("gitnoob-commit-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let sandbox = Sandbox { root };
        sandbox.git(&["init", "-q", "-b", "main"]);
        sandbox.git(&["config", "user.name", "Test"]);
        sandbox.git(&["config", "user.email", "test@example.com"]);
        sandbox.git(&["config", "commit.gpgsign", "false"]);
        // Pinned so a CRLF written here stays CRLF in the blob, on every
        // platform — Git for Windows defaults autocrlf to true.
        sandbox.git(&["config", "core.autocrlf", "false"]);
        // Hooks only where a test puts them, whatever the machine has set.
        std::fs::create_dir_all(sandbox.root.join(".hooks")).unwrap();
        sandbox.git(&["config", "core.hooksPath", ".hooks"]);
        sandbox
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .expect("git should be on PATH");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn write(&self, path: &str, content: &str) {
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }

    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root.join(path)).unwrap()
    }

    fn commit(&self, path: &str, content: &str, message: &str) {
        self.write(path, content);
        self.git(&["add", "--all"]);
        self.git(&["commit", "-q", "-m", message]);
    }

    /// What the index holds for a path, byte for byte.
    fn staged(&self, path: &str) -> String {
        self.git(&["show", &format!(":{path}")])
    }

    /// Puts a hook in place. Git for Windows runs it through its own sh, so
    /// the executable bit only matters elsewhere.
    fn hook(&self, name: &str, body: &str) {
        let path = self.root.join(".hooks").join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn state(&self) -> AppState {
        gitnoob_lib::config::silence_keychain();
        let state = AppState::new(self.root.join(".gitnoob-config"));
        state.set_path(self.root.clone());
        state
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The added and removed lines of one side's hunks, as the window draws them.
fn changes(state: &AppState, path: &str, side: diff::Side) -> Vec<Vec<String>> {
    diff::working_file_diff(state, path, side)
        .unwrap()
        .hunks
        .iter()
        .map(|hunk| {
            hunk.lines
                .iter()
                .filter(|line| line.origin == '+' || line.origin == '-')
                .map(|line| {
                    format!(
                        "{}{}",
                        line.origin,
                        line.content.trim_end_matches(['\n', '\r'])
                    )
                })
                .collect()
        })
        .collect()
}

fn numbered(lines: std::ops::RangeInclusive<u32>, eol: &str) -> String {
    lines.map(|n| format!("line {n}{eol}")).collect()
}

// --- line endings ------------------------------------------------------------

#[test]
fn a_hunk_of_a_crlf_file_stages_with_its_line_endings() {
    let sandbox = Sandbox::new("crlf-hunk");
    let original = numbered(1..=30, "\r\n");
    sandbox.commit("a.txt", &original, "Base");
    let edited = original
        .replace("line 2\r\n", "line 2\r\nfirst\r\n")
        .replace("line 25\r\n", "line 25\r\nsecond\r\n");
    sandbox.write("a.txt", &edited);
    let state = sandbox.state();

    work::apply_hunk(&state, "a.txt", 0, HunkAction::Stage, None).unwrap();
    let staged = sandbox.staged("a.txt");
    assert!(
        staged.contains("line 2\r\nfirst\r\nline 3\r\n"),
        "{staged:?}"
    );
    assert!(!staged.contains("second"));

    // And out again, and the other one thrown away, both through a patch
    // that has to carry the `\r` to match.
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Unstage, None).unwrap();
    assert!(refs::status(&state).unwrap().staged.is_empty());
    work::apply_hunk(&state, "a.txt", 1, HunkAction::Discard, None).unwrap();
    let left = sandbox.read("a.txt");
    assert!(left.contains("first\r\n") && !left.contains("second"));
}

#[test]
fn picked_lines_of_a_crlf_file_stage_with_their_line_endings() {
    let sandbox = Sandbox::new("crlf-lines");
    let original = numbered(1..=10, "\r\n");
    sandbox.commit("a.txt", &original, "Base");
    sandbox.write(
        "a.txt",
        &original.replace("line 5\r\n", "LINE FIVE\r\nline 5 and a half\r\n"),
    );
    let state = sandbox.state();

    // The second added line only: it lands on line 6 of the new file.
    let picked = Lines {
        added: vec![6],
        removed: vec![],
    };
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Stage, Some(picked)).unwrap();
    let staged = sandbox.staged("a.txt");
    assert!(
        staged.contains("line 5\r\nline 5 and a half\r\nline 6\r\n"),
        "{staged:?}"
    );
    assert!(!staged.contains("LINE FIVE"));

    // Taken back out line by line, as the staged side numbers it.
    let back = Lines {
        added: vec![6],
        removed: vec![],
    };
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Unstage, Some(back)).unwrap();
    assert_eq!(sandbox.staged("a.txt"), original);
}

#[test]
fn a_file_with_no_newline_at_the_end_stages_by_hunk_and_by_line() {
    let sandbox = Sandbox::new("no-eol");
    sandbox.commit("a.txt", "one\ntwo\nthree", "Base");
    sandbox.write("a.txt", "one\ntwo\nTHREE\nfour");
    let state = sandbox.state();

    // Only the new last line, without the change above it.
    let picked = Lines {
        added: vec![4],
        removed: vec![3],
    };
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Stage, Some(picked)).unwrap();
    assert_eq!(sandbox.staged("a.txt"), "one\ntwo\nfour");

    work::apply_hunk(&state, "a.txt", 0, HunkAction::Unstage, None).unwrap();
    assert_eq!(sandbox.staged("a.txt"), "one\ntwo\nthree");
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Stage, None).unwrap();
    assert_eq!(sandbox.staged("a.txt"), "one\ntwo\nTHREE\nfour");
}

#[test]
fn a_crlf_file_with_no_newline_at_the_end_stages_whole() {
    let sandbox = Sandbox::new("crlf-no-eol");
    sandbox.commit("a.txt", "one\r\ntwo\r\nthree", "Base");
    sandbox.write("a.txt", "one\r\nTWO\r\nthree");
    let state = sandbox.state();
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Stage, None).unwrap();
    assert_eq!(sandbox.staged("a.txt"), "one\r\nTWO\r\nthree");
}

// --- the user's diff settings ------------------------------------------------

/// A file whose two edits sit seven lines apart: separate hunks with three
/// lines of context, one hunk to anything that widens or joins them.
fn two_close_edits(sandbox: &Sandbox) {
    let original = numbered(1..=40, "\n");
    sandbox.commit("a.txt", &original, "Base");
    sandbox.write(
        "a.txt",
        &original
            .replace("line 10\n", "line 10 changed\n")
            .replace("line 18\n", "line 18 changed\n"),
    );
}

/// Stages the first hunk the window shows under a setting, and checks that
/// exactly that hunk — and not its neighbour — is what went in.
fn first_hunk_stages_alone(setting: &[&str]) {
    let tag = setting[0].replace('.', "-");
    let sandbox = Sandbox::new(&tag);
    two_close_edits(&sandbox);
    sandbox.git(&[&["config"], setting].concat());
    let state = sandbox.state();

    let shown = changes(&state, "a.txt", diff::Side::Unstaged);
    assert_eq!(
        shown.len(),
        2,
        "the window shows two hunks under {setting:?}"
    );

    work::apply_hunk(&state, "a.txt", 0, HunkAction::Stage, None)
        .unwrap_or_else(|e| panic!("staging under {setting:?}: {e}"));
    assert_eq!(
        changes(&state, "a.txt", diff::Side::Staged),
        vec![shown[0].clone()],
        "under {setting:?}"
    );
    assert_eq!(
        changes(&state, "a.txt", diff::Side::Unstaged),
        vec![shown[1].clone()],
        "under {setting:?}"
    );

    // And back, through the index side's diff.
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Unstage, None)
        .unwrap_or_else(|e| panic!("unstaging under {setting:?}: {e}"));
    assert!(refs::status(&state).unwrap().staged.is_empty());
}

#[test]
fn hunk_staging_ignores_a_wider_context_setting() {
    first_hunk_stages_alone(&["diff.context", "10"]);
}

#[test]
fn hunk_staging_ignores_inter_hunk_context() {
    first_hunk_stages_alone(&["diff.interHunkContext", "20"]);
}

#[test]
fn hunk_staging_ignores_noprefix() {
    first_hunk_stages_alone(&["diff.noprefix", "true"]);
}

#[test]
fn hunk_staging_ignores_mnemonic_prefixes() {
    first_hunk_stages_alone(&["diff.mnemonicPrefix", "true"]);
}

#[test]
fn hunk_staging_ignores_custom_prefixes() {
    first_hunk_stages_alone(&["diff.srcPrefix", "left/"]);
}

#[test]
fn hunk_staging_ignores_forced_colour() {
    first_hunk_stages_alone(&["color.diff", "always"]);
}

#[test]
fn hunk_staging_ignores_forced_colour_everywhere() {
    first_hunk_stages_alone(&["color.ui", "always"]);
}

#[test]
fn hunk_staging_ignores_an_external_diff_tool() {
    // Anything that prints something other than a patch, like difftastic.
    first_hunk_stages_alone(&["diff.external", "echo"]);
}

#[test]
fn hunk_staging_ignores_diff_relative() {
    first_hunk_stages_alone(&["diff.relative", "true"]);
}

#[test]
fn hunk_staging_ignores_a_textconv_driver() {
    let sandbox = Sandbox::new("textconv");
    two_close_edits(&sandbox);
    sandbox.write(".git/info/attributes", "*.txt diff=shout\n");
    sandbox.git(&["config", "diff.shout.textconv", "tr a-z A-Z <"]);
    let state = sandbox.state();
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Stage, None).unwrap();
    assert!(sandbox.staged("a.txt").contains("line 10 changed\n"));
}

/// Two hunks the CLI's other algorithms cut differently from Myers: the
/// textbook case where patience and histogram line up the function bodies.
const BEFORE_ALGO: &str = "#include <stdio.h>\n\n// Frobs foo heartily\nint frobnitz(int foo)\n{\n    int i;\n    for(i = 0; i < 10; i++)\n    {\n        printf(\"Your answer is: \");\n        printf(\"%d\\n\", foo);\n    }\n}\n\nint fact(int n)\n{\n    if(n > 1)\n    {\n        return fact(n-1) * n;\n    }\n    return 1;\n}\n\nint main(int argc, char **argv)\n{\n    frobnitz(fact(10));\n}\n";
const AFTER_ALGO: &str = "#include <stdio.h>\n\nint fib(int n)\n{\n    if(n > 2)\n    {\n        return fib(n-1) + fib(n-2);\n    }\n    return 1;\n}\n\n// Frobs foo heartily\nint frobnitz(int foo)\n{\n    int i;\n    for(i = 0; i < 10; i++)\n    {\n        printf(\"%d\\n\", foo);\n    }\n}\n\nint main(int argc, char **argv)\n{\n    frobnitz(fib(10));\n}\n";

fn every_hunk_stages_as_shown(algorithm: &str) {
    let sandbox = Sandbox::new(&format!("algo-{algorithm}"));
    sandbox.commit("a.c", BEFORE_ALGO, "Base");
    sandbox.git(&["config", "diff.algorithm", algorithm]);
    sandbox.write("a.c", AFTER_ALGO);
    let state = sandbox.state();

    let shown = changes(&state, "a.c", diff::Side::Unstaged);
    for (at, hunk) in shown.iter().enumerate() {
        sandbox.git(&["reset", "-q", "--", "a.c"]);
        work::apply_hunk(&state, "a.c", at, HunkAction::Stage, None)
            .unwrap_or_else(|e| panic!("hunk {at} under {algorithm}: {e}"));
        let staged: Vec<String> = changes(&state, "a.c", diff::Side::Staged)
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(&staged, hunk, "hunk {at} under {algorithm}");
    }
}

#[test]
fn hunk_staging_ignores_the_histogram_algorithm() {
    every_hunk_stages_as_shown("histogram");
}

#[test]
fn hunk_staging_ignores_the_patience_algorithm() {
    every_hunk_stages_as_shown("patience");
}

#[test]
fn the_pinned_diff_is_a_plain_patch_whatever_the_config_says() {
    let sandbox = Sandbox::new("plain-diff");
    two_close_edits(&sandbox);
    for (key, value) in [
        ("diff.external", "echo"),
        ("diff.noprefix", "true"),
        ("color.ui", "always"),
        ("diff.context", "9"),
    ] {
        sandbox.git(&["config", key, value]);
    }
    let out = Command::new("git")
        .args(work::plain_diff())
        .current_dir(&sandbox.root)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("diff --git a/a.txt b/a.txt\n"), "{text}");
    assert!(text.contains("--- a/a.txt\n+++ b/a.txt\n"));
    assert!(!text.contains('\u{1b}'), "no colour codes");
    assert_eq!(text.matches("\n@@ ").count(), 2, "{text}");
}

// --- before the first commit ---------------------------------------------------

#[test]
fn unstaging_before_the_first_commit_keeps_the_file() {
    let sandbox = Sandbox::new("unborn-unstage");
    sandbox.write("a.txt", "one\n");
    sandbox.write("dir/b[1].txt", "two\n");
    sandbox.write("dir/b1.txt", "lookalike\n");
    sandbox.git(&["add", "--all"]);
    // Staged, then edited on top: the edit is the copy that stays.
    sandbox.write("a.txt", "one, edited\n");
    let state = sandbox.state();

    work::unstage(&state, &["a.txt".to_string(), "dir/b[1].txt".to_string()]).unwrap();
    let status = refs::status(&state).unwrap();
    let staged: Vec<&str> = status.staged.iter().map(|e| e.path.as_str()).collect();
    // Literal: the bracketed name did not take its lookalike with it.
    assert_eq!(staged, vec!["dir/b1.txt"]);
    assert_eq!(sandbox.read("a.txt"), "one, edited\n");
    assert_eq!(sandbox.read("dir/b[1].txt"), "two\n");
}

#[test]
fn unstaging_a_hunk_before_the_first_commit() {
    let sandbox = Sandbox::new("unborn-hunk");
    sandbox.write("a.txt", &numbered(1..=5, "\n"));
    sandbox.git(&["add", "a.txt"]);
    let state = sandbox.state();

    // Two of the five lines back out; the rest stay staged.
    let picked = Lines {
        added: vec![2, 4],
        removed: vec![],
    };
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Unstage, Some(picked)).unwrap();
    assert_eq!(sandbox.staged("a.txt"), "line 1\nline 3\nline 5\n");
    assert_eq!(sandbox.read("a.txt"), numbered(1..=5, "\n"));

    // The whole of what is left takes the file out of the index.
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Unstage, None).unwrap();
    assert!(refs::status(&state).unwrap().staged.is_empty());
    assert_eq!(sandbox.read("a.txt"), numbered(1..=5, "\n"));
}

#[test]
fn unstaging_some_lines_of_a_new_file_after_the_first_commit() {
    let sandbox = Sandbox::new("new-file-lines");
    sandbox.commit("base.txt", "base\n", "Base");
    sandbox.write("new.txt", &numbered(1..=4, "\r\n"));
    sandbox.git(&["add", "new.txt"]);
    let state = sandbox.state();
    let picked = Lines {
        added: vec![1],
        removed: vec![],
    };
    work::apply_hunk(&state, "new.txt", 0, HunkAction::Unstage, Some(picked)).unwrap();
    assert_eq!(sandbox.staged("new.txt"), numbered(2..=4, "\r\n"));
}

#[test]
fn staging_some_lines_of_a_deletion_keeps_the_rest() {
    let sandbox = Sandbox::new("delete-lines");
    sandbox.commit("a.txt", &numbered(1..=4, "\n"), "Base");
    std::fs::remove_file(sandbox.root.join("a.txt")).unwrap();
    let state = sandbox.state();
    let picked = Lines {
        added: vec![],
        removed: vec![2, 3],
    };
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Stage, Some(picked)).unwrap();
    assert_eq!(sandbox.staged("a.txt"), "line 1\nline 4\n");
}

#[test]
fn unstaging_on_a_branch_with_commits_still_uses_restore() {
    let sandbox = Sandbox::new("born-unstage");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("a.txt", "two\n");
    sandbox.git(&["add", "a.txt"]);
    let state = sandbox.state();
    work::unstage(&state, &["a.txt".to_string()]).unwrap();
    // Back to HEAD's copy in the index, not out of it.
    assert_eq!(sandbox.staged("a.txt"), "one\n");
    assert_eq!(sandbox.read("a.txt"), "two\n");
}

// --- hooks -------------------------------------------------------------------

#[test]
fn a_failing_hook_is_reported_with_its_output() {
    let sandbox = Sandbox::new("hook-fail");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.hook("pre-commit", "echo 'lint: 3 problems' >&2\nexit 1");
    sandbox.write("a.txt", "two\n");
    sandbox.git(&["add", "a.txt"]);
    let state = sandbox.state();

    let tried = work::commit_attempt(&state, "Change", false, false).unwrap();
    assert!(!tried.ok);
    assert!(
        tried.output.contains("lint: 3 problems"),
        "{}",
        tried.output
    );
    let hook = tried.hook.expect("a hook stopped it");
    assert_eq!(hook.hooks, vec!["pre-commit".to_string()]);
    assert!(hook.changed.is_empty());

    // Skipping the hooks, asked for explicitly, commits.
    let skipped = work::commit_attempt(&state, "Change", false, true).unwrap();
    assert!(skipped.ok, "{}", skipped.output);
    assert_eq!(sandbox.git(&["log", "-1", "--format=%s"]).trim(), "Change");
}

#[test]
fn a_formatter_hook_names_the_files_it_rewrote() {
    let sandbox = Sandbox::new("hook-format");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.commit("b.txt", "one\n", "Base b");
    sandbox.commit("c.txt", "one\n", "Base c");
    // Rewrites what it is given, then fails, the way formatters run from a
    // hook do.
    sandbox.hook(
        "pre-commit",
        "for f in a.txt b.txt; do echo formatted >> \"$f\"; done\nexit 1",
    );
    sandbox.write("a.txt", "two\n");
    sandbox.write("b.txt", "two\n");
    sandbox.write("c.txt", "two\n");
    sandbox.git(&["add", "a.txt", "b.txt", "c.txt"]);
    // b has an edit of the user's own that was never staged.
    sandbox.write("b.txt", "two\nmine, not staged\n");
    let state = sandbox.state();

    let tried = work::commit_attempt(&state, "Change", false, false).unwrap();
    assert!(!tried.ok);
    let hook = tried.hook.unwrap();
    assert_eq!(hook.changed, vec!["a.txt".to_string()]);
    assert_eq!(hook.mixed, vec!["b.txt".to_string()]);
    // c was staged but untouched by the hook.
}

#[test]
fn a_commit_with_nothing_staged_is_not_blamed_on_a_hook() {
    let sandbox = Sandbox::new("hook-innocent");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.hook("pre-commit", "exit 0");
    let state = sandbox.state();
    let tried = work::commit_attempt(&state, "Nothing", false, false).unwrap();
    assert!(!tried.ok);
    assert!(tried.hook.is_none(), "{}", tried.output);
}

#[test]
fn a_commit_msg_hook_counts_but_is_skipped_by_no_verify() {
    let sandbox = Sandbox::new("hook-msg");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.hook("commit-msg", "echo 'needs a ticket' >&2\nexit 1");
    sandbox.write("a.txt", "two\n");
    sandbox.git(&["add", "a.txt"]);
    let state = sandbox.state();
    let tried = work::commit_attempt(&state, "No ticket", false, false).unwrap();
    assert_eq!(tried.hook.unwrap().hooks, vec!["commit-msg".to_string()]);
    assert!(
        work::commit_attempt(&state, "No ticket", false, true)
            .unwrap()
            .ok
    );
}

#[test]
fn an_amend_goes_through_the_same_way() {
    let sandbox = Sandbox::new("amend-attempt");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("a.txt", "two\n");
    sandbox.git(&["add", "a.txt"]);
    let state = sandbox.state();
    let tried = work::commit_attempt(&state, "Base, amended", true, false).unwrap();
    assert!(tried.ok, "{}", tried.output);
    assert_eq!(sandbox.git(&["rev-list", "--count", "HEAD"]).trim(), "1");
    assert_eq!(sandbox.git(&["show", "HEAD:a.txt"]), "two\n");
}

// --- the look before committing --------------------------------------------------

#[test]
fn the_guard_finds_markers_keys_and_files_in_what_is_staged() {
    let sandbox = Sandbox::new("guard");
    sandbox.commit("a.txt", "one\n", "Base");
    // Hostile settings, to be sure the patch it reads is a plain one.
    sandbox.git(&["config", "diff.noprefix", "true"]);
    sandbox.git(&["config", "diff.external", "echo"]);
    sandbox.write("a.txt", "one\n<<<<<<< HEAD\n");
    sandbox.write(
        "config.py",
        "key = 'AKIAIOSFODNN7EXAMPLE'\nconsole.log(1)\n",
    );
    sandbox.write(".env", "SECRET=1\n");
    sandbox.write("unstaged.txt", "<<<<<<< not staged, not looked at\n");
    sandbox.git(&["add", "a.txt", "config.py", ".env"]);
    let state = sandbox.state();

    let found = guard::scan(&state, false).unwrap();
    let said: Vec<(String, Option<u32>, String)> = found
        .iter()
        .map(|f| (f.path.clone(), f.line, f.kind.clone()))
        .collect();
    assert!(
        said.contains(&(".env".to_string(), None, "file".to_string())),
        "{said:?}"
    );
    assert!(
        said.contains(&("a.txt".to_string(), Some(2), "conflict".to_string())),
        "{said:?}"
    );
    assert!(
        said.contains(&("config.py".to_string(), Some(1), "secret".to_string())),
        "{said:?}"
    );
    assert!(!said.iter().any(|(path, _, _)| path == "unstaged.txt"));
    assert!(!said.iter().any(|(_, _, kind)| kind == "debug"));

    let with_debug = guard::scan(&state, true).unwrap();
    assert!(with_debug
        .iter()
        .any(|f| f.kind == "debug" && f.path == "config.py" && f.line == Some(2)));
}

#[test]
fn the_guard_has_nothing_to_say_about_an_ordinary_change() {
    let sandbox = Sandbox::new("guard-clean");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("a.txt", "one\ntwo\n");
    sandbox.git(&["add", "a.txt"]);
    let state = sandbox.state();
    assert!(guard::scan(&state, true).unwrap().is_empty());
}

#[test]
fn the_guard_reads_before_the_first_commit() {
    let sandbox = Sandbox::new("guard-unborn");
    sandbox.write("id_rsa", "-----BEGIN RSA PRIVATE KEY-----\n");
    sandbox.git(&["add", "id_rsa"]);
    let state = sandbox.state();
    let found = guard::scan(&state, false).unwrap();
    assert!(found.iter().any(|f| f.kind == "file"));
    assert!(found.iter().any(|f| f.kind == "secret"));
}

// --- picking lines next to a missing final newline -----------------------------------

/// One line of a single-hunk diff: its sign, its text, the number that names it
/// (where it lands for `+`, where it was for `-`) and whether the next line of
/// the diff says it has no newline.
struct DiffLine {
    sign: char,
    text: String,
    number: u32,
    bare: bool,
}

fn one_hunk(diff: &str) -> Vec<DiffLine> {
    let mut out: Vec<DiffLine> = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    let mut started = false;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("@@ -") {
            assert!(!started, "one hunk only:\n{diff}");
            started = true;
            let (a, b) = rest.split_once(" +").unwrap();
            old = a.split(',').next().unwrap().parse().unwrap();
            new = b.split([',', ' ']).next().unwrap().parse().unwrap();
            continue;
        }
        if !started {
            continue;
        }
        if line.starts_with('\\') {
            out.last_mut().unwrap().bare = true;
            continue;
        }
        let sign = line.chars().next().unwrap_or(' ');
        let number = match sign {
            '+' => new,
            '-' => old,
            _ => 0,
        };
        match sign {
            '+' => new += 1,
            '-' => old += 1,
            _ => {
                old += 1;
                new += 1;
            }
        }
        out.push(DiffLine {
            sign,
            text: line[1..].to_string(),
            number,
            bare: false,
        });
    }
    out
}

/// What a file should read after the picked changes: forwards (staging) an
/// unpicked `-` stays and an unpicked `+` never happened; backwards (unstaging,
/// discarding) the picked ones are undone and the rest stay. Every line ends in
/// a newline except a last one that had none where it came from.
fn expected(lines: &[DiffLine], picked: &[bool], forwards: bool) -> String {
    let mut kept: Vec<(&str, bool)> = Vec::new();
    let mut change = 0;
    for line in lines {
        let keep = match line.sign {
            ' ' => true,
            sign => {
                let on = picked[change];
                change += 1;
                if forwards {
                    (sign == '-') != on
                } else {
                    (sign == '+') != on
                }
            }
        };
        if keep {
            kept.push((&line.text, line.bare));
        }
    }
    let mut text = String::new();
    for (at, (line, bare)) in kept.iter().enumerate() {
        text.push_str(line);
        if !(at + 1 == kept.len() && *bare) {
            text.push('\n');
        }
    }
    text
}

fn picks(lines: &[DiffLine], picked: &[bool]) -> Lines {
    let mut out = Lines::default();
    for (line, on) in lines.iter().filter(|line| line.sign != ' ').zip(picked) {
        if *on {
            match line.sign {
                '+' => out.added.push(line.number),
                _ => out.removed.push(line.number),
            }
        }
    }
    out
}

#[test]
fn every_pick_of_lines_beside_a_missing_final_newline_comes_out_right() {
    every_pick_beside_a_missing_final_newline(false);
}

/// The same picks on a Windows checkout: the blobs hold bare newlines and the
/// working file CRLF, so the patch and the file it applies to differ in their
/// line endings until git converts one into the other.
#[test]
fn every_pick_beside_a_missing_final_newline_comes_out_right_under_autocrlf() {
    every_pick_beside_a_missing_final_newline(true);
}

fn every_pick_beside_a_missing_final_newline(autocrlf: bool) {
    let crlf = |text: &str| {
        if autocrlf {
            text.replace('\n', "\r\n")
        } else {
            text.to_string()
        }
    };
    let cases = [
        ("a\nb", "a\nb\nc\n"),
        ("one\ntwo\nthree", "one\ntwo\nTHREE\nfour"),
        ("a\nb", "a\nc"),
        ("a\nb\n", "a\nb\nc"),
        ("a\nb", "a\nB\nc"),
        ("x", "y\nz"),
        ("a\nb\nc", "a\nb"),
        ("a\nb", "X\na\nb\nc"),
        ("a\nb\nc", "a\nB\nc\nd\ne"),
        ("a\nb", "a\nb\n\nc"),
        ("a\nb\n", "a\nB"),
    ];
    let sandbox = Sandbox::new("bare-picks");
    let state = sandbox.state();
    for (base, edited) in cases {
        sandbox.commit("f.txt", base, "Base");
        if autocrlf {
            sandbox.git(&["config", "core.autocrlf", "true"]);
        }
        let edited = &crlf(edited);
        sandbox.write("f.txt", edited);
        let lines = one_hunk(&sandbox.git(&["diff", "--no-color", "--", "f.txt"]));
        let changes = lines.iter().filter(|line| line.sign != ' ').count();
        for mask in 1..(1u32 << changes) {
            let picked: Vec<bool> = (0..changes).map(|bit| mask & (1 << bit) != 0).collect();
            let chosen = || Some(picks(&lines, &picked));
            let case = format!("{base:?} -> {edited:?}, picked {picked:?}");

            // Staging: the index moves forwards from the base.
            let said = work::apply_hunk(&state, "f.txt", 0, HunkAction::Stage, chosen());
            {
                said.unwrap_or_else(|error| panic!("{case}: {error}"));
                assert_eq!(
                    sandbox.staged("f.txt"),
                    expected(&lines, &picked, true),
                    "stage {case}"
                );
            }
            sandbox.git(&["reset", "-q", "--", "f.txt"]);

            // Unstaging: the whole change staged, then the picks taken back.
            sandbox.git(&["add", "f.txt"]);
            let said = work::apply_hunk(&state, "f.txt", 0, HunkAction::Unstage, chosen());
            {
                said.unwrap_or_else(|error| panic!("{case}: {error}"));
                assert_eq!(
                    sandbox.staged("f.txt"),
                    expected(&lines, &picked, false),
                    "unstage {case}"
                );
            }
            sandbox.git(&["reset", "-q", "--", "f.txt"]);

            // Discarding: the working file loses the picks.
            let said = work::apply_hunk(&state, "f.txt", 0, HunkAction::Discard, chosen());
            {
                said.unwrap_or_else(|error| panic!("{case}: {error}"));
                assert_eq!(
                    sandbox.read("f.txt"),
                    crlf(&expected(&lines, &picked, false)),
                    "discard {case}"
                );
            }
            sandbox.write("f.txt", edited);
        }
        sandbox.git(&["config", "core.autocrlf", "false"]);
    }
}

// --- a tab switched while a hook runs ---------------------------------------------------

#[test]
fn a_commit_whose_hook_outlasts_a_tab_switch_is_recorded_where_it_was_made() {
    let a = Sandbox::new("slow-hook-a");
    let b = Sandbox::new("slow-hook-b");
    a.commit("a.txt", "one\n", "Base A");
    b.commit("b.txt", "one\n", "Base B");
    a.hook("pre-commit", "sleep 1");
    a.write("a.txt", "two\n");
    a.git(&["add", "a.txt"]);
    // The app's own state, whose path follows the tab in front.
    let state = std::sync::Arc::new(a.state());
    let running = {
        let state = state.clone();
        std::thread::spawn(move || work::commit_attempt(&state, "A change", false, false))
    };
    std::thread::sleep(std::time::Duration::from_millis(400));
    state.set_path(b.root.clone());
    assert!(running.join().unwrap().unwrap().ok);

    let undo = gitnoob_lib::journal::stacks(&state).undo;
    assert!(undo.is_empty(), "nothing recorded in B: {undo:?}");
    state.set_path(a.root.clone());
    let undo = gitnoob_lib::journal::stacks(&state).undo;
    assert_eq!(
        undo.first().map(|e| e.label.as_str()),
        Some("Commit: A change")
    );
}

// --- stage everything, then commit ---------------------------------------------------

#[test]
fn stage_all_for_a_commit_refuses_when_something_was_staged_unseen() {
    let sandbox = Sandbox::new("stage-all-unseen");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("a.txt", "two\n");
    sandbox.write("b.txt", "new\n");
    let state = sandbox.state();

    // A `git add -p` in a terminal, after the list was read.
    sandbox.git(&["add", "b.txt"]);
    assert!(work::stage_all_expecting(&state, Some(false)).is_err());
    assert_eq!(
        sandbox.git(&["diff", "--cached", "--name-only"]).trim(),
        "b.txt",
        "nothing more was staged"
    );

    // The window saw it staged: then it is what the user means.
    work::stage_all_expecting(&state, Some(true)).unwrap();
    assert_eq!(sandbox.staged("a.txt"), "two\n");

    // With nothing staged at all it stages the lot.
    sandbox.git(&["reset", "-q"]);
    work::stage_all_expecting(&state, Some(false)).unwrap();
    assert_eq!(sandbox.staged("b.txt"), "new\n");
}

#[test]
fn a_commit_with_no_message_is_not_blamed_on_a_hook() {
    let sandbox = Sandbox::new("no-message");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.hook("pre-commit", "exit 0");
    sandbox.write("a.txt", "two\n");
    sandbox.git(&["add", "a.txt"]);
    let attempt = work::commit_attempt(&sandbox.state(), "  ", false, false).unwrap();
    assert!(!attempt.ok);
    assert!(attempt.hook.is_none(), "{attempt:?}");
}

// --- picking lines under core.autocrlf ------------------------------------------------

/// A Windows checkout: the blob holds bare newlines, the working file CRLF.
fn autocrlf_sandbox(tag: &str, setting: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("a.txt", &numbered(1..=10, "\n"), "Base");
    sandbox.git(&["config", "core.autocrlf", setting]);
    // Checked out again so the working file is what that setting writes.
    std::fs::remove_file(sandbox.root.join("a.txt")).unwrap();
    sandbox.git(&["checkout", "--", "a.txt"]);
    sandbox
}

#[test]
fn picked_lines_stage_unstage_and_discard_under_autocrlf() {
    let sandbox = autocrlf_sandbox("autocrlf-lines", "true");
    let original = numbered(1..=10, "\r\n");
    assert_eq!(sandbox.read("a.txt"), original);
    let edited = original
        .replace("line 3\r\n", "THREE\r\n")
        .replace("line 5\r\n", "line 5\r\nextra\r\n");
    sandbox.write("a.txt", &edited);
    let state = sandbox.state();

    // Only the added line after line 5: the edit of line 3 stays out.
    let lines = changes(&state, "a.txt", diff::Side::Unstaged);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let picked = Lines {
        added: vec![6],
        removed: vec![],
    };
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Stage, Some(picked)).unwrap();
    assert_eq!(
        sandbox.staged("a.txt"),
        numbered(1..=10, "\n").replace("line 5\n", "line 5\nextra\n")
    );
    // The working file was not touched.
    assert_eq!(sandbox.read("a.txt"), edited);

    // And taken back out again by line.
    let back = Lines {
        added: vec![6],
        removed: vec![],
    };
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Unstage, Some(back)).unwrap();
    assert_eq!(sandbox.staged("a.txt"), numbered(1..=10, "\n"));

    // Discarding only the edit of line 3 keeps the rest, with CRLF endings.
    let gone = Lines {
        added: vec![3],
        removed: vec![3],
    };
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Discard, Some(gone)).unwrap();
    assert_eq!(
        sandbox.read("a.txt"),
        original.replace("line 5\r\n", "line 5\r\nextra\r\n")
    );
}

#[test]
fn a_whole_hunk_discards_under_autocrlf_with_crlf_left_in_place() {
    let sandbox = autocrlf_sandbox("autocrlf-hunk", "true");
    let original = numbered(1..=10, "\r\n");
    sandbox.write("a.txt", &original.replace("line 5\r\n", "FIVE\r\n"));
    let state = sandbox.state();
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Stage, None).unwrap();
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Unstage, None).unwrap();
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Discard, None).unwrap();
    assert_eq!(sandbox.read("a.txt"), original);
}

/// A file committed with CRLF in the blob, in a clone that turned autocrlf on
/// later: git leaves such a file's endings alone, and so must picking lines.
#[test]
fn picked_lines_of_a_crlf_blob_under_autocrlf_keep_their_endings() {
    let sandbox = Sandbox::new("autocrlf-crlf-blob");
    let original = numbered(1..=10, "\r\n");
    sandbox.commit("a.txt", &original, "Base");
    sandbox.git(&["config", "core.autocrlf", "true"]);
    let edited = original
        .replace("line 3\r\n", "THREE\r\n")
        .replace("line 5\r\n", "line 5\r\nextra\r\n");
    sandbox.write("a.txt", &edited);
    let state = sandbox.state();

    let picked = Lines {
        added: vec![6],
        removed: vec![],
    };
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Stage, Some(picked.clone())).unwrap();
    assert_eq!(
        sandbox.staged("a.txt"),
        original.replace("line 5\r\n", "line 5\r\nextra\r\n")
    );
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Unstage, Some(picked)).unwrap();
    assert_eq!(sandbox.staged("a.txt"), original);

    let gone = Lines {
        added: vec![3],
        removed: vec![3],
    };
    work::apply_hunk(&state, "a.txt", 0, HunkAction::Discard, Some(gone)).unwrap();
    assert_eq!(
        sandbox.read("a.txt"),
        original.replace("line 5\r\n", "line 5\r\nextra\r\n")
    );
}
