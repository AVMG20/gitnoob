//! End-to-end checks against real repositories built with the `git` CLI.
//!
//! These cover the parts that are easy to get subtly wrong — graph lane layout,
//! divergence reporting, conflict marker parsing — rather than the thin command
//! wrappers around them.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use gitnoob_lib::state::AppState;
use gitnoob_lib::{conflict, create, diff, graph, journal, rebase, refs, remote, work, worktree};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A throwaway repository, removed when the test ends.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("gitnoob-test-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let sandbox = Sandbox { root };
        sandbox.git(&["init", "-q", "-b", "main"]);
        sandbox.git(&["config", "user.name", "Test"]);
        sandbox.git(&["config", "user.email", "test@example.com"]);
        // Keep the test independent of the machine's global git config.
        sandbox.git(&["config", "commit.gpgsign", "false"]);
        // Git for Windows defaults `core.autocrlf` to true, which would hand
        // back CRLF from every checkout and fail the LF comparisons below. The
        // app honours whatever the user set; the tests pin it so the expected
        // content is the same on every platform.
        sandbox.git(&["config", "core.autocrlf", "false"]);
        sandbox.git(&["config", "core.eol", "lf"]);
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

    /// Runs git without asserting success — for commands expected to conflict.
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
        std::fs::write(self.root.join(path), content).unwrap();
    }

    fn commit(&self, path: &str, content: &str, message: &str) {
        self.write(path, content);
        self.git(&["add", "--all"]);
        self.git(&["commit", "-q", "-m", message]);
    }

    fn state(&self) -> AppState {
        // Reading a token on an unsigned build puts a macOS password dialog on
        // screen, and a test suite that stops to ask for a password is a test
        // suite nobody can run.
        gitnoob_lib::config::silence_keychain();
        // Point the config at the sandbox so tests never touch the real one.
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

#[test]
fn opens_a_repository_from_a_subdirectory() {
    let sandbox = Sandbox::new("discover");
    sandbox.commit("a.txt", "one\n", "First");
    std::fs::create_dir_all(sandbox.root.join("deep/nested")).unwrap();

    let found = gitnoob_lib::state::discover_workdir(&sandbox.root.join("deep/nested")).unwrap();
    // Compare canonically: macOS temp paths go through a /private symlink.
    assert_eq!(
        found.canonicalize().unwrap(),
        sandbox.root.canonicalize().unwrap()
    );
}

#[test]
fn rejects_a_directory_outside_any_repository() {
    let root = std::env::temp_dir().join(format!("gitnoob-not-a-repo-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    assert!(gitnoob_lib::state::discover_workdir(&root).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

/// A throwaway folder to clone or create repositories in.
fn scratch(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "gitnoob-{}-{}-{}",
        tag,
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// Runs git in a folder that is not a `Sandbox`, asserting it worked.
fn git_at(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
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

#[test]
fn clones_a_repository_into_a_folder_named_after_it() {
    let origin = Sandbox::new("clone-origin");
    origin.commit("a.txt", "one\n", "First");
    let parent = scratch("clone-into");

    let made = create::clone(origin.root.to_string_lossy().as_ref(), &parent).unwrap();
    let dest = Path::new(&made.path);
    assert_eq!(
        made.name,
        origin.root.file_name().unwrap().to_string_lossy()
    );
    assert_eq!(dest, parent.join(&made.name));
    assert!(dest.join(".git").exists());
    // Read back without the line ending: a Windows checkout writes CRLF where
    // the fixture wrote LF, and which one landed is not what this is about.
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "one\n",
        "the clone should carry the files"
    );
    assert_eq!(
        git_at(dest, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "main"
    );

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn refuses_to_clone_where_a_folder_already_exists() {
    let origin = Sandbox::new("clone-exists");
    origin.commit("a.txt", "one\n", "First");
    let parent = scratch("clone-exists-into");
    let name = origin
        .root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    std::fs::create_dir_all(parent.join(&name)).unwrap();

    let refused = create::clone(origin.root.to_string_lossy().as_ref(), &parent).unwrap_err();
    assert!(refused.contains("already has a folder"));

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn creating_a_repository_makes_a_first_commit_as_the_profile() {
    let parent = scratch("init");
    let made = create::init(
        &parent,
        "fresh",
        Some(("Test".to_string(), "test@example.com".to_string())),
    )
    .unwrap();

    let dest = Path::new(&made.path);
    assert!(dest.join(".git").exists());
    assert_eq!(
        git_at(dest, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "main"
    );
    assert_eq!(
        git_at(dest, &["config", "--local", "user.name"]).trim(),
        "Test"
    );
    // One commit, and it carries the .gitignore.
    assert_eq!(git_at(dest, &["rev-list", "--count", "HEAD"]).trim(), "1");
    assert_eq!(git_at(dest, &["ls-files"]).trim(), ".gitignore");
    assert!(made.note.is_none());

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn refuses_folder_names_that_cannot_exist() {
    let parent = scratch("init-bad");
    assert!(create::init(&parent, "a/b", None).is_err());
    assert!(create::init(&parent, ".hidden", None).is_err());
    assert!(create::init(&parent, "  ", None).is_err());
    // Nothing should have been created by the refusals above.
    assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn manages_the_remotes_themselves() {
    let sandbox = Sandbox::new("remote-manage");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();

    // A real remote to fetch from: the same setup `git push` tests use.
    let origin = scratch("remote-origin");
    git_at(&origin, &["init", "-q", "--bare", "-b", "main", "."]);
    sandbox.git(&["push", "-q", origin.to_string_lossy().as_ref(), "main"]);

    // Add, and the address reads back.
    assert!(remote::remote_add(&state, "upstream", origin.to_string_lossy().as_ref()).is_ok());
    assert_eq!(
        remote::remote_url(&state, "upstream").unwrap(),
        origin.to_string_lossy().as_ref()
    );

    // A duplicate name is refused by git itself.
    assert!(remote::remote_add(&state, "upstream", "/elsewhere.git").is_err());
    // So is a name git will not accept.
    assert!(remote::remote_add(&state, "not a name", "/x.git").is_err());
    assert!(remote::remote_add(&state, "-dash", "/x.git").is_err());
    assert!(remote::remote_add(&state, "ok", "   ").is_err());

    // Changing the address, to somewhere that does not answer: git accepts an
    // address without ever contacting it, which is exactly what an edit of the
    // destination should do.
    assert!(remote::remote_set_url(&state, "upstream", "/somewhere/widget.git").is_ok());
    assert_eq!(
        remote::remote_url(&state, "upstream").unwrap(),
        "/somewhere/widget.git"
    );
    assert!(remote::remote_set_url(&state, "upstream", origin.to_string_lossy().as_ref()).is_ok());

    // Renaming moves the remote-tracking branches with the name.
    sandbox.git(&["fetch", "-q", "upstream"]);
    assert!(remote::remote_rename(&state, "upstream", "source").is_ok());
    let names: Vec<String> = remote::remotes(&state).unwrap();
    assert!(names.contains(&"source".to_string()) && !names.contains(&"upstream".to_string()));
    assert!(!sandbox
        .git(&["rev-parse", "--verify", "refs/remotes/source/main"])
        .trim()
        .is_empty());

    // Removing takes the tracking branches and nothing else.
    assert!(remote::remote_remove(&state, "source").is_ok());
    assert!(!remote::remotes(&state)
        .unwrap()
        .contains(&"source".to_string()));
    assert!(!sandbox.git_may_fail(&["rev-parse", "--verify", "refs/remotes/source/main"]));
    // The local branch and its commit are untouched.
    assert_eq!(
        sandbox.git(&["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "main"
    );

    let _ = std::fs::remove_dir_all(&origin);
}

#[test]
fn lists_branches_with_ahead_and_behind_counts() {
    let sandbox = Sandbox::new("refs");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("b.txt", "two\n", "Second");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["tag", "v1"]);

    let state = sandbox.state();
    let info = refs::describe(&state).unwrap();
    assert_eq!(info.head, "main");
    assert!(!info.detached);

    let tree = refs::tree(&state).unwrap();
    assert_eq!(tree.locals.len(), 2);
    assert!(
        tree.locals
            .iter()
            .find(|b| b.name == "main")
            .unwrap()
            .is_head
    );
    assert_eq!(tree.tags.len(), 1);
    assert_eq!(tree.tags[0].name, "v1");
}

/// A file moved into a subfolder and edited afterwards.
///
/// libgit2 reports a status entry's path as the *old* side of the head-to-index
/// delta, so a rename used to be listed under the name the file no longer has —
/// and staging it answered "pathspec ... did not match any files", because
/// there is nothing there any more.
fn moved_and_edited(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    std::fs::create_dir_all(sandbox.root.join("tests/Feature/Filament")).unwrap();
    sandbox.commit(
        "tests/Feature/Filament/CreateTicketFormTest.php",
        "<?php\nold\n",
        "First",
    );
    std::fs::create_dir_all(sandbox.root.join("tests/Feature/Filament/Tickets")).unwrap();
    sandbox.git(&[
        "mv",
        "tests/Feature/Filament/CreateTicketFormTest.php",
        "tests/Feature/Filament/Tickets/CreateTicketFormTest.php",
    ]);
    sandbox.write(
        "tests/Feature/Filament/Tickets/CreateTicketFormTest.php",
        "<?php\nold\nedited after the move\n",
    );
    sandbox
}

const MOVED_TO: &str = "tests/Feature/Filament/Tickets/CreateTicketFormTest.php";
const MOVED_FROM: &str = "tests/Feature/Filament/CreateTicketFormTest.php";

#[test]
fn a_file_moved_and_then_edited_is_listed_where_it_now_is() {
    let sandbox = moved_and_edited("moved-status");
    let status = refs::status(&sandbox.state()).unwrap();

    let staged = status
        .staged
        .iter()
        .find(|e| e.kind == "renamed")
        .expect("the move is staged");
    assert_eq!(staged.path, MOVED_TO);
    assert_eq!(staged.from.as_deref(), Some(MOVED_FROM));

    // The edit made since is against the index, so it belongs to the new name.
    let unstaged = status
        .unstaged
        .iter()
        .find(|e| e.kind == "modified")
        .expect("the edit is not staged");
    assert_eq!(unstaged.path, MOVED_TO);
    assert!(
        !status.unstaged.iter().any(|e| e.path == MOVED_FROM),
        "nothing should be listed under a name that is no longer on disk"
    );
}

#[test]
fn staging_the_edit_made_after_a_move_finds_the_file() {
    let sandbox = moved_and_edited("moved-stage");
    let state = sandbox.state();
    let path = refs::status(&state)
        .unwrap()
        .unstaged
        .iter()
        .find(|e| e.kind == "modified")
        .expect("the edit is listed")
        .path
        .clone();

    // The path the window shows is the path it stages, and `git add` has to
    // find it: with the old name this failed with "did not match any files".
    work::stage(&state, &[path]).unwrap();

    let after = refs::status(&state).unwrap();
    assert!(
        after.unstaged.is_empty(),
        "the edit should be staged: {:?}",
        after.unstaged
    );
    assert!(after.staged.iter().any(|e| e.path == MOVED_TO));
}

#[test]
fn staging_everything_leaves_the_move_listed_where_it_landed() {
    let sandbox = moved_and_edited("moved-stage-all");
    let state = sandbox.state();
    work::stage_all(&state).unwrap();

    let status = refs::status(&state).unwrap();
    assert!(status.unstaged.is_empty(), "{:?}", status.unstaged);
    let moved = status
        .staged
        .iter()
        .find(|e| e.kind == "renamed")
        .expect("one entry for one file");
    assert_eq!(moved.path, MOVED_TO);
    assert_eq!(moved.from.as_deref(), Some(MOVED_FROM));
    assert!(
        !status.staged.iter().any(|e| e.path == MOVED_FROM),
        "the old name is not a second file: {:?}",
        status.staged
    );
}

#[test]
fn a_move_taken_off_the_index_is_still_a_move_in_the_working_tree() {
    let sandbox = moved_and_edited("moved-unstaged-move");
    let state = sandbox.state();
    work::stage_all(&state).unwrap();
    work::unstage(&state, &[MOVED_TO.to_string()]).unwrap();

    // Nothing is staged, and the working tree still says one file moved rather
    // than one deleted and another appearing out of nowhere.
    let status = refs::status(&state).unwrap();
    assert!(status.staged.is_empty(), "{:?}", status.staged);
    let moved = status
        .unstaged
        .iter()
        .find(|e| e.path == MOVED_TO)
        .expect("listed under the name on disk");
    assert_eq!(moved.kind, "renamed");
    assert_eq!(moved.from.as_deref(), Some(MOVED_FROM));
}

/// A file moved with the shell rather than with `git mv`: nothing is staged,
/// and git sees a deletion at the old name beside an untracked file at the new
/// one until something pairs them up.
fn moved_outside_git(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    std::fs::create_dir_all(sandbox.root.join("a")).unwrap();
    sandbox.commit("a/f.txt", "one\ntwo\nthree\nfour\nfive\n", "First");
    std::fs::create_dir_all(sandbox.root.join("b")).unwrap();
    std::fs::rename(sandbox.root.join("a/f.txt"), sandbox.root.join("b/f.txt")).unwrap();
    sandbox.write("b/f.txt", "one\ntwo\nthree\nfour\nsix\n");
    sandbox
}

#[test]
fn a_move_made_outside_git_is_one_row_under_the_name_on_disk() {
    let sandbox = moved_outside_git("wt-move-status");
    let status = refs::status(&sandbox.state()).unwrap();

    assert!(status.staged.is_empty());
    assert_eq!(status.unstaged.len(), 1, "{:?}", status.unstaged);
    let moved = &status.unstaged[0];
    assert_eq!(moved.path, "b/f.txt");
    assert_eq!(moved.from.as_deref(), Some("a/f.txt"));
    assert_eq!(moved.kind, "renamed");
}

#[test]
fn staging_a_move_made_outside_git_stages_both_halves_of_it() {
    let sandbox = moved_outside_git("wt-move-stage");
    let state = sandbox.state();

    work::stage(&state, &["b/f.txt".to_string()]).unwrap();

    // Staging only the new name would leave `a/f.txt` behind as an unstaged
    // deletion, and git would never see the two as one move.
    let status = refs::status(&state).unwrap();
    assert!(status.unstaged.is_empty(), "{:?}", status.unstaged);
    assert_eq!(status.staged.len(), 1, "{:?}", status.staged);
    assert_eq!(status.staged[0].path, "b/f.txt");
    assert_eq!(status.staged[0].from.as_deref(), Some("a/f.txt"));
    assert_eq!(status.staged[0].kind, "renamed");
}

#[test]
fn discarding_a_move_made_outside_git_puts_the_file_back_where_it_was() {
    let sandbox = moved_outside_git("wt-move-discard");
    let state = sandbox.state();

    work::discard(&state, &["b/f.txt".to_string()]).unwrap();

    assert!(
        sandbox.root.join("a/f.txt").exists(),
        "the deletion at the old name is what discarding can undo"
    );
    // The copy at the new name is untracked, and nothing here deletes an
    // untracked file as a side effect — it is listed as one, to be dealt with
    // deliberately.
    let status = refs::status(&state).unwrap();
    assert!(status
        .unstaged
        .iter()
        .any(|e| e.path == "b/f.txt" && e.kind == "untracked"));
    assert!(!status.unstaged.iter().any(|e| e.path == "a/f.txt"));
}

#[test]
fn the_change_inside_a_move_made_outside_git_can_be_read() {
    let sandbox = moved_outside_git("wt-move-diff");
    let found =
        diff::working_file_diff(&sandbox.state(), "b/f.txt", diff::Side::Unstaged).unwrap();
    let lines: Vec<(char, &str)> = found
        .hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.origin == '+' || line.origin == '-')
        .map(|line| (line.origin, line.content.as_str()))
        .collect();
    assert_eq!(lines, vec![('-', "five"), ('+', "six")]);
}

#[test]
fn a_deleted_file_is_listed_under_its_own_name_and_came_from_nowhere() {
    let sandbox = Sandbox::new("deleted-file");
    sandbox.commit("gone.txt", "one\n", "First");
    std::fs::remove_file(sandbox.root.join("gone.txt")).unwrap();

    let status = refs::status(&sandbox.state()).unwrap();
    let gone = status
        .unstaged
        .iter()
        .find(|e| e.kind == "deleted")
        .expect("it is gone");
    assert_eq!(gone.path, "gone.txt");
    assert!(gone.from.is_none(), "a deletion is not a move");
}

#[test]
fn an_untracked_file_is_listed_under_its_own_name() {
    let sandbox = Sandbox::new("untracked-file");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("fresh.txt", "new\n");

    let status = refs::status(&sandbox.state()).unwrap();
    let fresh = status
        .unstaged
        .iter()
        .find(|e| e.path == "fresh.txt")
        .expect("it is listed");
    assert_eq!(fresh.kind, "untracked");
    assert!(fresh.from.is_none());
}

#[test]
fn a_file_deleted_after_being_staged_is_still_one_file_on_each_side() {
    let sandbox = Sandbox::new("staged-then-deleted");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\ntwo\n");
    sandbox.git(&["add", "a.txt"]);
    std::fs::remove_file(sandbox.root.join("a.txt")).unwrap();

    let status = refs::status(&sandbox.state()).unwrap();
    assert!(status
        .staged
        .iter()
        .any(|e| e.path == "a.txt" && e.kind == "modified" && e.from.is_none()));
    assert!(status
        .unstaged
        .iter()
        .any(|e| e.path == "a.txt" && e.kind == "deleted" && e.from.is_none()));
}

#[test]
fn discarding_an_ordinary_edit_is_untouched_by_any_of_this() {
    let sandbox = Sandbox::new("plain-discard");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\ntwo\n");

    work::discard(&sandbox.state(), &["a.txt".to_string()]).unwrap();
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\n"
    );
}

#[test]
fn ordinary_changes_carry_no_rename_origin() {
    let sandbox = Sandbox::new("moved-plain");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\ntwo\n");
    sandbox.git(&["add", "a.txt"]);

    let status = refs::status(&sandbox.state()).unwrap();
    assert!(status.staged.iter().all(|e| e.from.is_none()));
}

#[test]
fn unstaging_a_move_takes_both_halves_back() {
    let sandbox = moved_and_edited("moved-unstage");
    let state = sandbox.state();
    work::stage(&state, &[MOVED_TO.to_string()]).unwrap();

    // The row names one path; the rename behind it is two index entries, and
    // putting back only one leaves the file both moved and deleted.
    work::unstage(&state, &[MOVED_TO.to_string()]).unwrap();

    let status = refs::status(&state).unwrap();
    assert!(
        status.staged.is_empty(),
        "nothing should still be staged: {:?}",
        status.staged
    );
    assert!(
        !status.unstaged.iter().any(|e| e.kind == "deleted"),
        "the file is not deleted, it moved: {:?}",
        status.unstaged
    );
    // What is on disk is untouched by any of it.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join(MOVED_TO)).unwrap(),
        "<?php\nold\nedited after the move\n"
    );
}

#[test]
fn the_change_a_moved_file_carries_can_still_be_read() {
    let sandbox = moved_and_edited("moved-diff");
    let state = sandbox.state();
    let found = diff::working_file_diff(&state, MOVED_TO, diff::Side::Unstaged).unwrap();
    assert!(
        !found.hunks.is_empty(),
        "clicking the row should show the edit, not an empty pane"
    );
    let added: Vec<&str> = found
        .hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.origin == '+')
        .map(|line| line.content.as_str())
        .collect();
    assert_eq!(added, vec!["edited after the move"]);
}

#[test]
fn a_staged_move_reads_as_a_move_rather_than_the_whole_file_deleted() {
    let sandbox = moved_and_edited("moved-staged-diff");
    let state = sandbox.state();
    work::stage(&state, &[MOVED_TO.to_string()]).unwrap();

    let found = diff::working_file_diff(&state, MOVED_TO, diff::Side::Staged).unwrap();
    let lines: Vec<(char, &str)> = found
        .hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.origin == '+' || line.origin == '-')
        .map(|line| (line.origin, line.content.as_str()))
        .collect();
    // The one line that actually changed, and not 160 deletions and 160
    // additions of a file that only moved.
    assert_eq!(lines, vec![('+', "edited after the move")]);
}

#[test]
fn reports_staged_and_unstaged_changes_separately() {
    let sandbox = Sandbox::new("status");
    sandbox.commit("tracked.txt", "one\n", "First");
    sandbox.write("tracked.txt", "one\ntwo\n");
    sandbox.git(&["add", "tracked.txt"]);
    sandbox.write("tracked.txt", "one\ntwo\nthree\n");
    sandbox.write("fresh.txt", "new\n");

    let status = refs::status(&sandbox.state()).unwrap();
    assert!(status.staged.iter().any(|e| e.path == "tracked.txt"));
    assert!(status
        .unstaged
        .iter()
        .any(|e| e.path == "tracked.txt" && e.kind == "modified"));
    assert!(status
        .unstaged
        .iter()
        .any(|e| e.path == "fresh.txt" && e.kind == "untracked"));
    assert!(status.conflicted.is_empty());
}

/// A branch whose tip is newer than HEAD must not take the trunk's column.
///
/// The walk reaches the newest commit first, so first come first served hands
/// lane 0 to whichever branch happens to have been committed to last, and the
/// line the user is standing on is pushed sideways around it — which reads as
/// the branch and the trunk trading places rather than as a branch leaving.
#[test]
fn a_repository_with_many_branches_gets_lanes_past_the_first_dozen() {
    let sandbox = Sandbox::new("wide-graph");
    sandbox.commit("a.txt", "root\n", "Root");

    // Twenty branches off the root, none of them merged: every one of them is
    // a line the graph has to hold open at once. Two commits each, so each line
    // has a stretch that runs down its own lane rather than only reaching for
    // the root.
    for i in 0..20 {
        let branch = format!("topic-{i}");
        sandbox.git(&["checkout", "-q", "-b", &branch, "main"]);
        sandbox.commit(
            &format!("{branch}.txt"),
            "work\n",
            &format!("Work on {branch}"),
        );
        sandbox.commit(
            &format!("{branch}.txt"),
            "more\n",
            &format!("More on {branch}"),
        );
    }
    sandbox.git(&["checkout", "-q", "main"]);

    let page = graph::build(&sandbox.state(), 500).unwrap();

    let widest = page.rows.iter().map(|row| row.lane).max().unwrap_or(0);
    assert!(
        widest >= 14,
        "twenty parallel branches should reach past the old ceiling, got {widest}"
    );
    // A line whose whole length is out there used to be dropped before it
    // reached the window, so the lanes past the ceiling drew nothing at all.
    assert!(
        page.rows
            .iter()
            .any(|row| row.segments.iter().any(|s| s.x1 >= 14 && s.x2 >= 14)),
        "a line living entirely past lane 14 was thrown away"
    );
}

#[test]
fn the_checked_out_line_keeps_the_leftmost_lane() {
    let sandbox = Sandbox::new("trunk-lane");
    sandbox.commit("a.txt", "1\n", "Root");
    sandbox.commit("a.txt", "2\n", "Second");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("t.txt", "t\n", "Topic work");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["merge", "-q", "--no-ff", "-m", "Merge topic", "topic"]);
    // Committed after the merge, so this branch's tip is the newest commit in
    // the repository while main stays checked out.
    sandbox.git(&["checkout", "-q", "-b", "later"]);
    sandbox.commit("l.txt", "l\n", "Newer than HEAD");
    sandbox.commit("l.txt", "l2\n", "Newer still");
    sandbox.git(&["checkout", "-q", "main"]);

    let page = graph::build(&sandbox.state(), 500).unwrap();

    let merge = page
        .rows
        .iter()
        .find(|row| row.summary == "Merge topic")
        .expect("the merge commit should be in the graph");
    assert_eq!(merge.lane, 0, "HEAD's line belongs in the leftmost lane");

    // The newer branch sits beside the trunk rather than in it, and runs
    // straight down its own lane: no row of it steps sideways on the way.
    let newer: Vec<&graph::GraphRow> = page
        .rows
        .iter()
        .filter(|row| row.summary.starts_with("Newer"))
        .collect();
    assert_eq!(newer.len(), 2);
    for row in &newer {
        assert!(row.lane > 0, "a branch must not take the trunk's column");
        for segment in &row.segments {
            if segment.x1 == row.lane || segment.x2 == row.lane {
                assert_eq!(
                    segment.x1, segment.x2,
                    "a branch keeps its lane until the commit it rejoins"
                );
            }
        }
    }

    // Nothing is drawn in the reserved lane above the row that claims it: the
    // trunk has no line before its own newest commit.
    let above: Vec<&graph::GraphRow> = page
        .rows
        .iter()
        .take_while(|row| row.summary != "Merge topic")
        .collect();
    for row in &above {
        assert!(
            row.segments.iter().all(|s| s.x1 != 0 && s.x2 != 0),
            "the reserved lane carries no line until the walk reaches it"
        );
    }
}

/// A merge into the checked-out branch, from a branch that is newer than it.
///
/// The shape a branch switch leaves behind, and the one that used to break: the
/// merge puts a line into the lane held for HEAD, and every row between the
/// merge and HEAD's own commit has to keep drawing it. Skipping those rows left
/// the line ending in mid-air a row below the merge.
#[test]
fn a_merge_into_the_checked_out_branch_keeps_its_line() {
    let sandbox = Sandbox::new("merge-into-head");
    sandbox.commit("a.txt", "1\n", "Root");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("t.txt", "t\n", "Topic work");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("m.txt", "m\n", "Main work");
    sandbox.commit("m.txt", "m2\n", "More main work");
    // Topic takes main's work, so the merge's second parent is main's tip.
    sandbox.git(&["checkout", "-q", "topic"]);
    sandbox.git(&[
        "merge",
        "-q",
        "--no-ff",
        "-m",
        "Merge main into topic",
        "main",
    ]);
    sandbox.commit("t.txt", "t2\n", "Newer than HEAD");
    sandbox.git(&["checkout", "-q", "main"]);

    let page = graph::build(&sandbox.state(), 500).unwrap();
    let at = |summary: &str| {
        page.rows
            .iter()
            .position(|row| row.summary == summary)
            .unwrap_or_else(|| panic!("{summary} should be in the graph"))
    };
    let merge = at("Merge main into topic");
    let head = at("More main work");
    assert!(merge < head, "the merge is newer than the commit it merged");

    // The lane the merge sends its second parent into.
    let into = page.rows[merge]
        .segments
        .iter()
        .filter(|s| s.y1 == 1 && s.y2 == 2 && s.x2 != page.rows[merge].lane)
        .map(|s| s.x2)
        .next()
        .expect("a merge sends a line to its second parent");

    for row in merge + 1..head {
        assert!(
            page.rows[row]
                .segments
                .iter()
                .any(|s| s.x1 == into && s.y1 == 0),
            "row {row} ({}) drops the line the merge left in lane {into}",
            page.rows[row].summary
        );
    }
    // And it arrives at the commit it was drawn for.
    assert!(
        page.rows[head]
            .segments
            .iter()
            .any(|s| s.x1 == into && s.y2 == 1),
        "the line should end at the commit that was merged"
    );
}

/// The graph is the piece most likely to be quietly wrong, so this asserts the
/// invariants a renderer depends on rather than an exact picture.
#[test]
fn graph_lanes_and_segments_stay_consistent() {
    let sandbox = Sandbox::new("graph");
    sandbox.commit("a.txt", "1\n", "Root");
    sandbox.commit("a.txt", "2\n", "Second");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("topic.txt", "t\n", "Topic work");
    sandbox.commit("topic.txt", "t2\n", "More topic work");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("main.txt", "m\n", "Main work");
    sandbox.git(&["merge", "-q", "--no-ff", "-m", "Merge topic", "topic"]);
    sandbox.git(&["checkout", "-q", "-b", "dangling", "main~2"]);
    sandbox.commit("d.txt", "d\n", "Unmerged work");
    sandbox.git(&["checkout", "-q", "main"]);

    let page = graph::build(&sandbox.state(), 500).unwrap();
    assert!(!page.has_more);
    // Six commits on main and topic, plus the one on the unmerged branch.
    assert_eq!(page.rows.len(), 7);

    let merge = page
        .rows
        .iter()
        .find(|row| row.summary == "Merge topic")
        .expect("the merge commit should be in the graph");
    assert_eq!(merge.parents.len(), 2);
    // A merge sends a line to each parent.
    assert!(merge.segments.iter().filter(|s| s.y1 == 1).count() >= 2);
    // Two branches are live at the merge, so the graph is at least two lanes wide.
    assert!(merge.width >= 2);

    for row in &page.rows {
        assert!(row.lane < row.width, "lane must fit inside the row width");
        for segment in &row.segments {
            assert!(segment.x1 < row.width && segment.x2 < row.width);
            assert!(segment.y1 <= 2 && segment.y2 <= 2);
            assert!(segment.y1 < segment.y2, "segments must run downwards");
        }
    }

    // Refs decorate the commits they point at.
    let tips: Vec<&str> = page
        .rows
        .iter()
        .flat_map(|row| row.labels.iter().map(|l| l.name.as_str()))
        .collect();
    assert!(tips.contains(&"main"));
    assert!(tips.contains(&"topic"));
    assert!(tips.contains(&"dangling"));

    // The root commit ends its lane: nothing continues below it.
    let root = page.rows.last().unwrap();
    assert!(root.parents.is_empty());
    assert!(root.segments.iter().all(|s| s.y2 != 2 || s.x1 != root.lane));
}

#[test]
fn graph_paginates() {
    let sandbox = Sandbox::new("page");
    for i in 0..12 {
        sandbox.commit("a.txt", &format!("{i}\n"), &format!("Commit {i}"));
    }
    let page = graph::build(&sandbox.state(), 5).unwrap();
    assert_eq!(page.rows.len(), 5);
    assert!(page.has_more);
}

#[test]
fn commit_detail_counts_lines_per_file() {
    let sandbox = Sandbox::new("detail");
    sandbox.commit("a.txt", "one\ntwo\n", "First");
    sandbox.write("a.txt", "one\ntwo\nthree\n");
    sandbox.write("b.txt", "new file\n");
    sandbox.git(&["add", "--all"]);
    sandbox.git(&["commit", "-q", "-m", "Second\n\nWith a body line."]);

    let state = sandbox.state();
    let head = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    let detail = diff::commit_detail(&state, &head).unwrap();

    assert_eq!(detail.summary, "Second");
    assert_eq!(detail.body, "With a body line.");
    assert_eq!(detail.files.len(), 2);

    let a = detail.files.iter().find(|f| f.path == "a.txt").unwrap();
    assert_eq!(a.status, "modified");
    assert_eq!(a.additions, 1);
    assert_eq!(a.deletions, 0);

    let b = detail.files.iter().find(|f| f.path == "b.txt").unwrap();
    assert_eq!(b.status, "added");
    assert_eq!(b.additions, 1);

    let file_diff = diff::commit_file_diff(&state, &head, "a.txt").unwrap();
    assert_eq!(file_diff.hunks.len(), 1);
    assert!(file_diff.hunks[0]
        .lines
        .iter()
        .any(|line| line.origin == '+' && line.content == "three"));
}

#[test]
fn working_diff_reads_both_sides_of_the_index() {
    let sandbox = Sandbox::new("wdiff");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\nstaged\n");
    sandbox.git(&["add", "a.txt"]);
    sandbox.write("a.txt", "one\nstaged\nunstaged\n");

    let state = sandbox.state();
    let staged = diff::working_file_diff(&state, "a.txt", diff::Side::Staged).unwrap();
    assert!(staged
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .any(|l| l.origin == '+' && l.content == "staged"));

    let unstaged = diff::working_file_diff(&state, "a.txt", diff::Side::Unstaged).unwrap();
    assert!(unstaged
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .any(|l| l.origin == '+' && l.content == "unstaged"));
}

/// The force-push guard is only useful if it names the right commits.
#[test]
fn push_preview_reports_divergence_and_what_a_force_would_drop() {
    let sandbox = Sandbox::new("push");
    sandbox.commit("a.txt", "one\n", "Shared base");
    sandbox.commit("a.txt", "two\n", "Published commit");

    // Stand in for a remote: a bare clone that already has both commits.
    let bare = sandbox.root.parent().unwrap().join(format!(
        "gitnoob-test-push-origin-{}.git",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["branch", "--set-upstream-to=origin/main", "main"]);

    let state = sandbox.state();

    // In sync: nothing to push, no force needed.
    let clean = remote::push_preview(&state, None, false).unwrap();
    assert_eq!(clean.ahead, 0);
    assert_eq!(clean.behind, 0);
    assert!(!clean.force_needed);
    assert!(clean.will_orphan.is_empty());
    assert!(!clean.new_upstream);

    // Rewrite the published commit, which is what makes a push need a rewrite.
    sandbox.git(&["reset", "-q", "--hard", "HEAD~1"]);
    sandbox.commit("a.txt", "different\n", "Rewritten commit");

    let diverged = remote::push_preview(&state, None, false).unwrap();
    assert_eq!(diverged.remote, "origin");
    assert_eq!(diverged.branch, "main");
    assert_eq!(diverged.ahead, 1);
    assert_eq!(diverged.behind, 1);
    assert!(diverged.force_needed);
    assert_eq!(diverged.will_orphan.len(), 1);
    assert_eq!(diverged.will_orphan[0].summary, "Published commit");
    assert_eq!(diverged.will_push.len(), 1);
    assert_eq!(diverged.will_push[0].summary, "Rewritten commit");

    let _ = std::fs::remove_dir_all(&bare);
}

/// A bare repository beside the sandbox, added as `origin`.
///
/// The preview is well covered; the push it previews was not run against
/// anything until these tests. Returns the path so the caller can read the
/// remote's own refs — the only honest way to say a push arrived.
fn bare_origin(sandbox: &Sandbox, tag: &str) -> PathBuf {
    let bare = scratch(&format!("{tag}-origin")).join("origin.git");
    let arg = bare.to_string_lossy().into_owned();
    git_at(
        bare.parent().unwrap(),
        &["init", "-q", "--bare", "-b", "main", &arg],
    );
    sandbox.git(&["remote", "add", "origin", &arg]);
    bare
}

/// What the remote thinks a branch points at, or `None` when it has no such ref.
fn remote_tip(bare: &Path, branch: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", &format!("refs/heads/{branch}")])
        .current_dir(bare)
        .output()
        .expect("git should be on PATH");
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[test]
fn a_first_push_sets_the_upstream_and_lands_on_the_remote() {
    let sandbox = Sandbox::new("push-first");
    sandbox.commit("a.txt", "one\n", "First");
    let bare = bare_origin(&sandbox, "push-first");
    let state = sandbox.state();

    let out = remote::push(&state, "origin", "main", false, true).unwrap();
    assert!(out.ok, "push failed: {}", out.stderr);
    // The command is the log's teaching, so it has to read the way it would be
    // typed — and it must never carry a bare --force.
    assert!(out.argv.contains(&"--set-upstream".to_string()));
    assert!(!out.argv.iter().any(|arg| arg == "--force"));

    assert_eq!(
        remote_tip(&bare, "main").as_deref(),
        Some(sandbox.git(&["rev-parse", "HEAD"]).trim()),
        "the remote should be at the commit that was pushed"
    );
    assert_eq!(
        sandbox
            .git(&["rev-parse", "--abbrev-ref", "main@{upstream}"])
            .trim(),
        "origin/main",
        "--set-upstream should have recorded the tracking branch"
    );
}

#[test]
fn a_push_that_would_rewrite_history_is_refused_without_force() {
    let sandbox = Sandbox::new("push-refused");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.commit("a.txt", "two\n", "Published");
    let bare = bare_origin(&sandbox, "push-refused");
    let state = sandbox.state();
    assert!(
        remote::push(&state, "origin", "main", false, true)
            .unwrap()
            .ok
    );
    let published = remote_tip(&bare, "main").unwrap();

    // Rewrite the tip. The remote still has the commit that just went.
    sandbox.git(&["reset", "-q", "--hard", "HEAD~1"]);
    sandbox.commit("a.txt", "different\n", "Rewritten");

    let refused = remote::push(&state, "origin", "main", false, false).unwrap();
    assert!(!refused.ok, "a non-fast-forward push should be refused");
    assert_eq!(
        remote_tip(&bare, "main"),
        Some(published),
        "a refused push must leave the remote where it was"
    );
}

#[test]
fn a_force_push_uses_a_lease_and_takes_the_branch_back() {
    let sandbox = Sandbox::new("push-force");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.commit("a.txt", "two\n", "Published");
    let bare = bare_origin(&sandbox, "push-force");
    let state = sandbox.state();
    assert!(
        remote::push(&state, "origin", "main", false, true)
            .unwrap()
            .ok
    );

    sandbox.git(&["reset", "-q", "--hard", "HEAD~1"]);
    sandbox.commit("a.txt", "different\n", "Rewritten");

    let forced = remote::push(&state, "origin", "main", true, false).unwrap();
    assert!(forced.ok, "force push failed: {}", forced.stderr);
    assert!(
        forced.argv.contains(&"--force-with-lease".to_string()),
        "the lease is the whole safety of this operation: {:?}",
        forced.argv
    );
    assert!(
        !forced.argv.iter().any(|arg| arg == "--force"),
        "a bare --force would overwrite work the lease is there to protect"
    );
    assert_eq!(
        remote_tip(&bare, "main").as_deref(),
        Some(sandbox.git(&["rev-parse", "HEAD"]).trim())
    );
}

/// The lease is not decoration: a remote that moved since the last fetch has to
/// stop even a force push.
#[test]
fn a_force_push_is_refused_when_the_remote_moved_behind_our_back() {
    let sandbox = Sandbox::new("push-lease");
    sandbox.commit("a.txt", "one\n", "Base");
    let bare = bare_origin(&sandbox, "push-lease");
    let state = sandbox.state();
    assert!(
        remote::push(&state, "origin", "main", false, true)
            .unwrap()
            .ok
    );

    // Somebody else pushes. We never fetch, so our lease is stale.
    let theirs = scratch("push-lease-other");
    let clone = theirs.join("clone");
    git_at(
        &theirs,
        &[
            "clone",
            "-q",
            bare.to_string_lossy().as_ref(),
            clone.to_string_lossy().as_ref(),
        ],
    );
    git_at(&clone, &["config", "user.name", "Other"]);
    git_at(&clone, &["config", "user.email", "other@example.com"]);
    git_at(&clone, &["config", "commit.gpgsign", "false"]);
    std::fs::write(clone.join("b.txt"), "theirs\n").unwrap();
    git_at(&clone, &["add", "--all"]);
    git_at(&clone, &["commit", "-q", "-m", "Their commit"]);
    git_at(&clone, &["push", "-q", "origin", "main"]);
    let theirs_tip = remote_tip(&bare, "main").unwrap();

    sandbox.commit("a.txt", "ours\n", "Our commit");
    let refused = remote::push(&state, "origin", "main", true, false).unwrap();
    assert!(
        !refused.ok,
        "the lease should refuse a force push over a remote we have not seen"
    );
    assert_eq!(
        remote_tip(&bare, "main"),
        Some(theirs_tip),
        "their commit must still be the tip"
    );
}

#[test]
fn push_preview_flags_a_branch_with_no_upstream() {
    let sandbox = Sandbox::new("upstream");
    sandbox.commit("a.txt", "one\n", "First");
    let preview = remote::push_preview(&sandbox.state(), None, false).unwrap();
    assert!(preview.new_upstream);
    assert!(!preview.force_needed);
    assert_eq!(preview.will_push.len(), 1);
}

/// Sets up a merge that stops with a conflict in `a.txt`.
fn conflicted() -> Sandbox {
    let sandbox = Sandbox::new("conflict");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "theirs"]);
    sandbox.commit("a.txt", "top\ntheir middle\nbottom\n", "Their change");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "top\nour middle\nbottom\n", "Our change");

    let merged = sandbox.git_may_fail(&["-c", "merge.conflictStyle=diff3", "merge", "theirs"]);
    assert!(!merged, "the merge was supposed to conflict");
    sandbox
}

#[test]
fn parses_conflict_markers_into_sides_and_base() {
    let sandbox = conflicted();
    let state = sandbox.state();

    let files = conflict::list(&state).unwrap();
    assert_eq!(files, vec!["a.txt".to_string()]);

    let file = conflict::read(&state, "a.txt").unwrap();
    assert_eq!(file.conflict_count, 1);

    let region = file
        .blocks
        .iter()
        .find_map(|block| match block {
            conflict::Block::Conflict {
                ours,
                base,
                theirs,
                has_base,
                ..
            } => Some((ours.clone(), base.clone(), theirs.clone(), *has_base)),
            _ => None,
        })
        .expect("there should be one conflict region");

    let (ours, base, theirs, has_base) = region;
    assert_eq!(ours, vec!["our middle".to_string()]);
    assert_eq!(theirs, vec!["their middle".to_string()]);
    assert!(has_base, "diff3 style should record the merge base");
    assert_eq!(base, vec!["middle".to_string()]);

    // The agreed lines survive as context on both sides of the region.
    let context: Vec<String> = file
        .blocks
        .iter()
        .filter_map(|block| match block {
            conflict::Block::Context { lines } => Some(lines.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(context.contains(&"top".to_string()));
    assert!(context.contains(&"bottom".to_string()));
}

#[test]
fn resolution_choices_produce_the_expected_file() {
    let sandbox = conflicted();
    let state = sandbox.state();

    let ours_only = vec![conflict::Resolution {
        take_ours: true,
        take_theirs: false,
        ours_first: true,
        custom: None,
    }];
    assert_eq!(
        conflict::preview(&state, "a.txt", &ours_only).unwrap(),
        "top\nour middle\nbottom\n"
    );

    let theirs_only = vec![conflict::Resolution {
        take_ours: false,
        take_theirs: true,
        ours_first: true,
        custom: None,
    }];
    assert_eq!(
        conflict::preview(&state, "a.txt", &theirs_only).unwrap(),
        "top\ntheir middle\nbottom\n"
    );

    let both = vec![conflict::Resolution {
        take_ours: true,
        take_theirs: true,
        ours_first: true,
        custom: None,
    }];
    assert_eq!(
        conflict::preview(&state, "a.txt", &both).unwrap(),
        "top\nour middle\ntheir middle\nbottom\n"
    );

    let both_swapped = vec![conflict::Resolution {
        take_ours: true,
        take_theirs: true,
        ours_first: false,
        custom: None,
    }];
    assert_eq!(
        conflict::preview(&state, "a.txt", &both_swapped).unwrap(),
        "top\ntheir middle\nour middle\nbottom\n"
    );

    let neither = vec![conflict::Resolution {
        take_ours: false,
        take_theirs: false,
        ours_first: true,
        custom: None,
    }];
    assert_eq!(
        conflict::preview(&state, "a.txt", &neither).unwrap(),
        "top\nbottom\n"
    );

    let hand_edited = vec![conflict::Resolution {
        take_ours: true,
        take_theirs: true,
        ours_first: true,
        custom: Some(vec!["a middle we agreed on".to_string()]),
    }];
    assert_eq!(
        conflict::preview(&state, "a.txt", &hand_edited).unwrap(),
        "top\na middle we agreed on\nbottom\n"
    );
}

#[test]
fn resolving_writes_the_file_and_clears_the_conflict() {
    let sandbox = conflicted();
    let state = sandbox.state();

    conflict::resolve(
        &state,
        "a.txt",
        &[conflict::Resolution {
            take_ours: true,
            take_theirs: true,
            ours_first: true,
            custom: None,
        }],
    )
    .unwrap();

    let on_disk = std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap();
    assert_eq!(on_disk, "top\nour middle\ntheir middle\nbottom\n");
    assert!(!on_disk.contains("<<<<<<<"));
    // Resolving stages the file, which is what takes it out of the conflict list.
    assert!(conflict::list(&state).unwrap().is_empty());
    assert!(refs::status(&state).unwrap().conflicted.is_empty());

    // With nothing conflicted left, the merge can be committed.
    sandbox.git(&["commit", "-q", "--no-edit"]);
    let head = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    let detail = diff::commit_detail(&state, &head).unwrap();
    assert_eq!(
        detail.parents.len(),
        2,
        "the merge commit should have two parents"
    );
}

#[test]
fn whole_file_resolution_takes_one_side() {
    let sandbox = conflicted();
    let state = sandbox.state();
    conflict::resolve_whole(&state, "a.txt", "theirs").unwrap();

    let on_disk = std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap();
    assert_eq!(on_disk, "top\ntheir middle\nbottom\n");
    assert!(conflict::list(&state).unwrap().is_empty());
}

#[test]
fn merge_reports_its_conflicts() {
    let sandbox = Sandbox::new("mergeout");
    sandbox.commit("a.txt", "base\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "other"]);
    sandbox.commit("a.txt", "theirs\n", "Theirs");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "ours\n", "Ours");

    let state = sandbox.state();
    let outcome = remote::merge(&state, "other", false).unwrap();
    assert!(!outcome.ok);
    assert_eq!(outcome.conflicts, vec!["a.txt".to_string()]);

    // Aborting puts the working tree back the way it was.
    remote::abort_merge(&state).unwrap();
    assert!(conflict::list(&state).unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "ours\n"
    );
}

#[test]
fn amend_draft_knows_when_a_commit_is_published() {
    let sandbox = Sandbox::new("amend");
    sandbox.commit("a.txt", "one\n", "Only commit\n\nWith a body.");
    let state = sandbox.state();

    let draft = gitnoob_lib::work::amend_draft(&state).unwrap();
    assert_eq!(draft.summary, "Only commit");
    assert_eq!(draft.body, "With a body.");
    assert!(!draft.is_pushed, "nothing has been pushed yet");

    let bare = sandbox.root.parent().unwrap().join(format!(
        "gitnoob-test-amend-origin-{}.git",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);

    let published = gitnoob_lib::work::amend_draft(&state).unwrap();
    assert!(published.is_pushed, "the commit is now on a remote");

    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn stage_unstage_and_discard_move_files_around() {
    let sandbox = Sandbox::new("work");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\ntwo\n");
    let state = sandbox.state();

    gitnoob_lib::work::stage(&state, &["a.txt".to_string()]).unwrap();
    assert!(refs::status(&state)
        .unwrap()
        .staged
        .iter()
        .any(|e| e.path == "a.txt"));

    gitnoob_lib::work::unstage(&state, &["a.txt".to_string()]).unwrap();
    assert!(refs::status(&state).unwrap().staged.is_empty());

    gitnoob_lib::work::discard(&state, &["a.txt".to_string()]).unwrap();
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\n"
    );
}

#[test]
fn checkout_of_a_remote_branch_creates_a_tracking_branch() {
    let sandbox = Sandbox::new("track");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("b.txt", "two\n", "Second");
    sandbox.git(&["checkout", "-q", "main"]);

    let bare = sandbox.root.parent().unwrap().join(format!(
        "gitnoob-test-track-origin-{}.git",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["branch", "-q", "-D", "feature"]);

    let state = sandbox.state();
    refs::checkout(&state, "origin/feature").unwrap();

    let tree = refs::tree(&state).unwrap();
    let feature = tree
        .locals
        .iter()
        .find(|b| b.name == "feature")
        .expect("a local branch should have been created");
    assert!(feature.is_head);
    assert_eq!(feature.upstream.as_deref(), Some("origin/feature"));

    let _ = std::fs::remove_dir_all(&bare);
}

/// Guards against the graph choking on a repository with no commits at all.
#[test]
fn handles_an_empty_repository() {
    let sandbox = Sandbox::new("empty");
    let state = sandbox.state();

    let info = refs::describe(&state).unwrap();
    assert_eq!(info.head, "(no commits yet)");

    let page = graph::build(&state, 100).unwrap();
    assert!(page.rows.is_empty());

    let tree = refs::tree(&state).unwrap();
    assert!(tree.locals.is_empty());

    sandbox.write("a.txt", "new\n");
    let status = refs::status(&state).unwrap();
    assert_eq!(status.unstaged.len(), 1);

    // A first commit is diffable against the empty tree.
    gitnoob_lib::work::stage(&state, &["a.txt".to_string()]).unwrap();
    let staged = diff::working_file_diff(&state, "a.txt", diff::Side::Staged).unwrap();
    assert!(staged
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .any(|l| l.origin == '+'));
}

#[test]
fn detects_a_detached_head() {
    let sandbox = Sandbox::new("detached");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("a.txt", "two\n", "Second");
    sandbox.git(&["checkout", "-q", "HEAD~1"]);

    let state = sandbox.state();
    let info = refs::describe(&state).unwrap();
    assert!(info.detached);
    assert_eq!(info.head.len(), 7, "a detached HEAD shows the short hash");

    // Pushing from a detached HEAD has no branch to name, so it must be refused.
    assert!(remote::push_preview(&state, None, false).is_err());
}

/// `Path` is used only through the sandbox helpers; keep the import honest.
#[allow(dead_code)]
fn _uses_path(_: &Path) {}

// --- undo, redo, stash ------------------------------------------------------

#[test]
fn undo_and_redo_a_commit() {
    let sandbox = Sandbox::new("undo");
    sandbox.commit("a.txt", "one\n", "First");
    let base = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();

    let state = sandbox.state();
    sandbox.write("a.txt", "one\ntwo\n");
    gitnoob_lib::work::stage(&state, &["a.txt".to_string()]).unwrap();
    gitnoob_lib::work::commit(&state, "Second commit", false).unwrap();

    let after = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    assert_ne!(after, base);

    let stacks = gitnoob_lib::journal::stacks(&state);
    assert_eq!(stacks.undo.len(), 1);
    assert_eq!(stacks.undo[0].label, "Commit: Second commit");
    assert!(stacks.redo.is_empty());

    gitnoob_lib::journal::undo(&state).unwrap();
    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), base);
    // A soft reset keeps the work staged rather than throwing it away.
    let status = refs::status(&state).unwrap();
    assert!(status.staged.iter().any(|e| e.path == "a.txt"));
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );

    let stacks = gitnoob_lib::journal::stacks(&state);
    assert!(stacks.undo.is_empty());
    assert_eq!(stacks.redo.len(), 1);

    gitnoob_lib::journal::redo(&state).unwrap();
    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), after);
}

#[test]
fn undo_of_an_amend_restores_the_original_commit() {
    let sandbox = Sandbox::new("undoamend");
    sandbox.commit("a.txt", "one\n", "Original message");
    let original = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();

    let state = sandbox.state();
    gitnoob_lib::work::commit(&state, "Reworded message", true).unwrap();
    assert_ne!(sandbox.git(&["rev-parse", "HEAD"]).trim(), original);

    gitnoob_lib::journal::undo(&state).unwrap();
    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), original);
    assert_eq!(
        sandbox.git(&["log", "-1", "--format=%s"]).trim(),
        "Original message"
    );
}

#[test]
fn rewording_changes_the_message_and_nothing_else() {
    let sandbox = Sandbox::new("reword");
    sandbox.commit("a.txt", "one\n", "Frist commit");
    let tree = sandbox
        .git(&["rev-parse", "HEAD^{tree}"])
        .trim()
        .to_string();
    let author = sandbox
        .git(&["log", "-1", "--format=%an <%ae> %at"])
        .trim()
        .to_string();

    let state = sandbox.state();
    let now = work::reword(&state, "HEAD", "First commit\n\nWith a body this time").unwrap();

    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), now);
    assert_eq!(
        sandbox.git(&["log", "-1", "--format=%s"]).trim(),
        "First commit"
    );
    assert_eq!(
        sandbox.git(&["log", "-1", "--format=%b"]).trim(),
        "With a body this time"
    );
    // Same content, same authorship: only the message moved.
    assert_eq!(sandbox.git(&["rev-parse", "HEAD^{tree}"]).trim(), tree);
    assert_eq!(
        sandbox.git(&["log", "-1", "--format=%an <%ae> %at"]).trim(),
        author
    );
}

#[test]
fn rewording_leaves_staged_work_out_of_the_commit() {
    let sandbox = Sandbox::new("reword-staged");
    sandbox.commit("a.txt", "one\n", "First");

    // Something staged but not meant to be part of the commit being reworded.
    sandbox.write("b.txt", "later\n");
    let state = sandbox.state();
    work::stage(&state, &["b.txt".to_string()]).unwrap();

    work::reword(&state, "HEAD", "First, said better").unwrap();

    assert_eq!(
        sandbox.git(&["log", "-1", "--format=%s"]).trim(),
        "First, said better"
    );
    let files = sandbox.git(&["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        !files.contains("b.txt"),
        "the staged file was swept in: {files}"
    );
    let status = refs::status(&state).unwrap();
    assert!(status.staged.iter().any(|entry| entry.path == "b.txt"));
}

#[test]
fn only_the_newest_commit_can_be_reworded() {
    let sandbox = Sandbox::new("reword-old");
    sandbox.commit("a.txt", "one\n", "First");
    let first = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    sandbox.commit("a.txt", "two\n", "Second");

    let state = sandbox.state();
    let check = work::reword_check(&state, &first).unwrap();
    assert!(!check.can);
    assert_eq!(check.summary, "First");

    let error = work::reword(&state, &first, "Something else").unwrap_err();
    assert!(error.contains("newest"), "unexpected message: {error}");
    // Refusing must not have touched anything.
    assert_eq!(sandbox.git(&["log", "-1", "--format=%s"]).trim(), "Second");
}

#[test]
fn undo_of_a_reword_restores_the_original_message() {
    let sandbox = Sandbox::new("reword-undo");
    sandbox.commit("a.txt", "one\n", "Original message");
    let original = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();

    let state = sandbox.state();
    work::reword(&state, "HEAD", "Reworded message").unwrap();
    assert_ne!(sandbox.git(&["rev-parse", "HEAD"]).trim(), original);

    journal::undo(&state).unwrap();
    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), original);
    assert_eq!(
        sandbox.git(&["log", "-1", "--format=%s"]).trim(),
        "Original message"
    );
}

