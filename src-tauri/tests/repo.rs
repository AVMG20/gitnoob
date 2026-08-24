//! End-to-end checks against real repositories built with the `git` CLI.
//!
//! These cover the parts that are easy to get subtly wrong — graph lane layout,
//! divergence reporting, conflict marker parsing — rather than the thin command
//! wrappers around them.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use gitui_lib::state::AppState;
use gitui_lib::{conflict, diff, graph, refs, remote};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A throwaway repository, removed when the test ends.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("gitui-test-{tag}-{}-{id}", std::process::id()));
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
        // Point the config at the sandbox so tests never touch the real one.
        let state = AppState::new(self.root.join(".gitui-config"));
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

    let found = gitui_lib::state::discover_workdir(&sandbox.root.join("deep/nested")).unwrap();
    // Compare canonically: macOS temp paths go through a /private symlink.
    assert_eq!(
        found.canonicalize().unwrap(),
        sandbox.root.canonicalize().unwrap()
    );
}

#[test]
fn rejects_a_directory_outside_any_repository() {
    let root = std::env::temp_dir().join(format!("gitui-not-a-repo-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    assert!(gitui_lib::state::discover_workdir(&root).is_err());
    let _ = std::fs::remove_dir_all(&root);
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
    assert!(tree.locals.iter().find(|b| b.name == "main").unwrap().is_head);
    assert_eq!(tree.tags.len(), 1);
    assert_eq!(tree.tags[0].name, "v1");
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
    assert!(file_diff
        .hunks[0]
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
        "gitui-test-push-origin-{}.git",
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
    assert_eq!(detail.parents.len(), 2, "the merge commit should have two parents");
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

    let draft = gitui_lib::work::amend_draft(&state).unwrap();
    assert_eq!(draft.summary, "Only commit");
    assert_eq!(draft.body, "With a body.");
    assert!(!draft.is_pushed, "nothing has been pushed yet");

    let bare = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitui-test-amend-origin-{}.git", std::process::id()));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);

    let published = gitui_lib::work::amend_draft(&state).unwrap();
    assert!(published.is_pushed, "the commit is now on a remote");

    let _ = std::fs::remove_dir_all(&bare);
}

#[test]
fn stage_unstage_and_discard_move_files_around() {
    let sandbox = Sandbox::new("work");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\ntwo\n");
    let state = sandbox.state();

    gitui_lib::work::stage(&state, &["a.txt".to_string()]).unwrap();
    assert!(refs::status(&state)
        .unwrap()
        .staged
        .iter()
        .any(|e| e.path == "a.txt"));

    gitui_lib::work::unstage(&state, &["a.txt".to_string()]).unwrap();
    assert!(refs::status(&state).unwrap().staged.is_empty());

    gitui_lib::work::discard(&state, &["a.txt".to_string()]).unwrap();
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

    let bare = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitui-test-track-origin-{}.git", std::process::id()));
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
    gitui_lib::work::stage(&state, &["a.txt".to_string()]).unwrap();
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
    gitui_lib::work::stage(&state, &["a.txt".to_string()]).unwrap();
    gitui_lib::work::commit(&state, "Second commit", false).unwrap();

    let after = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();
    assert_ne!(after, base);

    let stacks = gitui_lib::journal::stacks(&state);
    assert_eq!(stacks.undo.len(), 1);
    assert_eq!(stacks.undo[0].label, "Commit: Second commit");
    assert!(stacks.redo.is_empty());

    gitui_lib::journal::undo(&state).unwrap();
    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), base);
    // A soft reset keeps the work staged rather than throwing it away.
    let status = refs::status(&state).unwrap();
    assert!(status.staged.iter().any(|e| e.path == "a.txt"));
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );

    let stacks = gitui_lib::journal::stacks(&state);
    assert!(stacks.undo.is_empty());
    assert_eq!(stacks.redo.len(), 1);

    gitui_lib::journal::redo(&state).unwrap();
    assert_eq!(sandbox.git(&["rev-parse", "HEAD"]).trim(), after);
}

