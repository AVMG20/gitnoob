//! The forge side of a repository, against real repositories built with git:
//! which project its reviews live in, which profile it answers to, what the
//! home tab says about it, and tidying up after a merge.
//!
//! Nothing here reaches a forge. The remotes carry forge addresses that are
//! never fetched from, and the merge itself is played out between local bare
//! repositories the way the forge would leave them.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use gitnoob_lib::config::{ForgeKind, Profile, Project};
use gitnoob_lib::state::AppState;
use gitnoob_lib::{ai, forge, home, identity, refs, review};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn git_in(dir: &Path, args: &[&str]) -> String {
    git_env(dir, args, &[])
}

fn git_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let mut command = Command::new("git");
    command.args(args).current_dir(dir);
    for (key, value) in env {
        command.env(key, value);
    }
    let out = command.output().expect("git should be on PATH");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A folder holding a project's bare "forge" copy, a clone of it to work in,
/// and the app's config, each in a directory of its own.
struct Sandbox {
    root: PathBuf,
    work: PathBuf,
    bare: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("gitnoob-forge-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let work = root.join("work");
        let bare = root.join("forge.git");
        std::fs::create_dir_all(&work).unwrap();
        git_in(&root, &["init", "-q", "--bare", "-b", "main", "forge.git"]);
        let sandbox = Sandbox { root, work, bare };
        sandbox.git(&["init", "-q", "-b", "main"]);
        sandbox.configure(&sandbox.work);
        sandbox
    }

    fn configure(&self, dir: &Path) {
        git_in(dir, &["config", "user.name", "Test"]);
        git_in(dir, &["config", "user.email", "test@example.com"]);
        git_in(dir, &["config", "commit.gpgsign", "false"]);
        git_in(dir, &["config", "core.autocrlf", "false"]);
        git_in(dir, &["config", "core.eol", "lf"]);
    }

    fn git(&self, args: &[&str]) -> String {
        git_in(&self.work, args)
    }

    fn commit(&self, path: &str, content: &str, message: &str) {
        std::fs::write(self.work.join(path), content).unwrap();
        self.git(&["add", "--all"]);
        self.git(&["commit", "-q", "-m", message]);
    }

    fn commit_dated(&self, path: &str, content: &str, message: &str, date: &str) {
        std::fs::write(self.work.join(path), content).unwrap();
        self.git(&["add", "--all"]);
        git_env(
            &self.work,
            &["commit", "-q", "-m", message],
            &[("GIT_AUTHOR_DATE", date), ("GIT_COMMITTER_DATE", date)],
        );
    }

    /// Wires the work clone to the local bare repository as `origin`.
    fn with_origin(&self) {
        self.git(&["remote", "add", "origin", self.bare.to_str().unwrap()]);
    }

    fn state(&self) -> AppState {
        gitnoob_lib::config::silence_keychain();
        let state = AppState::new(self.root.join("config"));
        state.set_path(self.work.clone());
        state
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "--abbrev-ref", "HEAD"])
            .trim()
            .to_string()
    }

    fn sha(&self, rev: &str) -> String {
        self.git(&["rev-parse", rev]).trim().to_string()
    }

    /// A second clone of the forge, standing in for somebody — or the forge's
    /// merge button — changing it behind our back.
    fn elsewhere(&self) -> PathBuf {
        let other = self.root.join("elsewhere");
        if !other.exists() {
            git_in(
                &self.root,
                &["clone", "-q", self.bare.to_str().unwrap(), "elsewhere"],
            );
            self.configure(&other);
        }
        other
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn profile(name: &str, forge: ForgeKind, host: &str) -> Profile {
    Profile {
        host: host.to_string(),
        ..Profile::new(name, forge)
    }
}

/* ---------- which project ------------------------------------------------- */

#[test]
fn checking_out_a_forks_branch_does_not_move_the_project() {
    let sandbox = Sandbox::new("fork-head");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["remote", "add", "origin", "git@github.com:acme/widget.git"]);
    sandbox.git(&["remote", "add", "kai", "https://github.com/kai/widget.git"]);
    // What checking out kai's review leaves behind: a branch tracking the fork.
    sandbox.git(&["checkout", "-q", "-b", "fix-typo"]);
    sandbox.git(&["config", "branch.fix-typo.remote", "kai"]);
    sandbox.git(&["config", "branch.fix-typo.merge", "refs/heads/fix-typo"]);

    let state = sandbox.state();
    let slug = forge::remote_slug(&state).expect("a project");
    assert_eq!(
        slug.full(),
        "acme/widget",
        "the review's fork is not the project"
    );

    let local = forge::Local::read(&state).unwrap();
    assert_eq!(
        local.upstream,
        Some(("kai".to_string(), "fix-typo".to_string()))
    );
    let current = forge::Current::from_local(&local);
    assert_eq!(current.upstream_repo.as_deref(), Some("kai/widget"));
}