#[test]
fn undo_refuses_when_a_different_branch_is_checked_out() {
    let sandbox = Sandbox::new("undoguard");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();
    sandbox.commit("a.txt", "two\n", "Second");
    // Record a commit through the app so there is something to undo.
    sandbox.write("a.txt", "three\n");
    gitnoob_lib::work::stage(&state, &["a.txt".to_string()]).unwrap();
    gitnoob_lib::work::commit(&state, "Third", false).unwrap();

    sandbox.git(&["checkout", "-q", "-b", "elsewhere"]);
    let error = gitnoob_lib::journal::undo(&state).unwrap_err();
    assert!(error.contains("elsewhere"), "unexpected message: {error}");

    // Refusing must not consume the step.
    assert_eq!(gitnoob_lib::journal::stacks(&state).undo.len(), 1);
}

#[test]
fn undo_reports_when_there_is_nothing_to_do() {
    let sandbox = Sandbox::new("undoempty");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();
    assert!(gitnoob_lib::journal::undo(&state).is_err());
    assert!(gitnoob_lib::journal::redo(&state).is_err());
}

#[test]
fn switching_branches_brings_local_changes_along() {
    let sandbox = Sandbox::new("autostash");
    sandbox.commit("shared.txt", "base\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "other"]);
    sandbox.commit("other.txt", "other\n", "Other side");
    sandbox.git(&["checkout", "-q", "main"]);

    // An edit that would otherwise block the switch.
    sandbox.write("shared.txt", "base\nlocal edit\n");
    let state = sandbox.state();

    refs::checkout(&state, "other").unwrap();

    assert_eq!(refs::describe(&state).unwrap().head, "other");
    // The edit came back with us.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("shared.txt")).unwrap(),
        "base\nlocal edit\n"
    );
    // And it is not left sitting in the stash.
    assert!(gitnoob_lib::work::stash_list(&state).unwrap().is_empty());

    // Not a step in the history: switching branches is neither hard to notice
    // nor hard to reverse, and every one of them pushed something worth undoing
    // off the end of the list.
    assert!(gitnoob_lib::journal::stacks(&state).undo.is_empty());
}

