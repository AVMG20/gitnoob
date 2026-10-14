//! Keeping branches in step with their remotes: what a new branch tracks,
//! where a push lands, upstreams that were deleted, the fetch on a timer, the
//! trunk kept current, tidying merged branches, starting and updating work,
//! and the trial merge behind the drop menu.
//!
//! Each test builds a working clone, a bare `origin` it pushes to, and, when
//! somebody else has to push, a second clone of that origin.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use gitnoob_lib::state::AppState;
use gitnoob_lib::{refs, remote};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Sandbox {
    root: PathBuf,
    work: PathBuf,
    origin: PathBuf,
}

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

fn configure(dir: &Path, who: &str) {
    git_at(dir, &["config", "user.name", who]);
    git_at(dir, &["config", "user.email", "test@example.com"]);
    git_at(dir, &["config", "commit.gpgsign", "false"]);
    // Pinned so the content compared below is the same on every platform.
    git_at(dir, &["config", "core.autocrlf", "false"]);
    git_at(dir, &["config", "core.eol", "lf"]);
}

impl Sandbox {
    /// A clone on `main` with one commit pushed to its origin.
    fn new(tag: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("gitnoob-sync-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let origin = root.join("origin.git");
        git_at(&root, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        git_at(&work, &["init", "-q", "-b", "main"]);
        configure(&work, "Test");
        git_at(
            &work,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        let sandbox = Sandbox { root, work, origin };
        sandbox.commit("a.txt", "one\n", "First");
        sandbox.git(&["push", "-q", "-u", "origin", "main"]);
        sandbox
    }

    fn git(&self, args: &[&str]) -> String {
        git_at(&self.work, args)
    }

    fn try_git(&self, args: &[&str]) -> bool {
        Command::new("git")
            .args(args)
            .current_dir(&self.work)
            .output()
            .expect("git should be on PATH")
            .status
            .success()
    }

    fn write(&self, path: &str, content: &str) {
        std::fs::write(self.work.join(path), content).unwrap();
    }

    fn commit(&self, path: &str, content: &str, message: &str) -> String {
        self.write(path, content);
        self.git(&["add", "--all"]);
        self.git(&["commit", "-q", "-m", message]);
        self.oid("HEAD")
    }

    fn oid(&self, rev: &str) -> String {
        self.git(&["rev-parse", rev]).trim().to_string()
    }

    fn state(&self) -> AppState {
        gitnoob_lib::config::silence_keychain();
        let state = AppState::new(self.root.join("config"));
        state.set_path(self.work.clone());
        state
    }

    /// A second clone of origin, for the commits somebody else pushes.
    fn other(&self) -> PathBuf {
        let other = self.root.join("other");
        if !other.exists() {
            git_at(
                &self.root,
                &["clone", "-q", self.origin.to_str().unwrap(), "other"],
            );
            configure(&other, "Them");
        }
        other
    }

    /// Somebody else commits `path` on `branch` and pushes it.
    fn elsewhere(&self, branch: &str, path: &str, content: &str, message: &str) {
        let other = self.other();
        git_at(&other, &["fetch", "-q", "origin"]);
        let exists = Command::new("git")
            .args(["rev-parse", "--verify", "-q", &format!("origin/{branch}")])
            .current_dir(&other)
            .output()
            .unwrap()
            .status
            .success();
        if exists {
            git_at(
                &other,
                &["checkout", "-q", "-B", branch, &format!("origin/{branch}")],
            );
        } else {
            git_at(&other, &["checkout", "-q", "-B", branch]);
        }
        std::fs::write(other.join(path), content).unwrap();
        git_at(&other, &["add", "--all"]);
        git_at(&other, &["commit", "-q", "-m", message]);
        git_at(&other, &["push", "-q", "origin", branch]);
    }

    /// What origin itself says a branch points at.
    fn origin_tip(&self, branch: &str) -> Option<String> {
        let out = Command::new("git")
            .args([
                "rev-parse",
                "--verify",
                "-q",
                &format!("refs/heads/{branch}"),
            ])
            .current_dir(&self.origin)
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn upstream(&self, branch: &str) -> Option<String> {
        let out = Command::new("git")
            .args([
                "rev-parse",
                "--abbrev-ref",
                &format!("{branch}@{{upstream}}"),
            ])
            .current_dir(&self.work)
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "--abbrev-ref", "HEAD"])
            .trim()
            .to_string()
    }

    fn stash_count(&self) -> usize {
        self.git(&["stash", "list"]).lines().count()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// --- what a new branch tracks ------------------------------------------------

#[test]
fn a_branch_made_from_the_remote_trunk_tracks_nothing() {
    let sandbox = Sandbox::new("no-track");
    // The setting that makes git track even a local start point.
    sandbox.git(&["config", "branch.autoSetupMerge", "always"]);
    let state = sandbox.state();

    refs::create_branch(&state, "feature", Some("origin/main"), true).unwrap();
    assert_eq!(sandbox.head(), "feature");
    assert_eq!(
        sandbox.upstream("feature"),
        None,
        "a branch started from origin/main must not push to main"
    );

    refs::create_branch(&state, "other", Some("main"), false).unwrap();
    assert_eq!(sandbox.upstream("other"), None);
}

#[test]
fn a_branch_named_after_the_remote_branch_it_starts_from_tracks_it() {
    let sandbox = Sandbox::new("same-name");
    sandbox.elsewhere("topic", "t.txt", "theirs\n", "Their topic");
    sandbox.git(&["fetch", "-q", "origin"]);
    let state = sandbox.state();

    refs::create_branch(&state, "topic", Some("origin/topic"), true).unwrap();
    assert_eq!(sandbox.upstream("topic").as_deref(), Some("origin/topic"));
}

#[test]
fn checking_out_a_remote_branch_still_tracks_it() {
    let sandbox = Sandbox::new("checkout-tracks");
    sandbox.elsewhere("topic", "t.txt", "theirs\n", "Their topic");
    sandbox.git(&["fetch", "-q", "origin"]);
    let state = sandbox.state();

    refs::checkout(&state, "origin/topic").unwrap();
    assert_eq!(sandbox.head(), "topic");
    assert_eq!(sandbox.upstream("topic").as_deref(), Some("origin/topic"));

    refs::checkout_tracking(&state, "copy", "origin/topic").unwrap();
    assert_eq!(sandbox.upstream("copy").as_deref(), Some("origin/topic"));
}

#[test]
fn creating_a_branch_elsewhere_refuses_rather_than_loses_work_that_will_not_carry() {
    let sandbox = Sandbox::new("create-carry");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("a.txt", "side\n", "Side edits a.txt");
    // An edit to a file the two start points disagree about, which would
    // conflict on the other side.
    sandbox.write("a.txt", "side\nmine\n");
    let state = sandbox.state();

    assert!(refs::create_branch(&state, "fresh", Some("main"), true).is_err());
    assert_eq!(sandbox.head(), "side");
    assert_eq!(
        std::fs::read_to_string(sandbox.work.join("a.txt")).unwrap(),
        "side\nmine\n"
    );
    assert_eq!(sandbox.stash_count(), 0, "nothing left behind in the stash");
}

#[test]
fn creating_a_branch_elsewhere_carries_open_changes_that_fit() {
    let sandbox = Sandbox::new("create-carry-fits");
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("b.txt", "side\n", "Side adds b.txt");
    sandbox.write("a.txt", "one\nmine\n");
    let state = sandbox.state();

    let said = refs::create_branch(&state, "fresh", Some("main"), true).unwrap();
    assert_eq!(sandbox.head(), "fresh");
    assert_eq!(
        std::fs::read_to_string(sandbox.work.join("a.txt")).unwrap(),
        "one\nmine\n",
        "{said}"
    );
}

// --- where a push lands ------------------------------------------------------

#[test]
fn a_review_branch_pushes_to_the_branch_it_follows_on_the_fork() {
    let sandbox = Sandbox::new("push-name");
    // What a review checkout marks a fork's remote with.
    sandbox.git(&["config", "remote.origin.gitnoob-review", "true"]);
    sandbox.git(&[
        "checkout",
        "-q",
        "-b",
        "mine-main",
        "--track",
        "origin/main",
    ]);
    let tip = sandbox.commit("b.txt", "two\n", "Mine");
    let state = sandbox.state();

    let out = remote::push(&state, "origin", "mine-main", false, false, None).unwrap();
    assert!(out.ok, "{}", out.stderr);
    assert_eq!(sandbox.origin_tip("main"), Some(tip));
    assert_eq!(
        sandbox.origin_tip("mine-main"),
        None,
        "no new branch named after the local one"
    );
}

#[test]
fn a_force_push_leases_on_the_branch_it_actually_replaces() {
    let sandbox = Sandbox::new("push-lease-name");
    sandbox.git(&["config", "remote.origin.gitnoob-review", "true"]);
    sandbox.git(&[
        "checkout",
        "-q",
        "-b",
        "mine-main",
        "--track",
        "origin/main",
    ]);
    sandbox.commit("b.txt", "two\n", "Mine");
    let state = sandbox.state();
    assert!(
        remote::push(&state, "origin", "mine-main", false, false, None)
            .unwrap()
            .ok
    );
    let published = sandbox.oid("HEAD");

    sandbox.git(&["commit", "-q", "--amend", "-m", "Mine, reworded"]);
    let out = remote::push(
        &state,
        "origin",
        "mine-main",
        true,
        false,
        Some(published.as_str()),
    )
    .unwrap();
    assert!(out.ok, "{}", out.stderr);
    assert!(out
        .argv
        .iter()
        .any(|arg| arg == &format!("--force-with-lease=refs/heads/main:{published}")));
    assert_eq!(sandbox.origin_tip("main"), Some(sandbox.oid("HEAD")));
}

#[test]
fn a_branch_and_a_tag_with_the_same_name_push_and_delete_the_branch() {
    let sandbox = Sandbox::new("push-tag-clash");
    sandbox.git(&["checkout", "-q", "-b", "release"]);
    let tip = sandbox.commit("r.txt", "r\n", "Release work");
    sandbox.git(&["tag", "release", "main"]);
    sandbox.git(&["push", "-q", "origin", "refs/tags/release"]);
    let state = sandbox.state();

    let out = remote::push(&state, "origin", "release", false, true, None).unwrap();
    assert!(out.ok, "{}", out.stderr);
    assert_eq!(sandbox.origin_tip("release"), Some(tip));

    let deleted = remote::delete_remote_branch(&state, "origin", "release").unwrap();
    assert!(deleted.ok, "{}", deleted.stderr);
    assert_eq!(sandbox.origin_tip("release"), None);
    let tag = Command::new("git")
        .args(["rev-parse", "--verify", "-q", "refs/tags/release"])
        .current_dir(&sandbox.origin)
        .output()
        .unwrap();
    assert!(tag.status.success(), "the tag on the remote is left alone");
}

#[test]
fn a_first_push_always_sets_the_upstream() {
    let sandbox = Sandbox::new("push-sets-upstream");
    sandbox.git(&["checkout", "-q", "-b", "new"]);
    sandbox.commit("n.txt", "n\n", "New");
    let state = sandbox.state();

    let out = remote::push(&state, "origin", "new", false, false, None).unwrap();
    assert!(out.ok, "{}", out.stderr);
    assert_eq!(sandbox.upstream("new").as_deref(), Some("origin/new"));
}

// --- an upstream that was deleted ---------------------------------------------

#[test]
fn a_deleted_upstream_is_reported_as_gone_rather_than_missing() {
    let sandbox = Sandbox::new("gone");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("f.txt", "f\n", "Feature");
    sandbox.git(&["push", "-q", "-u", "origin", "feature"]);
    // Merged and deleted on the forge; pruned here by the next fetch.
    git_at(&sandbox.origin, &["update-ref", "-d", "refs/heads/feature"]);
    sandbox.git(&["fetch", "-q", "--prune", "origin"]);
    let state = sandbox.state();

    let tree = refs::tree(&state).unwrap();
    let feature = tree.locals.iter().find(|b| b.name == "feature").unwrap();
    assert_eq!(feature.upstream, None);
    assert_eq!(feature.gone.as_deref(), Some("origin/feature"));
    let main = tree.locals.iter().find(|b| b.name == "main").unwrap();
    assert_eq!(main.gone, None, "a live upstream is not gone");

    // Pushing it again on purpose puts it back under the name it had.
    let out = remote::push(&state, "origin", "feature", false, false, None).unwrap();
    assert!(out.ok, "{}", out.stderr);
    assert_eq!(sandbox.origin_tip("feature"), Some(sandbox.oid("feature")));
}

// --- the fetch on a timer -----------------------------------------------------

#[test]
fn the_background_fetch_asks_only_the_repository_s_own_remote() {
    let sandbox = Sandbox::new("dead-remote");
    let dead = sandbox.root.join("no-such-fork.git");
    sandbox.git(&["remote", "add", "fork", dead.to_str().unwrap()]);
    sandbox.elsewhere("main", "b.txt", "theirs\n", "Theirs");
    let state = sandbox.state();

    let all = remote::fetch(&state, None).unwrap();
    assert!(!all.ok, "one dead remote fails --all");

    let primary = remote::fetch_primary(&state).unwrap();
    assert!(primary.ok, "{}", primary.stderr);
    assert!(!primary.argv.iter().any(|arg| arg == "--all"));
    assert_eq!(
        sandbox.oid("origin/main"),
        sandbox.origin_tip("main").unwrap()
    );
}

#[test]
fn remotes_added_for_reviews_are_remembered_until_removed() {
    let sandbox = Sandbox::new("review-remote");
    let state = sandbox.state();
    remote::remote_add(&state, "alice", "https://example.com/alice/repo.git").unwrap();
    remote::remote_add(&state, "mirror", "https://example.com/mirror.git").unwrap();
    remote::mark_review_remote(&state, "alice").unwrap();

    assert_eq!(remote::review_remotes(&state), vec!["alice".to_string()]);
    remote::remote_remove(&state, "alice").unwrap();
    assert!(remote::review_remotes(&state).is_empty());
}

// --- is it merged? -------------------------------------------------------------

#[test]
fn a_branch_merged_on_the_remote_counts_as_merged_before_main_is_pulled() {
    let sandbox = Sandbox::new("merged-remote");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("f.txt", "f\n", "Feature");
    sandbox.git(&["push", "-q", "-u", "origin", "feature"]);
    // Merged with a merge commit on the forge.
    let other = sandbox.other();
    git_at(&other, &["fetch", "-q", "origin"]);
    git_at(&other, &["checkout", "-q", "main"]);
    git_at(
        &other,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge feature",
            "origin/feature",
        ],
    );
    git_at(&other, &["push", "-q", "origin", "main"]);
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["checkout", "-q", "-b", "elsewhere"]);
    let state = sandbox.state();

    let preview = refs::deletion_preview(&state, "feature").unwrap();
    assert!(
        preview.trunk_holds,
        "origin/main holds it even though main does not"
    );
    assert_eq!(preview.against.as_deref(), Some("origin/main"));
    assert_eq!(preview.landed.as_deref(), Some("merged"));
    assert_eq!(preview.only_here, 0);
}

#[test]
fn a_squash_merged_branch_is_found_by_its_patch() {
    let sandbox = Sandbox::new("squashed");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("f.txt", "one\n", "Feature one");
    sandbox.commit("f.txt", "one\ntwo\n", "Feature two");
    sandbox.git(&["push", "-q", "-u", "origin", "feature"]);
    // The forge squashes both into one commit on main and deletes the branch.
    let other = sandbox.other();
    git_at(&other, &["fetch", "-q", "origin"]);
    git_at(&other, &["checkout", "-q", "main"]);
    git_at(&other, &["merge", "-q", "--squash", "origin/feature"]);
    git_at(&other, &["commit", "-q", "-m", "Feature (#1)"]);
    git_at(&other, &["push", "-q", "origin", "main"]);
    git_at(&other, &["push", "-q", "origin", "--delete", "feature"]);
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["fetch", "-q", "--prune", "origin"]);
    let state = sandbox.state();

    let preview = refs::deletion_preview(&state, "feature").unwrap();
    assert!(preview.trunk_holds);
    assert_eq!(preview.landed.as_deref(), Some("squash"));
    assert_eq!(preview.gone.as_deref(), Some("origin/feature"));
    assert!(!preview.merged, "git itself still calls it unmerged");
}

#[test]
fn the_remote_default_branch_is_the_trunk() {
    let sandbox = Sandbox::new("origin-head");
    sandbox.git(&["checkout", "-q", "-b", "develop"]);
    sandbox.commit("d.txt", "d\n", "Develop");
    sandbox.git(&["push", "-q", "-u", "origin", "develop"]);
    sandbox.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/develop",
    ]);
    let state = sandbox.state();
    assert_eq!(refs::trunk(&state).name.as_deref(), Some("develop"));

    // Named by hand still wins.
    refs::set_trunk(&state, Some("main")).unwrap();
    assert_eq!(refs::trunk(&state).name.as_deref(), Some("main"));
}