#[test]
fn an_explicit_forge_remote_is_obeyed_and_a_missing_one_refused() {
    let sandbox = Sandbox::new("explicit");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["remote", "add", "origin", "git@github.com:arno/widget.git"]);
    sandbox.git(&[
        "remote",
        "add",
        "upstream",
        "git@github.com:acme/widget.git",
    ]);
    let state = sandbox.state();
    // Without being told, and without the forge saying origin is a fork, the
    // project is origin.
    assert_eq!(forge::remote_slug(&state).unwrap().full(), "arno/widget");

    sandbox.git(&["config", forge::FORGE_REMOTE_KEY, "upstream"]);
    assert_eq!(forge::remote_slug(&state).unwrap().full(), "acme/widget");

    sandbox.git(&["config", forge::FORGE_REMOTE_KEY, "gone"]);
    let local = forge::Local::read(&state).unwrap();
    assert!(forge::choose(&local, None).is_err());
}

#[test]
fn a_detached_head_and_an_unborn_branch_still_name_the_project() {
    let sandbox = Sandbox::new("detached");
    sandbox.git(&[
        "remote",
        "add",
        "origin",
        "git@gitlab.com:group/sub/app.git",
    ]);
    let state = sandbox.state();
    // Unborn: no commit yet.
    assert_eq!(forge::remote_slug(&state).unwrap().full(), "group/sub/app");
    let local = forge::Local::read(&state).unwrap();
    assert_eq!(local.upstream, None);

    sandbox.commit("a.txt", "one\n", "First");
    sandbox.commit("a.txt", "two\n", "Second");
    sandbox.git(&["checkout", "-q", "--detach", "HEAD~1"]);
    let local = forge::Local::read(&state).unwrap();
    assert_eq!(local.head, None);
    assert_eq!(forge::remote_slug(&state).unwrap().full(), "group/sub/app");
}

/* ---------- which profile ------------------------------------------------- */

#[test]
fn a_repository_on_another_forge_asks_for_the_profile_that_fits() {
    let sandbox = Sandbox::new("match");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["remote", "add", "origin", "git@gitlab.com:acme/widget.git"]);
    let state = sandbox.state();
    let github = profile("Personal", ForgeKind::GitHub, "github.com");
    let gitlab = profile("Work", ForgeKind::GitLab, "gitlab.com");
    let (github_id, gitlab_id) = (github.id.clone(), gitlab.id.clone());
    state
        .update_config(|config| {
            config.profiles = vec![github.clone(), gitlab.clone()];
            config.active_profile = Some(github_id.clone());
        })
        .unwrap();

    let found = forge::profile_match(&state);
    assert!(
        !found.fits,
        "a GitHub profile does not fit a gitlab.com remote"
    );
    assert_eq!(found.host.as_deref(), Some("gitlab.com"));
    assert_eq!(found.best.len(), 1);
    assert_eq!(found.best[0].name, "Work");

    // Given to the Work profile, the repository answers to it while Personal
    // stays the active profile everywhere else.
    let path = sandbox.work.to_string_lossy().into_owned();
    state
        .update_config(|config| {
            config.repo_profiles.insert(path.clone(), gitlab_id.clone());
        })
        .unwrap();
    let found = forge::profile_match(&state);
    assert!(found.fits);
    assert!(found.bound);
    assert_eq!(found.using.unwrap().name, "Work");
    assert_eq!(state.config().active().unwrap().name, "Personal");
}