/// A file long enough that two edits to it can be nowhere near each other.
const LINES: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";

/// The two branches differ in the last line of `f.txt` and nowhere else, which
/// is what git refuses a switch over as soon as that file is dirty.
fn carry_sandbox(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("f.txt", LINES, "Base");
    sandbox.git(&["checkout", "-q", "-b", "other"]);
    sandbox.commit("f.txt", &LINES.replace("eight", "EIGHT-other"), "Their end");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox
}

#[test]
fn a_change_that_does_not_collide_is_carried_across_the_switch() {
    let sandbox = carry_sandbox("carryok");
    // They changed the last line; this changes the first.
    sandbox.write("f.txt", &LINES.replace("one\n", "ONE-mine\n"));
    let state = sandbox.state();

    refs::checkout(&state, "other").unwrap();

    assert_eq!(refs::describe(&state).unwrap().head, "other");
    let after = std::fs::read_to_string(sandbox.root.join("f.txt")).unwrap();
    assert!(after.starts_with("ONE-mine"), "kept my line: {after}");
    assert!(after.contains("EIGHT-other"), "took theirs: {after}");
    // The stash it went through is gone again.
    assert!(gitnoob_lib::work::stash_list(&state).unwrap().is_empty());
}

#[test]
fn a_change_that_would_conflict_leaves_everything_where_it_was() {
    let sandbox = carry_sandbox("carryconflict");
    // The same line both sides changed, which is what cannot be carried.
    sandbox.write("f.txt", &LINES.replace("eight", "EIGHT-mine"));
    let state = sandbox.state();

    let refused = refs::checkout(&state, "other").unwrap_err();

    assert!(refused.contains("Cannot switch to other"), "{refused}");
    assert_eq!(refs::describe(&state).unwrap().head, "main");
    assert!(std::fs::read_to_string(sandbox.root.join("f.txt"))
        .unwrap()
        .contains("EIGHT-mine"));
    // Put back, not left in a stash for the user to find.
    assert!(gitnoob_lib::work::stash_list(&state).unwrap().is_empty());
}

#[test]
fn what_was_staged_is_still_staged_after_the_switch() {
    let sandbox = carry_sandbox("carrystaged");
    sandbox.write("f.txt", &LINES.replace("one\n", "ONE-staged\n"));
    sandbox.git(&["add", "f.txt"]);
    sandbox.write(
        "f.txt",
        &LINES
            .replace("one\n", "ONE-staged\n")
            .replace("three", "THREE-unstaged"),
    );
    sandbox.write("new.txt", "untracked\n");
    let state = sandbox.state();

    refs::checkout(&state, "other").unwrap();

    let status = sandbox.git(&["status", "--porcelain"]);
    // Half staged, half not, and the untracked file still untracked.
    assert!(status.contains("MM f.txt"), "{status}");
    assert!(status.contains("?? new.txt"), "{status}");
}

#[test]
fn a_stash_made_by_hand_is_not_the_one_the_switch_drops() {
    let sandbox = carry_sandbox("carrystash");
    sandbox.write("f.txt", &LINES.replace("two", "TWO-stashed"));
    sandbox.git(&["stash", "push", "-q", "-m", "mine to keep"]);
    sandbox.write("f.txt", &LINES.replace("one\n", "ONE-mine\n"));
    let state = sandbox.state();

    refs::checkout(&state, "other").unwrap();

    let left = gitnoob_lib::work::stash_list(&state).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].message, "mine to keep");
}

#[test]
fn with_auto_stash_off_the_switch_says_so_instead_of_helping() {
    let sandbox = carry_sandbox("carryoff");
    sandbox.write("f.txt", &LINES.replace("one\n", "ONE-mine\n"));
    let state = sandbox.state();
    state
        .update_config(|config| config.global.auto_stash = false)
        .unwrap();

    let refused = refs::checkout(&state, "other").unwrap_err();

    assert!(refused.contains("Cannot switch to other"), "{refused}");
    assert_eq!(refs::describe(&state).unwrap().head, "main");
    // Nothing was stashed on the way: the user asked to be told, not helped.
    assert!(gitnoob_lib::work::stash_list(&state).unwrap().is_empty());
}

#[test]
fn undoing_a_stash_puts_back_the_one_it_made() {
    let sandbox = Sandbox::new("undostash");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();

    sandbox.write("a.txt", "one\nmine\n");
    gitnoob_lib::work::stash_push(&state, Some("mine"), false).unwrap();

    // Somebody stashes something else afterwards — here, or in a terminal.
    sandbox.write("a.txt", "one\nsomething else\n");
    sandbox.git(&["stash", "push", "-q", "-m", "later"]);

    gitnoob_lib::journal::undo(&state).unwrap();

    // The stash that came back is the one that was recorded, not whatever
    // happened to be on top of the list.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\nmine\n"
    );
    let left = gitnoob_lib::work::stash_list(&state).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].message, "later");
}

#[test]
fn undoing_a_stash_says_so_once_it_is_gone() {
    let sandbox = Sandbox::new("undostashgone");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();
    sandbox.write("a.txt", "one\nmine\n");
    gitnoob_lib::work::stash_push(&state, Some("mine"), false).unwrap();
    sandbox.git(&["stash", "drop", "-q"]);

    let refused = gitnoob_lib::journal::undo(&state).unwrap_err();

    assert!(refused.contains("not in the list any more"), "{refused}");
}

/// `git stash push` exits 0 on a clean tree without creating anything — the
/// entry it would journal is whatever stash already topped the list, made by
/// someone else entirely.
#[test]
fn stashing_a_clean_tree_does_not_journal_an_unrelated_stash() {
    let sandbox = Sandbox::new("stashnoop");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();

    // A stash that already exists, made outside this call.
    sandbox.write("a.txt", "one\nsomeone else's\n");
    sandbox.git(&["stash", "push", "-q", "-m", "pre-existing"]);

    let output = gitnoob_lib::work::stash_push(&state, Some("mine"), false).unwrap();
    assert!(output.contains("No local changes"), "{output}");

    // Nothing to undo, and certainly not a pop of the pre-existing stash.
    assert!(gitnoob_lib::journal::stacks(&state).undo.is_empty());
    assert!(gitnoob_lib::journal::undo(&state).is_err());
    let left = gitnoob_lib::work::stash_list(&state).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].message, "pre-existing");
}

/// The redo half of the same hazard: stashing again after an undo, on an
/// already-clean tree, must not silently adopt whatever else is on the stash.
#[test]
fn redoing_a_stash_on_a_clean_tree_refuses_rather_than_stealing_one() {
    let sandbox = Sandbox::new("stashredoclean");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("b.txt", "one\n", "Second");
    let state = sandbox.state();

    sandbox.write("a.txt", "one\nmine\n");
    gitnoob_lib::work::stash_push(&state, Some("mine"), false).unwrap();
    let made = gitnoob_lib::journal::stacks(&state).undo[0].after.clone();

    gitnoob_lib::journal::undo(&state).unwrap();
    // Back to a clean tree, with a stash made elsewhere sitting on the list.
    gitnoob_lib::work::discard(&state, &["a.txt".to_string()]).unwrap();
    sandbox.write("b.txt", "one\nsomeone else's\n");
    sandbox.git(&["stash", "push", "-q", "-m", "unrelated"]);

    let error = gitnoob_lib::journal::redo(&state).unwrap_err();
    assert!(error.contains("nothing to stash"), "{error}");

    // Refusing must leave the step exactly as it was, for a later retry.
    let stacks = gitnoob_lib::journal::stacks(&state);
    assert_eq!(stacks.redo.len(), 1);
    assert_eq!(stacks.redo[0].after, made);

    let left = gitnoob_lib::work::stash_list(&state).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].message, "unrelated");
}

#[test]
fn undo_refuses_once_the_branch_has_moved_outside_the_history() {
    let sandbox = Sandbox::new("undomoved");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();
    sandbox.write("a.txt", "one\ntwo\n");
    gitnoob_lib::work::stage(&state, &["a.txt".to_string()]).unwrap();
    gitnoob_lib::work::commit(&state, "Second", false).unwrap();

    // A commit the app never saw: made in a terminal, in another window.
    sandbox.commit("a.txt", "one\ntwo\nthree\n", "Third, elsewhere");

    let refused = gitnoob_lib::journal::undo(&state).unwrap_err();

    assert!(refused.contains("outside this history"), "{refused}");
    // And it changed nothing.
    assert_eq!(sandbox.git(&["log", "--oneline"]).lines().count(), 3);
}

#[test]
fn renaming_a_stash_leaves_it_where_it_is_in_the_list() {
    let sandbox = Sandbox::new("stashrename");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();
    for (name, line) in [("first", "a"), ("second", "b"), ("third", "c")] {
        sandbox.write("a.txt", &format!("one\n{line}\n"));
        gitnoob_lib::work::stash_push(&state, Some(name), false).unwrap();
    }

    // The middle one: `git stash store` would have moved it to the top, which
    // is the whole reason the reflog is rewritten instead.
    gitnoob_lib::work::stash_rename(&state, 1, "renamed in place").unwrap();

    let list = gitnoob_lib::work::stash_list(&state).unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].message, "third");
    assert_eq!(list[1].message, "renamed in place");
    assert_eq!(list[2].message, "first");
    // Still on the branch it was made on.
    assert_eq!(list[1].branch.as_deref(), Some("main"));
}

#[test]
fn a_renamed_stash_still_holds_what_it_held() {
    let sandbox = Sandbox::new("stashrenamekeep");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();
    sandbox.write("a.txt", "one\nkept\n");
    gitnoob_lib::work::stash_push(&state, Some("before"), false).unwrap();

    gitnoob_lib::work::stash_rename(&state, 0, "after").unwrap();
    sandbox.git(&["stash", "pop", "-q"]);

    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\nkept\n"
    );
}

/// The message becomes one line of `logs/refs/stash`; a newline in it would
/// write an extra, malformed line and corrupt every other entry's indexing.
#[test]
fn stash_rename_rejects_a_message_with_a_line_break() {
    let sandbox = Sandbox::new("stashrenamenewline");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();
    sandbox.write("a.txt", "one\nmine\n");
    gitnoob_lib::work::stash_push(&state, Some("mine"), false).unwrap();

    let error = gitnoob_lib::work::stash_rename(&state, 0, "line one\nline two").unwrap_err();
    assert!(error.contains("line break"), "{error}");

    // Refusing must not have touched the reflog.
    let list = gitnoob_lib::work::stash_list(&state).unwrap();
    assert_eq!(list[0].message, "mine");
}

#[test]
fn deleting_new_files_leaves_the_tracked_ones_alone() {
    let sandbox = Sandbox::new("deletenew");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();
    sandbox.write("new.txt", "never seen\n");
    sandbox.write("a.txt", "one\nchanged\n");

    gitnoob_lib::work::delete_untracked(&state, &["new.txt".to_string()]).unwrap();

    assert!(!sandbox.root.join("new.txt").exists());
    // `git clean` will not touch a tracked file, which is the guard that makes
    // this safe to offer from the same place discard sits.
    let _ = gitnoob_lib::work::delete_untracked(&state, &["a.txt".to_string()]);
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\nchanged\n"
    );
}

/// A pathspec after `--` still wildmatches by default, so a file whose name
/// holds a metacharacter has to be asked for with `:(literal)` or git goes
/// looking for everything the name happens to match.
///
/// Square brackets rather than an asterisk, because `*` cannot be part of a
/// filename on Windows and this hazard is not a Unix one. `[a]` matches `a`
/// under wildmatch, so deleting the bracketed file would take the plain one
/// with it.
#[test]
fn deleting_a_file_named_with_glob_characters_does_not_take_everything_with_it() {
    let sandbox = Sandbox::new("globclean");
    sandbox.commit(
        "kept.txt", "one
", "First",
    );
    let state = sandbox.state();
    sandbox.write(
        "[a].txt",
        "literally named with brackets
",
    );
    sandbox.write(
        "a.txt",
        "should survive
",
    );
    sandbox.write(
        "also-untracked.txt",
        "should survive
",
    );

    gitnoob_lib::work::delete_untracked(&state, &["[a].txt".to_string()]).unwrap();

    assert!(!sandbox.root.join("[a].txt").exists());
    assert!(sandbox.root.join("a.txt").exists());
    assert!(sandbox.root.join("also-untracked.txt").exists());
}

/// The same hazard at its worst, which only a Unix filesystem can be asked to
/// hold: a file literally named `*`. Without the literal magic this is
/// `git clean -f -d -- '*'`, which matches — and deletes — every untracked
/// file in the repository.
#[cfg(not(windows))]
#[test]
fn deleting_a_file_named_star_does_not_clean_the_whole_work_tree() {
    let sandbox = Sandbox::new("globstar");
    sandbox.commit(
        "a.txt", "one
", "First",
    );
    let state = sandbox.state();
    sandbox.write(
        "*",
        "literally named star
",
    );
    sandbox.write(
        "also-untracked.txt",
        "should survive
",
    );

    gitnoob_lib::work::delete_untracked(&state, &["*".to_string()]).unwrap();

    assert!(!sandbox.root.join("*").exists());
    assert!(sandbox.root.join("also-untracked.txt").exists());
}

/// Same hazard for discard: `restore --worktree -- 'a[b].txt'` would otherwise
/// also discard `ab.txt`.
#[test]
fn discarding_a_path_with_glob_characters_leaves_similarly_named_files_alone() {
    let sandbox = Sandbox::new("globdiscard");
    sandbox.commit(
        "a[b].txt", "one
", "First",
    );
    sandbox.commit(
        "ab.txt", "one
", "Second",
    );
    let state = sandbox.state();
    sandbox.write(
        "a[b].txt",
        "one
edited
",
    );
    sandbox.write(
        "ab.txt",
        "one
edited
",
    );

    gitnoob_lib::work::discard(&state, &["a[b].txt".to_string()]).unwrap();

    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a[b].txt")).unwrap(),
        "one
"
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("ab.txt")).unwrap(),
        "one
edited
"
    );
}

#[test]
fn untracked_files_survive_an_auto_stash() {
    let sandbox = Sandbox::new("autostashnew");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "other"]);
    sandbox.commit("b.txt", "two\n", "Other");
    sandbox.git(&["checkout", "-q", "main"]);

    sandbox.write("brand-new.txt", "not committed anywhere\n");
    let state = sandbox.state();
    refs::checkout(&state, "other").unwrap();

    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("brand-new.txt")).unwrap(),
        "not committed anywhere\n"
    );
}

#[test]
fn stash_list_reports_branch_and_message() {
    let sandbox = Sandbox::new("stashlist");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\nedited\n");
    sandbox.write("new.txt", "fresh\n");

    let state = sandbox.state();
    gitnoob_lib::work::stash_push(&state, Some("my work in progress"), true).unwrap();

    let stashes = gitnoob_lib::work::stash_list(&state).unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0].index, 0);
    assert_eq!(stashes[0].message, "my work in progress");
    assert_eq!(stashes[0].branch.as_deref(), Some("main"));
    assert!(stashes[0].files >= 1);
    assert_eq!(stashes[0].oid.len(), 40);

    // The stash commit is diffable like any other, which is how the UI shows it.
    let oid = gitnoob_lib::work::stash_oid(&state, 0).unwrap();
    let detail = diff::commit_detail(&state, &oid).unwrap();
    assert!(detail.files.iter().any(|f| f.path == "a.txt"));
}

#[test]
fn stash_apply_keeps_the_entry_and_drop_removes_it() {
    let sandbox = Sandbox::new("stashops");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\nedited\n");

    let state = sandbox.state();
    gitnoob_lib::work::stash_push(&state, Some("keep me"), false).unwrap();
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\n"
    );

    gitnoob_lib::work::stash_apply(&state, 0).unwrap();
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\nedited\n"
    );
    // Apply, unlike pop, leaves the entry in place.
    assert_eq!(gitnoob_lib::work::stash_list(&state).unwrap().len(), 1);

    gitnoob_lib::work::stash_drop(&state, 0).unwrap();
    assert!(gitnoob_lib::work::stash_list(&state).unwrap().is_empty());
}

/// `pop`, `apply` and `drop` resolve the index they are given to a commit id
/// before acting, so a position nothing is at any more says so plainly instead
/// of surfacing git's own "ambiguous argument" text for a ref that never
/// existed as far as the caller is concerned.
#[test]
fn stash_operations_on_a_position_that_is_not_there_say_so_plainly() {
    let sandbox = Sandbox::new("stashgoneindex");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();

    let pop_error = gitnoob_lib::work::stash_pop(&state, 0).unwrap_err();
    assert!(pop_error.contains("no stash"), "{pop_error}");

    let apply_error = gitnoob_lib::work::stash_apply(&state, 0).unwrap_err();
    assert!(apply_error.contains("no stash"), "{apply_error}");

    let drop_error = gitnoob_lib::work::stash_drop(&state, 0).unwrap_err();
    assert!(drop_error.contains("no stash"), "{drop_error}");
}

#[test]
fn a_stash_can_become_a_branch() {
    let sandbox = Sandbox::new("stashbranch");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\nwork\n");

    let state = sandbox.state();
    gitnoob_lib::work::stash_push(&state, Some("rescue this"), false).unwrap();
    gitnoob_lib::work::stash_branch(&state, 0, "rescued").unwrap();

    assert_eq!(refs::describe(&state).unwrap().head, "rescued");
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\nwork\n"
    );
    // `git stash branch` consumes the entry.
    assert!(gitnoob_lib::work::stash_list(&state).unwrap().is_empty());
}

/// A branch name beginning with `-` is real once it exists — a fetch can hand
/// one back as `origin/-f` — and without `--end-of-options` git parses it as a
/// flag rather than the name to give the rescued branch.
#[test]
fn stash_branch_treats_a_dash_led_name_as_a_name_not_a_flag() {
    let sandbox = Sandbox::new("stashbranchdash");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\nwork\n");
    let state = sandbox.state();
    gitnoob_lib::work::stash_push(&state, Some("rescue this"), false).unwrap();

    let error = gitnoob_lib::work::stash_branch(&state, 0, "-weird").unwrap_err();
    // It should fail on git's branch-name validation, not on option parsing —
    // proof the name reached the right argument slot.
    assert!(!error.contains("unknown switch"), "{error}");
    assert!(!error.contains("unknown option"), "{error}");
}

