//! Undo, redo and getting things back, against real repositories.
//!
//! The history is kept per work tree and on disk, so most of these build a
//! repository, act through the app, and then look at the history from a
//! different angle: another worktree, another tab, a fresh start.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use gitnoob_lib::journal::{self, Mode};
use gitnoob_lib::state::AppState;
use gitnoob_lib::{conflict, rebase, recover, refs, remote, work};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A throwaway repository, removed when the test ends.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let root = scratch(tag);
        let sandbox = Sandbox { root };
        sandbox.git(&["init", "-q", "-b", "main"]);
        sandbox.git(&["config", "user.name", "Test"]);
        sandbox.git(&["config", "user.email", "test@example.com"]);
        sandbox.git(&["config", "commit.gpgsign", "false"]);
        // Pinned so a checkout writes exactly what the test wrote, on every
        // platform. The CRLF test below writes its own line endings.
        sandbox.git(&["config", "core.autocrlf", "false"]);
        sandbox.git(&["config", "core.eol", "lf"]);
        sandbox
    }

    fn git(&self, args: &[&str]) -> String {
        git_in(&self.root, args)
    }

    fn git_may_fail(&self, args: &[&str]) -> bool {
        Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .expect("git should be on PATH")
            .status
            .success()
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

    fn rev(&self, name: &str) -> String {
        self.git(&["rev-parse", name]).trim().to_string()
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

fn scratch(tag: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root =
        std::env::temp_dir().join(format!("gitnoob-journal-{tag}-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn git_in(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
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

/// A second folder checked out from the sandbox's repository, removed with it.
struct Linked {
    root: PathBuf,
}

impl Linked {
    fn add(sandbox: &Sandbox, tag: &str, branch: &str) -> Linked {
        let root = scratch(tag);
        std::fs::remove_dir_all(&root).unwrap();
        let arg = root.to_string_lossy().into_owned();
        sandbox.git(&["worktree", "add", "-q", "-b", branch, &arg]);
        Linked { root }
    }
}

impl Drop for Linked {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn top_label(state: &AppState) -> Option<String> {
    journal::stacks(state)
        .undo
        .first()
        .map(|entry| entry.label.clone())
}

fn lend_git_an_editor() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| std::env::set_var("GITNOOB_SEQUENCE_EDITOR", "cp"));
}

// --- one history per work tree ------------------------------------------------

#[test]
fn each_worktree_has_a_history_of_its_own() {
    let sandbox = Sandbox::new("per-tree");
    sandbox.commit("a.txt", "one\n", "First");
    let linked = Linked::add(&sandbox, "per-tree-side", "side");
    let state = sandbox.state();

    sandbox.write("a.txt", "two\n");
    work::stage(&state, &["a.txt".to_string()]).unwrap();
    work::commit(&state, "Second", false).unwrap();
    assert_eq!(journal::stacks(&state).undo.len(), 1);

    // The other tab, the other folder: none of that is its to undo.
    state.set_path(linked.root.clone());
    assert!(journal::stacks(&state).undo.is_empty());
    assert_eq!(journal::undo(&state).unwrap_err(), "Nothing to undo");

    state.set_path(sandbox.root.clone());
    assert_eq!(top_label(&state).as_deref(), Some("Commit: Second"));
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.git(&["log", "-1", "--format=%s"]).trim(), "First");
}

/// The stash is shared between worktrees. Undoing a stash push from the other
/// folder used to pop it there.
#[test]
fn a_stash_made_in_one_worktree_is_not_undone_into_another() {
    let sandbox = Sandbox::new("stash-tree");
    sandbox.commit("a.txt", "one\n", "First");
    let linked = Linked::add(&sandbox, "stash-tree-side", "side");
    let state = sandbox.state();

    sandbox.write("a.txt", "mine\n");
    work::stash_push(&state, Some("here"), false).unwrap();

    state.set_path(linked.root.clone());
    assert!(journal::undo(&state).is_err());
    assert_eq!(
        std::fs::read_to_string(linked.root.join("a.txt")).unwrap(),
        "one\n"
    );
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);

    state.set_path(sandbox.root.clone());
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("a.txt"), "mine\n");
}

#[test]
fn two_repositories_in_one_window_keep_their_steps_apart() {
    let first = Sandbox::new("tab-a");
    first.commit("a.txt", "one\n", "First");
    let second = Sandbox::new("tab-b");
    second.commit("b.txt", "one\n", "First");
    let state = first.state();

    first.write("a.txt", "two\n");
    work::stage(&state, &["a.txt".to_string()]).unwrap();
    work::commit(&state, "In the first", false).unwrap();

    state.set_path(second.root.clone());
    second.write("b.txt", "two\n");
    work::stage(&state, &["b.txt".to_string()]).unwrap();
    work::commit(&state, "In the second", false).unwrap();

    assert_eq!(top_label(&state).as_deref(), Some("Commit: In the second"));
    state.set_path(first.root.clone());
    assert_eq!(top_label(&state).as_deref(), Some("Commit: In the first"));
    assert_eq!(journal::stacks(&state).undo.len(), 1);
}

// --- surviving a restart ----------------------------------------------------------

#[test]
fn the_history_survives_a_restart() {
    let sandbox = Sandbox::new("restart");
    sandbox.commit("a.txt", "one\n", "First");
    {
        let state = sandbox.state();
        sandbox.write("a.txt", "two\n");
        work::stage(&state, &["a.txt".to_string()]).unwrap();
        work::commit(&state, "Second", false).unwrap();
    }

    let state = sandbox.state();
    assert_eq!(top_label(&state).as_deref(), Some("Commit: Second"));
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.git(&["log", "-1", "--format=%s"]).trim(), "First");

    // And the redo it left survives too.
    let state = sandbox.state();
    assert_eq!(journal::stacks(&state).redo.len(), 1);
    journal::redo(&state).unwrap();
    assert_eq!(sandbox.git(&["log", "-1", "--format=%s"]).trim(), "Second");
}

#[test]
fn a_step_that_no_longer_fits_is_shown_as_unavailable_and_can_be_forgotten() {
    let sandbox = Sandbox::new("stale");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();
    sandbox.write("a.txt", "two\n");
    work::stage(&state, &["a.txt".to_string()]).unwrap();
    work::commit(&state, "Second", false).unwrap();
    let id = journal::stacks(&state).undo[0].id;

    // Still fits: not forgettable.
    assert!(journal::stacks(&state).undo[0].unavailable.is_none());
    assert!(journal::forget(&state, id).is_err());

    // Moved on in a terminal, while the app was closed.
    sandbox.commit("a.txt", "three\n", "Third, by hand");
    let state = sandbox.state();
    let stacks = journal::stacks(&state);
    assert!(
        stacks.undo[0].unavailable.is_some(),
        "the step no longer fits"
    );
    assert!(journal::undo(&state).is_err());
    assert_eq!(
        sandbox.git(&["log", "-1", "--format=%s"]).trim(),
        "Third, by hand"
    );

    journal::forget(&state, id).unwrap();
    assert!(journal::stacks(&state).undo.is_empty());
}