#[test]
fn opening_under_the_wrong_profile_does_not_rewrite_the_identity() {
    let sandbox = Sandbox::new("identity");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["remote", "add", "origin", "git@gitlab.com:acme/widget.git"]);
    let state = sandbox.state();
    let mut github = profile("Personal", ForgeKind::GitHub, "github.com");
    github.git_name = Some("Robin".into());
    github.git_email = Some("robin@home.example".into());
    let id = github.id.clone();
    // gitlab.com has a profile of its own, so it is not the GitHub one's.
    let gitlab = profile("Work", ForgeKind::GitLab, "gitlab.com");
    state
        .update_config(|config| {
            config.profiles = vec![github.clone(), gitlab.clone()];
            config.active_profile = Some(id.clone());
        })
        .unwrap();

    let said = identity::apply(&state, false).unwrap().unwrap_or_default();
    assert!(said.contains("gitlab.com"), "{said}");
    // Said once, not on every open.
    assert_eq!(identity::apply(&state, false).unwrap(), None);
    assert_eq!(
        sandbox.git(&["config", "--local", "user.email"]).trim(),
        "test@example.com",
        "the address the repository already had is untouched"
    );

    // A profile that fits, but the repository already commits as somebody
    // else: said, not done.
    sandbox.git(&[
        "remote",
        "set-url",
        "origin",
        "git@github.com:acme/widget.git",
    ]);
    let said = identity::apply(&state, false).unwrap().unwrap_or_default();
    assert!(said.contains("Kept as it is"), "{said}");
    assert_eq!(
        sandbox.git(&["config", "--local", "user.email"]).trim(),
        "test@example.com"
    );
    let check = identity::check(&state).unwrap();
    assert!(check.differs && check.fits);

    // Said yes to: done.
    identity::apply(&state, true).unwrap();
    assert_eq!(
        sandbox.git(&["config", "--local", "user.email"]).trim(),
        "robin@home.example"
    );

    // A repository with no address of its own simply gets the profile's.
    sandbox.git(&["config", "--local", "--unset", "user.email"]);
    let said = identity::apply(&state, false).unwrap().unwrap_or_default();
    assert!(said.contains("Committing here as Robin"), "{said}");
}

/* ---------- the home tab -------------------------------------------------- */

#[test]
fn never_pushed_work_is_counted_on_every_branch_not_only_head() {
    let sandbox = Sandbox::new("home-unpushed");
    sandbox.with_origin();
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["push", "-q", "-u", "origin", "main"]);

    // Five commits on a branch that was never pushed, checked out.
    sandbox.git(&["checkout", "-q", "-b", "local-only"]);
    for n in 0..5 {
        sandbox.commit("a.txt", &format!("{n}\n"), &format!("Local {n}"));
    }
    // And two on another one that is not.
    sandbox.git(&["checkout", "-q", "-b", "side", "main"]);
    sandbox.commit("b.txt", "1\n", "Side 1");
    sandbox.commit("b.txt", "2\n", "Side 2");
    sandbox.git(&["checkout", "-q", "local-only"]);

    let card = home::card_for(&sandbox.work, "x", "x");
    assert_eq!(card.ahead, 0, "nothing to be ahead of without an upstream");
    let mut unpushed: Vec<(String, usize)> = card
        .unpushed
        .iter()
        .map(|b| (b.name.clone(), b.count))
        .collect();
    unpushed.sort();
    assert_eq!(
        unpushed,
        vec![("local-only".to_string(), 5), ("side".to_string(), 2)]
    );
    assert!(card.has_remote);
}

#[test]
fn a_repository_without_a_remote_has_nothing_unpushed() {
    let sandbox = Sandbox::new("home-noremote");
    sandbox.commit("a.txt", "one\n", "First");
    let card = home::card_for(&sandbox.work, "x", "x");
    assert!(!card.has_remote);
    assert!(card.unpushed.is_empty());
}

