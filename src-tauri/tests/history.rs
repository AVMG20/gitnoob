//! Conflicts, runs of picks, and rewriting history, against real repositories.
//!
//! The literal-path checks, the cherry-pick sequencer, rerere, stacked
//! branches, fixups, absorb and split all live here, so each can be read next
//! to the repository shape it was written for.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use gitnoob_lib::state::AppState;
use gitnoob_lib::{conflict, rebase, remote, rewrite, work};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A throwaway repository, removed when the test ends.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("gitnoob-history-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let sandbox = Sandbox { root };
        sandbox.git(&["init", "-q", "-b", "main"]);
        sandbox.git(&["config", "user.name", "Test"]);
        sandbox.git(&["config", "user.email", "test@example.com"]);
        sandbox.git(&["config", "commit.gpgsign", "false"]);
        sandbox.git(&["config", "core.autocrlf", "false"]);
        sandbox.git(&["config", "core.eol", "lf"]);
        // The app's own settings live beside the repository in these tests;
        // ignoring them keeps them out of every `add --all`.
        std::fs::write(sandbox.root.join(".git/info/exclude"), ".gitnoob-config/\n").unwrap();
        sandbox
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_EDITOR", "true")
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

    fn git_may_fail(&self, args: &[&str]) -> bool {
        Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_EDITOR", "true")
            .output()
            .expect("git should be on PATH")
            .status
            .success()
    }

    fn write(&self, path: &str, content: &str) {
        std::fs::write(self.root.join(path), content).unwrap();
    }

    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root.join(path)).unwrap()
    }

    fn commit(&self, path: &str, content: &str, message: &str) {
        self.write(path, content);
        self.git(&["add", "--all"]);
        self.git(&["commit", "-q", "-m", message]);
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    fn subjects(&self) -> Vec<String> {
        self.git(&["log", "--format=%s", "--reverse"])
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn oids(&self) -> Vec<String> {
        self.git(&["log", "--format=%H", "--reverse"])
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn staged(&self) -> Vec<String> {
        self.git(&["diff", "--cached", "--name-only"])
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn unstaged(&self) -> Vec<String> {
        self.git(&["diff", "--name-only"])
            .lines()
            .map(str::to_string)
            .collect()
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

/// Stands in for the app's own binary in the sequence-editor slot: the test
/// binary is not gitnoob, and `--write-todo` is a copy and nothing more.
fn lend_git_an_editor() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| std::env::set_var("GITNOOB_SEQUENCE_EDITOR", "cp"));
}

// --- literal paths ------------------------------------------------------------

/// `a[b].txt` conflicted, beside `ab.txt` with an edit nobody has staged.
///
/// A pathspec `a[b].txt` matches `ab.txt`, so every resolve that passed the
/// path bare checked out, staged or deleted the neighbour along with it.
fn bracket_conflict(tag: &str, theirs_deletes: bool) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.write("ab.txt", "neighbour\n");
    sandbox.commit("a[b].txt", "base\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    if theirs_deletes {
        sandbox.git(&["rm", "-q", "--", ":(literal)a[b].txt"]);
        sandbox.git(&["commit", "-q", "-m", "Delete it"]);
    } else {
        sandbox.commit("a[b].txt", "theirs\n", "Theirs");
    }
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a[b].txt", "ours\n", "Ours");
    sandbox.write("ab.txt", "an edit nobody staged\n");
    assert!(
        !sandbox.git_may_fail(&["merge", "side"]),
        "it should conflict"
    );
    sandbox
}

fn neighbour_untouched(sandbox: &Sandbox) {
    assert_eq!(sandbox.read("ab.txt"), "an edit nobody staged\n");
    assert!(
        !sandbox.staged().contains(&"ab.txt".to_string()),
        "ab.txt was staged"
    );
    assert!(sandbox.unstaged().contains(&"ab.txt".to_string()));
}

#[test]
fn reading_the_stages_of_a_bracketed_name_reads_that_file_only() {
    let sandbox = bracket_conflict("literal-stages", false);
    let state = sandbox.state();
    let found = conflict::stages(&state, "a[b].txt").unwrap();
    assert!(found.base && found.ours && found.theirs);
    neighbour_untouched(&sandbox);
}

#[test]
fn resolving_regions_leaves_the_neighbour_alone() {
    let sandbox = bracket_conflict("literal-resolve", false);
    let state = sandbox.state();
    let choice: conflict::Resolution =
        serde_json::from_str(r#"{"take_ours": false, "take_theirs": true}"#).unwrap();
    conflict::resolve(&state, "a[b].txt", &[choice]).unwrap();
    assert_eq!(sandbox.read("a[b].txt"), "theirs\n");
    neighbour_untouched(&sandbox);
}

#[test]
fn taking_a_whole_side_leaves_the_neighbour_alone() {
    for side in ["ours", "theirs"] {
        let sandbox = bracket_conflict(&format!("literal-whole-{side}"), false);
        let state = sandbox.state();
        conflict::resolve_whole(&state, "a[b].txt", side).unwrap();
        assert_eq!(sandbox.read("a[b].txt"), format!("{side}\n"));
        assert!(conflict::list(&state).unwrap().is_empty());
        neighbour_untouched(&sandbox);
    }
}

#[test]
fn taking_the_side_that_deleted_it_deletes_only_that_file() {
    let sandbox = bracket_conflict("literal-delete", true);
    let state = sandbox.state();
    conflict::resolve_whole(&state, "a[b].txt", "theirs").unwrap();
    assert!(!sandbox.root.join("a[b].txt").exists());
    assert!(
        sandbox.root.join("ab.txt").exists(),
        "rm -f took the neighbour"
    );
    neighbour_untouched(&sandbox);
}

#[test]
fn keeping_a_file_as_it_stands_stages_that_file_only() {
    let sandbox = bracket_conflict("literal-as-is", false);
    let state = sandbox.state();
    sandbox.write("a[b].txt", "settled by hand\n");
    conflict::resolve_as_is(&state, "a[b].txt").unwrap();
    assert!(sandbox.staged().contains(&"a[b].txt".to_string()));
    neighbour_untouched(&sandbox);

    // And the other way out of it: a file that is gone is removed, alone.
    let sandbox = bracket_conflict("literal-as-is-gone", false);
    let state = sandbox.state();
    std::fs::remove_file(sandbox.root.join("a[b].txt")).unwrap();
    conflict::resolve_as_is(&state, "a[b].txt").unwrap();
    assert!(sandbox.root.join("ab.txt").exists());
    neighbour_untouched(&sandbox);
}

// --- trivial regions and lockfiles --------------------------------------------

#[test]
fn a_region_that_differs_only_at_the_ends_of_lines_is_called_whitespace() {
    let sandbox = Sandbox::new("trivial");
    sandbox.commit("a.txt", "one\nfn call(a, b)\nthree\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("a.txt", "one\nfn call(a, b)  \nthree\n", "Trailing spaces");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "one\nfn call(a, b)\t\nthree\n", "Trailing tab");
    assert!(!sandbox.git_may_fail(&["merge", "side"]));

    let file = conflict::read(&sandbox.state(), "a.txt").unwrap();
    let json = serde_json::to_value(&file).unwrap();
    let region = json["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["kind"] == "conflict")
        .expect("one region");
    assert_eq!(region["trivial"], "whitespace");
    assert_eq!(json["lockfile"], false);
    // Nothing is staged on the strength of it: the user still confirms.
    assert_eq!(conflict::list(&sandbox.state()).unwrap(), vec!["a.txt"]);
}

#[test]
fn trivial_tells_identical_from_spacing_from_different() {
    let lines = |text: &str| text.lines().map(str::to_string).collect::<Vec<_>>();
    assert_eq!(
        conflict::trivial(&lines("a\nb"), &lines("a\nb")),
        Some("same")
    );
    assert_eq!(
        conflict::trivial(&lines("a b  \nc\r"), &lines("a b\nc")),
        Some("whitespace")
    );
    assert_eq!(conflict::trivial(&lines("a b"), &lines("ab")), None);
    assert_eq!(conflict::trivial(&lines("x"), &lines("y")), None);
    // Indentation means something in Python, YAML and a Makefile; spaces in
    // a string are the string; a blank line can end a block.
    assert_eq!(
        conflict::trivial(&lines("if x:\n    y()"), &lines("if x:\ny()")),
        None
    );
    assert_eq!(
        conflict::trivial(&lines("s = \"a  b\""), &lines("s = \"a b\"")),
        None
    );
    assert_eq!(conflict::trivial(&lines("a\n\nb"), &lines("a\nb")), None);
}

#[test]
fn a_conflicted_lockfile_says_so() {
    let sandbox = Sandbox::new("lockfile");
    sandbox.commit("Cargo.lock", "version = 1\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("Cargo.lock", "version = 2\n", "Theirs");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("Cargo.lock", "version = 3\n", "Ours");
    assert!(!sandbox.git_may_fail(&["merge", "side"]));
    let file = conflict::read(&sandbox.state(), "Cargo.lock").unwrap();
    assert!(file.lockfile);
}

// --- the base pane for every kind of stop --------------------------------------

#[test]
fn a_cherry_pick_that_stops_writes_the_base_into_the_markers() {
    let sandbox = Sandbox::new("pick-diff3");
    sandbox.commit("a.txt", "base\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("a.txt", "theirs\n", "Theirs");
    let theirs = sandbox.head();
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "ours\n", "Ours");

    let state = sandbox.state();
    let said = work::cherry_pick(&state, &[theirs], work::CherryPickOptions::default()).unwrap();
    assert!(said.contains("conflicts"), "{said}");
    let file = serde_json::to_value(conflict::read(&state, "a.txt").unwrap()).unwrap();
    let region = file["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["kind"] == "conflict")
        .unwrap()
        .clone();
    assert_eq!(region["has_base"], true);
    assert_eq!(region["base"][0], "base");
}

#[test]
fn a_stash_that_will_not_go_back_on_writes_the_base_too() {
    let sandbox = Sandbox::new("stash-diff3");
    sandbox.commit("a.txt", "base\n", "First");
    sandbox.write("a.txt", "stashed\n");
    sandbox.git(&["stash", "push", "-q"]);
    sandbox.commit("a.txt", "committed\n", "Moved on");
    let state = sandbox.state();
    let _ = work::stash_apply(&state, 0);
    assert!(
        sandbox.read("a.txt").contains("|||||||"),
        "{}",
        sandbox.read("a.txt")
    );
}

// --- runs of picks ------------------------------------------------------------

/// Three commits on `side`, the second of which clashes with main.
fn three_picks(tag: &str) -> (Sandbox, Vec<String>) {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("f.txt", "base\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("one.txt", "one\n", "One");
    sandbox.commit("f.txt", "side\n", "Two");
    sandbox.commit("three.txt", "three\n", "Three");
    let picks: Vec<String> = sandbox
        .git(&["rev-list", "--reverse", "main..side"])
        .lines()
        .map(str::to_string)
        .collect();
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("f.txt", "main\n", "Main");
    (sandbox, picks)
}

#[test]
fn a_run_of_picks_that_stopped_carries_on_to_the_end() {
    let (sandbox, picks) = three_picks("seq-continue");
    let state = sandbox.state();
    let said = work::cherry_pick(&state, &picks, work::CherryPickOptions::default()).unwrap();
    assert!(said.contains("conflicts"), "{said}");

    let now = remote::in_progress(&state).unwrap();
    assert_eq!(now.sequencer.as_deref(), Some("cherry-pick"));
    assert_eq!(now.sequence_left, 2);
    assert!(now.cherry_picking);

    conflict::resolve_whole(&state, "f.txt", "theirs").unwrap();
    let done = work::sequence_continue(&state).unwrap();
    assert!(done.finished, "{}", done.message);
    assert_eq!(
        sandbox.subjects(),
        vec!["First", "Main", "One", "Two", "Three"]
    );
    assert!(!sandbox.root.join(".git/sequencer").exists());
    assert!(remote::in_progress(&state).unwrap().sequencer.is_none());
}

#[test]
fn committing_the_stopped_pick_by_hand_still_leaves_the_rest_to_continue() {
    let (sandbox, picks) = three_picks("seq-by-hand");
    let state = sandbox.state();
    work::cherry_pick(&state, &picks, work::CherryPickOptions::default()).unwrap();
    conflict::resolve_whole(&state, "f.txt", "theirs").unwrap();
    work::commit(&state, "Two, by hand", false).unwrap();

    // The commit cleared the conflict, not the list: the third pick is still
    // waiting, and the window has to say so.
    let now = remote::in_progress(&state).unwrap();
    assert_eq!(now.sequencer.as_deref(), Some("cherry-pick"));
    assert!(!sandbox.root.join(".git/CHERRY_PICK_HEAD").exists());

    // And starting another run is refused rather than failing on git's words.
    let refused = work::cherry_pick(&state, &picks[..1], work::CherryPickOptions::default());
    assert!(refused.unwrap_err().contains("part-way"));

    let done = work::sequence_continue(&state).unwrap();
    assert!(done.finished, "{}", done.message);
    assert_eq!(
        sandbox.subjects(),
        vec!["First", "Main", "One", "Two, by hand", "Three"]
    );
}

#[test]
fn skipping_a_stopped_pick_applies_the_rest_without_it() {
    let (sandbox, picks) = three_picks("seq-skip");
    let state = sandbox.state();
    work::cherry_pick(&state, &picks, work::CherryPickOptions::default()).unwrap();
    let done = work::sequence_skip(&state).unwrap();
    assert!(done.finished, "{}", done.message);
    assert_eq!(sandbox.subjects(), vec!["First", "Main", "One", "Three"]);
}

#[test]
fn aborting_a_run_of_picks_goes_back_to_the_start_and_keeps_open_work() {
    let (sandbox, picks) = three_picks("seq-abort");
    let start = sandbox.head();
    sandbox.write("notes.txt", "mine, not committed\n");
    sandbox.commit("tracked.txt", "v1\n", "Tracked");
    let start_with_tracked = sandbox.head();
    assert_ne!(start, start_with_tracked);
    sandbox.write("tracked.txt", "v2, not committed\n");

    let state = sandbox.state();
    work::cherry_pick(&state, &picks, work::CherryPickOptions::default()).unwrap();
    let done = work::sequence_abort(&state).unwrap();
    assert!(done.finished);
    assert!(done.message.contains("as it was"), "{}", done.message);
    assert_eq!(sandbox.head(), start_with_tracked);
    assert_eq!(sandbox.read("tracked.txt"), "v2, not committed\n");
    assert_eq!(sandbox.read("notes.txt"), "mine, not committed\n");
    assert!(!sandbox.root.join(".git/sequencer").exists());
    assert!(conflict::list(&state).unwrap().is_empty());
}

#[test]
fn a_run_of_reverts_that_stopped_carries_on_the_same_way() {
    let sandbox = Sandbox::new("seq-revert");
    sandbox.commit("f.txt", "one\n", "First");
    sandbox.commit("g.txt", "g\n", "Add g");
    sandbox.commit("f.txt", "two\n", "Change f");
    sandbox.commit("f.txt", "three\n", "Change f again");
    let ids = sandbox.oids();
    // Reverting "Change f" before "Change f again" clashes; "Add g" does not.
    assert!(!sandbox.git_may_fail(&["revert", "--no-edit", &ids[2], &ids[1]]));

    let state = sandbox.state();
    let now = remote::in_progress(&state).unwrap();
    assert_eq!(now.sequencer.as_deref(), Some("revert"));
    assert!(now.reverting);

    conflict::resolve_whole(&state, "f.txt", "theirs").unwrap();
    let done = work::sequence_continue(&state).unwrap();
    assert!(done.finished, "{}", done.message);
    assert!(
        !sandbox.root.join("g.txt").exists(),
        "the second revert was applied"
    );
}

// --- the plan -----------------------------------------------------------------

#[test]
fn a_plan_over_a_merge_is_refused_rather_than_flattened() {
    let sandbox = Sandbox::new("plan-merge");
    sandbox.commit("a.txt", "one\n", "First");
    let base = sandbox.head();
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("b.txt", "side\n", "Side");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("c.txt", "main\n", "Main");
    sandbox.git(&["merge", "-q", "--no-ff", "-m", "Merge side", "side"]);
    sandbox.commit("d.txt", "after\n", "After");

    let state = sandbox.state();
    let refused = rebase::plan(&state, &base).unwrap_err();
    assert!(refused.contains("Merge side"), "{refused}");
    assert!(refused.contains("merge"), "{refused}");

    // Starting it anyway is refused too, and nothing moves.
    lend_git_an_editor();
    let head = sandbox.head();
    let steps: Vec<rebase::Step> =
        serde_json::from_str(&format!(r#"[{{"oid": "{head}", "action": "pick"}}]"#)).unwrap();
    assert!(rebase::start(&state, &base, steps).is_err());
    assert_eq!(sandbox.head(), head);

    // Above the merge is a straight line again.
    let merge = sandbox.git(&["rev-parse", "HEAD^"]).trim().to_string();
    let plan = rebase::plan(&state, &merge).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].summary, "After");
}

#[test]
fn a_commit_pushed_long_ago_still_reads_as_pushed() {
    let sandbox = Sandbox::new("plan-pushed-old");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("b.txt", "two\n", "Pushed long ago");
    let pushed = sandbox.head();
    sandbox.commit("c.txt", "three\n", "Not pushed");
    let base = sandbox.oids()[0].clone();
    sandbox.git(&["update-ref", "refs/remotes/origin/main", &pushed]);

    // Another remote branch with far more than two thousand newer commits on
    // it, which is what pushed the old one out of the window it was read from.
    let mut stream = String::new();
    for at in 0..2100 {
        stream.push_str("commit refs/remotes/origin/busy\n");
        stream.push_str(&format!(
            "committer Bot <bot@example.com> {} +0000\n",
            2_000_000_000 + at
        ));
        stream.push_str("data 4\nbusy\n");
        if at == 0 {
            stream.push_str(&format!("from {pushed}\n"));
        }
        stream.push('\n');
    }
    let mut child = Command::new("git")
        .args(["fast-import", "--quiet"])
        .current_dir(&sandbox.root)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stream.as_bytes())
        .unwrap();
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());

    let plan = rebase::plan(&sandbox.state(), &base).unwrap();
    assert_eq!(plan[0].summary, "Pushed long ago");
    assert!(plan[0].pushed, "an old push is still a push");
    assert!(!plan[1].pushed);
}

// --- rerere -------------------------------------------------------------------

#[test]
fn a_conflict_resolved_once_is_answered_the_same_way_next_time() {
    let sandbox = Sandbox::new("rerere");
    sandbox.commit("a.txt", "one\nbase\nthree\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("a.txt", "one\nside\nthree\n", "Side");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "one\nmain\nthree\n", "Main");
    let state = sandbox.state();
    assert!(!rebase::rerere_on(&state), "off unless turned on");
    rebase::set_rerere(&state, true).unwrap();

    let first = remote::merge(&state, "side", false).unwrap();
    assert_eq!(first.conflicts, vec!["a.txt"]);
    let choice: conflict::Resolution =
        serde_json::from_str(r#"{"take_ours": true, "take_theirs": true}"#).unwrap();
    conflict::resolve(&state, "a.txt", &[choice]).unwrap();
    remote::abort_merge(&state).unwrap();
    assert_eq!(sandbox.read("a.txt"), "one\nmain\nthree\n");

    // The same merge again: rerere writes last time's answer into the file,
    // and with autoupdate off the file stays unmerged for a person to accept.
    let again = remote::merge(&state, "side", false).unwrap();
    assert_eq!(again.conflicts, vec!["a.txt"], "still unmerged");
    assert_eq!(sandbox.read("a.txt"), "one\nmain\nside\nthree\n");
    let file = conflict::read(&state, "a.txt").unwrap();
    assert!(file.rerere, "read as resolved by rerere");
    assert_eq!(file.conflict_count, 0);

    conflict::resolve_as_is(&state, "a.txt").unwrap();
    assert!(conflict::list(&state).unwrap().is_empty());

    // The user's git config was never written.
    let local = sandbox.git(&["config", "--local", "--list"]);
    assert!(!local.contains("rerere"), "{local}");
}

#[test]
fn a_file_fixed_by_hand_is_not_mistaken_for_rerere() {
    let sandbox = Sandbox::new("rerere-hand");
    sandbox.commit("a.txt", "base\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("a.txt", "side\n", "Side");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "main\n", "Main");
    let state = sandbox.state();
    remote::merge(&state, "side", false).unwrap();
    sandbox.write("a.txt", "fixed in an editor\n");
    assert!(!conflict::read(&state, "a.txt").unwrap().rerere);
}

#[test]
fn rerere_can_be_turned_off_for_one_repository() {
    let sandbox = Sandbox::new("rerere-off");
    sandbox.commit("a.txt", "base\n", "First");
    let state = sandbox.state();
    rebase::set_rerere(&state, false).unwrap();
    assert!(!rebase::rerere_on(&state));
    // Said out loud: git turns rerere on by itself once an rr-cache exists.
    assert_eq!(
        rebase::rerere_flags(&state),
        vec!["-c", "rerere.enabled=false"]
    );
    rebase::set_rerere(&state, true).unwrap();
    assert!(rebase::rerere_on(&state));
    assert_eq!(
        rebase::rerere_flags(&state),
        vec!["-c", "rerere.enabled=true"]
    );
    // A value of the user's own is theirs.
    sandbox.git(&["config", "rerere.enabled", "false"]);
    assert!(rebase::rerere_flags(&state).is_empty());
}

// --- reopening a resolved file ------------------------------------------------

fn merge_conflict(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("a.txt", "base\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("a.txt", "side\n", "Side");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "main\n", "Main");
    assert!(!sandbox.git_may_fail(&["merge", "side"]));
    sandbox
}

#[test]
fn a_resolved_file_can_be_reopened_while_the_merge_runs() {
    let sandbox = merge_conflict("reopen");
    let state = sandbox.state();
    conflict::resolve_whole(&state, "a.txt", "ours").unwrap();
    assert!(conflict::list(&state).unwrap().is_empty());
    assert_eq!(conflict::resolved(&state).unwrap(), vec!["a.txt"]);

    conflict::reopen(&state, "a.txt").unwrap();
    assert_eq!(conflict::list(&state).unwrap(), vec!["a.txt"]);
    let text = sandbox.read("a.txt");
    assert!(
        text.contains("<<<<<<<") && text.contains("|||||||"),
        "{text}"
    );
    assert!(conflict::resolved(&state).unwrap().is_empty());
}

#[test]
fn reopening_refuses_over_changes_nobody_staged() {
    let sandbox = merge_conflict("reopen-dirty");
    let state = sandbox.state();
    conflict::resolve_whole(&state, "a.txt", "ours").unwrap();
    sandbox.write("a.txt", "an edit after resolving\n");
    assert!(conflict::reopen(&state, "a.txt")
        .unwrap_err()
        .contains("not staged"));
    assert_eq!(sandbox.read("a.txt"), "an edit after resolving\n");
}

#[test]
fn nothing_can_be_reopened_once_the_merge_is_over() {
    let sandbox = merge_conflict("reopen-over");
    let state = sandbox.state();
    conflict::resolve_whole(&state, "a.txt", "ours").unwrap();
    work::commit(&state, "Merged", false).unwrap();
    assert!(conflict::resolved(&state).unwrap().is_empty());
    assert!(conflict::reopen(&state, "a.txt").is_err());
}

// --- stacked branches ---------------------------------------------------------

/// main, and `top` on top of `low`, which branches off main.
fn stack(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "low"]);
    sandbox.commit("low.txt", "low\n", "Low");
    sandbox.git(&["checkout", "-q", "-b", "top"]);
    sandbox.commit("top.txt", "top\n", "Top");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("main.txt", "main\n", "Main moved on");
    sandbox.git(&["checkout", "-q", "top"]);
    sandbox
        .state()
        .update_config(|config| config.global.update_refs = true)
        .unwrap();
    sandbox
}

fn supports_update_refs() -> bool {
    rebase::git_version().is_some_and(|found| found >= (2, 38))
}

#[test]
fn a_rebase_carries_the_branches_stacked_under_it() {
    if !supports_update_refs() {
        return;
    }
    let sandbox = stack("update-refs");
    let old_low = sandbox.git(&["rev-parse", "low"]).trim().to_string();
    let state = sandbox.state();
    let outcome = remote::rebase(&state, "main").unwrap();
    assert!(outcome.ok, "{}", outcome.message);
    assert!(outcome.message.contains("low"), "{}", outcome.message);

    let low = sandbox.git(&["rev-parse", "low"]).trim().to_string();
    assert_ne!(low, old_low);
    assert!(sandbox.git_may_fail(&["merge-base", "--is-ancestor", "main", "low"]));
    // The old tip is one step back in its reflog.
    assert_eq!(sandbox.git(&["rev-parse", "low@{1}"]).trim(), old_low);
}

#[test]
fn turned_off_the_stack_stays_where_it_was() {
    let sandbox = stack("update-refs-off");
    let old_low = sandbox.git(&["rev-parse", "low"]).trim().to_string();
    let state = sandbox.state();
    state
        .update_config(|config| config.global.update_refs = false)
        .unwrap();
    remote::rebase(&state, "main").unwrap();
    assert_eq!(sandbox.git(&["rev-parse", "low"]).trim(), old_low);
}

#[test]
fn the_interactive_plan_carries_the_stack_too() {
    if !supports_update_refs() {
        return;
    }
    lend_git_an_editor();
    let sandbox = stack("update-refs-plan");
    let state = sandbox.state();
    let base = sandbox.oids()[0].clone();
    let plan = rebase::plan(&state, &base).unwrap();
    let steps: Vec<rebase::Step> = serde_json::from_value(serde_json::Value::Array(
        plan.iter()
            .map(|one| serde_json::json!({ "oid": one.oid, "action": "pick" }))
            .chain(std::iter::once(
                serde_json::json!({ "oid": "", "action": "drop" }),
            ))
            .filter(|one| one["oid"] != "")
            .collect(),
    ))
    .unwrap();
    // Reword nothing, just reverse the two so the hashes change.
    let reversed: Vec<rebase::Step> = steps.into_iter().rev().collect();
    let old_low = sandbox.git(&["rev-parse", "low"]).trim().to_string();
    let said = rebase::start(&state, &base, reversed).unwrap();
    assert!(said.contains("low"), "{said}");
    assert_ne!(sandbox.git(&["rev-parse", "low"]).trim(), old_low);
    assert_eq!(
        sandbox.git(&["log", "-1", "--format=%s", "low"]).trim(),
        "Low"
    );
}

#[test]
fn update_ref_lines_follow_the_fold_their_commit_ends_up_in() {
    let step = |oid: &str, action: &str| -> rebase::Step {
        serde_json::from_value(serde_json::json!({ "oid": oid, "action": action })).unwrap()
    };
    let stacked = vec![
        ("a-branch".to_string(), "aaa".to_string()),
        ("b-branch".to_string(), "bbb".to_string()),
    ];
    let todo = rebase::todo_with_refs(
        &[
            step("aaa", "pick"),
            step("bbb", "fixup"),
            step("ccc", "pick"),
        ],
        &stacked,
    )
    .unwrap();
    assert_eq!(
        todo,
        "pick aaa\nfixup bbb\nupdate-ref refs/heads/a-branch\nupdate-ref refs/heads/b-branch\npick ccc\n"
    );
    // A branch on a dropped commit lands on what was kept before it.
    let todo =
        rebase::todo_with_refs(&[step("aaa", "pick"), step("bbb", "drop")], &stacked).unwrap();
    assert_eq!(
        todo,
        "pick aaa\nupdate-ref refs/heads/a-branch\nupdate-ref refs/heads/b-branch\n"
    );
}

// --- fix up -------------------------------------------------------------------

fn three_files(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("b.txt", "two\n", "Add b");
    sandbox.commit("c.txt", "three\n", "Add c");
    sandbox
}

#[test]
fn a_fixup_folds_the_staged_change_into_an_older_commit() {
    lend_git_an_editor();
    let sandbox = three_files("fixup");
    let ids = sandbox.oids();
    sandbox.write("b.txt", "two, fixed\n");
    sandbox.git(&["add", "b.txt"]);
    sandbox.write("c.txt", "three, still open\n");

    let state = sandbox.state();
    let check = rewrite::check(&state, &ids[1]).unwrap();
    assert!(check.refusal.is_none());
    assert_eq!(check.staged, 1);
    assert_eq!(check.above, 1);

    let said = rewrite::fixup(&state, &ids[1], true, false).unwrap();
    assert!(said.contains("Folded"), "{said}");
    assert_eq!(sandbox.subjects(), vec!["First", "Add b", "Add c"]);
    assert_eq!(
        sandbox.git(&["show", "HEAD~1:b.txt"]),
        "two, fixed\n",
        "the change is inside Add b now"
    );
    // The open edit came back, unstaged.
    assert_eq!(sandbox.read("c.txt"), "three, still open\n");
    assert!(!sandbox.root.join(".git/rebase-merge").exists());
}

#[test]
fn a_fixup_left_for_later_is_one_ordinary_commit() {
    let sandbox = three_files("fixup-later");
    let ids = sandbox.oids();
    sandbox.write("b.txt", "two, fixed\n");
    sandbox.git(&["add", "b.txt"]);
    rewrite::fixup(&sandbox.state(), &ids[1], false, false).unwrap();
    assert_eq!(
        sandbox.subjects(),
        vec!["First", "Add b", "Add c", "fixup! Add b"]
    );
}

#[test]
fn a_fixup_of_a_pushed_commit_waits_for_a_yes() {
    let sandbox = three_files("fixup-pushed");
    let ids = sandbox.oids();
    sandbox.git(&["update-ref", "refs/remotes/origin/main", &ids[2]]);
    sandbox.write("b.txt", "two, fixed\n");
    sandbox.git(&["add", "b.txt"]);
    let state = sandbox.state();
    assert!(rewrite::check(&state, &ids[1]).unwrap().pushed);
    let refused = rewrite::fixup(&state, &ids[1], true, false).unwrap_err();
    assert!(refused.contains("remote"), "{refused}");
    assert_eq!(sandbox.oids(), ids, "nothing was committed");
    rewrite::fixup(&state, &ids[1], true, true).unwrap();
    assert_eq!(sandbox.git(&["show", "HEAD~1:b.txt"]), "two, fixed\n");
}

#[test]
fn a_fixup_with_nothing_staged_or_across_a_merge_is_refused() {
    let sandbox = three_files("fixup-refused");
    let ids = sandbox.oids();
    let state = sandbox.state();
    assert!(rewrite::fixup(&state, &ids[1], true, false)
        .unwrap_err()
        .contains("Nothing is staged"));

    sandbox.git(&["checkout", "-q", "-b", "side", &ids[0]]);
    sandbox.commit("s.txt", "s\n", "Side");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["merge", "-q", "--no-ff", "-m", "Merge side", "side"]);
    sandbox.write("b.txt", "two, fixed\n");
    sandbox.git(&["add", "b.txt"]);
    let check = rewrite::check(&state, &ids[1]).unwrap();
    assert!(check.refusal.unwrap().contains("merge"));
}

#[test]
fn the_message_of_an_older_commit_can_be_changed() {
    lend_git_an_editor();
    let sandbox = three_files("reword-older");
    let ids = sandbox.oids();
    let said = rewrite::reword_commit(&sandbox.state(), &ids[1], "Add b, properly", false).unwrap();
    assert!(said.contains("New message"), "{said}");
    assert_eq!(
        sandbox.subjects(),
        vec!["First", "Add b, properly", "Add c"]
    );
    // The tree is what it was.
    assert_eq!(sandbox.read("c.txt"), "three\n");

    // The newest one is an amend.
    let head = sandbox.head();
    rewrite::reword_commit(&sandbox.state(), &head, "Add c, properly", false).unwrap();
    assert_eq!(
        sandbox.subjects(),
        vec!["First", "Add b, properly", "Add c, properly"]
    );
}

// --- split --------------------------------------------------------------------

fn splittable(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("b.txt", "bee\n");
    sandbox.write("c.txt", "sea\n");
    sandbox.git(&["add", "--all"]);
    sandbox.git(&["commit", "-q", "-m", "Two things at once"]);
    sandbox.commit("d.txt", "dee\n", "Later");
    sandbox
}

#[test]
fn a_split_stops_with_the_commit_unstaged_and_carries_on_once_committed() {
    lend_git_an_editor();
    let sandbox = splittable("split");
    let ids = sandbox.oids();
    let state = sandbox.state();

    let said = rewrite::split_start(&state, &ids[1], false).unwrap();
    assert!(said.contains("Splitting"), "{said}");
    let progress = rebase::progress(&state).unwrap().expect("stopped");
    assert_eq!(progress.splitting.as_deref(), Some("Two things at once"));
    // Its changes are in the tree, not in a commit and not staged.
    assert_eq!(sandbox.git(&["log", "-1", "--format=%s"]).trim(), "First");
    assert!(sandbox.staged().is_empty());
    assert_eq!(sandbox.read("b.txt"), "bee\n");

    sandbox.git(&["add", "b.txt"]);
    sandbox.git(&["commit", "-q", "-m", "Add b"]);

    // c.txt is not committed: carrying on asks rather than drops it.
    let asked = rewrite::split_continue(&state, rewrite::Rest::Ask).unwrap();
    assert!(!asked.finished);
    assert_eq!(asked.left, vec!["c.txt"]);
    assert!(rebase::progress(&state).unwrap().is_some());

    let done = rewrite::split_continue(&state, rewrite::Rest::Commit).unwrap();
    assert!(done.finished, "{}", done.message);
    assert_eq!(
        sandbox.subjects(),
        vec!["First", "Add b", "Two things at once", "Later"]
    );
    assert!(rebase::progress(&state).unwrap().is_none());
    assert!(!sandbox.root.join(".git").join(rewrite::SPLIT).exists());
}

#[test]
fn aborting_a_split_puts_everything_back() {
    lend_git_an_editor();
    let sandbox = splittable("split-abort");
    let ids = sandbox.oids();
    let head = sandbox.head();
    let state = sandbox.state();
    rewrite::split_start(&state, &ids[1], false).unwrap();
    sandbox.git(&["add", "b.txt"]);
    sandbox.git(&["commit", "-q", "-m", "Add b"]);
    rebase::abort(&state).unwrap();
    assert_eq!(sandbox.head(), head);
    assert_eq!(sandbox.oids(), ids);
    assert!(sandbox.staged().is_empty() && sandbox.unstaged().is_empty());
    assert!(!sandbox.root.join(".git").join(rewrite::SPLIT).exists());
}

#[test]
fn the_first_commit_cannot_be_split() {
    let sandbox = splittable("split-root");
    let ids = sandbox.oids();
    assert!(rewrite::split_start(&sandbox.state(), &ids[0], false).is_err());
}

// --- absorb -------------------------------------------------------------------

/// main has a ten-line file; `feature` changes line 2 in one commit and line 8
/// in another.
fn absorbing(tag: &str, eol: &str) -> (Sandbox, String, String) {
    let sandbox = Sandbox::new(tag);
    let lines: Vec<String> = (1..=10).map(|n| format!("line {n}")).collect();
    let text = |lines: &[String]| {
        lines
            .iter()
            .map(|l| format!("{l}{eol}"))
            .collect::<String>()
    };
    sandbox.commit("f.txt", &text(&lines), "Ten lines");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    let mut two = lines.clone();
    two[1] = "line two, from A".into();
    sandbox.commit("f.txt", &text(&two), "A");
    let a = sandbox.head();
    let mut eight = two.clone();
    eight[7] = "line eight, from B".into();
    sandbox.commit("f.txt", &text(&eight), "B");
    let b = sandbox.head();

    // Staged: a change to A's line, one to B's line, one to a line from main,
    // and a new file.
    let mut staged = eight.clone();
    staged[1] = "line two, from A, fixed".into();
    staged[7] = "line eight, from B, fixed".into();
    staged[4] = "line five, from main, changed".into();
    sandbox.write("f.txt", &text(&staged));
    sandbox.write("new.txt", "brand new\n");
    sandbox.git(&["add", "--all"]);
    (sandbox, a, b)
}

#[test]
fn the_absorb_plan_names_one_commit_per_hunk_or_says_why_not() {
    let (sandbox, a, b) = absorbing("absorb-plan", "\n");
    let plan = rewrite::absorb_plan(&sandbox.state()).unwrap();
    assert!(plan.refusal.is_none(), "{:?}", plan.refusal);
    assert_eq!(plan.hunks.len(), 3, "{:?}", plan.hunks);
    let to = |at: usize| plan.hunks[at].target.as_ref().map(|t| t.oid.clone());
    assert_eq!(to(0), Some(a));
    assert_eq!(to(1), None, "line five is main's");
    assert!(plan.hunks[1].why.is_some());
    assert_eq!(to(2), Some(b));
    assert_eq!(plan.kept_files, vec!["new.txt"]);
    // Nothing happened yet.
    assert_eq!(sandbox.subjects(), vec!["Ten lines", "A", "B"]);
}

fn absorbs_and_folds(eol: &str) {
    lend_git_an_editor();
    let (sandbox, _, _) = absorbing(&format!("absorb-fold-{}", eol.len()), eol);
    let state = sandbox.state();
    let plan = rewrite::absorb_plan(&state).unwrap();
    let said = rewrite::absorb(&state, &plan.fingerprint, true).unwrap();
    assert!(said.contains("Absorb 2 hunks into 2 commits"), "{said}");

    assert_eq!(sandbox.subjects(), vec!["Ten lines", "A", "B"]);
    let at_a = sandbox.git(&["show", "HEAD~1:f.txt"]);
    assert!(
        at_a.contains(&format!("line two, from A, fixed{eol}")),
        "{at_a:?}"
    );
    assert!(!at_a.contains("line eight, from B"), "{at_a:?}");
    assert!(at_a.contains(&format!("line 5{eol}")), "{at_a:?}");
    let at_b = sandbox.git(&["show", "HEAD:f.txt"]);
    assert!(
        at_b.contains(&format!("line eight, from B, fixed{eol}")),
        "{at_b:?}"
    );
    assert!(
        at_b.contains(&format!("line 5{eol}")),
        "main's line stays as committed"
    );

    // What did not belong anywhere is still in the tree.
    let now = sandbox.read("f.txt");
    assert!(now.contains("line five, from main, changed"));
    assert!(now.contains("line two, from A, fixed"));
    assert_eq!(sandbox.read("new.txt"), "brand new\n");
    if eol == "\r\n" {
        assert_eq!(
            now.matches("\r\n").count(),
            10,
            "every ending kept: {now:?}"
        );
    }
}

#[test]
fn absorbing_folds_each_hunk_into_its_commit() {
    absorbs_and_folds("\n");
}

#[test]
fn absorbing_keeps_crlf_line_endings_byte_for_byte() {
    absorbs_and_folds("\r\n");
}

#[test]
fn absorbing_without_folding_leaves_the_fixups_and_the_rest_staged() {
    let (sandbox, _, _) = absorbing("absorb-later", "\n");
    let state = sandbox.state();
    let plan = rewrite::absorb_plan(&state).unwrap();
    rewrite::absorb(&state, &plan.fingerprint, false).unwrap();
    assert_eq!(
        sandbox.subjects(),
        vec!["Ten lines", "A", "B", "fixup! A", "fixup! B"]
    );
    // Only what stayed is staged against the new HEAD.
    let staged = sandbox.git(&["diff", "--cached"]);
    assert!(
        staged.contains("+line five, from main, changed"),
        "{staged}"
    );
    assert!(!staged.contains("+line two, from A, fixed"), "{staged}");
    assert!(staged.contains("brand new"), "{staged}");
}

#[test]
fn absorbing_refuses_a_plan_that_went_stale() {
    let (sandbox, _, _) = absorbing("absorb-stale", "\n");
    let state = sandbox.state();
    let plan = rewrite::absorb_plan(&state).unwrap();
    sandbox.write("new.txt", "changed since\n");
    sandbox.git(&["add", "new.txt"]);
    let refused = rewrite::absorb(&state, &plan.fingerprint, true).unwrap_err();
    assert!(refused.contains("changed since"), "{refused}");
    assert_eq!(sandbox.subjects(), vec!["Ten lines", "A", "B"]);
}

#[test]
fn a_hunk_spanning_two_of_your_commits_stays_staged() {
    let sandbox = Sandbox::new("absorb-ambiguous");
    sandbox.commit("f.txt", "a\nb\nc\nd\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("f.txt", "a\nB1\nc\nd\n", "One");
    sandbox.commit("f.txt", "a\nB1\nC2\nd\n", "Two");
    sandbox.write("f.txt", "a\nboth\nchanged\nd\n");
    sandbox.git(&["add", "f.txt"]);
    let plan = rewrite::absorb_plan(&sandbox.state()).unwrap();
    assert_eq!(plan.hunks.len(), 1);
    assert!(plan.hunks[0].target.is_none());
    assert!(plan.hunks[0]
        .why
        .as_deref()
        .unwrap()
        .contains("2 of your commits"));
    assert!(plan.refusal.is_some(), "there is nothing to absorb");
}

#[test]
fn pushed_commits_are_never_absorbed_into() {
    let (sandbox, a, _) = absorbing("absorb-pushed", "\n");
    sandbox.git(&["update-ref", "refs/remotes/origin/feature", &a]);
    let plan = rewrite::absorb_plan(&sandbox.state()).unwrap();
    assert!(plan.hunks[0].target.is_none(), "A is on a remote now");
    assert!(plan.hunks[2].target.is_some());
}

#[test]
fn a_region_that_differs_in_indentation_is_not_ticked_for_you() {
    let sandbox = Sandbox::new("trivial-indent");
    sandbox.commit("a.py", "if x:\n    y()\nz()\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("a.py", "if x:\n    y()\n    z()\n", "Into the block");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.py", "if x:\n    y()\nz() \n", "Trailing space");
    assert!(!sandbox.git_may_fail(&["merge", "side"]));
    let file = conflict::read(&sandbox.state(), "a.py").unwrap();
    let json = serde_json::to_value(&file).unwrap();
    let region = json["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["kind"] == "conflict")
        .expect("one region");
    assert!(region["trivial"].is_null(), "{region}");
}

// --- reopening, rerere and what it remembers ------------------------------------

fn choice(json: &str) -> conflict::Resolution {
    serde_json::from_str(json).unwrap()
}

fn two_file_conflict(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.write("b.txt", "one\nbase\nthree\n");
    sandbox.commit("a.txt", "one\nbase\nthree\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.write("b.txt", "one\nsideB\nthree\n");
    sandbox.commit("a.txt", "one\nside\nthree\n", "Side");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.write("b.txt", "one\nmainB\nthree\n");
    sandbox.commit("a.txt", "one\nmain\nthree\n", "Main");
    sandbox
}

#[test]
fn a_reopened_file_takes_its_new_answer_and_rerere_forgets_the_old_one() {
    let sandbox = two_file_conflict("reopen-rerere");
    let state = sandbox.state();
    rebase::set_rerere(&state, true).unwrap();
    remote::merge(&state, "side", false).unwrap();
    conflict::resolve(
        &state,
        "a.txt",
        &[choice(r#"{"take_ours": true, "take_theirs": true}"#)],
    )
    .unwrap();
    conflict::reopen(&state, "a.txt").unwrap();

    // Resolving another file does not write the old answer back into it.
    conflict::resolve_whole(&state, "b.txt", "ours").unwrap();
    assert!(
        sandbox.read("a.txt").contains("<<<<<<<"),
        "{}",
        sandbox.read("a.txt")
    );
    conflict::resolve(&state, "a.txt", &[choice(r#"{"take_theirs": true}"#)]).unwrap();
    assert_eq!(sandbox.git(&["show", ":a.txt"]), "one\nside\nthree\n");
    work::commit(&state, "Merged", false).unwrap();

    // The same merge again: the answer replayed is the new one.
    sandbox.git(&["reset", "-q", "--hard", "HEAD~1"]);
    remote::merge(&state, "side", false).unwrap();
    assert_eq!(sandbox.read("a.txt"), "one\nside\nthree\n");
}

#[test]
fn rerere_turned_off_stays_off_once_git_has_an_rr_cache() {
    let sandbox = two_file_conflict("rerere-cache");
    let state = sandbox.state();
    rebase::set_rerere(&state, true).unwrap();
    remote::merge(&state, "side", false).unwrap();
    conflict::resolve(
        &state,
        "a.txt",
        &[choice(r#"{"take_ours": true, "take_theirs": true}"#)],
    )
    .unwrap();
    remote::abort_merge(&state).unwrap();
    assert!(sandbox.root.join(".git/rr-cache").exists());

    rebase::set_rerere(&state, false).unwrap();
    remote::merge(&state, "side", false).unwrap();
    assert!(
        sandbox.read("a.txt").contains("<<<<<<<"),
        "{}",
        sandbox.read("a.txt")
    );
}

// --- splitting, abandoned ---------------------------------------------------------

fn split_sandbox(tag: &str) -> (Sandbox, Vec<String>) {
    lend_git_an_editor();
    let sandbox = Sandbox::new(tag);
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\ntwo\n");
    sandbox.write("b.txt", "bee\n");
    sandbox.git(&["add", "--all"]);
    sandbox.git(&["commit", "-q", "-m", "Two things"]);
    sandbox.commit("d.txt", "dee\n", "Later");
    let ids = sandbox.oids();
    (sandbox, ids)
}

#[test]
fn abandoning_a_split_keeps_what_was_changed_during_it_in_the_stash() {
    let (sandbox, ids) = split_sandbox("split-abort-kept");
    let state = sandbox.state();
    let before = sandbox.head();
    rewrite::split_start(&state, &ids[1], false).unwrap();
    sandbox.write("a.txt", "one\ntwo, tidied while splitting\n");
    sandbox.write("b.txt", "bee, tidied\n");

    let said = rebase::abort(&state).unwrap();
    assert!(said.contains("stash"), "{said}");
    assert_eq!(sandbox.head(), before, "the branch is as it was");
    let listed = sandbox.git(&["stash", "list"]);
    assert!(listed.contains("work from an abandoned split"), "{listed}");
    assert_eq!(
        sandbox.git(&["show", "stash@{0}:a.txt"]),
        "one\ntwo, tidied while splitting\n"
    );
    assert_eq!(sandbox.git(&["show", "stash@{0}^3:b.txt"]), "bee, tidied\n");
}

#[test]
fn abandoning_a_split_nobody_touched_leaves_nothing_behind() {
    let (sandbox, ids) = split_sandbox("split-abort-clean");
    let state = sandbox.state();
    rewrite::split_start(&state, &ids[1], false).unwrap();
    let said = rebase::abort(&state).unwrap();
    assert!(!said.contains("stash"), "{said}");
    assert_eq!(sandbox.git(&["stash", "list"]), "");
    assert_eq!(sandbox.read("b.txt"), "bee\n");
}

// --- absorb and fix up by commit, not by subject --------------------------------

#[test]
fn absorbing_into_two_commits_with_one_subject_lands_each_fix_in_its_own() {
    lend_git_an_editor();
    let sandbox = Sandbox::new("absorb-same-subject");
    let lines: Vec<String> = (1..=10).map(|n| format!("line {n}")).collect();
    let text = |lines: &[String]| lines.iter().map(|l| format!("{l}\n")).collect::<String>();
    sandbox.commit("f.txt", &text(&lines), "Ten lines");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    let mut two = lines.clone();
    two[1] = "line two, from A".into();
    sandbox.commit("f.txt", &text(&two), "wip");
    let mut eight = two.clone();
    eight[7] = "line eight, from B".into();
    sandbox.commit("f.txt", &text(&eight), "wip");
    let mut staged = eight.clone();
    staged[1] = "line two, from A, fixed".into();
    staged[7] = "line eight, from B, fixed".into();
    sandbox.write("f.txt", &text(&staged));
    sandbox.git(&["add", "--all"]);
    let state = sandbox.state();

    let plan = rewrite::absorb_plan(&state).unwrap();
    rewrite::absorb(&state, &plan.fingerprint, true).unwrap();
    assert!(!sandbox.root.join(".git/rebase-merge").exists());
    let at_a = sandbox.git(&["show", "HEAD~1:f.txt"]);
    assert!(at_a.contains("from A, fixed"), "{at_a}");
    assert!(!at_a.contains("from B"), "B's fix folded into A: {at_a}");
    assert!(sandbox.read("f.txt").contains("from B, fixed"));
    assert_eq!(sandbox.subjects(), vec!["Ten lines", "wip", "wip"]);
}

#[test]
fn a_fixup_that_stops_and_is_continued_is_one_step_on_the_history() {
    lend_git_an_editor();
    let sandbox = Sandbox::new("fixup-stop");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("a.txt", "two\n", "Target");
    sandbox.commit("a.txt", "three\n", "Above");
    let ids = sandbox.oids();
    sandbox.write("a.txt", "four\n");
    sandbox.git(&["add", "a.txt"]);
    let state = sandbox.state();

    let said = rewrite::fixup(&state, &ids[1], true, false).unwrap();
    assert!(said.contains("stopped"), "{said}");
    sandbox.write("a.txt", "two, fixed\n");
    sandbox.git(&["add", "a.txt"]);
    // Replaying `Above` on the fixed target stops as well; take its side.
    let mut said = rebase::resume(&state).unwrap();
    while said == "Rebase stopped" {
        sandbox.write("a.txt", "three\n");
        sandbox.git(&["add", "a.txt"]);
        said = rebase::resume(&state).unwrap();
    }
    let undo = gitnoob_lib::journal::stacks(&state).undo;
    let labels: Vec<&str> = undo.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(
        labels.first(),
        Some(&format!("Fix up {}", &ids[1][..7]).as_str()),
        "{labels:?}"
    );
    assert_eq!(labels.iter().filter(|l| l.starts_with("Fix up")).count(), 1);
}

#[test]
fn a_split_is_one_step_on_the_history_once_it_is_continued() {
    let (sandbox, ids) = split_sandbox("split-stop");
    let state = sandbox.state();
    rewrite::split_start(&state, &ids[1], false).unwrap();
    sandbox.git(&["add", "--all"]);
    sandbox.git(&["commit", "-q", "-m", "Both at once"]);
    let done = rewrite::split_continue(&state, rewrite::Rest::Ask).unwrap();
    assert!(done.finished, "{}", done.message);
    let undo = gitnoob_lib::journal::stacks(&state).undo;
    let splits = undo.iter().filter(|e| e.label.starts_with("Split")).count();
    assert_eq!(
        splits,
        1,
        "{:?}",
        undo.iter().map(|e| &e.label).collect::<Vec<_>>()
    );
}

// --- update-refs leaves backups and colleagues' branches alone --------------------

#[test]
fn a_rewrite_leaves_a_backup_at_the_tip_and_a_colleagues_branch_alone() {
    if !supports_update_refs() {
        return;
    }
    lend_git_an_editor();
    let sandbox = Sandbox::new("update-refs-others");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("b.txt", "bee\n", "Feature");
    let middle = sandbox.head();
    sandbox.git(&["branch", "alice"]);
    sandbox.git(&["config", "branch.alice.remote", "origin"]);
    sandbox.git(&["config", "branch.alice.merge", "refs/heads/alice-work"]);
    sandbox.git(&["branch", "mine"]);
    sandbox.commit("c.txt", "sea\n", "More");
    let tip = sandbox.head();
    sandbox.git(&["branch", "feature-backup"]);
    let state = sandbox.state();
    state
        .update_config(|config| config.global.update_refs = true)
        .unwrap();

    let ids = sandbox.oids();
    rewrite::reword_commit(&state, &ids[1], "Feature, reworded", false).unwrap();
    assert_eq!(sandbox.git(&["rev-parse", "feature-backup"]).trim(), tip);
    assert_eq!(sandbox.git(&["rev-parse", "alice"]).trim(), middle);
    // A branch of our own stacked in the middle still moves.
    assert_ne!(sandbox.git(&["rev-parse", "mine"]).trim(), middle);
}

/// The plain rebase — dragging a branch onto another, "Update from main" —
/// hands the work to git's own `--update-refs`, which moves every branch on the
/// replayed commits. A backup at the tip and a colleague's branch have to stay
/// where they are there too.
#[test]
fn a_plain_rebase_leaves_a_backup_at_the_tip_and_a_colleagues_branch_alone() {
    if !supports_update_refs() {
        return;
    }
    let sandbox = stack("update-refs-plain-others");
    let low = sandbox.git(&["rev-parse", "low"]).trim().to_string();
    sandbox.git(&["branch", "alice", "low"]);
    sandbox.git(&["config", "branch.alice.remote", "origin"]);
    sandbox.git(&["config", "branch.alice.merge", "refs/heads/alice-work"]);
    let tip = sandbox.head();
    sandbox.git(&["branch", "top-backup"]);
    let state = sandbox.state();

    let outcome = remote::rebase(&state, "main").unwrap();
    assert!(outcome.ok, "{}", outcome.message);
    assert_ne!(sandbox.head(), tip, "the branch itself was rebased");
    assert_eq!(sandbox.git(&["rev-parse", "top-backup"]).trim(), tip);
    assert_eq!(sandbox.git(&["rev-parse", "alice"]).trim(), low);
    // Git cannot be told to move only some of them, so the stack stays too,
    // and the message says why.
    assert_eq!(sandbox.git(&["rev-parse", "low"]).trim(), low);
    assert!(
        outcome.message.contains("Left low") && outcome.message.contains("top-backup"),
        "{}",
        outcome.message
    );
}