#[test]
fn older_steps_are_checked_as_if_the_newer_ones_were_taken() {
    let sandbox = Sandbox::new("chain");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();
    for (text, message) in [("two\n", "Second"), ("three\n", "Third")] {
        sandbox.write("a.txt", text);
        work::stage(&state, &["a.txt".to_string()]).unwrap();
        work::commit(&state, message, false).unwrap();
    }
    let stacks = journal::stacks(&state);
    assert_eq!(stacks.undo.len(), 2);
    assert!(stacks.undo.iter().all(|entry| entry.unavailable.is_none()));
}

// --- branches that are not checked out ----------------------------------------------

/// `target` at the base, `source` one commit ahead, `main` checked out.
fn behind_sandbox(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("a.txt", "base\n", "Base");
    sandbox.git(&["branch", "target"]);
    sandbox.git(&["checkout", "-q", "-b", "source"]);
    sandbox.commit("s.txt", "from source\n", "Source work");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox
}

#[test]
fn a_fast_forward_of_a_branch_not_checked_out_can_be_undone_and_redone() {
    let sandbox = behind_sandbox("ff-away");
    let base = sandbox.rev("target");
    let tip = sandbox.rev("source");
    sandbox.write("a.txt", "open work\n");
    let state = sandbox.state();

    let out = remote::merge_into(&state, "source", "target", false).unwrap();
    assert!(out.ok, "{}", out.message);
    assert_eq!(sandbox.rev("target"), tip);

    let said = journal::undo(&state).unwrap();
    assert!(said.contains("target"), "{said}");
    assert_eq!(sandbox.rev("target"), base);
    // Nothing here was touched: still on main, the open work as it was.
    assert_eq!(refs::describe(&state).unwrap().head, "main");
    assert_eq!(sandbox.read("a.txt"), "open work\n");

    journal::redo(&state).unwrap();
    assert_eq!(sandbox.rev("target"), tip);
}

#[test]
fn a_merge_made_on_a_visit_can_be_undone_from_home() {
    let sandbox = behind_sandbox("visit-away");
    sandbox.git(&["checkout", "-q", "target"]);
    sandbox.commit("t.txt", "target's own\n", "Target work");
    sandbox.git(&["checkout", "-q", "main"]);
    let before = sandbox.rev("target");
    let state = sandbox.state();

    let out = remote::merge_into(&state, "source", "target", false).unwrap();
    assert!(out.ok, "{}", out.message);
    assert_eq!(refs::describe(&state).unwrap().head, "main");
    let merged = sandbox.rev("target");
    assert_ne!(merged, before);

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.rev("target"), before);
    assert_eq!(refs::describe(&state).unwrap().head, "main");

    journal::redo(&state).unwrap();
    assert_eq!(sandbox.rev("target"), merged);
}

#[test]
fn undoing_a_branch_that_moved_since_is_refused() {
    let sandbox = behind_sandbox("ff-moved");
    let state = sandbox.state();
    remote::merge_into(&state, "source", "target", false).unwrap();

    // Somebody else moved it in the meantime.
    sandbox.git(&["checkout", "-q", "target"]);
    sandbox.commit("x.txt", "later\n", "Later");
    sandbox.git(&["checkout", "-q", "main"]);
    let later = sandbox.rev("target");

    let refused = journal::undo(&state).unwrap_err();
    assert!(refused.contains("target"), "{refused}");
    assert_eq!(sandbox.rev("target"), later);
    assert_eq!(journal::stacks(&state).undo.len(), 1);
}

#[test]
fn undoing_a_branch_checked_out_in_another_folder_is_refused() {
    let sandbox = behind_sandbox("ff-elsewhere");
    let state = sandbox.state();
    remote::merge_into(&state, "source", "target", false).unwrap();
    let tip = sandbox.rev("target");

    let elsewhere = scratch("ff-elsewhere-tree");
    std::fs::remove_dir_all(&elsewhere).unwrap();
    let arg = elsewhere.to_string_lossy().into_owned();
    sandbox.git(&["worktree", "add", "-q", &arg, "target"]);

    let refused = journal::undo(&state).unwrap_err();
    assert!(refused.contains("checked out in"), "{refused}");
    assert_eq!(sandbox.rev("target"), tip);
    let _ = std::fs::remove_dir_all(&elsewhere);
}

// --- steps that used to go missing --------------------------------------------------

#[test]
fn an_interactive_rebase_is_on_the_history() {
    lend_git_an_editor();
    let sandbox = Sandbox::new("irebase");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("b.txt", "two\n", "Second");
    sandbox.commit("c.txt", "three\n", "Third");
    let before = sandbox.rev("HEAD");
    let base = sandbox.rev("HEAD~2");
    let second = sandbox.rev("HEAD~1");
    let third = sandbox.rev("HEAD");
    let state = sandbox.state();

    rebase::start(
        &state,
        &base,
        vec![
            rebase::Step {
                oid: third,
                action: rebase::Action::Pick,
            },
            rebase::Step {
                oid: second,
                action: rebase::Action::Drop,
            },
        ],
    )
    .unwrap();
    assert!(!sandbox.root.join("b.txt").exists());
    let stacks = journal::stacks(&state);
    assert_eq!(stacks.undo[0].kind, "rebase");
    assert_eq!(stacks.undo[0].mode, Mode::Hard);

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.rev("HEAD"), before);
    assert_eq!(sandbox.read("b.txt"), "two\n");
}