#[test]
fn undo_restore_recovers_from_a_conflicted_auto_stash_pop() {
    let sandbox = carry_sandbox("undorestoreok");
    // The same line both sides changed, which is what a plain pop conflicts on.
    sandbox.write("f.txt", &LINES.replace("eight", "EIGHT-mine"));
    let state = sandbox.state();

    let held = gitnoob_lib::work::stash_before(&state, "testing").unwrap();
    sandbox.git(&["checkout", "-q", "other"]);
    gitnoob_lib::work::restore_after(&state, held).unwrap_err();

    gitnoob_lib::work::undo_restore(&state).unwrap();

    assert_eq!(refs::describe(&state).unwrap().head, "main");
    assert!(std::fs::read_to_string(sandbox.root.join("f.txt"))
        .unwrap()
        .contains("EIGHT-mine"));
    assert!(gitnoob_lib::work::stash_list(&state).unwrap().is_empty());
}

/// A hard reset guarded only by "the top stash looks like an auto-stash" would
/// throw away anything typed since a failed restore, because that new work
/// exists nowhere else — not even in the stash it is being confused for.
#[test]
fn undo_restore_refuses_to_throw_away_changes_the_stash_never_made() {
    let sandbox = carry_sandbox("undorestoreguard");
    sandbox.write("f.txt", &LINES.replace("eight", "EIGHT-mine"));
    let state = sandbox.state();

    let held = gitnoob_lib::work::stash_before(&state, "testing").unwrap();
    sandbox.git(&["checkout", "-q", "other"]);
    gitnoob_lib::work::restore_after(&state, held).unwrap_err();

    // New work since the failed restore, in a file the stash never touched.
    sandbox.write("unrelated.txt", "brand new, never stashed\n");

    let error = gitnoob_lib::work::undo_restore(&state).unwrap_err();
    assert!(error.contains("unrelated.txt"), "{error}");

    // Refusing must not have thrown anything away.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("unrelated.txt")).unwrap(),
        "brand new, never stashed\n"
    );
    assert_eq!(gitnoob_lib::work::stash_list(&state).unwrap().len(), 1);
}

#[test]
fn pull_stashes_local_work_and_puts_it_back() {
    let sandbox = Sandbox::new("pullstash");
    sandbox.commit("a.txt", "one\n", "First");

    // A remote that has moved on.
    let bare = sandbox.root.parent().unwrap().join(format!(
        "gitnoob-test-pull-origin-{}.git",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["branch", "--set-upstream-to=origin/main", "main"]);

    let clone = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitnoob-test-pull-clone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&clone);
    Command::new("git")
        .args(["clone", "-q", &bare_arg, clone.to_str().unwrap()])
        .output()
        .unwrap();
    for args in [
        vec!["config", "user.name", "Other"],
        vec!["config", "user.email", "other@example.com"],
    ] {
        Command::new("git")
            .args(&args)
            .current_dir(&clone)
            .output()
            .unwrap();
    }
    std::fs::write(clone.join("remote-side.txt"), "from the remote\n").unwrap();
    for args in [
        vec!["add", "--all"],
        vec!["commit", "-q", "-m", "Remote side work"],
        vec!["push", "-q", "origin", "main"],
    ] {
        Command::new("git")
            .args(&args)
            .current_dir(&clone)
            .output()
            .unwrap();
    }

    // Local edit that would make a plain pull refuse.
    sandbox.write("a.txt", "one\nlocal edit\n");
    let state = sandbox.state();
    let output = remote::pull(&state, false).unwrap();

    assert!(
        output.ok,
        "pull failed: {} {}",
        output.stdout, output.stderr
    );
    // Both the remote commit and the local edit are present.
    assert!(sandbox.root.join("remote-side.txt").exists());
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\nlocal edit\n"
    );
    assert!(gitnoob_lib::work::stash_list(&state).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&bare);
    let _ = std::fs::remove_dir_all(&clone);
}

#[test]
fn parses_forge_remotes_and_reports_status() {
    let sandbox = Sandbox::new("forge");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&[
        "remote",
        "add",
        "origin",
        "git@gitlab.bigbridge.nl:team/sub/app.git",
    ]);

    let state = sandbox.state();
    let status = gitnoob_lib::forge::status(&state);
    let slug = status.slug.expect("the remote should have parsed");
    assert_eq!(slug.host, "gitlab.bigbridge.nl");
    assert_eq!(slug.owner, "team/sub");
    assert_eq!(slug.name, "app");
    // A fresh config has no forge configured, so nothing is claimed to work.
    assert!(!status.has_token);
}

#[test]
fn stages_and_discards_one_hunk_at_a_time() {
    let sandbox = Sandbox::new("hunks");
    // Two changed regions far enough apart that git keeps them separate.
    let original: String = (1..=30).map(|n| format!("line {n}\n")).collect();
    sandbox.commit("a.txt", &original, "Base");

    let edited = original
        .replace("line 2\n", "line 2\nfirst addition\n")
        .replace("line 25\n", "line 25\nsecond addition\n");
    sandbox.write("a.txt", &edited);

    let state = sandbox.state();

    // Stage only the first region.
    gitnoob_lib::work::apply_hunk(
        &state,
        "a.txt",
        0,
        gitnoob_lib::work::HunkAction::Stage,
        None,
    )
    .unwrap();

    let staged = diff::working_file_diff(&state, "a.txt", diff::Side::Staged).unwrap();
    let staged_added: Vec<String> = staged
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| l.origin == '+')
        .map(|l| l.content.clone())
        .collect();
    assert_eq!(staged_added, vec!["first addition".to_string()]);

    let unstaged = diff::working_file_diff(&state, "a.txt", diff::Side::Unstaged).unwrap();
    let unstaged_added: Vec<String> = unstaged
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| l.origin == '+')
        .map(|l| l.content.clone())
        .collect();
    assert_eq!(unstaged_added, vec!["second addition".to_string()]);

    // Take it back out again.
    gitnoob_lib::work::apply_hunk(
        &state,
        "a.txt",
        0,
        gitnoob_lib::work::HunkAction::Unstage,
        None,
    )
    .unwrap();
    assert!(refs::status(&state).unwrap().staged.is_empty());

    // Discarding the remaining region leaves the other edit alone.
    let before = std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap();
    assert!(before.contains("first addition") && before.contains("second addition"));
    gitnoob_lib::work::apply_hunk(
        &state,
        "a.txt",
        1,
        gitnoob_lib::work::HunkAction::Discard,
        None,
    )
    .unwrap();
    let after = std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap();
    assert!(after.contains("first addition"));
    assert!(!after.contains("second addition"));
}

#[test]
fn hunk_staging_refuses_when_there_is_nothing_to_stage() {
    let sandbox = Sandbox::new("hunksempty");
    sandbox.commit("a.txt", "one\n", "Base");
    let state = sandbox.state();
    let error = gitnoob_lib::work::apply_hunk(
        &state,
        "a.txt",
        0,
        gitnoob_lib::work::HunkAction::Stage,
        None,
    )
    .unwrap_err();
    assert!(error.contains("No unstaged changes"), "unexpected: {error}");
}

#[test]
fn cherry_picking_several_commits_applies_them_oldest_first() {
    let sandbox = Sandbox::new("cherrymany");
    sandbox.commit("base.txt", "base\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("a.txt", "one\ntwo\n", "Second");
    sandbox.commit("a.txt", "one\ntwo\nthree\n", "Third");

    let first = sandbox.git(&["rev-parse", "feature~2"]).trim().to_string();
    let second = sandbox.git(&["rev-parse", "feature~1"]).trim().to_string();
    let third = sandbox.git(&["rev-parse", "feature"]).trim().to_string();

    sandbox.git(&["checkout", "-q", "main"]);
    let state = sandbox.state();

    // Newest first on purpose: applying them in this order would conflict, so a
    // clean result is the proof that they were reordered.
    gitnoob_lib::work::cherry_pick(
        &state,
        &[third, second, first],
        gitnoob_lib::work::CherryPickOptions::default(),
    )
    .unwrap();

    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\ntwo\nthree\n"
    );
    let log = sandbox.git(&["log", "--format=%s", "-3"]);
    let subjects: Vec<&str> = log.lines().collect();
    assert_eq!(subjects, vec!["Third", "Second", "First"]);
}

#[test]
fn cherry_picking_without_committing_leaves_the_change_staged() {
    let sandbox = Sandbox::new("cherrynocommit");
    sandbox.commit("base.txt", "base\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("a.txt", "one\n", "First");
    let oid = sandbox.git(&["rev-parse", "feature"]).trim().to_string();

    sandbox.git(&["checkout", "-q", "main"]);
    let state = sandbox.state();
    let head_before = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();

    gitnoob_lib::work::cherry_pick(
        &state,
        &[oid],
        gitnoob_lib::work::CherryPickOptions {
            no_commit: true,
            record_origin: false,
        },
    )
    .unwrap();

    // Nothing was committed, but the change is in the index ready to be.
    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), head_before);
    let status = refs::status(&state).unwrap();
    assert!(status.staged.iter().any(|e| e.path == "a.txt"));
}

#[test]
fn recording_the_origin_names_the_commit_it_came_from() {
    let sandbox = Sandbox::new("cherryx");
    sandbox.commit("base.txt", "base\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("a.txt", "one\n", "First");
    let oid = sandbox.git(&["rev-parse", "feature"]).trim().to_string();

    sandbox.git(&["checkout", "-q", "main"]);
    let state = sandbox.state();
    gitnoob_lib::work::cherry_pick(
        &state,
        std::slice::from_ref(&oid),
        gitnoob_lib::work::CherryPickOptions {
            no_commit: false,
            record_origin: true,
        },
    )
    .unwrap();

    let message = sandbox.git(&["log", "-1", "--format=%B"]);
    assert!(
        message.contains(&format!("(cherry picked from commit {oid})")),
        "unexpected message: {message}"
    );
}

#[test]
fn cherry_picking_nothing_is_refused() {
    let sandbox = Sandbox::new("cherrynone");
    sandbox.commit("a.txt", "one\n", "Base");
    let state = sandbox.state();
    let error = gitnoob_lib::work::cherry_pick(
        &state,
        &[],
        gitnoob_lib::work::CherryPickOptions::default(),
    )
    .unwrap_err();
    assert!(error.contains("No commits"), "unexpected: {error}");
}

#[test]
fn creating_a_branch_reports_what_it_did() {
    let sandbox = Sandbox::new("branchmsg");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();

    // `git checkout -b` writes its confirmation to stderr, so a bare stdout
    // would be empty here — and a caller testing the result for truth would
    // read that as failure.
    let message = refs::create_branch(&state, "feature", None, true).unwrap();
    assert!(
        !message.trim().is_empty(),
        "expected a message, got {message:?}"
    );
    assert!(message.contains("feature"));
    assert_eq!(refs::describe(&state).unwrap().head, "feature");
}

#[test]
fn the_graph_marks_commits_the_upstream_does_not_have() {
    let sandbox = Sandbox::new("unpushed");
    sandbox.commit("a.txt", "one\n", "Shared");

    let bare = sandbox.root.parent().unwrap().join(format!(
        "gitnoob-test-unpushed-origin-{}.git",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["branch", "-q", "--set-upstream-to=origin/main", "main"]);

    // Two commits made after the clone, so the bare remote has neither.
    sandbox.commit("a.txt", "one\ntwo\n", "Local only");
    sandbox.commit("a.txt", "one\ntwo\nthree\n", "Local only as well");

    let page = graph::build(&sandbox.state(), 500).unwrap();
    let unpushed: Vec<&str> = page
        .rows
        .iter()
        .filter(|row| row.unpushed)
        .map(|row| row.summary.as_str())
        .collect();
    assert_eq!(unpushed, vec!["Local only as well", "Local only"]);

    // The commit the remote already has is not marked.
    let shared = page.rows.iter().find(|r| r.summary == "Shared").unwrap();
    assert!(!shared.unpushed);

    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn the_checked_out_branch_is_the_only_label_marked_as_head() {
    let sandbox = Sandbox::new("headlabel");
    sandbox.commit("a.txt", "one\n", "First");
    // Two branches on the same commit: the label has to tell them apart.
    sandbox.git(&["branch", "other"]);

    let page = graph::build(&sandbox.state(), 500).unwrap();
    let row = page.rows.first().unwrap();
    let heads: Vec<&str> = row
        .labels
        .iter()
        .filter(|l| l.head)
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(heads, vec!["main"]);
}

#[test]
fn repository_info_reports_the_identity_a_commit_would_use() {
    let sandbox = Sandbox::new("author");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["config", "--local", "user.name", "AVMG20"]);

    let info = refs::describe(&sandbox.state()).unwrap();
    assert_eq!(info.author, "AVMG20");
}

#[test]
fn deleting_a_merged_branch_costs_nothing() {
    let sandbox = Sandbox::new("delmerged");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("b.txt", "two\n", "Topic work");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["merge", "-q", "--no-ff", "-m", "Merge topic", "topic"]);

    let preview = refs::deletion_preview(&sandbox.state(), "topic").unwrap();
    assert!(preview.merged, "HEAD can reach it, so nothing is lost");
    assert_eq!(preview.unpushed, 0);
    assert_eq!(preview.only_here, 0);
    assert!(preview.remote.is_none());
    assert!(preview.other_remotes.is_empty());
    assert!(!preview.is_head);
}

#[test]
fn deleting_an_unmerged_branch_is_flagged() {
    let sandbox = Sandbox::new("delunmerged");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("b.txt", "two\n", "Only here");
    sandbox.git(&["checkout", "-q", "main"]);

    let preview = refs::deletion_preview(&sandbox.state(), "topic").unwrap();
    assert!(!preview.merged, "the commit is reachable from nowhere else");
    assert!(preview.upstream.is_none());
    assert!(preview.also_on.is_empty());
    assert_eq!(preview.only_here, 1, "what the delete would cost");
}

#[test]
fn a_branch_that_also_lives_on_a_remote_is_reported() {
    let sandbox = Sandbox::new("delremote");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("b.txt", "two\n", "Topic work");

    let bare = sandbox.root.parent().unwrap().join(format!(
        "gitnoob-test-delremote-origin-{}.git",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["branch", "-q", "--set-upstream-to=origin/topic", "topic"]);
    // One commit made after the clone, so the remote copy is behind.
    sandbox.commit("c.txt", "three\n", "Not pushed");
    sandbox.git(&["checkout", "-q", "main"]);

    let preview = refs::deletion_preview(&sandbox.state(), "topic").unwrap();
    let remote = preview.remote.expect("origin carries a copy");
    assert_eq!(remote.name, "origin/topic");
    assert_eq!(remote.remote, "origin");
    assert_eq!(remote.unmerged, 1, "the topic commit main cannot reach");
    assert!(preview.other_remotes.is_empty());
    assert_eq!(preview.upstream.as_deref(), Some("origin/topic"));
    assert_eq!(preview.unpushed, 1, "the commit made after the clone");

    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn the_main_branch_is_guessed_and_can_be_named() {
    let sandbox = Sandbox::new("trunk");
    sandbox.commit("a.txt", "one\n", "Base");

    let state = sandbox.state();
    let found = refs::trunk(&state);
    assert_eq!(found.name.as_deref(), Some("main"));
    assert!(
        !found.chosen,
        "nobody said so; it was guessed from the name"
    );

    sandbox.git(&["branch", "trunk-by-another-name"]);
    refs::set_trunk(&state, Some("trunk-by-another-name")).unwrap();
    let found = refs::trunk(&state);
    assert_eq!(found.name.as_deref(), Some("trunk-by-another-name"));
    assert!(found.chosen);
    // Kept in the repository's own config, where git itself can read it.
    assert_eq!(
        sandbox.git(&["config", "--get", "gitnoob.trunk"]).trim(),
        "trunk-by-another-name"
    );

    // A name nothing answers to is refused rather than quietly stored.
    assert!(refs::set_trunk(&state, Some("no-such-branch")).is_err());

    refs::set_trunk(&state, None).unwrap();
    assert_eq!(refs::trunk(&state).name.as_deref(), Some("main"));
}

#[test]
fn a_chosen_main_branch_that_has_gone_falls_back_to_the_usual_names() {
    let sandbox = Sandbox::new("trunk-gone");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["branch", "release"]);

    let state = sandbox.state();
    refs::set_trunk(&state, Some("release")).unwrap();
    sandbox.git(&["branch", "-D", "release"]);

    let found = refs::trunk(&state);
    assert_eq!(found.name.as_deref(), Some("main"));
    assert!(!found.chosen, "the stored name no longer names anything");
}

#[test]
fn deletion_is_measured_against_the_main_branch_not_the_one_checked_out() {
    let sandbox = Sandbox::new("delagainst");
    sandbox.commit("a.txt", "one\n", "Base");
    // Work that reaches a branch that gets reset, and never reaches main.
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("b.txt", "two\n", "Topic work");
    sandbox.git(&["checkout", "-q", "-b", "staging", "main"]);
    sandbox.git(&["merge", "-q", "--no-ff", "-m", "Merge topic", "topic"]);

    let state = sandbox.state();
    let preview = refs::deletion_preview(&state, "topic").unwrap();

    // Standing on staging, which holds every commit — the old answer, and the
    // wrong one: main has never seen this work.
    assert!(
        preview.merged,
        "staging can reach it, so git -d would allow it"
    );
    assert_eq!(preview.against.as_deref(), Some("main"));
    assert!(!preview.trunk_holds, "main does not hold the work");
    assert_eq!(preview.only_here, 1, "one commit main cannot reach");
    assert_eq!(
        preview.also_on,
        vec!["staging".to_string()],
        "the branch that holds it is named, not counted as safety"
    );

    // Once it lands on main the answer changes, and staging stops being the
    // reason for it.
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["merge", "-q", "--no-ff", "-m", "Merge topic", "topic"]);
    let preview = refs::deletion_preview(&state, "topic").unwrap();
    assert!(preview.trunk_holds);
    assert_eq!(preview.only_here, 0);
}

#[test]
fn the_checked_out_branch_is_reported_as_such() {
    let sandbox = Sandbox::new("delhead");
    sandbox.commit("a.txt", "one\n", "Base");
    // A second branch on the same commit must not be mistaken for HEAD.
    sandbox.git(&["branch", "sibling"]);

    let state = sandbox.state();
    assert!(refs::deletion_preview(&state, "main").unwrap().is_head);
    assert!(!refs::deletion_preview(&state, "sibling").unwrap().is_head);
}

#[test]
fn switching_branches_leaves_untouched_edits_alone() {
    let sandbox = Sandbox::new("switchclean");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "other"]);
    sandbox.commit("b.txt", "other\n", "Only on other");
    sandbox.git(&["checkout", "-q", "main"]);

    // An edit to a file neither branch changes: git can carry it across, so
    // there is no reason to stash and pop it.
    sandbox.write("free.txt", "work in progress\n");
    sandbox.git(&["add", "free.txt"]);

    let state = sandbox.state();
    refs::checkout(&state, "other").unwrap();

    assert_eq!(refs::describe(&state).unwrap().head, "other");
    // Still staged, not round-tripped through a stash, which would have made it
    // unstaged.
    let status = refs::status(&state).unwrap();
    assert!(status.staged.iter().any(|e| e.path == "free.txt"));
    assert!(
        sandbox.git(&["stash", "list"]).trim().is_empty(),
        "nothing was stashed"
    );
}

#[test]
fn switching_branches_is_refused_when_an_edit_is_in_the_way() {
    let sandbox = Sandbox::new("switchcollide");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "other"]);
    sandbox.commit("a.txt", "other version\n", "Change a.txt on other");
    sandbox.git(&["checkout", "-q", "main"]);

    // An edit to the very file the other branch changes. Stashing it and
    // putting it back would land in a conflicted tree with no merge to abort,
    // which is not a place a click on a branch name should lead.
    sandbox.write("a.txt", "my own edit\n");

    let state = sandbox.state();
    let error = refs::checkout(&state, "other").unwrap_err();

    assert!(error.contains("1 file"), "should count them: {error}");
    assert!(error.contains("a.txt"), "should name it: {error}");
    assert!(
        error.contains("Commit, stash, or discard"),
        "should say what to do: {error}"
    );
    assert_eq!(
        refs::describe(&state).unwrap().head,
        "main",
        "a refused switch stays put"
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "my own edit\n",
        "the edit is left exactly as it was"
    );
    assert!(
        sandbox.git(&["stash", "list"]).trim().is_empty(),
        "nothing should have been stashed behind the user's back"
    );
}

#[test]
fn switching_branches_with_nothing_open_just_switches() {
    let sandbox = Sandbox::new("switchclean");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "other"]);
    sandbox.commit("a.txt", "other version\n", "Change a.txt on other");
    sandbox.git(&["checkout", "-q", "main"]);

    let state = sandbox.state();
    refs::checkout(&state, "other").unwrap();

    assert_eq!(refs::describe(&state).unwrap().head, "other");
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "other version\n"
    );
}

// --- the moves gitnoob offers on top of plain git -------------------------

#[test]
fn branch_relation_tells_the_menu_what_is_possible() {
    let sandbox = Sandbox::new("relation");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("b.txt", "two\n", "Ahead by one");
    sandbox.git(&["checkout", "-q", "main"]);
    let state = sandbox.state();

    // main can simply be moved forward to topic: nothing of its own in the way.
    let forward = remote::relation(&state, "topic", "main").unwrap();
    assert_eq!((forward.ahead, forward.behind), (1, 0));
    assert!(forward.fast_forward());
    assert!(!forward.merged());

    // The other direction has nothing to bring over.
    let back = remote::relation(&state, "main", "topic").unwrap();
    assert_eq!((back.ahead, back.behind), (0, 1));
    assert!(back.merged());
    assert!(!back.fast_forward());

    // Once main has a commit of its own, neither is a fast-forward.
    sandbox.commit("c.txt", "three\n", "Its own commit");
    let diverged = remote::relation(&state, "topic", "main").unwrap();
    assert_eq!((diverged.ahead, diverged.behind), (1, 1));
    assert!(!diverged.fast_forward());
    assert!(!diverged.merged());
}

#[test]
fn relation_refuses_a_name_that_is_not_there() {
    let sandbox = Sandbox::new("relationmissing");
    sandbox.commit("a.txt", "one\n", "Base");
    let error = remote::relation(&sandbox.state(), "nope", "main").unwrap_err();
    assert!(error.contains("nope"), "unexpected: {error}");
}

#[test]
fn reverting_adds_a_commit_rather_than_removing_one() {
    let sandbox = Sandbox::new("revert");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.commit("a.txt", "one\ntwo\n", "Add two");
    let target = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();

    let state = sandbox.state();
    gitnoob_lib::work::revert(&state, &target).unwrap();

    // The file is back to its earlier content, and history grew.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\n"
    );
    assert_eq!(sandbox.git(&["rev-list", "--count", "HEAD"]).trim(), "3");
    // The reverted commit is still reachable; nothing was rewritten.
    assert!(sandbox.git(&["log", "--format=%s"]).contains("Add two"));
}

#[test]
fn a_rebase_that_conflicts_can_be_abandoned() {
    let sandbox = Sandbox::new("rebaseabort");
    sandbox.commit("a.txt", "base\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("a.txt", "topic\n", "Topic edit");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "main\n", "Main edit");
    sandbox.git(&["checkout", "-q", "topic"]);

    let state = sandbox.state();
    let outcome = remote::rebase(&state, "main").unwrap();
    assert!(!outcome.ok);
    assert_eq!(outcome.conflicts, vec!["a.txt".to_string()]);

    // While it is stopped, the app can say what git is part-way through.
    let during = remote::in_progress(&state).unwrap();
    assert!(during.rebasing, "a stopped rebase should be reported");

    remote::abort_rebase(&state).unwrap();
    assert!(!remote::in_progress(&state).unwrap().rebasing);
    assert_eq!(refs::describe(&state).unwrap().head, "topic");
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "topic\n",
        "aborting puts the branch back the way it was"
    );
}

#[test]
fn a_resolved_rebase_can_be_continued() {
    let sandbox = Sandbox::new("rebasecontinue");
    sandbox.commit("a.txt", "base\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("a.txt", "topic\n", "Topic edit");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "main\n", "Main edit");
    sandbox.git(&["checkout", "-q", "topic"]);

    let state = sandbox.state();
    assert!(!remote::rebase(&state, "main").unwrap().ok);

    // Resolve by hand the way the conflict view would, then carry on.
    sandbox.write("a.txt", "resolved\n");
    sandbox.git(&["add", "a.txt"]);
    let outcome = remote::continue_rebase(&state).unwrap();
    assert!(outcome.ok, "unexpected: {}", outcome.message);
    assert!(!remote::in_progress(&state).unwrap().rebasing);
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "resolved\n"
    );
}