#[test]
fn home_finds_a_trunk_behind_old_stashes_and_branches_whose_upstream_went() {
    let sandbox = Sandbox::new("home-loose");
    sandbox.with_origin();
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["push", "-q", "-u", "origin", "main"]);

    // A branch last touched two months ago, pushed, and deleted on the forge.
    sandbox.git(&["checkout", "-q", "-b", "old-work"]);
    sandbox.commit_dated("c.txt", "c\n", "Old", "2020-01-01T12:00:00Z");
    sandbox.git(&["push", "-q", "-u", "origin", "old-work"]);
    sandbox.git(&["push", "-q", "origin", "--delete", "old-work"]);
    // A branch and a tag of the same name do not confuse the survey.
    sandbox.git(&["tag", "old-work"]);
    sandbox.git(&["checkout", "-q", "main"]);

    // Somebody else moved main on.
    let other = sandbox.elsewhere();
    std::fs::write(other.join("a.txt"), "two\n").unwrap();
    git_in(&other, &["commit", "-q", "-am", "Theirs"]);
    git_in(&other, &["push", "-q", "origin", "main"]);
    sandbox.git(&["fetch", "-q", "--prune", "origin"]);

    // A stash from a month ago.
    std::fs::write(sandbox.work.join("a.txt"), "dirty\n").unwrap();
    git_env(
        &sandbox.work,
        &["stash", "push", "-q", "-m", "forgotten"],
        &[
            ("GIT_AUTHOR_DATE", "2020-01-01T12:00:00Z"),
            ("GIT_COMMITTER_DATE", "2020-01-01T12:00:00Z"),
        ],
    );

    let card = home::card_for(&sandbox.work, "x", "x");
    assert_eq!(card.trunk.as_deref(), Some("main"));
    assert_eq!(card.trunk_behind, 1);
    assert_eq!(card.old_stashes, 1);
    assert!(card.oldest_stash_days > 14);
    assert_eq!(card.gone, vec!["old-work".to_string()]);
    assert!(
        card.unpushed.is_empty(),
        "old-work tracks something, gone or not"
    );
}

#[test]
fn home_counts_your_commits_by_the_address_git_uses_when_the_profile_has_none() {
    let sandbox = Sandbox::new("home-author");
    sandbox.commit("a.txt", "one\n", "Mine on main");
    // Somebody else's commit, on main too.
    std::fs::write(sandbox.work.join("a.txt"), "two\n").unwrap();
    sandbox.git(&["add", "--all"]);
    git_env(
        &sandbox.work,
        &["commit", "-q", "-m", "Theirs"],
        &[
            ("GIT_AUTHOR_EMAIL", "other@example.com"),
            ("GIT_COMMITTER_EMAIL", "other@example.com"),
        ],
    );
    // Mine on a branch that is not checked out.
    sandbox.git(&["checkout", "-q", "-b", "side"]);
    sandbox.commit("b.txt", "b\n", "Mine on a side branch");
    sandbox.git(&["checkout", "-q", "main"]);
    // Somebody whose address contains mine is not me.
    std::fs::write(sandbox.work.join("c.txt"), "c\n").unwrap();
    sandbox.git(&["add", "--all"]);
    git_env(
        &sandbox.work,
        &["commit", "-q", "-m", "Lookalike"],
        &[("GIT_AUTHOR_EMAIL", "not-test@example.com")],
    );

    let state = sandbox.state();
    let path = sandbox.work.to_string_lossy().into_owned();
    state
        .update_config(|config| {
            let profile = config.active_mut().unwrap();
            profile.git_email = None;
            profile.projects = vec![Project {
                path: path.clone(),
                name: "work".into(),
            }];
        })
        .unwrap();

    let summary = home::summary(&state).unwrap();
    assert_eq!(summary.author.as_deref(), Some("test@example.com"));
    assert_eq!(
        summary.stats.read, 2,
        "mine on main and on the side branch, nobody else's"
    );
    assert!(summary.uncounted.is_empty());
}

#[test]
fn with_no_address_anywhere_nothing_is_counted_as_yours() {
    assert_eq!(home::author_for(None, None), None);
    assert_eq!(home::author_for(Some("  "), None), None);
    assert_eq!(
        home::author_for(None, Some("me@example.com\n")).as_deref(),
        Some("me@example.com")
    );
    assert_eq!(
        home::author_for(Some("p@example.com"), Some("me@example.com")).as_deref(),
        Some("p@example.com"),
        "the profile's address wins"
    );
}