/// Two branches that change the same line of `f.txt`.
fn clash_sandbox(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("f.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.commit("other.txt", "untouched\n", "A file beside it");
    sandbox.git(&["checkout", "-q", "-b", "theirs"]);
    sandbox.commit("f.txt", "top\ntheirs\nbottom\n", "Theirs");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("f.txt", "top\nours\nbottom\n", "Ours");
    sandbox
}

/// A merge that stopped and was finished with Commit used to be a soft
/// "Commit: Merge…": undoing it left the whole merged branch staged as if it
/// were your own work, and the next commit squashed it in.
#[test]
fn a_merge_finished_by_hand_is_undone_as_a_merge() {
    let sandbox = clash_sandbox("merge-by-hand");
    let before = sandbox.rev("HEAD");
    let state = sandbox.state();

    let out = remote::merge(&state, "theirs", false).unwrap();
    assert!(!out.ok);
    sandbox.write("f.txt", "top\nboth\nbottom\n");
    sandbox.git(&["add", "f.txt"]);
    work::commit(&state, "Merge branch 'theirs'", false).unwrap();
    let merged = sandbox.rev("HEAD");

    let stacks = journal::stacks(&state);
    assert_eq!(stacks.undo[0].mode, Mode::Hard);
    assert_eq!(stacks.undo[0].label, "Merge theirs");

    // Open work beside it, which the undo must leave alone.
    sandbox.write("other.txt", "edited since\n");
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.rev("HEAD"), before);
    assert!(
        sandbox
            .git(&["diff", "--cached", "--name-only"])
            .trim()
            .is_empty(),
        "nothing of the merged branch is left staged"
    );
    assert_eq!(sandbox.read("f.txt"), "top\nours\nbottom\n");
    assert_eq!(sandbox.read("other.txt"), "edited since\n");

    journal::redo(&state).unwrap();
    assert_eq!(sandbox.rev("HEAD"), merged);
    assert_eq!(
        sandbox
            .git(&["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3,
        "still a merge, with both parents"
    );
}

#[test]
fn a_rebase_that_stopped_and_was_continued_is_on_the_history() {
    let sandbox = clash_sandbox("rebase-continued");
    let before = sandbox.rev("HEAD");
    let state = sandbox.state();

    let out = remote::rebase(&state, "theirs").unwrap();
    assert!(!out.ok);
    assert!(journal::stacks(&state).undo.is_empty(), "not finished yet");

    sandbox.write("f.txt", "top\nboth\nbottom\n");
    sandbox.git(&["add", "f.txt"]);
    let out = remote::continue_rebase(&state).unwrap();
    assert!(out.ok, "{}", out.message);
    assert_eq!(top_label(&state).as_deref(), Some("Rebase onto theirs"));

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.rev("HEAD"), before);
}

#[test]
fn an_aborted_merge_records_nothing() {
    let sandbox = clash_sandbox("merge-aborted");
    let state = sandbox.state();
    remote::merge(&state, "theirs", false).unwrap();
    remote::abort_merge(&state).unwrap();
    assert!(journal::stacks(&state).undo.is_empty());

    // And a later, unrelated commit is not taken for the end of it.
    sandbox.write("other.txt", "later\n");
    work::stage(&state, &["other.txt".to_string()]).unwrap();
    work::commit(&state, "Later", false).unwrap();
    assert_eq!(top_label(&state).as_deref(), Some("Commit: Later"));
}

#[test]
fn undo_says_plainly_when_a_merge_is_part_way_through() {
    let sandbox = clash_sandbox("undo-midmerge");
    let state = sandbox.state();
    sandbox.write("other.txt", "a step to undo\n");
    work::stage(&state, &["other.txt".to_string()]).unwrap();
    work::commit(&state, "A step", false).unwrap();
    remote::merge(&state, "theirs", false).unwrap();

    let refused = journal::undo(&state).unwrap_err();
    assert!(refused.contains("merge is part-way through"), "{refused}");
    // What the window offers agrees: nothing to put back while it runs.
    assert!(!remote::in_progress(&state).unwrap().restoring);
    assert_eq!(journal::stacks(&state).undo.len(), 1);
}

// --- put it back ----------------------------------------------------------------------

/// A pull whose carried work does not fit back on: the undo takes the pull
/// back and puts the work where it was — after a restart too, and with the
/// history file gone, from the branch's own reflog.
fn stuck_pull(tag: &str) -> (Sandbox, String, PathBuf) {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("a.txt", "one\n", "One");
    let bare = sandbox.root.with_extension("origin.git");
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["branch", "-q", "-u", "origin/main"]);
    let before = sandbox.rev("HEAD");

    // Somebody else's commit, straight into the bare repository.
    let other = sandbox.root.with_extension("other");
    let _ = std::fs::remove_dir_all(&other);
    let other_arg = other.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", &bare_arg, &other_arg]);
    git_in(&other, &["config", "user.name", "Someone"]);
    git_in(&other, &["config", "user.email", "someone@example.com"]);
    std::fs::write(other.join("a.txt"), "theirs\n").unwrap();
    git_in(&other, &["commit", "-q", "-am", "Theirs"]);
    git_in(&other, &["push", "-q", "origin", "main"]);
    let _ = std::fs::remove_dir_all(&other);
    sandbox.git(&["fetch", "-q", "origin"]);

    sandbox.write("a.txt", "mine\n");
    let state = sandbox.state();
    let out = remote::pull(&state, Some(false)).unwrap();
    assert!(!out.ok);
    assert!(remote::in_progress(&state).unwrap().restoring);
    (sandbox, before, bare)
}

