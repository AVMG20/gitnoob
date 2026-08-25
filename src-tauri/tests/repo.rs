//! End-to-end checks against real repositories built with the `git` CLI.
//!
//! These cover the parts that are easy to get subtly wrong — graph lane layout,
//! divergence reporting, conflict marker parsing — rather than the thin command
//! wrappers around them.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use gitnoob_lib::state::AppState;
use gitnoob_lib::{conflict, create, diff, graph, journal, refs, remote, work};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A throwaway repository, removed when the test ends.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("gitnoob-test-{tag}-{}-{id}", std::process::id()));
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
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).unwrap(),
        "one\n",
        "the clone should carry the files"
    );
    assert_eq!(git_at(dest, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "main");

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn refuses_to_clone_where_a_folder_already_exists() {
    let origin = Sandbox::new("clone-exists");
    origin.commit("a.txt", "one\n", "First");
    let parent = scratch("clone-exists-into");
    let name = origin.root.file_name().unwrap().to_string_lossy().into_owned();
    std::fs::create_dir_all(parent.join(&name)).unwrap();

    let refused = create::clone(origin.root.to_string_lossy().as_ref(), &parent).unwrap_err();
    assert!(refused.contains("already has a folder"));

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn creating_a_repository_makes_a_first_commit_as_the_profile() {
    let parent = scratch("init");
    let made = create::init(&parent, "fresh", Some(("Test".to_string(), "test@example.com".to_string())))
        .unwrap();

    let dest = Path::new(&made.path);
    assert!(dest.join(".git").exists());
    assert_eq!(git_at(dest, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "main");
    assert_eq!(git_at(dest, &["config", "--local", "user.name"]).trim(), "Test");
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
    assert!(
        sandbox
            .git(&["rev-parse", "--verify", "refs/remotes/source/main"])
            .trim()
            .len()
            > 0
    );

    // Removing takes the tracking branches and nothing else.
    assert!(remote::remote_remove(&state, "source").is_ok());
    assert!(!remote::remotes(&state).unwrap().contains(&"source".to_string()));
    assert!(!sandbox.git_may_fail(&["rev-parse", "--verify", "refs/remotes/source/main"]));
    // The local branch and its commit are untouched.
    assert_eq!(sandbox.git(&["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "main");

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

/// A branch whose tip is newer than HEAD must not take the trunk's column.
///
/// The walk reaches the newest commit first, so first come first served hands
/// lane 0 to whichever branch happens to have been committed to last, and the
/// line the user is standing on is pushed sideways around it — which reads as
/// the branch and the trunk trading places rather than as a branch leaving.
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
    sandbox.git(&["merge", "-q", "--no-ff", "-m", "Merge main into topic", "main"]);
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
            page.rows[row].segments.iter().any(|s| s.x1 == into && s.y1 == 0),
            "row {row} ({}) drops the line the merge left in lane {into}",
            page.rows[row].summary
        );
    }
    // And it arrives at the commit it was drawn for.
    assert!(
        page.rows[head].segments.iter().any(|s| s.x1 == into && s.y2 == 1),
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
    git_at(bare.parent().unwrap(), &["init", "-q", "--bare", "-b", "main", &arg]);
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
        sandbox.git(&["rev-parse", "--abbrev-ref", "main@{upstream}"]).trim(),
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
    assert!(remote::push(&state, "origin", "main", false, true).unwrap().ok);
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
    assert!(remote::push(&state, "origin", "main", false, true).unwrap().ok);

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
    assert!(remote::push(&state, "origin", "main", false, true).unwrap().ok);

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

    let draft = gitnoob_lib::work::amend_draft(&state).unwrap();
    assert_eq!(draft.summary, "Only commit");
    assert_eq!(draft.body, "With a body.");
    assert!(!draft.is_pushed, "nothing has been pushed yet");

    let bare = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitnoob-test-amend-origin-{}.git", std::process::id()));
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

    let bare = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitnoob-test-track-origin-{}.git", std::process::id()));
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
    assert!(gitnoob_lib::work::stash_list(&state).unwrap().is_empty());

    // The switch is undoable.
    let stacks = gitnoob_lib::journal::stacks(&state);
    assert_eq!(stacks.undo[0].label, "Switch to other");
    gitnoob_lib::journal::undo(&state).unwrap();
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

#[test]
fn pull_stashes_local_work_and_puts_it_back() {
    let sandbox = Sandbox::new("pullstash");
    sandbox.commit("a.txt", "one\n", "First");

    // A remote that has moved on.
    let bare = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitnoob-test-pull-origin-{}.git", std::process::id()));
    let _ = std::fs::remove_dir_all(&bare);
    let bare_arg = bare.to_string_lossy().into_owned();
    sandbox.git(&["clone", "-q", "--bare", ".", &bare_arg]);
    sandbox.git(&["remote", "add", "origin", &bare_arg]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["branch", "--set-upstream-to=origin/main", "main"]);

    let clone = sandbox.root.parent().unwrap().join(format!(
        "gitnoob-test-pull-clone-{}",
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
    assert!(gitnoob_lib::work::stash_list(&state).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&bare);
    let _ = std::fs::remove_dir_all(&clone);
}

#[test]
fn parses_forge_remotes_and_reports_status() {
    let sandbox = Sandbox::new("forge");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["remote", "add", "origin", "git@gitlab.bigbridge.nl:team/sub/app.git"]);

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
    gitnoob_lib::work::apply_hunk(&state, "a.txt", 0, gitnoob_lib::work::HunkAction::Stage).unwrap();

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
    gitnoob_lib::work::apply_hunk(&state, "a.txt", 0, gitnoob_lib::work::HunkAction::Unstage).unwrap();
    assert!(refs::status(&state).unwrap().staged.is_empty());

    // Discarding the remaining region leaves the other edit alone.
    let before = std::fs::read_to_string(sandbox.root.join("a.txt")).unwrap();
    assert!(before.contains("first addition") && before.contains("second addition"));
    gitnoob_lib::work::apply_hunk(&state, "a.txt", 1, gitnoob_lib::work::HunkAction::Discard).unwrap();
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
        gitnoob_lib::work::apply_hunk(&state, "a.txt", 0, gitnoob_lib::work::HunkAction::Stage)
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
        &[oid.clone()],
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
    assert!(!message.trim().is_empty(), "expected a message, got {message:?}");
    assert!(message.contains("feature"));
    assert_eq!(refs::describe(&state).unwrap().head, "feature");
}

#[test]
fn the_graph_marks_commits_the_upstream_does_not_have() {
    let sandbox = Sandbox::new("unpushed");
    sandbox.commit("a.txt", "one\n", "Shared");

    let bare = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitnoob-test-unpushed-origin-{}.git", std::process::id()));
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
    assert!(preview.remotes.is_empty());
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
}

#[test]
fn a_branch_that_also_lives_on_a_remote_is_reported() {
    let sandbox = Sandbox::new("delremote");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("b.txt", "two\n", "Topic work");

    let bare = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitnoob-test-delremote-origin-{}.git", std::process::id()));
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
    assert_eq!(preview.remotes, vec!["origin/topic".to_string()]);
    assert_eq!(preview.upstream.as_deref(), Some("origin/topic"));
    assert_eq!(preview.unpushed, 1, "the commit made after the clone");

    let _ = std::fs::remove_dir_all(&bare);
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
    assert!(sandbox.git(&["stash", "list"]).trim().is_empty(), "nothing was stashed");
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

#[test]
fn a_branch_whose_remote_is_gone_is_reported_as_stale() {
    let sandbox = Sandbox::new("stale");
    sandbox.commit("a.txt", "one\n", "Base");
    sandbox.git(&["checkout", "-q", "-b", "topic"]);
    sandbox.commit("b.txt", "two\n", "Topic work");

    let bare = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitnoob-test-stale-origin-{}.git", std::process::id()));
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

    assert_eq!(refs::stale_branches(&state).unwrap(), vec!["topic".to_string()]);
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

    let bare = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitnoob-test-upstream-origin-{}.git", std::process::id()));
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
    assert!(sandbox.git(&["tag", "-l", "-n", "v2"]).contains("the second one"));

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
    assert!(patch.contains("diff --git a/a.txt b/a.txt"), "unexpected: {patch}");
    assert!(patch.contains("+two"));
}

/// Sets up a repository with a bare `origin` it can push to and pull from,
/// returning the path of the bare one so the test can commit into it.
fn with_origin(sandbox: &Sandbox, tag: &str) -> String {
    let bare = sandbox
        .root
        .parent()
        .unwrap()
        .join(format!("gitnoob-test-{tag}-origin-{}.git", std::process::id()));
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
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
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
    assert_eq!(refs::describe(&state).unwrap().head, "main", "still on main");
    assert!(sandbox.git(&["stash", "list"]).trim().is_empty(), "nothing was stashed");
    assert!(
        refs::status(&state).unwrap().staged.iter().any(|e| e.path == "mine.txt"),
        "the staged file is untouched"
    );
    assert!(
        !sandbox.root.join("theirs.txt").exists(),
        "their file belongs to topic, not to the tree we are standing in"
    );
    let log = sandbox.git(&["log", "--format=%s", "topic"]);
    assert!(log.contains("Someone else"), "topic has their commit: {log}");

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
    assert!(sandbox.git(&["stash", "list"]).trim().is_empty(), "the stash was put back");

    // topic has both sides of the history now.
    let log = sandbox.git(&["log", "--format=%s", "topic"]);
    assert!(log.contains("Someone else") && log.contains("Our own"), "unexpected: {log}");

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
    assert!(!remote::in_progress(&state).unwrap().merging, "no merge left dangling");
    assert!(refs::status(&state).unwrap().conflicted.is_empty());
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("mine.txt")).unwrap(),
        "half-finished\n"
    );
    assert!(sandbox.git(&["stash", "list"]).trim().is_empty(), "the stash was put back");

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
    assert_eq!(String::from_utf8(on_disk).unwrap(), "top\r\nour middle\r\nbottom\r\n");

    // Keeping our side reproduces our commit exactly, so nothing is staged.
    // Rewriting the endings would show up here as all three lines changed.
    let diff = sandbox.git(&["diff", "--cached", "--numstat"]);
    assert!(diff.trim().is_empty(), "the file should be unchanged from ours: {diff}");
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
    assert!(result.is_err(), "checking out a path should fail, not succeed silently");
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
    assert!(sandbox.root.join("theirs.txt").exists(), "side's file should be here now");
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
    assert!(outcome.message.contains("Resolve it here"), "{}", outcome.message);

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
    assert_eq!(log.lines().collect::<Vec<_>>(), vec!["On side", "On main", "First"]);
}