#[test]
fn undo_of_an_amend_restores_the_original_commit() {
    let sandbox = Sandbox::new("undoamend");
    sandbox.commit("a.txt", "one\n", "Original message");
    let original = sandbox.git(&["rev-parse", "HEAD"]).trim().to_string();

    let state = sandbox.state();
    gitui_lib::work::commit(&state, "Reworded message", true).unwrap();
    assert_ne!(sandbox.git(&["rev-parse", "HEAD"]).trim(), original);

    gitui_lib::journal::undo(&state).unwrap();
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
    gitui_lib::work::stage(&state, &["a.txt".to_string()]).unwrap();
    gitui_lib::work::commit(&state, "Third", false).unwrap();

    sandbox.git(&["checkout", "-q", "-b", "elsewhere"]);
    let error = gitui_lib::journal::undo(&state).unwrap_err();
    assert!(error.contains("elsewhere"), "unexpected message: {error}");

    // Refusing must not consume the step.
    assert_eq!(gitui_lib::journal::stacks(&state).undo.len(), 1);
}

#[test]
fn undo_reports_when_there_is_nothing_to_do() {
    let sandbox = Sandbox::new("undoempty");
    sandbox.commit("a.txt", "one\n", "First");
    let state = sandbox.state();
    assert!(gitui_lib::journal::undo(&state).is_err());
    assert!(gitui_lib::journal::redo(&state).is_err());
}

#[test]
fn switching_branches_stashes_and_restores_local_changes() {
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
    assert!(gitui_lib::work::stash_list(&state).unwrap().is_empty());

    // The switch is undoable.
    let stacks = gitui_lib::journal::stacks(&state);
    assert_eq!(stacks.undo[0].label, "Switch to other");
    gitui_lib::journal::undo(&state).unwrap();
    assert_eq!(refs::describe(&state).unwrap().head, "main");
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
    gitui_lib::work::stash_push(&state, Some("my work in progress"), true).unwrap();

    let stashes = gitui_lib::work::stash_list(&state).unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0].index, 0);
    assert_eq!(stashes[0].message, "my work in progress");
    assert_eq!(stashes[0].branch.as_deref(), Some("main"));
    assert!(stashes[0].files >= 1);
    assert_eq!(stashes[0].oid.len(), 40);

    // The stash commit is diffable like any other, which is how the UI shows it.
    let oid = gitui_lib::work::stash_oid(&state, 0).unwrap();
    let detail = diff::commit_detail(&state, &oid).unwrap();
    assert!(detail.files.iter().any(|f| f.path == "a.txt"));
}

#[test]
fn stash_apply_keeps_the_entry_and_drop_removes_it() {
    let sandbox = Sandbox::new("stashops");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\nedited\n");

    let state = sandbox.state();
    gitui_lib::work::stash_push(&state, Some("keep me"), false).unwrap();
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\n"
    );

    gitui_lib::work::stash_apply(&state, 0).unwrap();
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\nedited\n"
    );
    // Apply, unlike pop, leaves the entry in place.
    assert_eq!(gitui_lib::work::stash_list(&state).unwrap().len(), 1);

    gitui_lib::work::stash_drop(&state, 0).unwrap();
    assert!(gitui_lib::work::stash_list(&state).unwrap().is_empty());
}

#[test]
fn a_stash_can_become_a_branch() {
    let sandbox = Sandbox::new("stashbranch");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.write("a.txt", "one\nwork\n");

    let state = sandbox.state();
    gitui_lib::work::stash_push(&state, Some("rescue this"), false).unwrap();
    gitui_lib::work::stash_branch(&state, 0, "rescued").unwrap();

    assert_eq!(refs::describe(&state).unwrap().head, "rescued");
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\nwork\n"
    );
    // `git stash branch` consumes the entry.
    assert!(gitui_lib::work::stash_list(&state).unwrap().is_empty());
}