#[test]
fn putting_it_back_after_a_restart_still_finds_the_step() {
    let (sandbox, before, bare) = stuck_pull("putback-restart");
    let state = sandbox.state();
    let said = journal::undo(&state).unwrap();
    assert!(said.contains("Pull"), "{said}");
    assert_eq!(sandbox.rev("HEAD"), before);
    assert_eq!(sandbox.read("a.txt"), "mine\n");
    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn putting_it_back_with_no_history_reads_the_branch_log() {
    let (sandbox, before, bare) = stuck_pull("putback-reflog");
    let journal_file = sandbox.root.join(".git/gitnoob-journal.json");
    std::fs::remove_file(&journal_file).unwrap();
    let state = sandbox.state();
    work::undo_restore(&state).unwrap();
    assert_eq!(sandbox.rev("HEAD"), before);
    assert_eq!(sandbox.read("a.txt"), "mine\n");
    assert!(sandbox.git(&["stash", "list"]).trim().is_empty());
    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn putting_it_back_with_nowhere_to_go_does_not_stop_on_the_same_conflict() {
    let sandbox = Sandbox::new("putback-here");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.write("a.txt", "top\nmine\nbottom\n");
    sandbox.git(&[
        "stash",
        "push",
        "-q",
        "-m",
        &format!("{} on main: switching to other", work::AUTO_STASH),
    ]);
    sandbox.commit("a.txt", "top\nsomebody else\nbottom\n", "Moved on");
    assert!(!sandbox.git_may_fail(&["stash", "apply"]));

    let state = sandbox.state();
    let said = work::undo_restore(&state).unwrap();
    assert!(said.contains("still in the stash"), "{said}");
    assert!(conflict::list(&state).unwrap().is_empty());
    assert_eq!(sandbox.read("a.txt"), "top\nsomebody else\nbottom\n");
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);
}

// --- discards -----------------------------------------------------------------------

#[test]
fn a_discard_is_undone_byte_for_byte() {
    let sandbox = Sandbox::new("discard");
    sandbox.commit("crlf.txt", "one\r\ntwo\r\n", "Base");
    sandbox.commit("a[b].txt", "committed\n", "Brackets");
    sandbox.commit("ab.txt", "not me\n", "The name a[b] would match");
    let binary: Vec<u8> = (0u8..=255).chain([0, 0, 13, 10, 13]).collect();
    std::fs::write(sandbox.root.join("blob.bin"), b"old").unwrap();
    sandbox.git(&["add", "blob.bin"]);
    sandbox.git(&["commit", "-q", "-m", "Binary"]);

    sandbox.write("crlf.txt", "one\r\nchanged\r\nthree\r\n");
    sandbox.write("a[b].txt", "edited\n");
    sandbox.write("ab.txt", "also edited, kept\n");
    std::fs::write(sandbox.root.join("blob.bin"), &binary).unwrap();

    let state = sandbox.state();
    let paths: Vec<String> = ["crlf.txt", "a[b].txt", "blob.bin"]
        .iter()
        .map(|p| p.to_string())
        .collect();
    work::discard(&state, &paths).unwrap();
    assert_eq!(sandbox.read("crlf.txt"), "one\r\ntwo\r\n");
    assert_eq!(sandbox.read("a[b].txt"), "committed\n");
    assert_eq!(sandbox.read("ab.txt"), "also edited, kept\n");

    let stacks = journal::stacks(&state);
    assert_eq!(stacks.undo[0].mode, Mode::Files);
    assert_eq!(stacks.undo[0].files.len(), 3);

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("crlf.txt"), "one\r\nchanged\r\nthree\r\n");
    assert_eq!(sandbox.read("a[b].txt"), "edited\n");
    assert_eq!(
        std::fs::read(sandbox.root.join("blob.bin")).unwrap(),
        binary
    );

    journal::redo(&state).unwrap();
    assert_eq!(sandbox.read("a[b].txt"), "committed\n");
    assert_eq!(
        std::fs::read(sandbox.root.join("blob.bin")).unwrap(),
        b"old"
    );
}

#[test]
fn undoing_a_discard_refuses_once_the_file_has_changed_again() {
    let sandbox = Sandbox::new("discard-guard");
    sandbox.commit("a.txt", "committed\n", "Base");
    sandbox.write("a.txt", "thrown away\n");
    let state = sandbox.state();
    work::discard(&state, &["a.txt".to_string()]).unwrap();

    sandbox.write("a.txt", "typed since\n");
    let refused = journal::undo(&state).unwrap_err();
    assert!(refused.contains("a.txt"), "{refused}");
    assert_eq!(sandbox.read("a.txt"), "typed since\n");
    assert_eq!(journal::stacks(&state).undo.len(), 1);
}

#[test]
fn deleted_untracked_files_come_back_folders_and_all() {
    let sandbox = Sandbox::new("untracked");
    sandbox.commit("a.txt", "committed\n", "Base");
    sandbox.write("loose[1].txt", "only on disk\n");
    sandbox.write("notes/deep/one.md", "first\n");
    sandbox.write("notes/two.md", "second\n");
    let state = sandbox.state();

    work::delete_untracked(&state, &["loose[1].txt".to_string(), "notes/".to_string()]).unwrap();
    assert!(!sandbox.root.join("loose[1].txt").exists());
    assert!(!sandbox.root.join("notes").exists());

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("loose[1].txt"), "only on disk\n");
    assert_eq!(sandbox.read("notes/deep/one.md"), "first\n");
    assert_eq!(sandbox.read("notes/two.md"), "second\n");

    journal::redo(&state).unwrap();
    assert!(!sandbox.root.join("notes").exists(), "empty folders go too");
}

#[test]
fn a_discarded_hunk_comes_back() {
    let sandbox = Sandbox::new("hunk");
    let lines: String = (1..=30).map(|n| format!("line {n}\n")).collect();
    sandbox.commit("a.txt", &lines, "Base");
    let edited = lines
        .replace("line 2\n", "line 2 edited\n")
        .replace("line 28\n", "line 28 edited\n");
    sandbox.write("a.txt", &edited);
    let state = sandbox.state();

    work::apply_hunk(&state, "a.txt", 1, work::HunkAction::Discard, None).unwrap();
    assert!(!sandbox.read("a.txt").contains("line 28 edited"));
    assert!(sandbox.read("a.txt").contains("line 2 edited"));
    assert_eq!(journal::stacks(&state).undo[0].kind, "discard");

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("a.txt"), edited);
}

#[cfg(unix)]
#[test]
fn a_discarded_mode_change_and_link_come_back() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new("modes");
    sandbox.commit("run.sh", "echo hi\n", "Base");
    std::os::unix::fs::symlink("run.sh", sandbox.root.join("link")).unwrap();
    sandbox.git(&["add", "link"]);
    sandbox.git(&["commit", "-q", "-m", "Link"]);

    let script = sandbox.root.join("run.sh");
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();
    std::fs::remove_file(sandbox.root.join("link")).unwrap();
    std::os::unix::fs::symlink("elsewhere", sandbox.root.join("link")).unwrap();

    let state = sandbox.state();
    work::discard(&state, &["run.sh".to_string(), "link".to_string()]).unwrap();
    assert_eq!(
        std::fs::read_link(sandbox.root.join("link")).unwrap(),
        PathBuf::from("run.sh")
    );

    journal::undo(&state).unwrap();
    let mode = std::fs::metadata(&script).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "executable again");
    assert_eq!(
        std::fs::read_link(sandbox.root.join("link")).unwrap(),
        PathBuf::from("elsewhere")
    );
}

// --- stashes ----------------------------------------------------------------------

#[test]
fn a_dropped_stash_comes_back_as_it_was() {
    let sandbox = Sandbox::new("drop");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("a.txt", "kept\n");
    sandbox.git(&["stash", "push", "-q", "-m", "worth keeping"]);
    let oid = sandbox.rev("stash@{0}");
    let state = sandbox.state();

    work::stash_drop(&state, 0).unwrap();
    assert!(work::stash_list(&state).unwrap().is_empty());

    journal::undo(&state).unwrap();
    let back = work::stash_list(&state).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].oid, oid);
    assert_eq!(back[0].message, "worth keeping");
    assert_eq!(back[0].branch.as_deref(), Some("main"));

    journal::redo(&state).unwrap();
    assert!(work::stash_list(&state).unwrap().is_empty());
}