// --- pulling a branch you are not on --------------------------------------------

#[test]
fn pulling_a_branch_that_is_only_ahead_touches_nothing() {
    let sandbox = Sandbox::new("pull-ahead");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("f.txt", "f\n", "Feature");
    sandbox.git(&["push", "-q", "-u", "origin", "feature"]);
    let ahead = sandbox.commit("f.txt", "f\ng\n", "Not pushed");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.write("a.txt", "dirty\n");
    let state = sandbox.state();

    let outcome = remote::pull_branch(&state, "feature", None).unwrap();
    assert!(outcome.ok, "{}", outcome.message);
    assert!(
        outcome.message.contains("nothing to pull"),
        "{}",
        outcome.message
    );
    assert_eq!(sandbox.head(), "main", "no trip to the branch and back");
    assert_eq!(sandbox.stash_count(), 0, "nothing was stashed");
    assert_eq!(sandbox.oid("feature"), ahead);
    assert_eq!(
        std::fs::read_to_string(sandbox.work.join("a.txt")).unwrap(),
        "dirty\n"
    );
}

#[test]
fn pulling_a_branch_that_is_only_behind_moves_the_ref_alone() {
    let sandbox = Sandbox::new("pull-behind");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("f.txt", "f\n", "Feature");
    sandbox.git(&["push", "-q", "-u", "origin", "feature"]);
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.elsewhere("feature", "g.txt", "g\n", "Their feature work");
    sandbox.write("a.txt", "dirty\n");
    let state = sandbox.state();

    let outcome = remote::pull_branch(&state, "feature", None).unwrap();
    assert!(outcome.ok, "{}", outcome.message);
    assert_eq!(
        sandbox.oid("feature"),
        sandbox.origin_tip("feature").unwrap()
    );
    assert_eq!(sandbox.head(), "main");
    assert_eq!(sandbox.stash_count(), 0);
}