#[test]
fn reads_branch_lines_and_how_far_behind_they_are() {
    let lines = home::parse_branches(
        "main\u{1f}refs/remotes/origin/main\u{1f}[behind 3]\u{1f}1700000000\n\
         feature\u{1f}\u{1f}\u{1f}1700000001\n\
         gone\u{1f}refs/remotes/origin/gone\u{1f}[gone]\u{1f}1600000000\n",
    );
    assert_eq!(lines.len(), 3);
    assert_eq!(home::behind_in(&lines[0].track), 3);
    assert!(lines[1].upstream.is_empty());
    assert_eq!(home::behind_in("[ahead 2, behind 7]"), 7);
    assert_eq!(home::behind_in("[ahead 2]"), 0);
    assert_eq!(home::behind_in(""), 0);
}

/* ---------- the AI description's range ------------------------------------ */

#[test]
fn a_review_is_described_against_the_remote_target_not_a_stale_local_one() {
    let sandbox = Sandbox::new("ai-range");
    sandbox.with_origin();
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["push", "-q", "-u", "origin", "main"]);

    // main moves on the forge; the local main is never pulled.
    let other = sandbox.elsewhere();
    std::fs::write(other.join("a.txt"), "two\n").unwrap();
    git_in(&other, &["commit", "-q", "-am", "Somebody else's"]);
    git_in(&other, &["push", "-q", "origin", "main"]);
    // And a branch exists only on the forge.
    git_in(&other, &["checkout", "-q", "-b", "remote-only"]);
    std::fs::write(other.join("b.txt"), "b\n").unwrap();
    git_in(&other, &["add", "--all"]);
    git_in(&other, &["commit", "-q", "-m", "Theirs to review"]);
    git_in(&other, &["push", "-q", "origin", "remote-only"]);
    sandbox.git(&["fetch", "-q", "origin"]);

    assert_eq!(
        ai::resolve_target(&sandbox.work, "main", Some("origin")).unwrap(),
        "origin/main"
    );
    assert_eq!(
        ai::resolve_ref(&sandbox.work, "remote-only").unwrap(),
        "origin/remote-only",
        "a branch that is only on the remote is still a branch"
    );
    // With no remote copy, the local branch is all there is.
    sandbox.git(&["branch", "local-target"]);
    assert_eq!(
        ai::resolve_target(&sandbox.work, "local-target", Some("origin")).unwrap(),
        "local-target"
    );
}

/* ---------- after a merge ------------------------------------------------- */

/// A clone on `feature`, pushed, and merged into main on the forge with the
/// branch deleted there — what the merge button leaves behind.
fn merged_feature(tag: &str) -> Sandbox {
    let sandbox = Sandbox::new(tag);
    sandbox.with_origin();
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["push", "-q", "-u", "origin", "main"]);
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("f.txt", "feature\n", "The feature");
    sandbox.git(&["push", "-q", "-u", "origin", "feature"]);

    let other = sandbox.elsewhere();
    git_in(&other, &["fetch", "-q", "origin"]);
    git_in(
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
    git_in(&other, &["push", "-q", "origin", "main"]);
    git_in(&other, &["push", "-q", "origin", "--delete", "feature"]);
    sandbox
}

#[test]
fn after_a_merge_the_clone_steps_onto_an_up_to_date_trunk() {
    let sandbox = merged_feature("after-merge");
    let state = sandbox.state();
    let tip = sandbox.sha("feature");

    let done =
        review::after_merge(&state, "feature", Some(&tip), Some("origin"), true, None).unwrap();
    assert_eq!(done.switched_to.as_deref(), Some("main"));
    assert_eq!(sandbox.head(), "main");
    assert_eq!(
        sandbox.sha("main"),
        sandbox.sha("origin/main"),
        "main was fast-forwarded to the merge"
    );
    assert_eq!(
        done.branch.as_deref(),
        Some("feature"),
        "offered, not deleted"
    );
    assert_eq!(done.sha.as_deref(), Some(tip.as_str()));
    assert!(refs::has_local_branch(&state, "feature"));
    // The prune removed the copy the forge deleted.
    assert!(!sandbox.git(&["branch", "-r"]).contains("origin/feature"));

    // Deleting it, and putting it back.
    review::delete_merged(&state, "feature", &tip).unwrap();
    assert!(!refs::has_local_branch(&state, "feature"));
    review::restore_branch(&state, "feature", &tip).unwrap();
    assert_eq!(sandbox.sha("feature"), tip);
}