#[test]
fn pull_stashes_local_work_and_puts_it_back() {
    let sandbox = Sandbox::new("pullstash");
    sandbox.commit("a.txt", "one\n", "First");

    // A remote that has moved on.
    let bare = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitui-test-pull-origin-{}.git", std::process::id()));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["branch", "--set-upstream-to=origin/main", "main"]);

    let clone = sandbox.root.parent().unwrap().join(format!(
        "gitui-test-pull-clone-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&clone);
    Command::new("git")
        .args(["clone", "-q", &bare_arg, clone.to_str().unwrap()])
        .output()
        .unwrap();
    for args in [
        vec!["config", "user.name", "Other"],
        vec!["config", "user.email", "other@example.com"],
    ] {
        Command::new("git").args(&args).current_dir(&clone).output().unwrap();
    }
    std::fs::write(clone.join("remote-side.txt"), "from the remote\n").unwrap();
    for args in [
        vec!["add", "--all"],
        vec!["commit", "-q", "-m", "Remote side work"],
        vec!["push", "-q", "origin", "main"],
    ] {
        Command::new("git").args(&args).current_dir(&clone).output().unwrap();
    }

    // Local edit that would make a plain pull refuse.
    sandbox.write("a.txt", "one\nlocal edit\n");
    let state = sandbox.state();
    let output = remote::pull(&state, false).unwrap();

    assert!(output.ok, "pull failed: {} {}", output.stdout, output.stderr);
    // Both the remote commit and the local edit are present.
    assert!(sandbox.root.join("remote-side.txt").exists());
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap(),
        "one\nlocal edit\n"
    );
    assert!(gitui_lib::work::stash_list(&state).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&bare);
    let _ = std::fs::remove_dir_all(&clone);
}

#[test]
fn parses_forge_remotes_and_reports_status() {
    let sandbox = Sandbox::new("forge");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["remote", "add", "origin", "git@gitlab.bigbridge.nl:team/sub/app.git"]);

    let state = sandbox.state();
    let status = gitui_lib::forge::status(&state);
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
    gitui_lib::work::apply_hunk(&state, "a.txt", 0, gitui_lib::work::HunkAction::Stage).unwrap();

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
    gitui_lib::work::apply_hunk(&state, "a.txt", 0, gitui_lib::work::HunkAction::Unstage).unwrap();
    assert!(refs::status(&state).unwrap().staged.is_empty());

    // Discarding the remaining region leaves the other edit alone.
    let before = std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap();
    assert!(before.contains("first addition") && before.contains("second addition"));
    gitui_lib::work::apply_hunk(&state, "a.txt", 1, gitui_lib::work::HunkAction::Discard).unwrap();
    let after = std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap();
    assert!(after.contains("first addition"));
    assert!(!after.contains("second addition"));
}

#[test]
fn hunk_staging_refuses_when_there_is_nothing_to_stage() {
    let sandbox = Sandbox::new("hunksempty");
    sandbox.commit("a.txt", "one\n", "Base");
    let state = sandbox.state();
    let error =
        gitui_lib::work::apply_hunk(&state, "a.txt", 0, gitui_lib::work::HunkAction::Stage)
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
    gitui_lib::work::cherry_pick(
        &state,
        &[third, second, first],
        gitui_lib::work::CherryPickOptions::default(),
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

    gitui_lib::work::cherry_pick(
        &state,
        &[oid],
        gitui_lib::work::CherryPickOptions {
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
    gitui_lib::work::cherry_pick(
        &state,
        &[oid.clone()],
        gitui_lib::work::CherryPickOptions {
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
    let error = gitui_lib::work::cherry_pick(
        &state,
        &[],
        gitui_lib::work::CherryPickOptions::default(),
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
    assert!(!message.trim().is_empty(), "expected a message, got {message:?}");
    assert!(message.contains("feature"));
    assert_eq!(refs::describe(&state).unwrap().head, "feature");
}