// --- a refused push after a rewrite ----------------------------------------------

#[test]
fn a_rebased_branch_is_known_to_be_a_rewrite_of_what_was_pushed() {
    let sandbox = Sandbox::new("rewritten");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("f.txt", "f\n", "Feature one");
    sandbox.commit("g.txt", "g\n", "Feature two");
    sandbox.git(&["push", "-q", "-u", "origin", "feature"]);
    sandbox.elsewhere("main", "m.txt", "m\n", "Main moved");
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["rebase", "-q", "origin/main"]);
    let state = sandbox.state();

    let preview = remote::push_preview(&state, Some("feature"), false).unwrap();
    assert!(preview.force_needed);
    assert!(
        preview.rewritten,
        "every commit it would drop has a copy here"
    );

    // Somebody else's commit on the branch is not a rewrite of anything here.
    sandbox.elsewhere("feature", "h.txt", "h\n", "Their commit");
    sandbox.git(&["fetch", "-q", "origin"]);
    let preview = remote::push_preview(&state, Some("feature"), false).unwrap();
    assert!(preview.force_needed);
    assert!(!preview.rewritten);
}

// --- keeping the trunk current ---------------------------------------------------

#[test]
fn the_local_trunk_is_fast_forwarded_when_nobody_is_on_it() {
    let sandbox = Sandbox::new("keep-trunk");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.elsewhere("main", "b.txt", "b\n", "Main moved");
    sandbox.git(&["fetch", "-q", "origin"]);
    let state = sandbox.state();

    let moved = remote::keep_trunk_current(&state)
        .unwrap()
        .expect("main moves");
    assert_eq!(moved.branch, "main");
    assert_eq!(moved.upstream, "origin/main");
    assert_eq!(moved.commits, 1);
    assert_eq!(sandbox.oid("main"), sandbox.oid("origin/main"));
    assert_eq!(sandbox.head(), "feature");
    let history = gitnoob_lib::journal::stacks(&state);
    assert!(history.undo.is_empty(), "housekeeping is not an undo step");
}