#[test]
fn a_dirty_tree_stays_where_it_is_after_a_merge() {
    let sandbox = merged_feature("after-merge-dirty");
    std::fs::write(sandbox.work.join("f.txt"), "half done\n").unwrap();
    let state = sandbox.state();
    let tip = sandbox.sha("feature");

    let done =
        review::after_merge(&state, "feature", Some(&tip), Some("origin"), true, None).unwrap();
    assert_eq!(done.switched_to, None);
    assert_eq!(sandbox.head(), "feature");
    assert!(
        done.message.contains("stayed on feature"),
        "{}",
        done.message
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.work.join("f.txt")).unwrap(),
        "half done\n",
        "the work in progress is untouched"
    );
}

#[test]
fn a_branch_with_work_the_review_did_not_have_is_not_offered() {
    let sandbox = merged_feature("after-merge-extra");
    let tip = sandbox.sha("feature");
    sandbox.commit("g.txt", "more\n", "After the review");
    let state = sandbox.state();

    let done =
        review::after_merge(&state, "feature", Some(&tip), Some("origin"), true, None).unwrap();
    assert_eq!(done.extra, 1);
    assert_eq!(done.branch, None);
    assert!(done.message.contains("1 commit"), "{}", done.message);

    // And a branch that moved after the offer is not deleted on its strength.
    assert!(review::delete_merged(&state, "feature", &tip).is_err());
    assert!(refs::has_local_branch(&state, "feature"));
}

#[test]
fn with_the_switch_off_only_the_fetch_happens() {
    let sandbox = merged_feature("after-merge-off");
    let state = sandbox.state();
    let tip = sandbox.sha("feature");
    let done =
        review::after_merge(&state, "feature", Some(&tip), Some("origin"), false, None).unwrap();
    assert_eq!(done.switched_to, None);
    assert_eq!(sandbox.head(), "feature");
    assert!(!sandbox.git(&["branch", "-r"]).contains("origin/feature"));
}

#[test]
fn a_trunk_with_commits_of_its_own_is_switched_to_but_not_forced() {
    let sandbox = merged_feature("after-merge-diverged");
    // A commit on the local main that the forge does not have.
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.commit("m.txt", "mine\n", "Local only on main");
    let local_main = sandbox.sha("main");
    sandbox.git(&["checkout", "-q", "feature"]);
    let state = sandbox.state();
    let tip = sandbox.sha("feature");

    let done =
        review::after_merge(&state, "feature", Some(&tip), Some("origin"), true, None).unwrap();
    assert!(
        done.message.contains("not fast-forwarded"),
        "{}",
        done.message
    );
    // Whatever checking out decided to do with a diverged branch, the local
    // commit is still in main's history.
    let contains = sandbox.git(&["branch", "--contains", &local_main]);
    assert!(contains.contains("main"), "{contains}");
}

#[test]
fn errors_from_the_forge_for_a_moved_head_are_recognised() {
    assert!(forge::head_moved(
        409,
        "409 Conflict: Head branch was modified. Review and try the merge again."
    ));
    assert!(forge::head_moved(
        409,
        "409 Conflict: SHA does not match HEAD of source branch: abc"
    ));
    assert!(!forge::head_moved(405, "405: Merge method not allowed"));
    assert!(!forge::head_moved(409, "409: merge conflict"));
}

/* ---------- a new review's description ------------------------------------ */