#[test]
fn a_clean_pop_is_undone_back_onto_the_list() {
    let sandbox = Sandbox::new("pop");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("a.txt", "stashed\n");
    sandbox.write("new.txt", "brand new\n");
    sandbox.git(&["add", "new.txt"]);
    sandbox.git(&["stash", "push", "-q", "-m", "popped"]);
    let state = sandbox.state();

    work::stash_pop(&state, 0).unwrap();
    assert_eq!(sandbox.read("a.txt"), "stashed\n");

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("a.txt"), "one\n");
    assert!(!sandbox.root.join("new.txt").exists());
    let listed = work::stash_list(&state).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].message, "popped");

    journal::redo(&state).unwrap();
    assert_eq!(sandbox.read("a.txt"), "stashed\n");
    assert!(work::stash_list(&state).unwrap().is_empty());
}

#[test]
fn undoing_a_pop_refuses_once_its_files_were_worked_on() {
    let sandbox = Sandbox::new("pop-guard");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("a.txt", "stashed\n");
    sandbox.git(&["stash", "push", "-q", "-m", "popped"]);
    let state = sandbox.state();
    work::stash_pop(&state, 0).unwrap();

    sandbox.write("a.txt", "stashed, then edited\n");
    assert!(journal::undo(&state).is_err());
    assert_eq!(sandbox.read("a.txt"), "stashed, then edited\n");
    assert!(
        work::stash_list(&state).unwrap().is_empty(),
        "not stored twice"
    );
}

// --- branches ---------------------------------------------------------------------

#[test]
fn a_deleted_branch_comes_back_tracking_what_it_tracked() {
    let sandbox = Sandbox::new("branch-back");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("t.txt", "unmerged\n", "Topic work");
    sandbox.git(&["config", "branch.topic.remote", "origin"]);
    sandbox.git(&["config", "branch.topic.merge", "refs/heads/topic"]);
    let tip = sandbox.rev("topic");
    sandbox.git(&["checkout", "-q", "main"]);
    let state = sandbox.state();

    refs::delete_branch(&state, "topic", true).unwrap();
    assert!(!sandbox.git_may_fail(&["rev-parse", "--verify", "--quiet", "refs/heads/topic"]));

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.rev("refs/heads/topic"), tip);
    assert_eq!(
        sandbox
            .git(&["config", "--get", "branch.topic.merge"])
            .trim(),
        "refs/heads/topic"
    );

    journal::redo(&state).unwrap();
    assert!(!sandbox.git_may_fail(&["rev-parse", "--verify", "--quiet", "refs/heads/topic"]));
}

#[test]
fn a_deleted_branch_is_not_put_back_over_a_new_one() {
    let sandbox = Sandbox::new("branch-taken");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["branch", "topic"]);
    let state = sandbox.state();
    refs::delete_branch(&state, "topic", false).unwrap();

    sandbox.commit("a.txt", "two\n", "Later");
    sandbox.git(&["branch", "topic"]);
    let fresh = sandbox.rev("topic");

    let refused = journal::undo(&state).unwrap_err();
    assert!(refused.contains("topic"), "{refused}");
    assert_eq!(sandbox.rev("topic"), fresh);
}

// --- the reflog view ------------------------------------------------------------------

#[test]
fn the_reflog_lists_where_head_has_been() {
    let sandbox = Sandbox::new("moments");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("a.txt", "two\n", "Second");
    sandbox.git(&["reset", "-q", "--hard", "HEAD~1"]);
    let state = sandbox.state();

    let moments = recover::moments(&state, 50).unwrap();
    assert!(moments[0].action.starts_with("reset"), "{:?}", moments[0]);
    let second = moments
        .iter()
        .find(|moment| moment.subject == "Second")
        .expect("the commit the reset left is still listed");
    // HEAD and main both logged the commit; it is one row.
    assert!(second.logs.contains(&"HEAD".to_string()));
    assert!(second.logs.contains(&"main".to_string()));
}

#[test]
fn a_reset_to_an_older_moment_keeps_open_work() {
    let sandbox = Sandbox::new("keep-reset");
    sandbox.commit("a.txt", "one\n", "First");
    let first = sandbox.rev("HEAD");
    sandbox.commit("b.txt", "two\n", "Second");
    sandbox.write("a.txt", "open work\n");
    let state = sandbox.state();

    work::reset(&state, &first, work::ResetMode::Keep).unwrap();
    assert_eq!(sandbox.rev("HEAD"), first);
    assert_eq!(sandbox.read("a.txt"), "open work\n");
    assert!(!sandbox.root.join("b.txt").exists());

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("b.txt"), "two\n");
    assert_eq!(sandbox.read("a.txt"), "open work\n");
}

#[test]
fn a_stash_dropped_long_ago_is_found_and_put_back() {
    let sandbox = Sandbox::new("lost");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("a.txt", "lost work\n");
    sandbox.git(&["stash", "push", "-q", "-m", "the lost one"]);
    let oid = sandbox.rev("stash@{0}");
    sandbox.write("a.txt", "still listed\n");
    sandbox.git(&["stash", "push", "-q", "-m", "the listed one"]);
    // Dropped in a terminal, so not on any history.
    sandbox.git(&["stash", "drop", "-q", "stash@{1}"]);
    let state = sandbox.state();

    let lost = recover::lost_stashes(&state).unwrap();
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert_eq!(lost[0].oid, oid);
    assert_eq!(lost[0].message, "On main: the lost one");

    recover::restore_stash(&state, &oid).unwrap();
    let listed = work::stash_list(&state).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].oid, oid);
    assert_eq!(listed[0].message, "the lost one");

    // An ordinary commit is not a stash, and is refused.
    let head = sandbox.rev("HEAD");
    assert!(recover::restore_stash(&state, &head).is_err());
}

/// With `core.autocrlf` on, git would hand back what it would check in rather
/// than what was on disk. The saved copy is the bytes as they were.
#[test]
fn a_discard_under_autocrlf_comes_back_with_its_own_line_endings() {
    let sandbox = Sandbox::new("discard-autocrlf");
    sandbox.git(&["config", "core.autocrlf", "true"]);
    sandbox.commit("mixed.txt", "one\ntwo\n", "Base");
    let mixed = "one\r\ntwo\nthree\r\n";
    sandbox.write("mixed.txt", mixed);
    let state = sandbox.state();

    work::discard(&state, &["mixed.txt".to_string()]).unwrap();
    assert_ne!(sandbox.read("mixed.txt"), mixed);

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("mixed.txt"), mixed);
}