#[test]
fn the_local_trunk_is_left_alone_when_checked_out_or_diverged_or_switched_off() {
    let sandbox = Sandbox::new("keep-trunk-not");
    sandbox.elsewhere("main", "b.txt", "b\n", "Main moved");
    sandbox.git(&["fetch", "-q", "origin"]);
    let state = sandbox.state();
    let before = sandbox.oid("main");

    // Checked out right here.
    assert!(remote::keep_trunk_current(&state).unwrap().is_none());
    assert_eq!(sandbox.oid("main"), before);

    // Checked out in another worktree.
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    let tree = sandbox.root.join("main-tree");
    sandbox.git(&["worktree", "add", "-q", tree.to_str().unwrap(), "main"]);
    assert!(remote::keep_trunk_current(&state).unwrap().is_none());
    assert_eq!(sandbox.oid("main"), before);
    sandbox.git(&["worktree", "remove", "--force", tree.to_str().unwrap()]);

    // Switched off.
    state
        .update_config(|config| config.global.keep_trunk_current = false)
        .unwrap();
    assert!(remote::keep_trunk_current(&state).unwrap().is_none());
    state
        .update_config(|config| config.global.keep_trunk_current = true)
        .unwrap();

    // Commits of its own: never anything but a fast-forward.
    sandbox.git(&["checkout", "-q", "main"]);
    let mine = sandbox.commit("c.txt", "c\n", "Local main work");
    sandbox.git(&["checkout", "-q", "feature"]);
    assert!(remote::keep_trunk_current(&state).unwrap().is_none());
    assert_eq!(sandbox.oid("main"), mine);
}