#[test]
fn a_new_review_is_filled_in_from_the_branchs_own_commits() {
    let sandbox = Sandbox::new("draft-commits");
    sandbox.with_origin();
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["push", "-q", "-u", "origin", "main"]);
    sandbox.git(&["checkout", "-q", "-b", "42-parser"]);
    sandbox.commit("p.txt", "p\n", "Add the parser\n\nIt parses.");
    sandbox.commit("q.txt", "q\n", "Test the parser");
    let state = sandbox.state();

    let commits = review::branch_commits(&state, "42-parser", "main").unwrap();
    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(
        subjects,
        vec!["Add the parser", "Test the parser"],
        "oldest first"
    );
    assert_eq!(commits[0].body, "It parses.");

    // Measured against the forge's main, not a local one left behind.
    let other = sandbox.elsewhere();
    std::fs::write(other.join("a.txt"), "two\n").unwrap();
    git_in(&other, &["commit", "-q", "-am", "Theirs"]);
    git_in(&other, &["push", "-q", "origin", "main"]);
    sandbox.git(&["fetch", "-q", "origin"]);
    sandbox.git(&["checkout", "-q", "main"]);
    sandbox.git(&["checkout", "-q", "-b", "after"]);
    sandbox.git(&["merge", "-q", "--ff-only", "origin/main"]);
    sandbox.commit("r.txt", "r\n", "Mine");
    let commits = review::branch_commits(&state, "after", "main").unwrap();
    assert_eq!(
        commits
            .iter()
            .map(|c| c.subject.as_str())
            .collect::<Vec<_>>(),
        vec!["Mine"],
        "the commit pulled from the forge is not this branch's"
    );
}

#[test]
fn a_host_no_profile_is_for_gets_the_profiles_identity_and_signing() {
    let sandbox = Sandbox::new("identity-unclaimed");
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["config", "--local", "--unset", "user.email"]);
    sandbox.git(&[
        "remote",
        "add",
        "origin",
        "git@bitbucket.org:acme/widget.git",
    ]);
    let state = sandbox.state();
    let mut github = profile("Personal", ForgeKind::GitHub, "github.com");
    github.git_name = Some("Robin".into());
    github.git_email = Some("robin@home.example".into());
    github.sign_commits = Some(false);
    let id = github.id.clone();
    state
        .update_config(|config| {
            config.profiles = vec![github.clone()];
            config.active_profile = Some(id.clone());
        })
        .unwrap();

    let said = identity::apply(&state, false).unwrap().unwrap_or_default();
    assert!(said.contains("Committing here as Robin"), "{said}");
    assert_eq!(
        sandbox.git(&["config", "--local", "user.email"]).trim(),
        "robin@home.example"
    );
    assert_eq!(
        sandbox.git(&["config", "--local", "commit.gpgsign"]).trim(),
        "false"
    );
    assert!(identity::check(&state).unwrap().fits);
}

/* ---------- after a merge, in the right repository and on the right branch -- */

#[test]
fn after_a_merge_stays_in_its_repository_when_the_tab_changes_mid_fetch() {
    let a = merged_feature("after-merge-race-a");
    let b = Sandbox::new("after-merge-race-b");
    b.commit("x.txt", "x\n", "First");
    b.git(&["checkout", "-q", "-b", "mywork"]);
    b.commit("y.txt", "y\n", "Mine");
    // A slow fetch, the way a real network is.
    a.git(&[
        "config",
        "remote.origin.uploadpack",
        "sleep 2; git-upload-pack",
    ]);
    let state = a.state();
    let tip = a.sha("feature");
    std::thread::scope(|scope| {
        let running = scope.spawn(|| {
            review::after_merge(&state, "feature", Some(&tip), Some("origin"), true, None)
        });
        std::thread::sleep(std::time::Duration::from_millis(700));
        // Another tab comes to the front: its calls point the state at it.
        state.set_path(b.work.clone());
        running.join().unwrap().unwrap();
    });
    assert_eq!(b.head(), "mywork", "the other repository was left alone");
    assert_eq!(a.head(), "main", "this one stepped onto its trunk");
}