#[test]
fn nothing_in_progress_is_reported_as_nothing() {
    let sandbox = Sandbox::new("idle");
    sandbox.commit("a.txt", "one\n", "Base");
    let state = sandbox.state();
    let idle = remote::in_progress(&state).unwrap();
    assert!(!idle.merging && !idle.rebasing && !idle.cherry_picking && !idle.reverting);
}

/// A stash that will not go back on, which is what a hand-made `stash apply`
/// leaves behind: files conflicted, nothing running, and no auto-stash.
fn stash_conflict() -> Sandbox {
    let sandbox = Sandbox::new("stash-clash");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.write("a.txt", "top\nstashed middle\nbottom\n");
    sandbox.git(&["stash", "push", "-q", "-m", "work in hand"]);
    sandbox.commit("a.txt", "top\ncommitted middle\nbottom\n", "Moved on");

    // Through the app's own apply rather than raw git: that is what writes
    // down which stash left the tree in this state.
    assert!(
        work::stash_apply(&sandbox.state(), 0).is_err(),
        "the apply was supposed to conflict"
    );
    sandbox
}

#[test]
fn conflicts_with_nothing_to_abort_are_not_offered_an_undo() {
    let sandbox = stash_conflict();
    let state = sandbox.state();
    let stuck = remote::in_progress(&state).unwrap();

    // Nothing is running, so there is no abort — and the stash on the list is
    // the user's own, not one this app made, so there is no switch to undo.
    assert!(!stuck.merging && !stuck.rebasing && !stuck.cherry_picking && !stuck.reverting);
    assert!(
        !stuck.restoring,
        "only an auto-stash left by a switch can be put back"
    );
}

#[test]
fn throwing_the_conflicts_away_leaves_what_the_branch_had() {
    let sandbox = stash_conflict();
    let state = sandbox.state();
    assert_eq!(conflict::list(&state).unwrap(), vec!["a.txt".to_string()]);

    conflict::discard(&state, &["a.txt".to_string()]).unwrap();

    // The committed side is back, whole, and git is no longer part-way
    // through anything.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "top\ncommitted middle\nbottom\n"
    );
    assert!(conflict::list(&state).unwrap().is_empty());
    let status = refs::status(&state).unwrap();
    assert!(status.staged.is_empty() && status.unstaged.is_empty());
}

#[test]
fn throwing_away_a_conflict_the_branch_never_had_removes_the_file() {
    let sandbox = Sandbox::new("delete-clash");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.commit("gone.txt", "the other side keeps this\n", "A file to argue over");
    sandbox.git(&["checkout", "-q", "-b", "theirs"]);
    sandbox.commit("gone.txt", "the other side edits it\n", "Their edit");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["rm", "-q", "gone.txt"]);
    sandbox.git(&["commit", "-q", "-m", "We deleted it"]);

    let merged = sandbox.git_may_fail(&["merge", "theirs"]);
    assert!(!merged, "the merge was supposed to conflict");

    // Deleted by us: the path is unmerged but HEAD has no copy to restore
    // from, so throwing it away means the file goes.
    let state = sandbox.state();
    assert_eq!(conflict::list(&state).unwrap(), vec!["gone.txt".to_string()]);
    conflict::discard(&state, &["gone.txt".to_string()]).unwrap();
    assert!(conflict::list(&state).unwrap().is_empty());
    assert!(!sandbox.root.join("gone.txt").exists());
}

#[test]
fn a_stash_that_would_not_go_on_can_be_taken_back_off() {
    let sandbox = stash_conflict();
    let state = sandbox.state();

    // The apply is remembered while its mess is here, so the way out survives
    // the window being closed and reopened.
    let stuck = remote::in_progress(&state).unwrap();
    assert!(stuck.applied_stash.is_some());
    assert!(!stuck.restoring, "there is no switch here, only an apply");

    let said = work::undo_stash_apply(&state).unwrap();
    assert!(said.contains("still on the list"), "{said}");

    // The tree reads as it did before the apply, and the stash is untouched.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "top\ncommitted middle\nbottom\n"
    );
    let after = refs::status(&state).unwrap();
    assert!(after.staged.is_empty() && after.unstaged.is_empty() && after.conflicted.is_empty());
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);

    // And with nothing conflicted the offer goes with it.
    assert!(remote::in_progress(&state).unwrap().applied_stash.is_none());
}

#[test]
fn undoing_an_apply_leaves_staged_work_beside_it_alone() {
    let sandbox = stash_conflict();
    // A file the stash never touched, staged while the conflict stood. Undoing
    // the apply is about the apply's own paths and nothing else.
    sandbox.write("other.txt", "typed while stuck\n");
    sandbox.git(&["add", "other.txt"]);
    let state = sandbox.state();

    work::undo_stash_apply(&state).unwrap();

    assert!(conflict::list(&state).unwrap().is_empty());
    let after = refs::status(&state).unwrap();
    assert_eq!(
        after
            .staged
            .iter()
            .map(|e| e.path.clone())
            .collect::<Vec<_>>(),
        vec!["other.txt".to_string()]
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("other.txt")).unwrap(),
        "typed while stuck\n"
    );
}

#[test]
fn undoing_an_apply_leaves_an_unstaged_edit_beside_it_alone() {
    let sandbox = Sandbox::new("apply-loose");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.commit("notes.md", "as committed\n", "A file beside it");
    sandbox.write("a.txt", "top\nstashed middle\nbottom\n");
    sandbox.git(&["stash", "push", "-q", "-m", "work in hand"]);
    sandbox.commit("a.txt", "top\ncommitted middle\nbottom\n", "Moved on");

    let state = sandbox.state();
    assert!(work::stash_apply(&state, 0).is_err());

    // Not staged, and not the stash's doing either. A reset of the whole tree
    // would have taken it; only the apply's own paths are put back.
    sandbox.write("notes.md", "edited while stuck\n");

    work::undo_stash_apply(&state).unwrap();

    assert!(conflict::list(&state).unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("notes.md")).unwrap(),
        "edited while stuck\n"
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "top\ncommitted middle\nbottom\n"
    );
}

#[test]
fn work_the_stash_would_land_on_stops_the_apply_before_it_starts() {
    let sandbox = Sandbox::new("apply-onto-dirty");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.write("a.txt", "top\nstashed\nbottom\n");
    sandbox.git(&["stash", "push", "-q", "-m", "work in hand"]);
    // The same file, edited again since. Git refuses rather than merging into
    // it, which is what keeps the undo's rule — everything dirty is the
    // apply's doing — true in the first place.
    sandbox.write("a.txt", "top\nedited since\nbottom\n");

    let state = sandbox.state();
    assert!(work::stash_apply(&state, 0).is_err());

    // Nothing was applied, nothing is conflicted, and no undo is offered for
    // an apply that never happened.
    assert!(conflict::list(&state).unwrap().is_empty());
    assert!(remote::in_progress(&state).unwrap().applied_stash.is_none());
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "top\nedited since\nbottom\n"
    );
}

#[test]
fn undoing_an_apply_resets_every_path_it_brought_in() {
    // Two files in the stash: one lands cleanly, the other cannot. Undoing
    // takes both back — they are all the apply's doing — and the branch's own
    // copies come back.
    let sandbox = Sandbox::new("apply-shared");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.commit("shared.txt", "as committed\n", "A second file");
    sandbox.write("a.txt", "top\nstashed\nbottom\n");
    sandbox.write("shared.txt", "from the stash\n");
    sandbox.git(&["stash", "push", "-q", "-m", "two files"]);
    sandbox.commit("a.txt", "top\nsomebody else\nbottom\n", "Moved on");

    let state = sandbox.state();
    assert!(work::stash_apply(&state, 0).is_err());
    assert_eq!(conflict::list(&state).unwrap(), vec!["a.txt".to_string()]);
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("shared.txt")).unwrap(),
        "from the stash\n"
    );

    work::undo_stash_apply(&state).unwrap();
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("shared.txt")).unwrap(),
        "as committed\n"
    );
    let after = refs::status(&state).unwrap();
    assert!(after.staged.is_empty() && after.unstaged.is_empty() && after.conflicted.is_empty());
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);
}

#[test]
fn throwing_a_conflict_away_leaves_the_rest_of_the_tree_as_it_was() {
    let sandbox = stash_conflict();
    // One staged file and one only edited, neither of them the stash's.
    sandbox.commit("staged.txt", "as committed\n", "One");
    sandbox.commit("loose.txt", "as committed\n", "Two");
    sandbox.write("staged.txt", "staged by hand\n");
    sandbox.git(&["add", "staged.txt"]);
    sandbox.write("loose.txt", "edited by hand\n");

    let state = sandbox.state();
    conflict::discard(&state, &["a.txt".to_string()]).unwrap();

    // The conflict is gone; the work beside it is exactly where it was.
    assert!(conflict::list(&state).unwrap().is_empty());
    let after = refs::status(&state).unwrap();
    assert_eq!(
        after
            .staged
            .iter()
            .map(|e| e.path.clone())
            .collect::<Vec<_>>(),
        vec!["staged.txt".to_string()]
    );
    assert_eq!(
        after
            .unstaged
            .iter()
            .map(|e| e.path.clone())
            .collect::<Vec<_>>(),
        vec!["loose.txt".to_string()]
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("loose.txt")).unwrap(),
        "edited by hand\n"
    );
}

#[test]
fn undoing_an_apply_takes_the_stashs_new_files_with_it() {
    let sandbox = Sandbox::new("apply-untracked");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.write("a.txt", "top\nstashed\nbottom\n");
    sandbox.write("fresh.txt", "only in the stash\n");
    sandbox.git(&["stash", "push", "-q", "--include-untracked", "-m", "work in hand"]);
    sandbox.commit("a.txt", "top\nsomebody else\nbottom\n", "Moved on");

    let state = sandbox.state();
    assert!(
        work::stash_apply(&state, 0).is_err(),
        "the apply was supposed to conflict"
    );
    assert!(sandbox.root.join("fresh.txt").exists());

    work::undo_stash_apply(&state).unwrap();

    // A reset alone leaves untracked files behind; the branch never had this
    // one, so putting the tree back means it goes.
    assert!(!sandbox.root.join("fresh.txt").exists());
    assert!(refs::status(&state).unwrap().unstaged.is_empty());
}

#[test]
fn an_apply_that_went_on_cleanly_is_not_offered_an_undo() {
    let sandbox = Sandbox::new("apply-clean");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("b.txt", "beside it\n");
    sandbox.git(&["add", "b.txt"]);
    sandbox.git(&["stash", "push", "-q", "-m", "a second file"]);
    sandbox.commit("c.txt", "somewhere else entirely\n", "Moved on");

    let state = sandbox.state();
    work::stash_apply(&state, 0).unwrap();

    // Those are ordinary working-tree changes now — discarding them is the
    // file menu's job, not an undo the banner offers.
    assert!(remote::in_progress(&state).unwrap().applied_stash.is_none());
    assert!(work::undo_stash_apply(&state).is_err());
}

#[test]
fn a_conflicted_pop_leaves_the_stash_and_can_be_undone() {
    let sandbox = Sandbox::new("pop-clash");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.write("a.txt", "top\nstashed\nbottom\n");
    sandbox.git(&["stash", "push", "-q", "-m", "work in hand"]);
    sandbox.commit("a.txt", "top\nsomebody else\nbottom\n", "Moved on");

    let state = sandbox.state();
    // A pop that stops does not drop the entry, so it is as undoable as an
    // apply — and the stash has to still be there afterwards.
    assert!(work::stash_pop(&state, 0).is_err());
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);

    work::undo_stash_apply(&state).unwrap();
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);
    assert!(conflict::list(&state).unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "top\nsomebody else\nbottom\n"
    );
}

#[test]
fn the_undo_stack_carries_a_stash_that_would_not_go_on() {
    let sandbox = Sandbox::new("apply-stack");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.write("a.txt", "top\nstashed middle\nbottom\n");
    sandbox.git(&["stash", "push", "-q", "-m", "work in hand"]);
    sandbox.commit("a.txt", "top\ncommitted middle\nbottom\n", "Moved on");

    // One state throughout: the undo stack lives in it, the way it does in a
    // running window.
    let state = sandbox.state();
    assert!(work::stash_apply(&state, 0).is_err());

    // The same undo every other step uses, not a button of its own.
    let stacks = journal::stacks(&state);
    let top = stacks.undo.first().expect("the apply is on the stack");
    assert!(top.label.starts_with("Apply:"), "{}", top.label);

    let said = journal::undo(&state).unwrap();
    assert!(said.contains("Undid"), "{said}");
    assert!(conflict::list(&state).unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "top\ncommitted middle\nbottom\n"
    );
    // The work is still where it was: on the list.
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);
}

#[test]
fn an_apply_that_went_on_cleanly_is_undone_the_same_way() {
    let sandbox = Sandbox::new("apply-clean-undo");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("b.txt", "beside it\n");
    sandbox.git(&["add", "b.txt"]);
    sandbox.git(&["stash", "push", "-q", "-m", "a second file"]);

    let state = sandbox.state();
    work::stash_apply(&state, 0).unwrap();
    assert!(sandbox.root.join("b.txt").exists());

    journal::undo(&state).unwrap();
    assert!(!sandbox.root.join("b.txt").exists());
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);
}

#[test]
fn undoing_a_clean_apply_refuses_once_those_files_have_been_worked_on() {
    let sandbox = Sandbox::new("apply-worked-on");
    sandbox.commit("a.txt", "as committed\n", "Base");
    sandbox.write("a.txt", "from the stash\n");
    sandbox.git(&["stash", "push", "-q", "-m", "work in hand"]);

    let state = sandbox.state();
    work::stash_apply(&state, 0).unwrap();
    // An hour's work on top of what the stash brought. Undo would put the
    // file back to the committed copy and take this with it.
    sandbox.write("a.txt", "from the stash, and then some\n");

    let refused = journal::undo(&state).unwrap_err();
    assert!(refused.contains("a.txt"), "{refused}");
    assert!(refused.contains("worked on since"), "{refused}");
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "from the stash, and then some\n"
    );
}

#[test]
fn undoing_an_apply_that_stopped_works_however_far_the_resolving_got() {
    let sandbox = Sandbox::new("apply-half-resolved");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.write("a.txt", "top\nstashed\nbottom\n");
    sandbox.git(&["stash", "push", "-q", "-m", "work in hand"]);
    sandbox.commit("a.txt", "top\nsomebody else\nbottom\n", "Moved on");

    let state = sandbox.state();
    assert!(work::stash_apply(&state, 0).is_err());
    // Half-resolved by hand, which changes the file the apply left. A
    // half-merged file is git's mess, not work — the way out stays open.
    sandbox.write("a.txt", "top\nhalf sorted out\nbottom\n");

    journal::undo(&state).unwrap();
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "top\nsomebody else\nbottom\n"
    );
    assert_eq!(work::stash_list(&state).unwrap().len(), 1);
}

#[test]
fn several_stashes_applied_at_once_step_back_one_at_a_time() {
    let sandbox = Sandbox::new("apply-many-undo");
    sandbox.commit("a.txt", "base\n", "Base");
    sandbox.write("one.txt", "the first\n");
    sandbox.git(&["add", "one.txt"]);
    sandbox.git(&["stash", "push", "-q", "-m", "first"]);
    sandbox.write("two.txt", "the second\n");
    sandbox.git(&["add", "two.txt"]);
    sandbox.git(&["stash", "push", "-q", "-m", "second"]);

    let state = sandbox.state();
    let run = work::stash_apply_many(&state, vec![0, 1], false).unwrap();
    assert_eq!(run.applied.len(), 2);
    assert!(sandbox.root.join("one.txt").exists() && sandbox.root.join("two.txt").exists());

    // Newest first, the order they went on reversed.
    journal::undo(&state).unwrap();
    journal::undo(&state).unwrap();
    assert!(!sandbox.root.join("one.txt").exists());
    assert!(!sandbox.root.join("two.txt").exists());
    assert_eq!(work::stash_list(&state).unwrap().len(), 2);
}

#[test]
fn popping_several_records_nothing_to_undo() {
    let sandbox = Sandbox::new("pop-many");
    sandbox.commit("a.txt", "base\n", "Base");
    sandbox.write("one.txt", "the first\n");
    sandbox.git(&["add", "one.txt"]);
    sandbox.git(&["stash", "push", "-q", "-m", "first"]);

    let state = sandbox.state();
    work::stash_apply_many(&state, vec![0], true).unwrap();

    // The entries are gone, so there is nowhere for the files to go back to.
    assert!(work::stash_list(&state).unwrap().is_empty());
    assert!(!journal::stacks(&state)
        .undo
        .iter()
        .any(|entry| entry.kind == "stash-apply"));
}

#[test]
fn a_redo_that_stops_on_a_conflict_can_be_undone_again() {
    let sandbox = Sandbox::new("redo-clash");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.write("a.txt", "top\nstashed\nbottom\n");
    sandbox.git(&["stash", "push", "-q", "-m", "work in hand"]);
    sandbox.commit("a.txt", "top\nsomebody else\nbottom\n", "Moved on");

    let state = sandbox.state();
    assert!(work::stash_apply(&state, 0).is_err());
    journal::undo(&state).unwrap();

    // Putting it back on stops the same way it did the first time. The step
    // belongs on the undo side either way: the tree has the mess.
    let said = journal::redo(&state).unwrap();
    assert!(said.contains("conflict"), "{said}");
    assert!(!conflict::list(&state).unwrap().is_empty());
    assert!(journal::stacks(&state)
        .undo
        .first()
        .is_some_and(|entry| entry.kind == "stash-apply"));

    journal::undo(&state).unwrap();
    assert!(conflict::list(&state).unwrap().is_empty());
}

#[test]
fn a_pop_that_went_through_is_not_offered_as_an_apply_to_undo() {
    let sandbox = Sandbox::new("pop-clean");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("b.txt", "beside it\n");
    sandbox.git(&["add", "b.txt"]);
    sandbox.git(&["stash", "push", "-q", "-m", "a second file"]);

    let state = sandbox.state();
    work::stash_pop(&state, 0).unwrap();

    // The entry is gone, so putting the files back would leave the work
    // nowhere. Nothing is recorded rather than recording a step that lies.
    assert!(work::stash_list(&state).unwrap().is_empty());
    let stacks = journal::stacks(&state);
    assert!(!stacks.undo.iter().any(|entry| entry.kind == "stash-apply"));
}

#[test]
fn undoing_an_apply_from_the_stack_keeps_work_it_never_made() {
    let sandbox = Sandbox::new("apply-stack-dirty");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.commit("notes.md", "as committed\n", "A file beside it");
    sandbox.write("a.txt", "top\nstashed\nbottom\n");
    sandbox.git(&["stash", "push", "-q", "-m", "work in hand"]);
    sandbox.commit("a.txt", "top\nsomebody else\nbottom\n", "Moved on");

    let state = sandbox.state();
    assert!(work::stash_apply(&state, 0).is_err());
    sandbox.write("notes.md", "typed while stuck\n");

    journal::undo(&state).unwrap();

    // The apply is off, the file typed beside it stands, and the step moved
    // over to the redo side.
    assert!(conflict::list(&state).unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("notes.md")).unwrap(),
        "typed while stuck\n"
    );
    let stacks = journal::stacks(&state);
    assert!(stacks
        .redo
        .first()
        .is_some_and(|entry| entry.kind == "stash-apply"));
}

#[test]
fn a_file_whose_name_reads_as_a_glob_takes_only_itself_back_off() {
    let sandbox = Sandbox::new("apply-glob");
    sandbox.commit("a[1].txt", "as committed\n", "Base");
    sandbox.commit("ab.txt", "a bystander\n", "Another");
    sandbox.write("a[1].txt", "from the stash\n");
    sandbox.git(&["stash", "push", "-q", "-m", "an awkward name"]);

    let state = sandbox.state();
    work::stash_apply(&state, 0).unwrap();
    // Edited beside it, and matched by `a[1]` read as a pattern.
    sandbox.write("ab.txt", "edited by hand\n");

    work::undo_applied(&state, &work::stash_list(&state).unwrap()[0].oid, None).unwrap();

    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a[1].txt")).unwrap(),
        "as committed\n"
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("ab.txt")).unwrap(),
        "edited by hand\n"
    );
}

#[test]
fn throwing_a_conflict_away_names_the_one_file_even_when_it_reads_as_a_glob() {
    let sandbox = Sandbox::new("discard-glob");
    sandbox.commit("a[1].txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.commit("ab.txt", "a bystander\n", "Another");
    sandbox.git(&["checkout", "-q", "-b", "theirs"]);
    sandbox.commit("a[1].txt", "top\ntheirs\nbottom\n", "Their change");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a[1].txt", "top\nours\nbottom\n", "Our change");
    assert!(!sandbox.git_may_fail(&["merge", "theirs"]));

    let state = sandbox.state();
    sandbox.write("ab.txt", "edited by hand\n");
    conflict::discard(&state, &["a[1].txt".to_string()]).unwrap();

    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a[1].txt")).unwrap(),
        "top\nours\nbottom\n"
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("ab.txt")).unwrap(),
        "edited by hand\n"
    );
}

#[test]
fn a_switch_that_would_not_go_back_on_is_still_offered_the_undo() {
    let sandbox = Sandbox::new("switch-clash");
    sandbox.commit("a.txt", "top\nmiddle\nbottom\n", "Base");
    sandbox.write("a.txt", "top\nmine\nbottom\n");
    // The stash a switch makes, named the way this app names it.
    sandbox.git(&[
        "stash",
        "push",
        "-q",
        "-m",
        &format!("{} on main: switching to other", work::AUTO_STASH),
    ]);
    sandbox.commit("a.txt", "top\nsomebody else\nbottom\n", "Moved on");

    let applied = sandbox.git_may_fail(&["stash", "apply"]);
    assert!(!applied, "the apply was supposed to conflict");

    let state = sandbox.state();
    assert!(
        remote::in_progress(&state).unwrap().restoring,
        "the auto-stash is still there, so the switch can be undone"
    );
}

#[test]
fn a_branch_whose_remote_is_gone_is_reported_as_stale() {
    let sandbox = Sandbox::new("stale");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("b.txt", "two\n", "Topic work");

    let bare = sandbox.root.parent().unwrap().join(format!(
        "gitnoob-test-stale-origin-{}.git",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["branch", "-q", "--set-upstream-to=origin/topic", "topic"]);

    let state = sandbox.state();
    assert!(
        refs::stale_branches(&state).unwrap().is_empty(),
        "the upstream is still there"
    );

    // The branch is deleted on the remote and the stale ref pruned locally,
    // which is what happens after someone merges and tidies up.
    sandbox.git(&["--git-dir", &bare_arg, "branch", "-D", "topic"]);
    sandbox.git(&["fetch", "-q", "--prune", "origin"]);

    assert_eq!(
        refs::stale_branches(&state).unwrap(),
        vec!["topic".to_string()]
    );
    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn renaming_a_branch_keeps_its_commits() {
    let sandbox = Sandbox::new("rename");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "old-name"]);
    sandbox.commit("b.txt", "two\n", "Work");
    let before = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();

    let state = sandbox.state();
    refs::rename_branch(&state, "old-name", "new-name").unwrap();

    assert_eq!(refs::describe(&state).unwrap().head, "new-name");
    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), before);
    let tree = refs::tree(&state).unwrap();
    assert!(tree.locals.iter().all(|b| b.name != "old-name"));
}

#[test]
fn an_upstream_can_be_set_and_taken_away() {
    let sandbox = Sandbox::new("upstream");
    sandbox.commit("a.txt", "one\n", "Base");

    let bare = sandbox.root.parent().unwrap().join(format!(
        "gitnoob-test-upstream-origin-{}.git",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);

    let state = sandbox.state();
    refs::set_upstream(&state, "main", "origin/main").unwrap();
    let tracked = refs::tree(&state).unwrap();
    let main = tracked.locals.iter().find(|b| b.name == "main").unwrap();
    assert_eq!(main.upstream.as_deref(), Some("origin/main"));

    refs::unset_upstream(&state, "main").unwrap();
    let untracked = refs::tree(&state).unwrap();
    let main = untracked.locals.iter().find(|b| b.name == "main").unwrap();
    assert!(main.upstream.is_none());

    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn tags_can_be_made_and_removed() {
    let sandbox = Sandbox::new("tags");
    sandbox.commit("a.txt", "one\n", "Base");
    let oid = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    let state = sandbox.state();

    // A lightweight tag, then an annotated one carrying a message.
    gitnoob_lib::work::create_tag(&state, "v1", &oid, None).unwrap();
    gitnoob_lib::work::create_tag(&state, "v2", &oid, Some("the second one")).unwrap();

    let tree = refs::tree(&state).unwrap();
    let names: Vec<&str> = tree.tags.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"v1") && names.contains(&"v2"));
    assert!(sandbox
        .git(&["tag", "-l", "-n", "v2"])
        .contains("the second one"));

    gitnoob_lib::work::delete_tag(&state, "v1").unwrap();
    let after = refs::tree(&state).unwrap();
    assert!(after.tags.iter().all(|t| t.name != "v1"));
    assert!(after.tags.iter().any(|t| t.name == "v2"));
}