// --- tidying merged and gone branches ---------------------------------------------

#[test]
fn tidy_offers_finished_branches_and_proves_only_what_landed() {
    let sandbox = Sandbox::new("tidy");
    // Merged with a merge commit.
    sandbox.git(&["checkout", "-q", "-b", "merged"]);
    sandbox.commit("m.txt", "m\n", "Merged work");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["merge", "-q", "--no-ff", "-m", "Merge merged", "merged"]);
    // Squashed in.
    sandbox.git(&["checkout", "-q", "-b", "squashed"]);
    sandbox.commit("s.txt", "s\n", "Squashed work");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["merge", "-q", "--squash", "squashed"]);
    sandbox.git(&["commit", "-q", "-m", "Squashed (#2)"]);
    sandbox.git(&["push", "-q", "origin", "main"]);
    // Pushed, never merged, deleted on the remote.
    sandbox.git(&["checkout", "-q", "-b", "abandoned"]);
    sandbox.commit("x.txt", "x\n", "Abandoned work");
    sandbox.git(&["push", "-q", "-u", "origin", "abandoned"]);
    git_at(
        &sandbox.origin,
        &["update-ref", "-d", "refs/heads/abandoned"],
    );
    sandbox.git(&["fetch", "-q", "--prune", "origin"]);
    // Started this morning, nothing on it yet.
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["branch", "fresh"]);
    // Merged, but checked out in a worktree.
    sandbox.git(&["branch", "busy", "merged"]);
    let tree = sandbox.root.join("busy-tree");
    sandbox.git(&["worktree", "add", "-q", tree.to_str().unwrap(), "busy"]);
    // Merged, and the branch you are on.
    sandbox.git(&["checkout", "-q", "-b", "here", "merged"]);
    let state = sandbox.state();

    let found = refs::tidy_candidates(&state).unwrap();
    let names: Vec<&str> = found.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["abandoned", "merged", "squashed"], "{found:?}");

    let reason = |name: &str| found.iter().find(|t| t.name == name).unwrap();
    assert_eq!(reason("merged").reason, "merged");
    assert!(reason("merged").proven);
    assert_eq!(reason("squashed").reason, "squash");
    assert!(reason("squashed").proven);
    assert_eq!(reason("abandoned").reason, "gone");
    assert!(!reason("abandoned").proven, "its work is on no trunk");
    assert_eq!(
        reason("abandoned").gone.as_deref(),
        Some("origin/abandoned")
    );
}