/// A merge into another branch that stopped, finished on that branch, and is
/// undone once you are back where you started.
#[test]
fn a_visit_merge_that_stopped_is_undone_from_home_once_finished() {
    let sandbox = clash_sandbox("visit-clash");
    sandbox.git(&["checkout", "-q", "-b", "home"]);
    let before = sandbox.rev("main");
    let state = sandbox.state();

    let out = remote::merge_into(&state, "theirs", "main", false).unwrap();
    assert!(!out.ok);
    assert_eq!(refs::describe(&state).unwrap().head, "main");

    sandbox.write("f.txt", "top\nboth\nbottom\n");
    sandbox.git(&["add", "f.txt"]);
    work::commit(&state, "Merge branch 'theirs'", false).unwrap();
    assert_eq!(
        refs::describe(&state).unwrap().head,
        "home",
        "back where it started"
    );
    let merged = sandbox.rev("main");
    assert_eq!(top_label(&state).as_deref(), Some("Merge theirs"));

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.rev("main"), before);
    assert_eq!(refs::describe(&state).unwrap().head, "home");

    journal::redo(&state).unwrap();
    assert_eq!(sandbox.rev("main"), merged);
}

// --- tidying branches away ----------------------------------------------------------

fn doomed(sandbox: &Sandbox, name: &str) -> refs::Doomed {
    refs::Doomed {
        name: name.to_string(),
        oid: sandbox.rev(&format!("refs/heads/{name}")),
    }
}

fn exists(sandbox: &Sandbox, name: &str) -> bool {
    sandbox.git_may_fail(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/heads/{name}"),
    ])
}

#[test]
fn a_tidy_leaves_a_branch_that_moved_after_the_list_was_read() {
    let sandbox = Sandbox::new("tidy-moved");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["branch", "done"]);
    sandbox.git(&["branch", "busy"]);
    sandbox.git(&["config", "branch.done.remote", "origin"]);
    let listed = vec![doomed(&sandbox, "done"), doomed(&sandbox, "busy")];

    // Work lands on `busy` after the tidy list was shown.
    sandbox.git(&["checkout", "-q", "busy"]);
    sandbox.commit("b.txt", "new work\n", "New work");
    sandbox.git(&["checkout", "-q", "main"]);
    let moved = sandbox.rev("busy");
    let state = sandbox.state();

    let removal = refs::delete_branches(&state, &listed).unwrap();
    assert_eq!(
        removal
            .removed
            .iter()
            .map(|one| one.name.as_str())
            .collect::<Vec<_>>(),
        vec!["done"]
    );
    assert!(
        removal.refused.iter().any(|line| line.starts_with("busy")),
        "{:?}",
        removal.refused
    );
    assert_eq!(sandbox.rev("busy"), moved, "the new work is still there");
    assert!(!exists(&sandbox, "done"));
    assert!(
        !sandbox.git_may_fail(&["config", "--get", "branch.done.remote"]),
        "its config went with it"
    );
}

#[test]
fn a_tidy_is_one_step_and_keeps_an_older_discard_on_the_history() {
    let sandbox = Sandbox::new("tidy-one-step");
    sandbox.commit("a.txt", "committed\n", "Base");
    sandbox.write("a.txt", "an afternoon of work\n");
    let state = sandbox.state();
    work::discard(&state, &["a.txt".to_string()]).unwrap();
    let mut listed = Vec::new();
    for n in 0..101 {
        let name = format!("old-{n}");
        sandbox.git(&["branch", &name]);
        listed.push(doomed(&sandbox, &name));
    }

    let removal = refs::delete_branches(&state, &listed).unwrap();
    assert_eq!(removal.removed.len(), 101);
    let stacks = journal::stacks(&state);
    assert_eq!(
        stacks.undo.len(),
        2,
        "one step for the tidy, one for the discard"
    );
    assert_eq!(stacks.undo[0].label, "Delete 101 branches");
    assert_eq!(Some(stacks.undo[0].id), removal.step);

    journal::undo(&state).unwrap();
    assert!((0..101).all(|n| exists(&sandbox, &format!("old-{n}"))));
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("a.txt"), "an afternoon of work\n");
}

#[test]
fn a_full_history_lets_go_of_other_steps_before_a_discard() {
    let sandbox = Sandbox::new("evict");
    sandbox.commit("a.txt", "committed\n", "Base");
    sandbox.write("a.txt", "an afternoon of work\n");
    let state = sandbox.state();
    work::discard(&state, &["a.txt".to_string()]).unwrap();
    for n in 0..100 {
        let name = format!("b{n}");
        sandbox.git(&["branch", &name]);
        refs::delete_branch(&state, &name, false).unwrap();
    }
    let undo = journal::stacks(&state).undo;
    assert_eq!(undo.len(), 100);
    assert_eq!(
        undo.last().unwrap().mode,
        Mode::Files,
        "the discard is kept"
    );
    assert_eq!(undo[undo.len() - 2].label, "Delete branch b1");
}

#[test]
fn undoing_a_tidy_never_overwrites_a_branch_made_since_and_redo_leaves_moved_ones() {
    let sandbox = Sandbox::new("tidy-undo-redo");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["branch", "one"]);
    sandbox.git(&["branch", "two"]);
    let old = sandbox.rev("one");
    let state = sandbox.state();
    refs::delete_branches(&state, &[doomed(&sandbox, "one"), doomed(&sandbox, "two")]).unwrap();

    // A new `two` somewhere else: the undo refuses rather than overwrite it.
    sandbox.commit("a.txt", "two\n", "Later");
    sandbox.git(&["branch", "two"]);
    let fresh = sandbox.rev("two");
    let refused = journal::undo(&state).unwrap_err();
    assert!(refused.contains("two"), "{refused}");
    assert_eq!(sandbox.rev("two"), fresh);
    assert!(!exists(&sandbox, "one"), "all or nothing");

    sandbox.git(&["branch", "-D", "two"]);
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.rev("one"), old);
    assert_eq!(sandbox.rev("two"), old);

    // `one` moves on; redoing deletes only `two`, which is where it was.
    sandbox.git(&["update-ref", "refs/heads/one", "HEAD"]);
    let said = journal::redo(&state).unwrap();
    assert!(said.contains("one"), "{said}");
    assert!(exists(&sandbox, "one"));
    assert!(!exists(&sandbox, "two"));

    // And undoing again brings back `two` alone, without touching `one`.
    let moved = sandbox.rev("one");
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.rev("two"), old);
    assert_eq!(sandbox.rev("one"), moved);
}