#[test]
fn a_pattern_added_to_gitignore_takes_effect() {
    let sandbox = Sandbox::new("ignore");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.write("noise.log", "chatter\n");

    let state = sandbox.state();
    assert!(refs::status(&state)
        .unwrap()
        .unstaged
        .iter()
        .any(|e| e.path == "noise.log"));

    refs::add_to_gitignore(&state, "*.log").unwrap();

    let after = refs::status(&state).unwrap();
    assert!(
        !after.unstaged.iter().any(|e| e.path == "noise.log"),
        "the ignored file should drop out of the status"
    );
    assert!(std::fs::read_to_string(sandbox.root.join(".gitignore"))
        .unwrap()
        .contains("*.log"));
}

#[test]
fn a_commits_patch_can_be_read_back() {
    let sandbox = Sandbox::new("patch");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.commit("a.txt", "one\ntwo\n", "Add a line");
    let oid = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();

    let patch = gitnoob_lib::work::commit_patch(&sandbox.state(), &oid).unwrap();
    assert!(
        patch.contains("diff --git a/a.txt b/a.txt"),
        "unexpected: {patch}"
    );
    assert!(patch.contains("+two"));
}

/// Sets up a repository with a bare `origin` it can push to and pull from,
/// returning the path of the bare one so the test can commit into it.
fn with_origin(sandbox: &Sandbox, tag: &str) -> String {
    let bare = sandbox.root.parent().unwrap().join(format!(
        "gitnoob-test-{tag}-origin-{}.git",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bare);
    let arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &arg]);
    sandbox.git(&["remote", "add", "origin", &arg]);
    sandbox.git(&["fetch", "-q", "origin"]);
    arg
}

/// Adds a commit to a branch inside the bare remote, standing in for someone
/// else pushing while you were working.
fn commit_on_remote(sandbox: &Sandbox, bare: &str, branch: &str, file: &str, body: &str) {
    // Named after this sandbox, not just the branch: the tests run in parallel
    // and several of them use a branch called `topic`.
    let work = sandbox.root.parent().unwrap().join(format!(
        "{}-clone-{branch}",
        sandbox.root.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&work);
    let work_arg = work.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "-b", branch, bare, &work_arg]);

    let run = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&work)
            .output()
            .expect("git should be on PATH");
        assert!(
            out.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };
    std::fs::write(work.join(file), body).unwrap();
    run(&["config", "user.name", "Someone"]);
    run(&["config", "user.email", "someone@example.com"]);
    run(&["config", "commit.gpgsign", "false"]);
    run(&["add", "--all"]);
    run(&["commit", "-q", "-m", "Someone else's work"]);
    run(&["push", "-q", "origin", branch]);
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn pulling_another_branch_never_touches_the_working_tree() {
    let sandbox = Sandbox::new("pullother");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("t.txt", "topic\n", "Topic base");
    sandbox.git(&["checkout", "-q", "main"]);
    let bare = with_origin(&sandbox, "pullother");
    sandbox.git(&["branch", "-q", "--set-upstream-to=origin/topic", "topic"]);

    // Someone else moves topic on, while you are on main with work in progress.
    commit_on_remote(&sandbox, &bare, "topic", "theirs.txt", "theirs\n");
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.write("mine.txt", "half-finished\n");
    sandbox.git(&["add", "mine.txt"]);

    let state = sandbox.state();
    let out = remote::pull_branch(&state, "topic", false).unwrap();
    assert!(out.ok, "unexpected: {} {}", out.stdout, out.stderr);

    // topic moved, and nothing else did.
    assert_eq!(
        refs::describe(&state).unwrap().head,
        "main",
        "still on main"
    );
    assert!(
        sandbox.git(&["stash", "list"]).trim().is_empty(),
        "nothing was stashed"
    );
    assert!(
        refs::status(&state)
            .unwrap()
            .staged
            .iter()
            .any(|e| e.path == "mine.txt"),
        "the staged file is untouched"
    );
    assert!(
        !sandbox.root.join("theirs.txt").exists(),
        "their file belongs to topic, not to the tree we are standing in"
    );
    let log = sandbox.git(&["log", "--format=%s", "topic"]);
    assert!(
        log.contains("Someone else"),
        "topic has their commit: {log}"
    );

    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn pulling_a_diverged_branch_visits_it_and_comes_back() {
    let sandbox = Sandbox::new("pulldiverged");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("t.txt", "topic\n", "Topic base");
    sandbox.git(&["checkout", "-q", "main"]);
    let bare = with_origin(&sandbox, "pulldiverged");
    sandbox.git(&["branch", "-q", "--set-upstream-to=origin/topic", "topic"]);

    // They add a commit; so do you, on the same branch, so it cannot simply be
    // moved forward. The two touch different files, so the merge itself works.
    commit_on_remote(&sandbox, &bare, "topic", "theirs.txt", "theirs\n");
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["checkout", "-q", "topic"]);
    sandbox.commit("ours.txt", "ours\n", "Our own");
    sandbox.git(&["checkout", "-q", "main"]);

    // And you are mid-edit on main the whole time.
    sandbox.write("mine.txt", "half-finished\n");

    let state = sandbox.state();
    let out = remote::pull_branch(&state, "topic", false).unwrap();
    assert!(out.ok, "unexpected: {} {}", out.stdout, out.stderr);

    // Back where we started, with the work in progress back in place.
    assert_eq!(refs::describe(&state).unwrap().head, "main");
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("mine.txt")).unwrap(),
        "half-finished\n"
    );
    assert!(
        sandbox.git(&["stash", "list"]).trim().is_empty(),
        "the stash was put back"
    );

    // topic has both sides of the history now.
    let log = sandbox.git(&["log", "--format=%s", "topic"]);
    assert!(
        log.contains("Someone else") && log.contains("Our own"),
        "unexpected: {log}"
    );

    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn a_pull_that_cannot_merge_leaves_everything_as_it_was() {
    let sandbox = Sandbox::new("pullconflict");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("shared.txt", "base\n", "Topic base");
    sandbox.git(&["checkout", "-q", "main"]);
    let bare = with_origin(&sandbox, "pullconflict");
    sandbox.git(&["branch", "-q", "--set-upstream-to=origin/topic", "topic"]);

    // Both sides change the same file, so the merge cannot go through.
    commit_on_remote(&sandbox, &bare, "topic", "shared.txt", "theirs\n");
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["checkout", "-q", "topic"]);
    sandbox.commit("shared.txt", "ours\n", "Our version");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.write("mine.txt", "half-finished\n");

    let state = sandbox.state();
    let out = remote::pull_branch(&state, "topic", false).unwrap();
    assert!(!out.ok, "the merge cannot succeed");
    assert!(
        out.stderr.contains("left as it was"),
        "should say so plainly: {}",
        out.stderr
    );

    // The point of the exercise: we are home, nothing is half-merged, and the
    // work in progress is where it was left.
    assert_eq!(refs::describe(&state).unwrap().head, "main");
    assert!(
        !remote::in_progress(&state).unwrap().merging,
        "no merge left dangling"
    );
    assert!(refs::status(&state).unwrap().conflicted.is_empty());
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("mine.txt")).unwrap(),
        "half-finished\n"
    );
    assert!(
        sandbox.git(&["stash", "list"]).trim().is_empty(),
        "the stash was put back"
    );

    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn pulling_a_branch_with_no_upstream_says_so() {
    let sandbox = Sandbox::new("pullnoupstream");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["branch", "orphan"]);
    let error = remote::pull_branch(&sandbox.state(), "orphan", false).unwrap_err();
    assert!(error.contains("not tracking"), "unexpected: {error}");
}

/// A conflict in a file that uses Windows line endings, built with autocrlf
/// off so the CRLF is really in the blob rather than added by the checkout.
fn conflicted_crlf() -> Sandbox {
    let sandbox = Sandbox::new("conflict-crlf");
    sandbox.commit("a.txt", "top\r\nmiddle\r\nbottom\r\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "theirs"]);
    sandbox.commit("a.txt", "top\r\ntheir middle\r\nbottom\r\n", "Their change");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "top\r\nour middle\r\nbottom\r\n", "Our change");

    let merged = sandbox.git_may_fail(&["merge", "theirs"]);
    assert!(!merged, "the merge was supposed to conflict");
    sandbox
}

#[test]
fn resolving_a_crlf_file_keeps_its_line_endings() {
    let sandbox = conflicted_crlf();
    let state = sandbox.state();

    let choices = vec![conflict::Resolution {
        take_ours: true,
        take_theirs: false,
        ours_first: true,
        custom: None,
    }];
    // Rejoining with LF would rewrite every line of the file, so resolving one
    // conflict would show up as a change to all of it.
    assert_eq!(
        conflict::preview(&state, "a.txt", &choices).unwrap(),
        "top\r\nour middle\r\nbottom\r\n"
    );

    conflict::resolve(&state, "a.txt", &choices).unwrap();
    let on_disk = std::fs::read(sandbox.root.join("a.txt")).unwrap();
    assert_eq!(
        String::from_utf8(on_disk).unwrap(),
        "top\r\nour middle\r\nbottom\r\n"
    );

    // Keeping our side reproduces our commit exactly, so nothing is staged.
    // Rewriting the endings would show up here as all three lines changed.
    let diff = sandbox.git(&["diff", "--cached", "--numstat"]);
    assert!(
        diff.trim().is_empty(),
        "the file should be unchanged from ours: {diff}"
    );
}

#[test]
fn resolving_does_not_add_a_newline_the_file_never_had() {
    let sandbox = Sandbox::new("conflict-no-eof-newline");
    // The conflict is in the middle: a file whose last line is agreed context
    // is one where the missing final newline is still visible to the parser.
    // When the conflict is the last thing in the file git has to write a
    // newline before the closing marker, and the original is unknowable.
    sandbox.commit("a.txt", "middle\nlast", "Base");
    sandbox.git(&["checkout", "-q", "-b", "theirs"]);
    sandbox.commit("a.txt", "their middle\nlast", "Their change");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "our middle\nlast", "Our change");
    assert!(!sandbox.git_may_fail(&["merge", "theirs"]));

    let state = sandbox.state();
    let choices = vec![conflict::Resolution {
        take_ours: true,
        take_theirs: false,
        ours_first: true,
        custom: None,
    }];
    conflict::resolve(&state, "a.txt", &choices).unwrap();
    let on_disk = std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap();
    assert_eq!(on_disk, "our middle\nlast");
}

#[test]
fn a_setext_heading_in_our_side_does_not_flip_the_parser() {
    let sandbox = Sandbox::new("conflict-setext");
    sandbox.commit("a.txt", "Title\nintro\nbottom\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "theirs"]);
    sandbox.commit("a.txt", "Title\ntheir intro\nbottom\n", "Their change");
    sandbox.git(&["checkout", "-q", "main"]);
    // Our change turns the line into a Markdown heading, whose underline is
    // eight `=` — one more than git's own split marker.
    sandbox.commit(
        "a.txt",
        "Title\n========\nour intro\nbottom\n",
        "Our change",
    );
    assert!(!sandbox.git_may_fail(&["merge", "theirs"]));

    let state = sandbox.state();
    let file = conflict::read(&state, "a.txt").unwrap();
    assert_eq!(file.conflict_count, 1);

    let (ours, theirs) = file
        .blocks
        .iter()
        .find_map(|block| match block {
            conflict::Block::Conflict { ours, theirs, .. } => Some((ours.clone(), theirs.clone())),
            _ => None,
        })
        .expect("there should be one conflict region");

    assert_eq!(ours, vec!["========".to_string(), "our intro".to_string()]);
    assert_eq!(theirs, vec!["their intro".to_string()]);

    let choices = vec![conflict::Resolution {
        take_ours: true,
        take_theirs: false,
        ours_first: true,
        custom: None,
    }];
    assert_eq!(
        conflict::preview(&state, "a.txt", &choices).unwrap(),
        "Title\n========\nour intro\nbottom\n"
    );
}

/// Sets up a merge that conflicts in two separate regions of the same file.
/// The regions are kept far enough apart that git treats them as distinct
/// hunks rather than folding them into one.
fn conflicted_twice() -> Sandbox {
    let filler = "context\n".repeat(10);
    let base = format!("one\ntwo\n{filler}three\nfour\n");
    let theirs = format!("one\ntheir two\n{filler}three\ntheir four\n");
    let ours = format!("one\nour two\n{filler}three\nour four\n");

    let sandbox = Sandbox::new("conflict-twice");
    sandbox.commit("a.txt", &base, "Base");
    sandbox.git(&["checkout", "-q", "-b", "theirs"]);
    sandbox.commit("a.txt", &theirs, "Their change");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", &ours, "Our change");
    assert!(!sandbox.git_may_fail(&["merge", "theirs"]));
    sandbox
}

#[test]
fn resolving_with_fewer_choices_than_conflicts_is_refused() {
    let sandbox = conflicted_twice();
    let state = sandbox.state();
    let file = conflict::read(&state, "a.txt").unwrap();
    assert_eq!(file.conflict_count, 2);

    let one_choice = vec![conflict::Resolution {
        take_ours: true,
        take_theirs: false,
        ours_first: true,
        custom: None,
    }];

    // The preview stays forgiving, defaulting the unanswered region to ours,
    // which here reproduces our commit exactly.
    let filler = "context\n".repeat(10);
    assert_eq!(
        conflict::preview(&state, "a.txt", &one_choice).unwrap(),
        format!("one\nour two\n{filler}three\nour four\n")
    );

    // Writing on that same guess would stage a region nobody actually chose.
    let error = conflict::resolve(&state, "a.txt", &one_choice).unwrap_err();
    assert!(
        error.contains('2') && error.contains('1'),
        "unexpected: {error}"
    );
    assert!(
        !conflict::list(&state).unwrap().is_empty(),
        "the conflict must not be cleared"
    );

    let empty: Vec<conflict::Resolution> = Vec::new();
    assert!(conflict::resolve(&state, "a.txt", &empty).is_err());
}

#[test]
fn a_non_utf8_conflicted_file_is_still_recognised_as_marked() {
    let sandbox = conflicted();
    // Overwrite the live markers with a copy that also carries a byte no
    // encoding recognises, the way a Latin-1 comment or a mixed-encoding
    // fixture would.
    let mut broken = std::fs::read(sandbox.root.join("a.txt")).unwrap();
    broken.push(b'\n');
    broken.extend_from_slice(b"non-utf8: \xff\n");
    assert!(String::from_utf8(broken.clone()).is_err());
    std::fs::write(sandbox.root.join("a.txt"), &broken).unwrap();

    let state = sandbox.state();
    assert_eq!(conflict::marked(&state).unwrap(), vec!["a.txt".to_string()]);

    let error = conflict::stage_all(&state).unwrap_err();
    assert!(error.contains("a.txt"), "unexpected: {error}");
}

#[test]
fn pulling_a_diverged_branch_does_not_ask_how_to_reconcile() {
    let sandbox = Sandbox::new("pull-divergent");
    sandbox.commit("a.txt", "one\n", "One");
    let bare = with_origin(&sandbox, "pull-divergent");
    sandbox.git(&["push", "-q", "-u", "origin", "main"]);

    // Both sides move, so the pull has to reconcile rather than fast-forward.
    commit_on_remote(&sandbox, &bare, "main", "remote.txt", "from the remote\n");
    sandbox.commit("local.txt", "from here\n", "Local work");
    sandbox.git(&["fetch", "-q", "origin"]);

    // Git refuses a bare `git pull` across a divergence unless `pull.rebase` is
    // configured. Nobody opening this app has configured it.
    let state = sandbox.state();
    let out = remote::pull(&state, false).unwrap();
    assert!(
        out.ok,
        "a merging pull should not need `pull.rebase` set: {}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("reconcile divergent"),
        "git should not be left to ask: {}",
        out.stderr
    );
}

#[test]
fn checking_out_a_name_that_is_only_a_file_is_refused() {
    let sandbox = Sandbox::new("checkout-pathspec");
    sandbox.commit("notes.txt", "committed\n", "One");
    sandbox.write("notes.txt", "work in progress\n");

    let state = sandbox.state();
    // "notes.txt" is no branch, tag or revision — only a path. Without a `--`
    // git reads it as one and restores the file, throwing the edit away.
    let result = refs::checkout(&state, "notes.txt");
    assert!(
        result.is_err(),
        "checking out a path should fail, not succeed silently"
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("notes.txt")).unwrap(),
        "work in progress\n",
        "the uncommitted edit must survive"
    );
}

#[test]
fn a_branch_named_like_a_file_still_checks_out() {
    let sandbox = Sandbox::new("checkout-ambiguous");
    sandbox.commit("release", "committed\n", "One");
    sandbox.git(&["branch", "release"]);

    let state = sandbox.state();
    // Both a branch and a path called "release"; the branch is what was asked
    // for and what the `--` guarantees.
    refs::checkout(&state, "release").unwrap();
    assert_eq!(sandbox.git(&["branch", "--show-current"]).trim(), "release");
}

#[test]
fn checking_out_a_branch_named_like_a_flag_does_not_discard_uncommitted_changes() {
    let sandbox = Sandbox::new("checkout-dash-branch");
    sandbox.commit("notes.txt", "committed\n", "One");
    // Porcelain refuses to create a branch called `-f` — `git branch -- -f` is
    // an error — but `update-ref` writes the ref directly, and a remote that
    // carries one hands it to the app exactly like any other branch name.
    sandbox.git(&["update-ref", "refs/heads/-f", "HEAD"]);
    sandbox.write("notes.txt", "work in progress\n");

    let state = sandbox.state();
    // Without `--end-of-options`, `-f` is read as `checkout`'s force flag with
    // no branch named on the command line, which resets the working tree to
    // HEAD and throws the edit away without a word.
    refs::checkout(&state, "-f").unwrap();

    assert_eq!(sandbox.git(&["branch", "--show-current"]).trim(), "-f");
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("notes.txt")).unwrap(),
        "work in progress\n",
        "the uncommitted edit must survive switching to a branch named -f"
    );
}

#[test]
fn creating_a_branch_from_a_start_point_named_like_a_flag_uses_that_start_point() {
    let sandbox = Sandbox::new("create-dash-start");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["update-ref", "refs/heads/-f", "HEAD"]);
    let base = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    sandbox.commit("a.txt", "two\n", "Ahead");

    let state = sandbox.state();
    // Without `--end-of-options`, `-f` is read as `branch`'s force flag and the
    // start point silently falls back to HEAD, which is the wrong commit here.
    refs::create_branch(&state, "topic", Some("-f"), false).unwrap();

    assert_eq!(
        sandbox.git(&["rev-parse", "topic"]).trim(),
        base,
        "topic must be created from -f, not from whatever HEAD happens to be"
    );
}

#[test]
fn a_branch_named_like_a_flag_can_be_deleted() {
    let sandbox = Sandbox::new("delete-dash-branch");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["update-ref", "refs/heads/-f", "HEAD"]);

    let state = sandbox.state();
    // Without `--end-of-options`, `-f` is read as another flag, leaving no
    // branch name on the command line at all — git then lists branches
    // instead of deleting one, and the app reports success for nothing done.
    refs::delete_branch(&state, "-f", true).unwrap();

    assert!(refs::tree(&state)
        .unwrap()
        .locals
        .iter()
        .all(|b| b.name != "-f"));
}

#[test]
fn an_enormous_diff_is_capped_and_says_so() {
    let sandbox = Sandbox::new("diff-cap");
    // Stands in for a regenerated lockfile: far more changed lines than anyone
    // is going to read, and every one of them a DOM node if it is sent.
    let original: String = (0..12_000).map(|n| format!("line {n}\n")).collect();
    sandbox.commit("generated.txt", &original, "Generated");
    let rewritten: String = (0..12_000).map(|n| format!("changed {n}\n")).collect();
    sandbox.write("generated.txt", &rewritten);

    let state = sandbox.state();
    let diff = diff::working_file_diff(&state, "generated.txt", diff::Side::Unstaged).unwrap();

    let shown: usize = diff.hunks.iter().map(|hunk| hunk.lines.len()).sum();
    assert!(shown <= 10_000, "the cap should hold: {shown} lines");
    assert!(shown > 0, "the diff should not be empty");
    assert!(
        diff.truncated > 0,
        "the lines left out should be counted, so the view can say so"
    );
}

#[test]
fn an_ordinary_diff_is_not_reported_as_truncated() {
    let sandbox = Sandbox::new("diff-uncapped");
    sandbox.commit("a.txt", "one\ntwo\nthree\n", "One");
    sandbox.write("a.txt", "one\ntwo changed\nthree\n");

    let state = sandbox.state();
    let diff = diff::working_file_diff(&state, "a.txt", diff::Side::Unstaged).unwrap();
    assert_eq!(diff.truncated, 0);
}

/// A file git has never seen has nothing to diff against, and the naive answer
/// — a delta with no hunks — is indistinguishable from "this file is unchanged".
#[test]
fn a_new_file_shows_its_whole_contents_as_added() {
    let sandbox = Sandbox::new("untracked");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("new.txt", "hello\nworld\n");

    let state = sandbox.state();
    let diff = diff::working_file_diff(&state, "new.txt", diff::Side::Unstaged).unwrap();
    let added: Vec<&str> = diff
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.origin == '+')
        .map(|line| line.content.as_str())
        .collect();
    assert_eq!(added, vec!["hello", "world"]);
}

/// Undo moves a branch, and a branch is all it can move. When the commit it
/// moved off has already been pushed, saying only "undid" leaves the window
/// reporting the branch as behind — and pulling, the obvious thing to do about
/// that, brings the undone commit straight back.
#[test]
fn undoing_a_pushed_commit_says_the_remote_still_has_it() {
    let sandbox = Sandbox::new("undopush");
    sandbox.commit("a.txt", "one\n", "Shared base");

    let bare = sandbox.root.parent().unwrap().join(format!(
        "gitnoob-test-undo-origin-{}.git",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["branch", "--set-upstream-to=origin/main", "main"]);

    let state = sandbox.state();

    // A commit that stays here is undone without ceremony.
    sandbox.write("a.txt", "two\n");
    sandbox.git(&["add", "a.txt"]);
    work::commit(&state, "Local only", false).unwrap();
    let said = journal::undo(&state).unwrap();
    assert!(!said.contains("origin/main"), "{said}");

    // One that has been pushed cannot be undone there by a reset here.
    journal::redo(&state).unwrap();
    sandbox.git(&["push", "-q", "origin", "main"]);
    let said = journal::undo(&state).unwrap();
    assert!(said.contains("origin/main"), "{said}");
    assert!(said.contains("Push to undo it there"), "{said}");

    let _ = std::fs::remove_dir_all(&bare);
}

// --- merging into a branch you are not standing on ---------------------------

/// Two branches that have each moved on, with no overlap in what they touched.
fn diverged(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("shared.txt", "base\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("theirs.txt", "side\n", "On side");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("ours.txt", "main\n", "On main");
    sandbox
}

fn head_of(sandbox: &Sandbox, branch: &str) -> String {
    sandbox.git(&["rev-parse", branch]).trim().to_string()
}

fn current(state: &AppState) -> String {
    refs::describe(state).unwrap().head
}

#[test]
fn merging_into_a_branch_that_is_only_behind_moves_its_ref() {
    let sandbox = Sandbox::new("ff-ref");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["branch", "old"]);
    sandbox.commit("b.txt", "two\n", "Second");
    // Open work that must survive untouched: a fast-forward has no business
    // going near the working tree.
    sandbox.write("dirty.txt", "in progress\n");

    let state = sandbox.state();
    let outcome = remote::merge_into(&state, "main", "old", false).unwrap();

    assert!(outcome.ok, "{}", outcome.message);
    assert!(outcome.conflicts.is_empty());
    assert_eq!(head_of(&sandbox, "old"), head_of(&sandbox, "main"));
    assert_eq!(current(&state), "main", "should never have left main");
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("dirty.txt")).unwrap(),
        "in progress\n"
    );
    // Nothing was stashed, because nothing needed to be.
    assert!(sandbox.git(&["stash", "list"]).trim().is_empty());
}