#[test]
fn deleted_branches_come_back_where_they_were_with_their_upstream() {
    let sandbox = Sandbox::new("tidy-undo");
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    let tip = sandbox.commit("f.txt", "f\n", "Feature");
    sandbox.git(&["push", "-q", "-u", "origin", "feature"]);
    git_at(&sandbox.origin, &["update-ref", "-d", "refs/heads/feature"]);
    sandbox.git(&["fetch", "-q", "--prune", "origin"]);
    sandbox.git(&["checkout", "-q", "main"]);
    // A name `git branch` will not make, but a remote can hand over.
    sandbox.git(&["update-ref", "refs/heads/-odd", "HEAD"]);
    let state = sandbox.state();

    let doomed = |name: &str| refs::Doomed {
        name: name.to_string(),
        oid: sandbox.oid(&format!("refs/heads/{name}")),
    };
    let removal =
        refs::delete_branches(&state, &[doomed("feature"), doomed("-odd"), doomed("main")])
            .unwrap();
    assert_eq!(removal.removed.len(), 2, "{:?}", removal.refused);
    assert_eq!(removal.refused.len(), 1, "the branch you are on is refused");
    assert!(!sandbox.try_git(&["rev-parse", "--verify", "-q", "refs/heads/feature"]));
    assert!(!sandbox.try_git(&["rev-parse", "--verify", "-q", "refs/heads/-odd"]));

    // The notice's Undo goes through the history: one step for the batch.
    gitnoob_lib::journal::undo_step(&state, removal.step.expect("one undo step")).unwrap();
    assert_eq!(sandbox.oid("feature"), tip);
    assert!(sandbox.try_git(&["rev-parse", "--verify", "-q", "refs/heads/-odd"]));
    assert_eq!(
        sandbox.git(&["config", "branch.feature.merge"]).trim(),
        "refs/heads/feature"
    );
    assert_eq!(
        sandbox.git(&["config", "branch.feature.remote"]).trim(),
        "origin"
    );
}

// --- starting and updating work -----------------------------------------------------

#[test]
fn starting_work_fetches_and_branches_from_the_remote_trunk() {
    let sandbox = Sandbox::new("start");
    sandbox.elsewhere("main", "b.txt", "b\n", "Main moved");
    // A change of mine nowhere near what moved.
    sandbox.write("mine.txt", "work in progress\n");
    let state = sandbox.state();

    let said = refs::start_work(&state, "task", None).unwrap();
    assert!(said.contains("origin/main"), "{said}");
    assert_eq!(sandbox.head(), "task");
    assert_eq!(sandbox.oid("task"), sandbox.origin_tip("main").unwrap());
    assert_eq!(sandbox.upstream("task"), None);
    assert_eq!(
        std::fs::read_to_string(sandbox.work.join("mine.txt")).unwrap(),
        "work in progress\n"
    );

    assert!(
        refs::start_work(&state, "task", None).is_err(),
        "the name is taken"
    );
}

#[test]
fn updating_an_unpushed_branch_rebases_it_onto_the_remote_trunk() {
    let sandbox = Sandbox::new("update-rebase");
    sandbox.git(&["checkout", "-q", "-b", "task"]);
    sandbox.commit("t.txt", "t\n", "Task");
    sandbox.elsewhere("main", "b.txt", "b\n", "Main moved");
    let state = sandbox.state();

    let outcome = remote::update_from_trunk(&state, "task", None).unwrap();
    assert!(outcome.ok, "{}", outcome.message);
    assert!(outcome.message.contains("rebased"), "{}", outcome.message);
    let parent = sandbox.oid("task~1");
    assert_eq!(
        parent,
        sandbox.origin_tip("main").unwrap(),
        "a straight line"
    );
}

#[test]
fn updating_a_pushed_branch_you_are_not_on_merges_the_trunk_in() {
    let sandbox = Sandbox::new("update-merge");
    sandbox.git(&["checkout", "-q", "-b", "task"]);
    sandbox.commit("t.txt", "t\n", "Task");
    sandbox.git(&["push", "-q", "-u", "origin", "task"]);
    let pushed = sandbox.oid("task");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.elsewhere("main", "b.txt", "b\n", "Main moved");
    let state = sandbox.state();

    let outcome = remote::update_from_trunk(&state, "task", None).unwrap();
    assert!(outcome.ok, "{}", outcome.message);
    assert!(
        outcome.message.contains("merged into task"),
        "{}",
        outcome.message
    );
    assert_eq!(sandbox.head(), "main", "came back home");
    assert_eq!(
        sandbox.oid("task^1"),
        pushed,
        "nothing pushed was rewritten"
    );
    assert_eq!(sandbox.oid("task^2"), sandbox.origin_tip("main").unwrap());

    assert!(
        remote::update_from_trunk(&state, "main", None).is_err(),
        "the trunk is pulled, not updated from itself"
    );
}

