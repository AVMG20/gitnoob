//! A real repository in a temporary directory, for the tests that need one.
//!
//! Most of what this app does is a conversation with `git`, and the parts worth
//! testing hardest — putting work down and picking it up again around a branch
//! switch, undoing a step someone else has moved on from — are exactly the
//! parts a pure function cannot stand in for. So those tests get a repository:
//! made from nothing, driven with the same CLI the app drives, and deleted when
//! the test ends.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::state::AppState;

static COUNT: AtomicUsize = AtomicUsize::new(0);

pub struct Fixture {
    pub dir: PathBuf,
    pub config_dir: PathBuf,
    pub state: AppState,
}

impl Fixture {
    /// A repository on `main` with one commit in it, and nothing else.
    pub fn new() -> Fixture {
        let unique = format!(
            "gitnoob-test-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::SeqCst)
        );
        let root = std::env::temp_dir().join(unique);
        let dir = root.join("repo");
        let config_dir = root.join("config");
        std::fs::create_dir_all(&dir).expect("make the repository directory");
        std::fs::create_dir_all(&config_dir).expect("make the config directory");

        let state = AppState::new(config_dir.clone());
        state.set_path(dir.clone());
        let fixture = Fixture {
            dir,
            config_dir,
            state,
        };

        // `-b main` is not old enough to rely on, and the default branch name
        // depends on whoever is running the test.
        fixture.git(&["init", "--quiet"]);
        fixture.git(&["symbolic-ref", "HEAD", "refs/heads/main"]);
        fixture.git(&["config", "user.name", "Test"]);
        fixture.git(&["config", "user.email", "test@example.com"]);
        fixture.git(&["config", "commit.gpgsign", "false"]);
        fixture
    }

    /// Runs git in the repository and answers with its output, panicking with
    /// git's own words when it fails — a broken fixture should say why.
    pub fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn write(&self, name: &str, text: &str) {
        std::fs::write(self.dir.join(name), text).expect("write a file");
    }

    pub fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
    }

    pub fn commit(&self, message: &str) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "-m", message]);
    }

    /// `git status --porcelain`, which says what is staged and what is not.
    pub fn status(&self) -> String {
        self.git(&["status", "--porcelain"])
    }

    pub fn branch(&self) -> String {
        self.git(&["rev-parse", "--abbrev-ref", "HEAD"]).trim().to_string()
    }

    pub fn stashes(&self) -> Vec<String> {
        self.git(&["stash", "list"])
            .lines()
            .map(str::to_string)
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(root) = self.dir.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