#[test]
fn merging_into_a_diverged_branch_visits_it_and_comes_back() {
    let sandbox = diverged("visit");
    let state = sandbox.state();
    let before_main = head_of(&sandbox, "main");

    let outcome = remote::merge_into(&state, "main", "side", false).unwrap();

    assert!(outcome.ok, "{}", outcome.message);
    assert_eq!(current(&state), "main", "should have come home");
    assert_eq!(head_of(&sandbox, "main"), before_main, "main must not move");
    // side now holds both sides of the history.
    let log = sandbox.git(&["log", "--format=%s", "side"]);
    assert!(log.contains("On main"), "{log}");
    assert!(log.contains("On side"), "{log}");
    // And the working tree is main's again: side's file is not here.
    assert!(!sandbox.root.join("theirs.txt").exists());
}

#[test]
fn merging_into_another_branch_puts_open_changes_back_afterwards() {
    let sandbox = diverged("carry");
    sandbox.write("ours.txt", "edited but not committed\n");
    let state = sandbox.state();

    let outcome = remote::merge_into(&state, "main", "side", false).unwrap();

    assert!(outcome.ok, "{}", outcome.message);
    assert_eq!(current(&state), "main");
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("ours.txt")).unwrap(),
        "edited but not committed\n",
        "the open edit should be back in the tree"
    );
    assert!(
        sandbox.git(&["stash", "list"]).trim().is_empty(),
        "the stash it took should have been used up"
    );
}

#[test]
fn merging_into_the_branch_you_are_on_is_an_ordinary_merge() {
    let sandbox = diverged("here");
    let state = sandbox.state();

    let outcome = remote::merge_into(&state, "side", "main", false).unwrap();

    assert!(outcome.ok, "{}", outcome.message);
    assert_eq!(current(&state), "main");
    assert!(
        sandbox.root.join("theirs.txt").exists(),
        "side's file should be here now"
    );
}

#[test]
fn a_conflicting_merge_leaves_you_on_the_branch_that_needs_resolving() {
    let sandbox = Sandbox::new("stuck");
    sandbox.commit("a.txt", "base\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("a.txt", "their version\n", "On side");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "our version\n", "On main");

    let state = sandbox.state();
    let outcome = remote::merge_into(&state, "main", "side", false).unwrap();

    assert!(!outcome.ok);
    assert_eq!(outcome.conflicts, vec!["a.txt".to_string()]);
    assert_eq!(
        current(&state),
        "side",
        "a half-done merge cannot be carried off the branch"
    );
    assert!(
        outcome.message.contains("Resolve it here"),
        "{}",
        outcome.message
    );

    // And the way out is the one the resolver offers.
    remote::abort_merge(&state).unwrap();
}

#[test]
fn rebasing_a_branch_you_are_not_on_replays_it_and_comes_back() {
    let sandbox = diverged("rebase-away");
    sandbox.write("ours.txt", "still editing\n");
    let state = sandbox.state();

    let outcome = remote::rebase_branch(&state, "side", "main").unwrap();

    assert!(outcome.ok, "{}", outcome.message);
    assert_eq!(current(&state), "main", "should have come home");
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("ours.txt")).unwrap(),
        "still editing\n"
    );
    // side is now a straight line on top of main: no merge commit, and main's
    // commit is in its history.
    let log = sandbox.git(&["log", "--format=%s", "side"]);
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        vec!["On side", "On main", "First"]
    );
}

#[test]
fn an_annotated_tag_names_its_commit_rather_than_its_own_object() {
    let sandbox = Sandbox::new("annotated-tag");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["tag", "light"]);
    sandbox.git(&["tag", "-a", "v1.0.0", "-m", "The first release"]);

    let head = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    let object = sandbox.git(&["rev-parse", "v1.0.0"]).trim().to_string();
    // The whole reason this needs a test: `git tag -a` writes an object of its
    // own, and its id is not the commit's. Reading the ref without peeling
    // yields an id that nothing in the history has.
    assert_ne!(
        head, object,
        "an annotated tag is an object in its own right"
    );

    let state = sandbox.state();
    let tree = refs::tree(&state).unwrap();

    let annotated = tree.tags.iter().find(|t| t.name == "v1.0.0").unwrap();
    assert_eq!(
        annotated.oid, head,
        "the tag should name the commit it tags"
    );
    assert!(annotated.annotated);
    assert_eq!(annotated.message.as_deref(), Some("The first release"));
    assert!(annotated.when > 0);

    let light = tree.tags.iter().find(|t| t.name == "light").unwrap();
    assert_eq!(light.oid, head);
    assert!(!light.annotated, "a bare `git tag` writes no object");
    assert!(light.message.is_none());

    // And the graph, which is where it showed: a chip hung on the tag object's
    // id decorates no row, because no row carries that id.
    let page = graph::build(&state, 500).unwrap();
    let row = page.rows.iter().find(|r| r.oid == head).unwrap();
    let tags: Vec<&str> = row
        .labels
        .iter()
        .filter(|l| l.kind == "tag")
        .map(|l| l.name.as_str())
        .collect();
    assert!(
        tags.contains(&"v1.0.0"),
        "the annotated tag should decorate its commit"
    );
    assert!(tags.contains(&"light"));
}

// --- checking out a remote branch whose local branch already exists ----------

/// A commit somebody else pushes to the bare origin's main — the everyday
/// reason a shared branch is ahead of its local copy.
fn someone_pushes(bare: &Path, tag: &str, path: &str, content: &str) -> String {
    let theirs = scratch(&format!("{tag}-theirs"));
    let clone = theirs.join("clone");
    git_at(
        &theirs,
        &[
            "clone",
            "-q",
            bare.to_string_lossy().as_ref(),
            clone.to_string_lossy().as_ref(),
        ],
    );
    git_at(&clone, &["config", "user.name", "Other"]);
    git_at(&clone, &["config", "user.email", "other@example.com"]);
    git_at(&clone, &["config", "commit.gpgsign", "false"]);
    std::fs::write(clone.join(path), content).unwrap();
    git_at(&clone, &["add", "--all"]);
    git_at(&clone, &["commit", "-q", "-m", "Their commit"]);
    git_at(&clone, &["push", "-q", "origin", "main"]);
    let tip = git_at(&clone, &["rev-parse", "HEAD"]).trim().to_string();
    let _ = std::fs::remove_dir_all(&theirs);
    tip
}

/// Pushes main, has somebody move it on, and fetches: a local main one commit
/// behind origin/main, which is the state the sync checkout exists for.
fn behind_origin(sandbox: &Sandbox, tag: &str) -> String {
    sandbox.commit("shared.txt", "top\nmiddle\nbottom\n", "Base");
    let bare = bare_origin(sandbox, tag);
    sandbox.git(&["push", "-q", "-u", "origin", "main"]);
    let tip = someone_pushes(&bare, tag, "shared.txt", "THEIRS\nmiddle\nbottom\n");
    sandbox.git(&["fetch", "-q", "origin"]);
    tip
}

#[test]
fn checking_out_a_remote_branch_pulls_its_stale_local_branch_up() {
    let sandbox = Sandbox::new("sync-behind");
    let tip = behind_origin(&sandbox, "sync-behind");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    let state = sandbox.state();

    let outcome = refs::checkout(&state, "origin/main").unwrap();

    // This used to be "a branch named 'main' already exists".
    assert_eq!(current(&state), "main");
    assert_eq!(
        head_of(&sandbox, "main"),
        tip,
        "main should now be origin's main"
    );
    assert!(outcome.diverged.is_none());
    assert!(
        outcome.message.contains("pulled 1 commit"),
        "unexpected message: {}",
        outcome.message
    );
}

#[test]
fn checking_out_the_remote_copy_of_the_branch_you_are_on_pulls_it() {
    let sandbox = Sandbox::new("sync-standing");
    let tip = behind_origin(&sandbox, "sync-standing");
    let state = sandbox.state();

    let outcome = refs::checkout(&state, "origin/main").unwrap();

    assert_eq!(current(&state), "main");
    assert_eq!(head_of(&sandbox, "main"), tip);
    assert!(outcome.diverged.is_none());
}

#[test]
fn open_changes_ride_across_the_pull_and_come_back() {
    let sandbox = Sandbox::new("sync-dirty");
    let tip = behind_origin(&sandbox, "sync-dirty");
    // An edit to the very file the remote commit changes, so the fast-forward
    // is refused until the work is set down.
    sandbox.write("shared.txt", "top\nmiddle\nMINE\n");
    let state = sandbox.state();

    refs::checkout(&state, "origin/main").unwrap();

    assert_eq!(head_of(&sandbox, "main"), tip);
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("shared.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "THEIRS\nmiddle\nMINE\n",
        "their commit and the open edit should both be in the file"
    );
    assert_eq!(
        sandbox.git(&["stash", "list"]).trim(),
        "",
        "the stash was a means, not a destination"
    );
}

#[test]
fn a_local_branch_that_is_only_ahead_is_left_alone() {
    let sandbox = Sandbox::new("sync-ahead");
    sandbox.commit("shared.txt", "top\n", "Base");
    bare_origin(&sandbox, "sync-ahead");
    sandbox.git(&["push", "-q", "-u", "origin", "main"]);
    sandbox.commit("ours.txt", "mine\n", "Unpushed work");
    let mine = head_of(&sandbox, "main");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    let state = sandbox.state();

    let outcome = refs::checkout(&state, "origin/main").unwrap();

    assert_eq!(current(&state), "main");
    assert_eq!(
        head_of(&sandbox, "main"),
        mine,
        "nothing to pull, nothing moved"
    );
    assert!(outcome.diverged.is_none());
    assert!(
        outcome.message.contains("ahead"),
        "unexpected message: {}",
        outcome.message
    );
}

/// A diverged branch under the default setting: the switch happens, nothing is
/// decided, and the question rides back for the window to put up.
#[test]
fn a_diverged_branch_is_checked_out_and_the_question_handed_back() {
    let sandbox = Sandbox::new("sync-diverged");
    behind_origin(&sandbox, "sync-diverged");
    sandbox.commit("ours.txt", "mine\n", "Our own work");
    let mine = head_of(&sandbox, "main");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    let state = sandbox.state();

    let outcome = refs::checkout(&state, "origin/main").unwrap();

    assert_eq!(current(&state), "main");
    assert_eq!(
        head_of(&sandbox, "main"),
        mine,
        "asking must not move anything"
    );
    let asked = outcome.diverged.expect("the default is to ask");
    assert_eq!(asked.branch, "main");
    assert_eq!(asked.upstream, "origin/main");
    assert_eq!((asked.ahead, asked.behind), (1, 1));
}

#[test]
fn a_diverged_branch_is_rebased_when_settings_say_so() {
    let sandbox = Sandbox::new("sync-rebase");
    let tip = behind_origin(&sandbox, "sync-rebase");
    sandbox.commit("ours.txt", "mine\n", "Our own work");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    let state = sandbox.state();
    state
        .update_config(|config| config.global.diverged_checkout = "rebase".to_string())
        .unwrap();

    let outcome = refs::checkout(&state, "origin/main").unwrap();

    assert_eq!(current(&state), "main");
    assert!(outcome.diverged.is_none(), "the setting already answered");
    // Their commit is now under ours: a straight line, no merge commit.
    assert!(sandbox.git_may_fail(&["merge-base", "--is-ancestor", &tip, "main"]));
    assert_eq!(
        sandbox
            .git(&["rev-list", "--count", "origin/main..main"])
            .trim(),
        "1",
        "only our own commit should sit above origin/main"
    );
}

#[test]
fn a_branch_tracking_another_remote_is_not_this_ones_to_move() {
    let sandbox = Sandbox::new("sync-fork");
    sandbox.commit("shared.txt", "top\n", "Base");
    let origin = bare_origin(&sandbox, "sync-fork");
    sandbox.git(&["push", "-q", "origin", "main"]);

    // A second remote, and main tracks that one.
    let fork = scratch("sync-fork-fork").join("fork.git");
    git_at(
        fork.parent().unwrap(),
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            fork.to_string_lossy().as_ref(),
        ],
    );
    sandbox.git(&["remote", "add", "fork", fork.to_string_lossy().as_ref()]);
    sandbox.git(&["push", "-q", "-u", "fork", "main"]);

    someone_pushes(&origin, "sync-fork", "shared.txt", "THEIRS\n");
    sandbox.git(&["fetch", "-q", "origin"]);
    let mine = head_of(&sandbox, "main");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    let state = sandbox.state();

    let outcome = refs::checkout(&state, "origin/main").unwrap();

    assert_eq!(current(&state), "main");
    assert_eq!(
        head_of(&sandbox, "main"),
        mine,
        "a fork's click must not move it"
    );
    assert!(
        outcome.message.contains("tracks fork/main"),
        "unexpected message: {}",
        outcome.message
    );
}

#[test]
fn a_branch_with_no_upstream_starts_tracking_the_remote_it_was_synced_to() {
    let sandbox = Sandbox::new("sync-link");
    sandbox.commit("shared.txt", "top\n", "Base");
    bare_origin(&sandbox, "sync-link");
    // Push without -u: origin/main exists, main tracks nothing.
    sandbox.git(&["push", "-q", "origin", "main"]);
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    let state = sandbox.state();

    let outcome = refs::checkout(&state, "origin/main").unwrap();

    assert_eq!(current(&state), "main");
    assert_eq!(
        sandbox
            .git(&["rev-parse", "--abbrev-ref", "main@{upstream}"])
            .trim(),
        "origin/main"
    );
    assert!(
        outcome.message.contains("now tracks origin/main"),
        "unexpected message: {}",
        outcome.message
    );
}

// --- worktrees ---------------------------------------------------------------

#[test]
fn a_worktree_is_added_listed_and_removed() {
    let sandbox = Sandbox::new("worktree");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["branch", "side"]);
    let state = sandbox.state();

    let alone = worktree::list(&state).unwrap();
    assert_eq!(alone.len(), 1);
    assert!(alone[0].is_main && alone[0].is_current);
    assert_eq!(alone[0].branch.as_deref(), Some("main"));

    let parent = scratch("worktree-dest");
    let dest = parent.join("side-folder");
    let dest_arg = dest.to_string_lossy().into_owned();
    worktree::add(&state, &dest_arg, "side", None).unwrap();

    let both = worktree::list(&state).unwrap();
    assert_eq!(both.len(), 2);
    let added = both.iter().find(|tree| !tree.is_main).unwrap();
    assert_eq!(added.name, "side-folder");
    assert_eq!(added.branch.as_deref(), Some("side"));
    assert!(!added.is_current);
    // The folder is an ordinary checkout of the branch.
    assert_eq!(
        git_at(&dest, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "side"
    );

    // One branch, one folder: a second worktree on the same branch is refused,
    // which is what makes the sidebar disable the menu item.
    let second = parent.join("side-again").to_string_lossy().into_owned();
    assert!(worktree::add(&state, &second, "side", None).is_err());

    worktree::remove(&state, &dest_arg, false).unwrap();
    assert_eq!(worktree::list(&state).unwrap().len(), 1);
    assert!(!dest.exists());

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn a_worktree_with_open_changes_is_kept_unless_forced() {
    let sandbox = Sandbox::new("worktree-dirty");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["branch", "side"]);
    let state = sandbox.state();

    let parent = scratch("worktree-dirty-dest");
    let dest = parent.join("side-folder");
    let dest_arg = dest.to_string_lossy().into_owned();
    worktree::add(&state, &dest_arg, "side", None).unwrap();
    std::fs::write(dest.join("forgotten.txt"), "half-finished\n").unwrap();

    assert!(
        worktree::remove(&state, &dest_arg, false).is_err(),
        "uncommitted work is exactly what a remove must not eat quietly"
    );
    assert!(dest.exists());

    worktree::remove(&state, &dest_arg, true).unwrap();
    assert!(!dest.exists());

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn a_remote_only_branch_gets_a_tracking_branch_in_its_worktree() {
    let sandbox = Sandbox::new("worktree-remote");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("b.txt", "two\n", "Second");
    sandbox.git(&["checkout", "-q", "main"]);
    bare_origin(&sandbox, "worktree-remote");
    sandbox.git(&["push", "-q", "origin", "feature"]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["branch", "-q", "-D", "feature"]);
    let state = sandbox.state();

    let parent = scratch("worktree-remote-dest");
    let dest = parent.join("feature-folder");
    let dest_arg = dest.to_string_lossy().into_owned();
    worktree::add(&state, &dest_arg, "feature", Some("origin/feature")).unwrap();

    assert_eq!(
        git_at(&dest, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "feature"
    );
    assert_eq!(
        git_at(&dest, &["rev-parse", "--abbrev-ref", "feature@{upstream}"]).trim(),
        "origin/feature",
        "the branch should follow the remote one it was made from"
    );

    let _ = std::fs::remove_dir_all(&parent);
}

// --- interactive rebase ------------------------------------------------------

/// Puts a rebase into the state `rebase::start` would, without going through
/// the sequence editor: the editor is this app's own binary, and the test
/// binary is not it. Everything after the todo list is written is the code
/// under test.
fn begin_rebase(sandbox: &Sandbox, onto: &str, todo: &str, rewords: &[&str]) -> bool {
    let list = sandbox.root.join("todo-under-test");
    std::fs::write(&list, todo).unwrap();
    let git_dir = sandbox.root.join(".git");
    std::fs::write(git_dir.join("gitnoob-rebase-rewords"), rewords.join("\n")).unwrap();

    Command::new("git")
        .args(["rebase", "-i", onto])
        .current_dir(&sandbox.root)
        .env("GIT_SEQUENCE_EDITOR", format!("cp '{}'", list.display()))
        .env("GIT_EDITOR", "true")
        .output()
        .expect("git should be on PATH")
        .status
        .success()
}

fn oids(sandbox: &Sandbox) -> Vec<String> {
    sandbox
        .git(&["log", "--format=%H", "--reverse"])
        .lines()
        .map(str::to_string)
        .collect()
}

fn subjects(sandbox: &Sandbox) -> Vec<String> {
    sandbox
        .git(&["log", "--format=%s", "--reverse"])
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn a_plan_lists_the_commits_above_a_chosen_one_oldest_first() {
    let sandbox = Sandbox::new("rebase-plan");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("b.txt", "two\n", "Second");
    sandbox.commit("c.txt", "three\n", "Third");
    let base = oids(&sandbox)[0].clone();

    let plan = rebase::plan(&sandbox.state(), &base).unwrap();
    let summaries: Vec<&str> = plan.iter().map(|c| c.summary.as_str()).collect();
    assert_eq!(summaries, vec!["Second", "Third"]);
    // Nothing has a remote here, so nothing is published.
    assert!(plan.iter().all(|c| !c.pushed));
}

#[test]
fn a_plan_from_head_itself_is_refused_rather_than_empty() {
    let sandbox = Sandbox::new("rebase-empty");
    sandbox.commit("a.txt", "one\n", "First");
    let head = oids(&sandbox)[0].clone();

    let refused = rebase::plan(&sandbox.state(), &head);
    assert!(refused.unwrap_err().contains("Nothing to rebase"));
}

#[test]
fn a_plan_says_which_commits_a_remote_already_has() {
    let sandbox = Sandbox::new("rebase-pushed");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("b.txt", "two\n", "Second");
    // A remote-tracking ref standing where the second commit is.
    let second = oids(&sandbox)[1].clone();
    sandbox.git(&["update-ref", "refs/remotes/origin/main", &second]);
    sandbox.commit("c.txt", "three\n", "Third");
    let base = oids(&sandbox)[0].clone();

    let plan = rebase::plan(&sandbox.state(), &base).unwrap();
    assert_eq!(plan[0].summary, "Second");
    assert!(plan[0].pushed, "the remote has this one");
    assert!(!plan[1].pushed, "the remote has never seen this one");
}

#[test]
fn a_fixup_folds_a_commit_into_the_one_before_it() {
    let sandbox = Sandbox::new("rebase-fixup");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("b.txt", "two\n", "Second");
    sandbox.commit("b.txt", "two fixed\n", "typo");
    let ids = oids(&sandbox);
    let todo = format!("pick {}\nfixup {}\n", ids[1], ids[2]);

    assert!(begin_rebase(&sandbox, &ids[0], &todo, &[]));
    assert_eq!(subjects(&sandbox), vec!["First", "Second"]);
    // The folded change is still there; only its commit is gone.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("b.txt")).unwrap(),
        "two fixed\n"
    );
    assert!(rebase::progress(&sandbox.state()).unwrap().is_none());
}

#[test]
fn a_reword_stops_and_reports_itself_as_one() {
    let sandbox = Sandbox::new("rebase-reword");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("b.txt", "two\n", "wip");
    let ids = oids(&sandbox);
    // A reword is written into the todo as an edit; the sidecar is what says
    // it was meant as a reword.
    let todo = format!("edit {}\n", ids[1]);
    begin_rebase(&sandbox, &ids[0], &todo, &[ids[1].clone().as_str()]);

    let state = sandbox.state();
    let stopped = rebase::progress(&state)
        .unwrap()
        .expect("it should have stopped");
    assert!(stopped.rewording, "the sidecar names this one a reword");
    assert_eq!(stopped.stopped.as_deref(), Some(ids[1].as_str()));
    assert_eq!(stopped.summary.as_deref(), Some("wip"));
    assert_eq!(stopped.message.as_deref(), Some("wip"));
    assert_eq!(stopped.at, 1);
    assert_eq!(stopped.total, 1);

    let said = rebase::reword(&state, "feat: a proper message").unwrap();
    assert_eq!(said, "Rebase finished");
    assert_eq!(subjects(&sandbox), vec!["First", "feat: a proper message"]);
    assert!(rebase::progress(&state).unwrap().is_none());
    // The sidecar goes with the rebase it belonged to.
    assert!(!sandbox.root.join(".git/gitnoob-rebase-rewords").exists());
}

#[test]
fn an_edit_that_was_not_a_reword_is_not_reported_as_one() {
    let sandbox = Sandbox::new("rebase-edit");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("b.txt", "two\n", "Second");
    let ids = oids(&sandbox);
    let todo = format!("edit {}\n", ids[1]);
    begin_rebase(&sandbox, &ids[0], &todo, &[]);

    let state = sandbox.state();
    let stopped = rebase::progress(&state).unwrap().unwrap();
    assert!(!stopped.rewording);
    assert!(stopped.message.is_none(), "nothing to prefill a box with");

    assert_eq!(rebase::resume(&state).unwrap(), "Rebase finished");
    assert_eq!(subjects(&sandbox), vec!["First", "Second"]);
}

#[test]
fn a_conflict_stops_the_rebase_and_abort_puts_the_branch_back() {
    let sandbox = Sandbox::new("rebase-conflict");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("a.txt", "from the side\n", "Side change");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "from main\n", "Main change");
    let before = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();

    // Replaying main's commit on top of side's conflicts in a.txt.
    let side = sandbox.git(&["rev-parse", "side"]).trim().to_string();
    let head = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    let todo = format!("pick {head}\n");
    assert!(
        !begin_rebase(&sandbox, &side, &todo, &[]),
        "it should conflict"
    );

    let state = sandbox.state();
    assert!(rebase::progress(&state).unwrap().is_some());
    assert!(remote::in_progress(&state).unwrap().rebasing);

    let said = rebase::abort(&state).unwrap();
    assert!(said.contains("as it was"));
    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), before);
    assert!(rebase::progress(&state).unwrap().is_none());
    assert!(!sandbox.root.join(".git/gitnoob-rebase-todo").exists());
}

#[test]
fn skipping_leaves_the_commit_it_stopped_at_out() {
    let sandbox = Sandbox::new("rebase-skip");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("a.txt", "from the side\n", "Side change");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a.txt", "from main\n", "Main change");

    let side = sandbox.git(&["rev-parse", "side"]).trim().to_string();
    let head = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    begin_rebase(&sandbox, &side, &format!("pick {head}\n"), &[]);

    let state = sandbox.state();
    assert_eq!(rebase::skip(&state).unwrap(), "Rebase finished");
    assert_eq!(subjects(&sandbox), vec!["First", "Side change"]);
}

#[test]
fn rebase_progress_says_nothing_when_no_rebase_is_running() {
    let sandbox = Sandbox::new("rebase-idle");
    sandbox.commit("a.txt", "one\n", "First");
    assert!(rebase::progress(&sandbox.state()).unwrap().is_none());
}

// --- squash ------------------------------------------------------------------

/// Puts a stand-in in the slot the app's own binary fills.
///
/// `rebase::squash` starts a real `git rebase -i`, and the editor it names is
/// `gitnoob --write-todo`. The test binary is not gitnoob, so it hands git
/// `cp` instead — which is the whole of what `--write-todo` does. Set once for
/// the process, to the same value, so tests running side by side agree.
fn lend_git_an_editor() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| std::env::set_var("GITNOOB_SEQUENCE_EDITOR", "cp"));
}