// --- the trial merge behind the drop menu -----------------------------------------------

#[test]
fn a_trial_merge_names_the_conflicts_without_touching_anything() {
    let sandbox = Sandbox::new("trial");
    sandbox.commit("a[b].txt", "base\n", "Bracketed file");
    sandbox.git(&["checkout", "-q", "-b", "theirs"]);
    sandbox.commit("a[b].txt", "theirs\n", "Theirs");
    sandbox.commit("clean.txt", "c\n", "Clean");
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("a[b].txt", "ours\n", "Ours");
    sandbox.write("dirty.txt", "open work\n");
    let status_before = sandbox.git(&["status", "--porcelain"]);
    let head_before = sandbox.oid("HEAD");
    let state = sandbox.state();

    let relation = remote::relation(&state, "theirs", "main").unwrap();
    let supports = Command::new("git")
        .args(["merge-tree", "--write-tree", "main", "main"])
        .current_dir(&sandbox.work)
        .output()
        .unwrap()
        .status
        .success();
    if supports {
        assert_eq!(relation.clean, Some(false));
        assert_eq!(relation.conflicts, vec!["a[b].txt".to_string()]);
    } else {
        assert_eq!(relation.clean, None, "an older git says nothing");
    }
    assert_eq!(sandbox.git(&["status", "--porcelain"]), status_before);
    assert_eq!(sandbox.oid("HEAD"), head_before);

    // Something that merges cleanly says so.
    sandbox.git(&["checkout", "-q", "-b", "other", "HEAD~1"]);
    sandbox.commit("z.txt", "z\n", "Elsewhere");
    let relation = remote::relation(&state, "other", "main").unwrap();
    if supports {
        assert_eq!(relation.clean, Some(true));
        assert!(relation.conflicts.is_empty());
    }
}

// --- the edges ---------------------------------------------------------------------------