#[test]
fn a_fork_review_from_a_branch_of_my_name_leaves_mine_alone() {
    let sandbox = Sandbox::new("fork-same-name");
    sandbox.with_origin();
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["push", "-q", "-u", "origin", "main"]);
    sandbox.git(&["checkout", "-q", "-b", "feature"]);
    sandbox.commit("f.txt", "mine\n", "My own feature");
    sandbox.git(&["push", "-q", "-u", "origin", "feature"]);
    let state = sandbox.state();
    let from = review::ReviewFrom {
        fork: true,
        repo: Some("contributor/app".into()),
    };
    let done = review::after_merge(
        &state,
        "feature",
        Some("1111111111111111111111111111111111111111"),
        Some("origin"),
        true,
        Some(&from),
    )
    .unwrap();
    assert_eq!(sandbox.head(), "feature");
    assert_eq!(done.branch, None);
}

#[test]
fn the_trunk_is_never_offered_for_deletion_after_a_merge() {
    let sandbox = Sandbox::new("trunk-not-offered");
    sandbox.with_origin();
    sandbox.commit("a.txt", "one\n", "First");
    sandbox.git(&["push", "-q", "-u", "origin", "main"]);
    sandbox.git(&["checkout", "-q", "-b", "dev"]);
    let state = sandbox.state();
    for fork in [true, false] {
        let from = review::ReviewFrom { fork, repo: None };
        let done =
            review::after_merge(&state, "main", None, Some("origin"), true, Some(&from)).unwrap();
        assert_ne!(done.branch.as_deref(), Some("main"), "fork: {fork}");
    }
}

/* ---------- hosts with a port, and the home tab's inbox ------------------- */

#[test]
fn a_self_hosted_forge_on_a_port_is_the_same_host_as_its_remotes() {
    assert!(forge::same_host(
        "gitlab.example.com",
        "gitlab.example.com:8443"
    ));
    let slug = forge::parse_remote("https://gitlab.example.com:8443/team/app.git").unwrap();
    let lab = profile("Lab", ForgeKind::GitLab, "gitlab.example.com:8443");
    assert!(forge::same_host(&slug.host, &forge::profile_host(&lab)));
    assert_ne!(forge::fit(&lab, &slug), forge::Fit::No);
}

#[test]
fn the_inbox_does_not_send_a_fork_review_to_a_project_that_only_looked_at_it() {
    let mine = Sandbox::new("inbox-mine");
    mine.commit("a.txt", "one\n", "First");
    mine.git(&["remote", "add", "origin", "git@github.com:acme/app.git"]);
    // A remote added to check out a contributor's review from their fork.
    mine.git(&["remote", "add", "robin", "git@github.com:robin/app.git"]);
    mine.git(&["config", "remote.robin.gitnoob-review", "true"]);
    let theirs = Sandbox::new("inbox-theirs");
    theirs.commit("a.txt", "one\n", "First");
    theirs.git(&["remote", "add", "upstream", "git@github.com:acme/app.git"]);
    theirs.git(&["remote", "add", "origin", "git@github.com:robin/tools.git"]);

    let state = mine.state();
    let project = |sandbox: &Sandbox| Project {
        path: sandbox.work.to_string_lossy().into_owned(),
        name: "x".into(),
    };
    let (one, two) = (project(&theirs), project(&mine));
    state
        .update_config(|config| {
            config.active_mut().unwrap().projects = vec![one.clone(), two.clone()];
        })
        .unwrap();
    let found = forge::projects_by_slug(&state);
    assert_eq!(found.get("github.com/robin/app"), None);
    assert_eq!(
        found.get("github.com/acme/app"),
        Some(&two.path),
        "the project whose own remote it is wins"
    );
}

/// GitLab says nothing about where a review's branch lives when it is in the
/// project itself, so the window hands over an empty repository name. That is
/// not knowing, not a different project: the branch is still mine to step off.
#[test]
fn a_same_project_review_with_no_repository_name_is_still_tidied_after() {
    let sandbox = merged_feature("after-merge-no-repo");
    let state = sandbox.state();
    let tip = sandbox.sha("feature");
    let from = review::ReviewFrom {
        fork: false,
        repo: Some(String::new()),
    };
    let done = review::after_merge(
        &state,
        "feature",
        Some(&tip),
        Some("origin"),
        true,
        Some(&from),
    )
    .unwrap();
    assert_eq!(done.switched_to.as_deref(), Some("main"));
    assert_eq!(done.branch.as_deref(), Some("feature"));
}