/// Three commits on main, the last two of which are the obvious fold.
fn three_commits(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("b.txt", "two\n", "Add the parser");
    sandbox.commit("b.txt", "two fixed\n", "wip: fix the parser");
    sandbox
}

#[test]
fn squashing_a_run_leaves_one_commit_carrying_the_message_given() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-run");
    let ids = oids(&sandbox);

    let said = rebase::squash(
        &sandbox.state(),
        &[ids[1].clone(), ids[2].clone()],
        "feat: a parser that works",
    )
    .unwrap();

    assert!(said.contains("Squashed 2"), "{said}");
    assert_eq!(subjects(&sandbox), vec!["First", "feat: a parser that works"]);
    // The changes both commits made are still there; only the second commit is.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("b.txt")).unwrap(),
        "two fixed\n"
    );
    assert!(rebase::progress(&sandbox.state()).unwrap().is_none());
    // Nothing of the fold is left lying beside git's own files.
    assert!(!sandbox.root.join(".git/gitnoob-squash-message").exists());
    assert!(!sandbox.root.join(".git/gitnoob-rebase-todo").exists());
}

#[test]
fn the_message_is_taken_whole_body_and_all() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-body");
    let ids = oids(&sandbox);

    rebase::squash(
        &sandbox.state(),
        &[ids[1].clone(), ids[2].clone()],
        "feat: a parser\n\nWhy it had to change, at length.\n",
    )
    .unwrap();

    let body = sandbox.git(&["log", "-1", "--format=%B"]);
    assert_eq!(body.trim(), "feat: a parser\n\nWhy it had to change, at length.");
}

#[test]
fn the_commits_above_the_fold_are_replayed_onto_it() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-above");
    sandbox.commit("c.txt", "three\n", "Later work");
    sandbox.commit("d.txt", "four\n", "Later still");
    let ids = oids(&sandbox);

    rebase::squash(
        &sandbox.state(),
        &[ids[1].clone(), ids[2].clone()],
        "feat: the parser",
    )
    .unwrap();

    assert_eq!(
        subjects(&sandbox),
        vec!["First", "feat: the parser", "Later work", "Later still"]
    );
    // Everything each of them changed is still in the tree.
    for (file, content) in [("b.txt", "two fixed\n"), ("c.txt", "three\n"), ("d.txt", "four\n")] {
        assert_eq!(
            std::fs::read_to_string(sandbox.root.join(file)).unwrap(),
            content,
            "{file}"
        );
    }
}

#[test]
fn a_fold_that_reaches_the_first_commit_rebases_from_the_root() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-root");
    let ids = oids(&sandbox);

    rebase::squash(&sandbox.state(), &ids, "feat: all of it at once").unwrap();

    assert_eq!(subjects(&sandbox), vec!["feat: all of it at once"]);
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\n"
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("b.txt")).unwrap(),
        "two fixed\n"
    );
}

#[test]
fn uncommitted_work_is_stashed_over_the_fold_and_put_back() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-dirty");
    let ids = oids(&sandbox);
    sandbox.write("a.txt", "one, half-edited\n");

    rebase::squash(
        &sandbox.state(),
        &[ids[1].clone(), ids[2].clone()],
        "feat: the parser",
    )
    .unwrap();

    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one, half-edited\n",
        "the edit in progress should survive the fold"
    );
    assert_eq!(subjects(&sandbox), vec!["First", "feat: the parser"]);
}

#[test]
fn undoing_a_squash_puts_the_commits_back_and_redoing_folds_them_again() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-undo");
    let ids = oids(&sandbox);
    let state = sandbox.state();
    let before = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();

    rebase::squash(&state, &[ids[1].clone(), ids[2].clone()], "feat: the parser").unwrap();
    let folded = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    assert_ne!(folded, before);

    let said = journal::undo(&state).unwrap();
    assert!(said.contains("Squash 2 commits"), "{said}");
    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), before);
    assert_eq!(
        subjects(&sandbox),
        vec!["First", "Add the parser", "wip: fix the parser"]
    );

    journal::redo(&state).unwrap();
    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), folded);
    assert_eq!(subjects(&sandbox), vec!["First", "feat: the parser"]);
}

#[test]
fn undoing_a_squash_leaves_the_files_alone() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-undo-files");
    let ids = oids(&sandbox);
    let state = sandbox.state();

    rebase::squash(&state, &[ids[1].clone(), ids[2].clone()], "feat: the parser").unwrap();
    sandbox.write("a.txt", "edited after the fold\n");
    journal::undo(&state).unwrap();

    // The fold left the same tree it started from, so stepping the branch back
    // is all an undo has to do — and work started since is not its business.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "edited after the fold\n"
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("b.txt")).unwrap(),
        "two fixed\n"
    );
}

#[test]
fn the_preview_joins_the_messages_oldest_first() {
    let sandbox = three_commits("squash-preview");
    let ids = oids(&sandbox);

    // Handed newest first, as a shift-click upwards would.
    let preview =
        rebase::squash_preview(&sandbox.state(), &[ids[2].clone(), ids[1].clone()]).unwrap();

    let summaries: Vec<&str> = preview.commits.iter().map(|c| c.summary.as_str()).collect();
    assert_eq!(summaries, vec!["Add the parser", "wip: fix the parser"]);
    assert_eq!(preview.message, "Add the parser\n\nwip: fix the parser");
    assert_eq!(preview.onto.as_deref(), Some(&ids[0][..7]));
    assert_eq!(preview.above, 0);
    assert_eq!(preview.branch.as_deref(), Some("main"));
    assert!(preview.refusal.is_none());
    assert!(preview.commits.iter().all(|c| !c.pushed));
}

#[test]
fn the_preview_counts_what_would_be_replayed_over_the_fold() {
    let sandbox = three_commits("squash-preview-above");
    sandbox.commit("c.txt", "three\n", "Later work");
    let ids = oids(&sandbox);

    let preview =
        rebase::squash_preview(&sandbox.state(), &[ids[1].clone(), ids[2].clone()]).unwrap();
    assert_eq!(preview.above, 1);
}

#[test]
fn the_preview_says_at_the_root_that_there_is_nothing_underneath() {
    let sandbox = three_commits("squash-preview-root");
    let ids = oids(&sandbox);

    let preview = rebase::squash_preview(&sandbox.state(), &ids).unwrap();
    assert!(preview.onto.is_none(), "there is no commit under the first");
    assert_eq!(preview.commits.len(), 3);
}

#[test]
fn the_preview_says_which_commits_a_remote_already_has() {
    let sandbox = three_commits("squash-preview-pushed");
    let ids = oids(&sandbox);
    sandbox.git(&["update-ref", "refs/remotes/origin/main", &ids[1]]);

    let preview =
        rebase::squash_preview(&sandbox.state(), &[ids[1].clone(), ids[2].clone()]).unwrap();
    assert!(preview.commits[0].pushed, "the remote has this one");
    assert!(!preview.commits[1].pushed);
}

#[test]
fn commits_with_another_between_them_are_refused_with_the_reason() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-gap");
    sandbox.commit("c.txt", "three\n", "In between");
    sandbox.commit("d.txt", "four\n", "The far one");
    let ids = oids(&sandbox);

    // The second and the fourth, with two commits standing between them.
    let picked = [ids[1].clone(), ids[4].clone()];
    let preview = rebase::squash_preview(&sandbox.state(), &picked).unwrap();
    let refusal = preview.refusal.expect("it cannot fold these");
    assert!(refusal.contains("not next to each other"), "{refusal}");
    assert!(refusal.contains("2 other commits sit"), "{refusal}");
    // It still describes what was picked, so the dialog can name them.
    assert_eq!(preview.commits.len(), 2);

    let refused = rebase::squash(&sandbox.state(), &picked, "feat: nope").unwrap_err();
    assert!(refused.contains("not next to each other"), "{refused}");
    // And nothing was rewritten on the way to saying so.
    assert_eq!(
        subjects(&sandbox),
        vec![
            "First",
            "Add the parser",
            "wip: fix the parser",
            "In between",
            "The far one"
        ]
    );
}

#[test]
fn one_commit_between_two_is_counted_in_the_singular() {
    let sandbox = three_commits("squash-gap-one");
    sandbox.commit("c.txt", "three\n", "The far one");
    let ids = oids(&sandbox);

    let preview =
        rebase::squash_preview(&sandbox.state(), &[ids[1].clone(), ids[3].clone()]).unwrap();
    let refusal = preview.refusal.unwrap();
    assert!(refusal.contains("1 other commit sits"), "{refusal}");
}

#[test]
fn a_commit_that_is_not_on_this_branch_is_refused() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-elsewhere");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("side.txt", "aside\n", "Off to the side");
    let elsewhere = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    sandbox.git(&["checkout", "-q", "main"]);
    let ids = oids(&sandbox);

    let picked = [ids[2].clone(), elsewhere];
    let preview = rebase::squash_preview(&sandbox.state(), &picked).unwrap();
    assert!(preview
        .refusal
        .as_deref()
        .unwrap()
        .contains("not on the branch you are on"));
    assert!(rebase::squash(&sandbox.state(), &picked, "feat: nope").is_err());
}

#[test]
fn a_merge_between_the_run_and_the_tip_is_refused_rather_than_flattened() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-merge");
    let ids = oids(&sandbox);
    sandbox.git(&["checkout", "-q", "-b", "side", &ids[0]]);
    sandbox.commit("side.txt", "aside\n", "Off to the side");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["merge", "-q", "--no-ff", "-m", "Merge side", "side"]);

    let picked = [ids[1].clone(), ids[2].clone()];
    let preview = rebase::squash_preview(&sandbox.state(), &picked).unwrap();
    assert!(preview
        .refusal
        .as_deref()
        .unwrap()
        .contains("merge commit"));

    let refused = rebase::squash(&sandbox.state(), &picked, "feat: nope").unwrap_err();
    assert!(refused.contains("merge commit"), "{refused}");
    // The merge is untouched.
    assert_eq!(
        sandbox.git(&["rev-list", "--merges", "--count", "HEAD"]).trim(),
        "1"
    );
}

#[test]
fn one_commit_on_its_own_is_not_a_squash() {
    let sandbox = three_commits("squash-single");
    let ids = oids(&sandbox);

    let refused = rebase::squash_preview(&sandbox.state(), &[ids[2].clone()]).unwrap_err();
    assert!(refused.contains("at least two"), "{refused}");
    // The same commit twice is still one commit.
    let refused =
        rebase::squash(&sandbox.state(), &[ids[2].clone(), ids[2].clone()], "x").unwrap_err();
    assert!(refused.contains("at least two"), "{refused}");
}

#[test]
fn a_commit_that_is_not_in_the_repository_is_refused_rather_than_crashing() {
    let sandbox = three_commits("squash-bogus");
    let ids = oids(&sandbox);
    let nowhere = "0".repeat(40);

    let preview =
        rebase::squash_preview(&sandbox.state(), &[ids[2].clone(), nowhere.clone()]).unwrap();
    assert!(preview
        .refusal
        .as_deref()
        .unwrap()
        .contains("not on the branch you are on"));
    // The one commit that does exist is still described; the other is left out
    // rather than turning the whole answer into "bad object".
    assert_eq!(preview.commits.len(), 1);
    assert_eq!(preview.commits[0].oid, ids[2]);
    assert!(rebase::squash(&sandbox.state(), &[ids[2].clone(), nowhere], "feat: nope").is_err());
}

#[test]
fn a_squash_on_a_detached_head_moves_the_detached_head() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-detached");
    let ids = oids(&sandbox);
    sandbox.git(&["checkout", "-q", "--detach"]);
    let state = sandbox.state();

    let preview = rebase::squash_preview(&state, &[ids[1].clone(), ids[2].clone()]).unwrap();
    assert!(preview.branch.is_none(), "there is no branch to name");
    assert!(preview.refusal.is_none(), "it is still a run of commits");

    rebase::squash(&state, &[ids[1].clone(), ids[2].clone()], "feat: the parser").unwrap();
    assert_eq!(subjects(&sandbox), vec!["First", "feat: the parser"]);
    // main is where it was: the fold moved HEAD, and HEAD is all it moved.
    assert_eq!(
        sandbox.git(&["log", "--format=%s", "--reverse", "main"]).lines().count(),
        3
    );
    // And it can still be stepped back, without a branch name to check.
    journal::undo(&state).unwrap();
    assert_eq!(
        subjects(&sandbox),
        vec!["First", "Add the parser", "wip: fix the parser"]
    );
}

#[test]
fn staged_work_survives_a_squash() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-staged-dirty");
    let ids = oids(&sandbox);
    sandbox.write("staged.txt", "half an idea\n");
    sandbox.git(&["add", "staged.txt"]);

    rebase::squash(
        &sandbox.state(),
        &[ids[1].clone(), ids[2].clone()],
        "feat: the parser",
    )
    .unwrap();

    // The autostash puts it back. Git's stash does not remember what was
    // staged, so it comes back as an edit rather than as a staged one — but it
    // comes back, which is the part that matters.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("staged.txt")).unwrap(),
        "half an idea\n"
    );
    assert_eq!(subjects(&sandbox), vec!["First", "feat: the parser"]);
}

#[test]
fn a_squash_needs_a_message() {
    let sandbox = three_commits("squash-nomessage");
    let ids = oids(&sandbox);

    let refused =
        rebase::squash(&sandbox.state(), &[ids[1].clone(), ids[2].clone()], "  \n ").unwrap_err();
    assert!(refused.contains("needs a message"), "{refused}");
}

#[test]
fn the_same_commit_picked_twice_is_folded_once() {
    lend_git_an_editor();
    let sandbox = three_commits("squash-dupe");
    let ids = oids(&sandbox);

    // A shift-range over a ctrl-click can hand the same oid twice.
    let said = rebase::squash(
        &sandbox.state(),
        &[ids[1].clone(), ids[2].clone(), ids[1].clone()],
        "feat: the parser",
    )
    .unwrap();
    assert!(said.contains("Squashed 2"), "{said}");
    assert_eq!(subjects(&sandbox), vec!["First", "feat: the parser"]);
}

#[test]
fn folding_keeps_the_author_of_the_oldest_commit() {
    lend_git_an_editor();
    let sandbox = Sandbox::new("squash-author");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["config", "user.name", "Someone Else"]);
    sandbox.git(&["config", "user.email", "else@example.com"]);
    sandbox.commit("b.txt", "two\n", "Theirs");
    sandbox.git(&["config", "user.name", "Test"]);
    sandbox.git(&["config", "user.email", "test@example.com"]);
    sandbox.commit("b.txt", "two fixed\n", "Mine");
    let ids = oids(&sandbox);

    rebase::squash(&sandbox.state(), &[ids[1].clone(), ids[2].clone()], "feat: both").unwrap();

    // git's own fixup rule: the commit that survives is the first one, and it
    // keeps its author.
    assert_eq!(
        sandbox.git(&["log", "-1", "--format=%ae"]).trim(),
        "else@example.com"
    );
}

// --- line-level staging ------------------------------------------------------

#[test]
fn staging_one_line_of_a_hunk_leaves_the_others_unstaged() {
    let sandbox = Sandbox::new("stage-lines");
    sandbox.commit("a.txt", "one\ntwo\nthree\nfour\n", "First");
    // Two separate changes close enough together to land in one hunk.
    sandbox.write("a.txt", "one\nTWO\nthree\nFOUR\n");

    let state = sandbox.state();
    let picked = gitnoob_lib::work::Lines {
        added: vec![2],
        removed: vec![2],
    };
    gitnoob_lib::work::apply_hunk(
        &state,
        "a.txt",
        0,
        gitnoob_lib::work::HunkAction::Stage,
        Some(picked),
    )
    .unwrap();

    let staged = sandbox.git(&["diff", "--cached"]);
    assert!(
        staged.contains("+TWO"),
        "the picked line is staged: {staged}"
    );
    assert!(!staged.contains("+FOUR"), "the other one is not: {staged}");

    // And the working tree still holds both changes.
    let unstaged = sandbox.git(&["diff"]);
    assert!(unstaged.contains("+FOUR"));
    assert!(!unstaged.contains("+TWO"));
}

#[test]
fn unstaging_one_line_leaves_the_rest_of_the_hunk_staged() {
    let sandbox = Sandbox::new("unstage-lines");
    sandbox.commit("a.txt", "one\ntwo\nthree\nfour\n", "First");
    sandbox.write("a.txt", "one\nTWO\nthree\nFOUR\n");
    sandbox.git(&["add", "a.txt"]);

    let state = sandbox.state();
    // Take just the second change back out of the index.
    let picked = gitnoob_lib::work::Lines {
        added: vec![4],
        removed: vec![4],
    };
    gitnoob_lib::work::apply_hunk(
        &state,
        "a.txt",
        0,
        gitnoob_lib::work::HunkAction::Unstage,
        Some(picked),
    )
    .unwrap();

    let staged = sandbox.git(&["diff", "--cached"]);
    assert!(
        staged.contains("+TWO"),
        "the first change stays staged: {staged}"
    );
    assert!(
        !staged.contains("+FOUR"),
        "the second one came back out: {staged}"
    );
    // Nothing was lost from the file itself.
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\nTWO\nthree\nFOUR\n"
    );
}

#[test]
fn discarding_one_line_puts_only_that_line_back() {
    let sandbox = Sandbox::new("discard-lines");
    sandbox.commit("a.txt", "one\ntwo\nthree\nfour\n", "First");
    sandbox.write("a.txt", "one\nTWO\nthree\nFOUR\n");

    let state = sandbox.state();
    let picked = gitnoob_lib::work::Lines {
        added: vec![2],
        removed: vec![2],
    };
    gitnoob_lib::work::apply_hunk(
        &state,
        "a.txt",
        0,
        gitnoob_lib::work::HunkAction::Discard,
        Some(picked),
    )
    .unwrap();

    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\ntwo\nthree\nFOUR\n",
        "the picked line went back, the other change stayed"
    );
}

#[test]
fn staging_only_an_addition_keeps_the_removal_in_the_working_tree() {
    let sandbox = Sandbox::new("stage-added-only");
    sandbox.commit("a.txt", "one\ntwo\nthree\n", "First");
    // One line replaced: a removal and an addition in the same place.
    sandbox.write("a.txt", "one\nTWO\nthree\n");

    let state = sandbox.state();
    // Take the addition without the removal, which is the awkward half.
    let picked = gitnoob_lib::work::Lines {
        added: vec![2],
        removed: vec![],
    };
    gitnoob_lib::work::apply_hunk(
        &state,
        "a.txt",
        0,
        gitnoob_lib::work::HunkAction::Stage,
        Some(picked),
    )
    .unwrap();

    // The index now has both lines; the working tree still has only the new
    // one, so what is left unstaged is the removal.
    let staged = sandbox.git(&["diff", "--cached"]);
    assert!(staged.contains("+TWO"));
    assert!(!staged.contains("-two"));
    let unstaged = sandbox.git(&["diff"]);
    assert!(unstaged.contains("-two"));
}

#[test]
fn no_lines_at_all_still_stages_the_whole_hunk() {
    let sandbox = Sandbox::new("stage-whole");
    sandbox.commit("a.txt", "one\ntwo\n", "First");
    sandbox.write("a.txt", "one\nTWO\n");

    let state = sandbox.state();
    // An empty selection is the same request as no selection: the whole hunk.
    gitnoob_lib::work::apply_hunk(
        &state,
        "a.txt",
        0,
        gitnoob_lib::work::HunkAction::Stage,
        Some(gitnoob_lib::work::Lines::default()),
    )
    .unwrap();
    assert!(sandbox.git(&["diff", "--cached"]).contains("+TWO"));
    assert!(sandbox.git(&["diff"]).trim().is_empty());
}

// --- applying several stashes at once ----------------------------------------

/// Three stashes, each touching a file of its own, so they can all go on
/// together. Made newest-last, so `stash@{0}` is the third.
fn three_stashes(sandbox: &Sandbox) {
    sandbox.commit("base.txt", "base\n", "First");
    for name in ["one", "two", "three"] {
        sandbox.write(&format!("{name}.txt"), &format!("{name}\n"));
        sandbox.git(&["add", "-A"]);
        sandbox.git(&["stash", "push", "-q", "-m", name]);
    }
}

#[test]
fn several_stashes_go_on_oldest_first_and_stay_in_the_list() {
    let sandbox = Sandbox::new("stash-many");
    three_stashes(&sandbox);

    let state = sandbox.state();
    let run = gitnoob_lib::work::stash_apply_many(&state, vec![0, 1, 2], false).unwrap();

    assert_eq!(run.applied, vec!["one", "two", "three"], "oldest first");
    assert!(run.stopped.is_none());
    for name in ["one", "two", "three"] {
        assert!(
            sandbox.root.join(format!("{name}.txt")).exists(),
            "{name} went on"
        );
    }
    // Applying keeps them.
    assert_eq!(sandbox.git(&["stash", "list"]).lines().count(), 3);
}

#[test]
fn popping_several_takes_each_one_off_the_list() {
    let sandbox = Sandbox::new("stash-pop-many");
    three_stashes(&sandbox);

    let state = sandbox.state();
    let run = gitnoob_lib::work::stash_apply_many(&state, vec![0, 1, 2], true).unwrap();

    assert_eq!(run.applied.len(), 3);
    assert!(run.stopped.is_none());
    assert_eq!(
        sandbox.git(&["stash", "list"]).trim(),
        "",
        "every one of them was dropped"
    );
}

#[test]
fn popping_some_of_them_drops_those_and_leaves_the_rest() {
    let sandbox = Sandbox::new("stash-pop-some");
    three_stashes(&sandbox);

    // The oldest and the newest; `two` is left alone. Positions renumber as
    // the first drop lands, which is the trap this is here for.
    let state = sandbox.state();
    let run = gitnoob_lib::work::stash_apply_many(&state, vec![0, 2], true).unwrap();

    assert_eq!(run.applied, vec!["one", "three"]);
    let left = sandbox.git(&["stash", "list"]);
    assert_eq!(left.lines().count(), 1);
    assert!(
        left.contains("two"),
        "the one not picked is still there: {left}"
    );
    assert!(sandbox.root.join("one.txt").exists());
    assert!(sandbox.root.join("three.txt").exists());
    assert!(!sandbox.root.join("two.txt").exists());
}

#[test]
fn uncommitted_work_is_not_in_the_way_of_a_stash_going_on() {
    let sandbox = Sandbox::new("stash-over-dirty");
    three_stashes(&sandbox);
    // Something already changed in the working tree, on a file no stash touches.
    sandbox.write("base.txt", "base, and edited\n");

    let state = sandbox.state();
    let run = gitnoob_lib::work::stash_apply_many(&state, vec![0, 1], false).unwrap();

    assert_eq!(run.applied.len(), 2);
    assert!(run.stopped.is_none());
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("base.txt")).unwrap(),
        "base, and edited\n",
        "the edit is still there"
    );
}

#[test]
fn a_stash_that_collides_stops_the_run_and_says_which_one() {
    let sandbox = Sandbox::new("stash-collide");
    sandbox.commit("a.txt", "one\n", "First");

    // Two stashes that both rewrite the same line: the second cannot go on
    // over the first.
    sandbox.write("a.txt", "from the first stash\n");
    sandbox.git(&["stash", "push", "-q", "-m", "first"]);
    sandbox.write("a.txt", "from the second stash\n");
    sandbox.git(&["stash", "push", "-q", "-m", "second"]);

    let state = sandbox.state();
    let run = gitnoob_lib::work::stash_apply_many(&state, vec![0, 1], true).unwrap();

    assert_eq!(run.applied, vec!["first"], "the older one went on");
    let stopped = run.stopped.expect("the second should have stopped it");
    assert_eq!(stopped.message, "second");
    assert!(!stopped.reason.is_empty(), "git said why");

    // The one that stopped it is still in the list: nothing is dropped that
    // did not go on.
    let left = sandbox.git(&["stash", "list"]);
    assert!(left.contains("second"), "{left}");
    assert!(!left.contains("first"), "{left}");
}

#[test]
fn picking_a_stash_that_is_not_there_is_refused_before_anything_happens() {
    let sandbox = Sandbox::new("stash-missing");
    three_stashes(&sandbox);

    let state = sandbox.state();
    let refused = gitnoob_lib::work::stash_apply_many(&state, vec![0, 9], true);
    assert!(refused.unwrap_err().contains("no stash at position 9"));
    // Nothing was applied, and nothing dropped.
    assert_eq!(sandbox.git(&["stash", "list"]).lines().count(), 3);
    assert!(!sandbox.root.join("three.txt").exists());
}