#[test]
fn nothing_here_breaks_on_a_repository_with_no_commits() {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root =
        std::env::temp_dir().join(format!("gitnoob-sync-unborn-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    git_at(&root, &["init", "-q", "-b", "main"]);
    gitnoob_lib::config::silence_keychain();
    let state = AppState::new(root.join(".config"));
    state.set_path(root.clone());

    assert!(refs::tidy_candidates(&state).unwrap().is_empty());
    assert!(remote::keep_trunk_current(&state).unwrap().is_none());
    assert_eq!(refs::trunk_target(&state), None);
    assert!(refs::start_work(&state, "task", None).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn starting_work_from_a_detached_head_lands_on_the_new_branch() {
    let sandbox = Sandbox::new("start-detached");
    sandbox.git(&["checkout", "-q", "--detach", "HEAD"]);
    let state = sandbox.state();

    refs::start_work(&state, "task", None).unwrap();
    assert_eq!(sandbox.head(), "task");
    assert_eq!(sandbox.upstream("task"), None);
}

#[test]
fn updating_into_a_conflict_stops_for_the_resolver_with_work_kept() {
    let sandbox = Sandbox::new("update-conflict");
    sandbox.git(&["checkout", "-q", "-b", "task"]);
    sandbox.commit("a.txt", "mine\n", "Mine");
    sandbox.elsewhere("main", "a.txt", "theirs\n", "Theirs");
    sandbox.write("notes.txt", "open work\n");
    let state = sandbox.state();

    let outcome = remote::update_from_trunk(&state, "task", Some("rebase")).unwrap();
    assert!(!outcome.ok);
    assert_eq!(outcome.conflicts, vec!["a.txt".to_string()]);
    assert!(remote::in_progress(&state).unwrap().rebasing);

    remote::abort_rebase(&state).unwrap();
    assert_eq!(sandbox.head(), "task");
    assert_eq!(
        std::fs::read_to_string(sandbox.work.join("a.txt")).unwrap(),
        "mine\n"
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.work.join("notes.txt")).unwrap(),
        "open work\n",
        "untracked work is never touched"
    );
}

#[test]
fn pulling_a_branch_with_no_upstream_says_so() {
    let sandbox = Sandbox::new("pull-none");
    sandbox.git(&["branch", "loose"]);
    let state = sandbox.state();
    let error = remote::pull_branch(&state, "loose", None).unwrap_err();
    assert!(error.contains("not tracking"), "{error}");
}

/// A branch that follows `origin/main` without being `main` — what earlier
/// versions of the app made, and what `git checkout -b x origin/main` makes —
/// is pushed under its own name and set to follow that. Pushing it to `main`
/// would put unreviewed work on the trunk.
#[test]
fn a_branch_following_main_pushes_under_its_own_name() {
    let sandbox = Sandbox::new("push-inherited-main");
    let main_before = sandbox.origin_tip("main");
    sandbox.git(&["checkout", "-q", "-b", "feature", "--track", "origin/main"]);
    let tip = sandbox.commit("f.txt", "f\n", "Feature");
    let state = sandbox.state();

    let out = remote::push(&state, "origin", "feature", false, false, None).unwrap();
    assert!(out.ok, "{}", out.stderr);
    assert_eq!(sandbox.origin_tip("main"), main_before, "main is untouched");
    assert_eq!(sandbox.origin_tip("feature"), Some(tip));
    assert_eq!(
        sandbox.git(&["config", "branch.feature.merge"]).trim(),
        "refs/heads/feature",
        "the branch now follows what it was pushed to"
    );
}

/// The same branch, force pushed after somebody moved `main`: the preview
/// measures against `feature` on the remote, and the lease never names `main`.
#[test]
fn a_force_push_of_a_branch_following_main_never_replaces_main() {
    let sandbox = Sandbox::new("push-inherited-force");
    sandbox.git(&["checkout", "-q", "-b", "feature", "--track", "origin/main"]);
    sandbox.commit("f.txt", "f\n", "Feature");
    sandbox.elsewhere("main", "o.txt", "o\n", "Someone else's work");
    sandbox.git(&["fetch", "-q", "origin"]);
    let theirs = sandbox.origin_tip("main");
    let state = sandbox.state();

    let preview = remote::push_preview(&state, Some("feature"), false).unwrap();
    assert!(
        !preview.force_needed,
        "nothing on origin/feature to replace"
    );
    assert!(preview.will_orphan.is_empty());

    let out = remote::push(&state, "origin", "feature", true, false, None).unwrap();
    assert!(out.ok, "{}", out.stderr);
    assert!(!out.argv.iter().any(|arg| arg.contains("refs/heads/main")));
    assert_eq!(
        sandbox.origin_tip("main"),
        theirs,
        "main keeps the other commit"
    );
}

/// A project that renamed `master` to `main`: git before 2.48 leaves
/// `origin/HEAD` naming `origin/master` after the prune took it away, and a
/// stale local `master` is still lying around. The trunk is the live `main`.
#[test]
fn a_stale_origin_head_does_not_pick_a_dead_trunk() {
    let sandbox = Sandbox::new("stale-origin-head");
    sandbox.git(&["branch", "-m", "main", "master"]);
    sandbox.git(&["push", "-q", "-u", "origin", "master"]);
    git_at(
        &sandbox.origin,
        &["symbolic-ref", "HEAD", "refs/heads/master"],
    );
    git_at(&sandbox.origin, &["update-ref", "-d", "refs/heads/main"]);
    sandbox.git(&["fetch", "-q", "--prune", "origin"]);
    git_at(&sandbox.origin, &["branch", "-m", "master", "main"]);
    git_at(
        &sandbox.origin,
        &["symbolic-ref", "HEAD", "refs/heads/main"],
    );
    sandbox.git(&["fetch", "-q", "--prune", "origin"]);
    sandbox.git(&["checkout", "-q", "-b", "main", "--track", "origin/main"]);
    sandbox.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/master",
    ]);
    assert!(!sandbox.try_git(&["rev-parse", "--verify", "-q", "origin/master"]));

    let repo = git2::Repository::open(&sandbox.work).unwrap();
    assert_eq!(refs::trunk_of(&repo).as_deref(), Some("main"));
}

/// While the trunk is being rebased it shows as a detached HEAD, and git
/// refuses to move it. That is waited out quietly, not reported on every
/// fetch.
#[test]
fn a_trunk_being_rebased_is_left_alone_without_a_complaint() {
    let sandbox = Sandbox::new("keep-trunk-rebasing");
    sandbox.commit("b.txt", "b\n", "Second");
    sandbox.git(&["push", "-q", "origin", "main"]);
    sandbox.elsewhere("main", "c.txt", "c\n", "Main moved");
    sandbox.git(&["fetch", "-q", "origin"]);
    let before = sandbox.oid("main");
    // Stops part-way with main checked out as a detached HEAD.
    assert!(!sandbox.try_git(&["rebase", "--exec", "false", "HEAD~1"]));
    assert!(!sandbox.try_git(&["symbolic-ref", "-q", "HEAD"]));
    let state = sandbox.state();

    let moved = remote::keep_trunk_current(&state);
    assert!(matches!(moved, Ok(None)), "{moved:?}");
    assert_eq!(sandbox.oid("main"), before);
    sandbox.git(&["rebase", "--abort"]);
}

/// A branch checked out from a review on a fork tracks the remote gitnoob
/// added for it. The fetch on a timer still asks the repository's own remote,
/// or the trunk would stop moving while the review is open.
#[test]
fn the_background_fetch_skips_a_review_remote_the_branch_tracks() {
    let sandbox = Sandbox::new("primary-review");
    sandbox.git(&["remote", "add", "alice", sandbox.origin.to_str().unwrap()]);
    sandbox.git(&["config", "remote.alice.gitnoob-review", "true"]);
    sandbox.git(&["fetch", "-q", "alice"]);
    sandbox.git(&[
        "checkout",
        "-q",
        "-b",
        "alice-main",
        "--track",
        "alice/main",
    ]);
    let state = sandbox.state();
    assert_eq!(remote::primary(&state).as_deref(), Some("origin"));
}