#[test]
fn the_notice_undo_takes_the_tidy_step_off_the_history() {
    let sandbox = Sandbox::new("tidy-notice");
    sandbox.commit("a.txt", "committed\n", "Base");
    sandbox.write("a.txt", "work\n");
    let state = sandbox.state();
    work::discard(&state, &["a.txt".to_string()]).unwrap();
    sandbox.git(&["branch", "one"]);
    sandbox.git(&["branch", "two"]);
    let removal =
        refs::delete_branches(&state, &[doomed(&sandbox, "one"), doomed(&sandbox, "two")]).unwrap();

    journal::undo_step(&state, removal.step.unwrap()).unwrap();
    assert!(exists(&sandbox, "one") && exists(&sandbox, "two"));
    let stacks = journal::stacks(&state);
    assert!(stacks.undo.iter().all(|entry| entry.unavailable.is_none()));
    assert_eq!(stacks.redo.len(), 1, "the tidy can be redone");

    // Nothing stale on top: the next undo is the discard.
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("a.txt"), "work\n");
}

#[test]
fn a_deleted_branch_can_be_undone_from_its_notice_under_newer_steps() {
    let sandbox = Sandbox::new("tidy-notice-under");
    sandbox.commit("a.txt", "committed\n", "Base");
    sandbox.git(&["branch", "one"]);
    let state = sandbox.state();
    let removal = refs::delete_branches(&state, &[doomed(&sandbox, "one")]).unwrap();
    sandbox.write("a.txt", "work\n");
    work::discard(&state, &["a.txt".to_string()]).unwrap();

    journal::undo_step(&state, removal.step.unwrap()).unwrap();
    assert!(exists(&sandbox, "one"));
    assert_eq!(
        top_label(&state).as_deref(),
        Some("Discard changes to a.txt")
    );
}

// --- odd names and stashes ------------------------------------------------------------

// Quotes are not allowed in file names on Windows.
#[cfg(unix)]
#[test]
fn a_discarded_file_named_in_quotes_comes_back_as_itself() {
    let sandbox = Sandbox::new("quoted");
    sandbox.commit("x", "committed x\n", "x");
    sandbox.commit("\"x\"", "committed quoted\n", "quoted");
    sandbox.write("\"x\"", "precious edit\n");
    let state = sandbox.state();
    work::discard(&state, &["\"x\"".to_string()]).unwrap();
    assert_eq!(sandbox.read("\"x\""), "committed quoted\n");
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("\"x\""), "precious edit\n");
    assert_eq!(sandbox.read("x"), "committed x\n");
}

#[cfg(unix)]
#[test]
fn a_deleted_untracked_file_named_in_quotes_comes_back() {
    let sandbox = Sandbox::new("quoted-untracked");
    sandbox.commit("a", "base\n", "a");
    sandbox.write("\"a\"", "untracked and precious\n");
    let state = sandbox.state();
    work::delete_untracked(&state, &["\"a\"".to_string()]).unwrap();
    assert!(!sandbox.root.join("\"a\"").exists());
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("\"a\""), "untracked and precious\n");
}

/// A file with staged work at the bottom and a stash that changes its top.
fn staged_under_a_stash(tag: &str) -> (Sandbox, String) {
    let sandbox = Sandbox::new(tag);
    let lines: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    sandbox.commit("a.txt", &lines, "Base");
    sandbox.write("a.txt", &lines.replace("line 1\n", "line 1 stashed\n"));
    sandbox.git(&["stash", "push", "-q", "-m", "top"]);
    let staged = lines.replace("line 20\n", "line 20 staged work\n");
    sandbox.write("a.txt", &staged);
    sandbox.git(&["add", "a.txt"]);
    (sandbox, staged)
}

#[test]
fn undoing_a_pop_keeps_work_staged_before_it() {
    let (sandbox, staged) = staged_under_a_stash("pop-staged");
    let state = sandbox.state();
    work::stash_pop(&state, 0).unwrap();
    assert!(sandbox.read("a.txt").contains("line 1 stashed"));

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("a.txt"), staged);
    assert_eq!(sandbox.git(&["show", ":a.txt"]), staged, "still staged");
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);

    journal::redo(&state).unwrap();
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.git(&["show", ":a.txt"]), staged);
}

#[test]
fn undoing_an_apply_keeps_work_staged_before_it() {
    let (sandbox, staged) = staged_under_a_stash("apply-staged");
    let state = sandbox.state();
    work::stash_apply(&state, 0).unwrap();
    assert!(sandbox.read("a.txt").contains("line 1 stashed"));

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("a.txt"), staged);
    assert_eq!(sandbox.git(&["show", ":a.txt"]), staged, "still staged");
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);
}

#[test]
fn undoing_a_pop_checks_a_file_with_a_name_outside_ascii() {
    let sandbox = Sandbox::new("pop-utf8");
    sandbox.commit("é.txt", "one\n", "Base");
    sandbox.write("é.txt", "stashed\n");
    sandbox.git(&["stash", "push", "-q", "-m", "popped"]);
    let state = sandbox.state();
    work::stash_pop(&state, 0).unwrap();

    sandbox.write("é.txt", "stashed, then edited\n");
    assert!(journal::undo(&state).is_err());
    assert_eq!(sandbox.read("é.txt"), "stashed, then edited\n");

    sandbox.write("é.txt", "stashed\n");
    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("é.txt"), "one\n");
}

#[test]
fn undoing_a_popped_move_puts_both_halves_back() {
    let sandbox = Sandbox::new("pop-rename");
    let body: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    sandbox.commit("a.txt", &body, "Base");
    sandbox.git(&["mv", "a.txt", "b.txt"]);
    sandbox.git(&["stash", "push", "-q", "-m", "moved"]);
    let state = sandbox.state();
    work::stash_pop(&state, 0).unwrap();
    assert!(!sandbox.root.join("a.txt").exists());

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.read("a.txt"), body);
    assert!(!sandbox.root.join("b.txt").exists());
    assert_eq!(sandbox.git(&["status", "--porcelain"]), "");
}

// --- the file on disk -------------------------------------------------------------------

#[test]
fn a_history_this_version_cannot_read_is_set_aside_not_written_over() {
    let sandbox = Sandbox::new("unreadable");
    sandbox.commit("a.txt", "committed\n", "Base");
    sandbox.write("a.txt", "work\n");
    work::discard(&sandbox.state(), &["a.txt".to_string()]).unwrap();
    let file = sandbox.root.join(".git/gitnoob-journal.json");
    let text = std::fs::read_to_string(&file).unwrap();
    // A newer version adds a mode this one does not know.
    let newer = text.replacen("\"mode\":\"files\"", "\"mode\":\"future\"", 1);
    std::fs::write(&file, &newer).unwrap();

    let state = sandbox.state();
    sandbox.git(&["branch", "x"]);
    refs::delete_branch(&state, "x", false).unwrap();
    let aside: Vec<_> = std::fs::read_dir(sandbox.root.join(".git"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("gitnoob-journal.json.bad-")
        })
        .collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(std::fs::read_to_string(aside[0].path()).unwrap(), newer);
    assert_eq!(top_label(&state).as_deref(), Some("Delete branch x"));
}

#[test]
fn two_windows_on_one_repository_keep_each_others_steps() {
    let sandbox = Sandbox::new("two-processes");
    sandbox.commit("a.txt", "committed\n", "Base");
    // Two app states stand for two processes: each has its own memory.
    let one = sandbox.state();
    let two = sandbox.state();
    let _ = journal::stacks(&one);
    let _ = journal::stacks(&two);

    sandbox.write("a.txt", "work in one\n");
    work::discard(&one, &["a.txt".to_string()]).unwrap();
    sandbox.git(&["branch", "x"]);
    refs::delete_branch(&two, "x", false).unwrap();

    let fresh = sandbox.state();
    let undo = journal::stacks(&fresh).undo;
    let labels: Vec<&str> = undo.iter().map(|entry| entry.label.as_str()).collect();
    assert_eq!(labels, vec!["Delete branch x", "Discard changes to a.txt"]);
    assert_ne!(undo[0].id, undo[1].id);

    // And the first window sees the second one's step too.
    assert_eq!(top_label(&one).as_deref(), Some("Delete branch x"));
}

/// A file that is there but cannot be read just now — another program holding
/// it open on Windows, a permission gone wrong — is not an empty history. Taken
/// as one, the next step written out replaced every saved step, discards and all.
#[cfg(unix)]
#[test]
fn a_history_that_cannot_be_read_for_a_moment_is_not_written_over() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new("locked");
    sandbox.commit("a.txt", "committed\n", "Base");
    let state = sandbox.state();
    sandbox.write("a.txt", "work\n");
    work::discard(&state, &["a.txt".to_string()]).unwrap();
    let file = sandbox.root.join(".git/gitnoob-journal.json");
    let saved = std::fs::read_to_string(&file).unwrap();

    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Root reads whatever the mode says, and then there is nothing to test.
    if std::fs::read_to_string(&file).is_ok() {
        return;
    }
    sandbox.git(&["branch", "x"]);
    refs::delete_branch(&state, "x", false).unwrap();
    let other = sandbox.state();
    let _ = journal::stacks(&other);
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    let on_disk = std::fs::read_to_string(&file).unwrap();
    assert!(
        on_disk.contains("Discard changes to a.txt"),
        "the discard's step is gone: {on_disk} (was {saved})"
    );
    let labels: Vec<String> = journal::stacks(&state)
        .undo
        .into_iter()
        .map(|entry| entry.label)
        .collect();
    assert_eq!(labels, vec!["Delete branch x", "Discard changes to a.txt"]);
}

/// A stash carrying an untracked file and a new file that was staged: undoing
/// its pop takes both away again and leaves the rest of the tree as it was.
#[test]
fn undoing_a_pop_with_untracked_and_added_files_takes_them_back_off() {
    let sandbox = Sandbox::new("pop-untracked");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("new.txt", "staged new\n");
    sandbox.git(&["add", "new.txt"]);
    sandbox.write("loose.txt", "untracked\n");
    sandbox.git(&["stash", "push", "-q", "-u", "-m", "with extras"]);
    sandbox.write("other.txt", "left alone\n");
    let state = sandbox.state();
    work::stash_pop(&state, 0).unwrap();
    assert_eq!(sandbox.read("loose.txt"), "untracked\n");
    assert_eq!(sandbox.read("new.txt"), "staged new\n");

    journal::undo(&state).unwrap();
    assert!(!sandbox.root.join("loose.txt").exists());
    assert!(!sandbox.root.join("new.txt").exists());
    assert_eq!(sandbox.read("other.txt"), "left alone\n");
    assert_eq!(sandbox.git(&["status", "--porcelain"]), "?? other.txt\n");
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);

    // And on again, the same as the first time.
    journal::redo(&state).unwrap();
    assert_eq!(sandbox.read("loose.txt"), "untracked\n");
    assert_eq!(sandbox.git(&["show", ":new.txt"]), "staged new\n");
}

/// A history that has filled up with discards over a few days must still take
/// a new step of another kind. Letting go of "the oldest step that is not a
/// discard" let go of the step just recorded, so a commit, a merge or a rebase
/// could no longer be undone at all.
#[test]
fn a_history_full_of_discards_still_keeps_the_newest_step() {
    let sandbox = Sandbox::new("full-of-discards");
    sandbox.commit("a.txt", "committed\n", "Base");
    let blob = sandbox.git(&["rev-parse", "HEAD:a.txt"]).trim().to_string();
    let steps: Vec<String> = (1..=100)
        .map(|id| {
            format!(
                r#"{{"id":{id},"label":"Discard changes to a.txt","kind":"discard","branch":null,"before":null,"after":null,"mode":"files","destructive":true,"at":0,"files":[{{"path":"a.txt","before":{{"blob":"{blob}","kind":"file"}},"after":{{"blob":"{blob}","kind":"file"}}}}]}}"#
            )
        })
        .collect();
    std::fs::write(
        sandbox.root.join(".git/gitnoob-journal.json"),
        format!(
            r#"{{"undo":[{}],"redo":[],"next_id":100}}"#,
            steps.join(",")
        ),
    )
    .unwrap();

    let state = sandbox.state();
    assert_eq!(journal::stacks(&state).undo.len(), 100);
    sandbox.git(&["branch", "x"]);
    refs::delete_branch(&state, "x", false).unwrap();
    let undo = journal::stacks(&state).undo;
    assert_eq!(undo.len(), 100);
    assert_eq!(undo[0].label, "Delete branch x");
    journal::undo(&state).unwrap();
    assert!(sandbox.git_may_fail(&["rev-parse", "--verify", "--quiet", "refs/heads/x"]));
}
