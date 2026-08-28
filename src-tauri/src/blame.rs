//! Who last touched each line, and when.
//!
//! Through libgit2 rather than `git blame`, for the same reason the diffs are:
//! the answer is wanted for a file the window is already showing, and parsing
//! porcelain output to rebuild what the library hands over as a struct is work
//! with nothing at the end of it.

use std::collections::HashMap;

use git2::{BlameOptions, Repository};
use serde::Serialize;

use crate::state::AppState;

/// One run of consecutive lines that came in with the same commit.
///
/// A run rather than a line, because that is how it reads: a file of four
/// hundred lines is a dozen or so runs, and the panel draws one chip per run
/// instead of four hundred identical ones.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct BlameRun {
    pub oid: String,
    pub short: String,
    pub summary: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    /// The first line of the run, counting from one.
    pub start: usize,
    pub lines: usize,
    /// A commit that is not in the history yet: the line is uncommitted work.
    pub uncommitted: bool,
}

/// The blame for a file, as runs covering it from the first line to the last.
///
/// `at` is a commit to read the file as of; without one the working tree is
/// blamed, so the lines you are looking at are the lines answered about.
pub fn of(state: &AppState, path: &str, at: Option<&str>) -> Result<Vec<BlameRun>, String> {
    let repo = state.repo()?;
    let mut options = BlameOptions::new();
    // Following the file through renames costs a walk per rename and is what
    // makes blame slow on an old file; the answer without it is still the
    // answer to "who wrote this line", which is the question being asked.
    options.track_copies_same_file(true);

    if let Some(oid) = at {
        let commit = repo
            .revparse_single(oid)
            .map_err(|e| format!("No commit {oid}: {e}"))?
            .peel_to_commit()
            .map_err(|e| format!("{oid} is not a commit: {e}"))?;
        options.newest_commit(commit.id());
    }

    let blame = match repo.blame_file(std::path::Path::new(path), Some(&mut options)) {
        Ok(blame) => blame,
        // A file that has never been committed is not in the tree blame reads
        // from, and libgit2 says so as an error. It is not one: every line of
        // it is yours and uncommitted, which is exactly what the column would
        // have said had the file been added a minute ago instead of not at
        // all. Only when the working tree is what is being blamed — asking
        // about a commit that really does not have the file is a question with
        // no answer.
        Err(e) if at.is_none() && e.code() == git2::ErrorCode::NotFound => {
            return Ok(all_uncommitted(state, path))
        }
        Err(e) => return Err(format!("Could not blame {path}: {e}")),
    };

    let mut summaries: HashMap<git2::Oid, (String, String, String, i64)> = HashMap::new();
    let mut runs = Vec::with_capacity(blame.len());

    for hunk in blame.iter() {
        let oid = hunk.final_commit_id();
        // libgit2 gives the not-yet-committed lines the zero id, which no
        // commit lookup will ever answer for.
        let uncommitted = oid.is_zero();
        let (summary, author, email, time) = if uncommitted {
            (
                "Not committed yet".to_string(),
                hunk.final_signature().name().unwrap_or("You").to_string(),
                hunk.final_signature().email().unwrap_or("").to_string(),
                hunk.final_signature().when().seconds(),
            )
        } else {
            summaries
                .entry(oid)
                .or_insert_with(|| read_commit(&repo, oid))
                .clone()
        };

        runs.push(BlameRun {
            oid: oid.to_string(),
            short: if uncommitted {
                String::new()
            } else {
                oid.to_string().chars().take(7).collect()
            },
            summary,
            author,
            email,
            time,
            start: hunk.final_start_line(),
            lines: hunk.lines_in_hunk(),
            uncommitted,
        });
    }

    Ok(runs)
}

/// One run over the whole of a file nothing has committed yet.
///
/// Named for whoever the repository would commit as, since that is who wrote
/// the lines; an empty or unreadable file gets no runs at all rather than a
/// run of nothing.
fn all_uncommitted(state: &AppState, path: &str) -> Vec<BlameRun> {
    let Ok(root) = state.path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(root.join(path)) else {
        return Vec::new();
    };
    let lines = text.lines().count();
    if lines == 0 {
        return Vec::new();
    }
    let who = state
        .repo()
        .ok()
        .and_then(|repo| repo.signature().ok())
        .map(|signature| {
            (
                signature.name().unwrap_or("You").to_string(),
                signature.email().unwrap_or("").to_string(),
            )
        })
        .unwrap_or_else(|| ("You".to_string(), String::new()));

    vec![BlameRun {
        oid: git2::Oid::zero().to_string(),
        short: String::new(),
        summary: "Not committed yet".to_string(),
        author: who.0,
        email: who.1,
        time: chrono_now(),
        start: 1,
        lines,
        uncommitted: true,
    }]
}

/// Seconds since the epoch, for a run that has no commit to take a time from.
fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// The parts of a commit a blame chip shows. Read once per commit, however
/// many runs it accounts for.
fn read_commit(repo: &Repository, oid: git2::Oid) -> (String, String, String, i64) {
    let Ok(commit) = repo.find_commit(oid) else {
        return (
            "Unknown commit".to_string(),
            String::new(),
            String::new(),
            0,
        );
    };
    let author = commit.author();
    (
        commit.summary().unwrap_or("").to_string(),
        author.name().unwrap_or("").to_string(),
        author.email().unwrap_or("").to_string(),
        commit.time().seconds(),
    )
}

/// Every commit that touched a file, newest first.
///
/// `--follow` so a file that was moved keeps its history, which is the whole
/// reason to ask: the commits before the rename are the ones you cannot find
/// any other way.
pub fn history(state: &AppState, path: &str, limit: usize) -> Result<Vec<FileCommit>, String> {
    let root = state.path()?;
    let count = limit.to_string();
    let raw = crate::git_cmd::run_checked(
        &root,
        &[
            "log",
            "--follow",
            "--no-show-signature",
            "--format=%H%x1f%an%x1f%ae%x1f%at%x1f%s",
            "-n",
            &count,
            "--",
            path,
        ],
    )?;
    Ok(read_history(&raw))
}

/// One commit in a file's history.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct FileCommit {
    pub oid: String,
    pub short: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub summary: String,
}

fn read_history(raw: &str) -> Vec<FileCommit> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let oid = parts.next()?.trim().to_string();
            if oid.is_empty() {
                return None;
            }
            let author = parts.next().unwrap_or("").to_string();
            let email = parts.next().unwrap_or("").to_string();
            let time = parts.next().unwrap_or("0").trim().parse().unwrap_or(0);
            // A summary may hold anything, separators included, so it takes
            // the rest of the line rather than the next field.
            let summary = parts.collect::<Vec<_>>().join("\u{1f}");
            Some(FileCommit {
                short: oid.chars().take(7).collect(),
                oid,
                author,
                email,
                time,
                summary,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// A file nobody has committed is every line yours, not an error.
    ///
    /// libgit2 blames against a tree, and a new file is in no tree: it answers
    /// "the path does not exist in the given tree", which the window used to
    /// print across the top of the file the user was reading.
    #[test]
    fn a_file_with_no_commits_is_blamed_as_uncommitted_work() {
        let root = std::env::temp_dir().join(format!("gitnoob-blame-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--quiet", "--initial-branch=main"]);
        git(&root, &["config", "user.email", "test@example.com"]);
        git(&root, &["config", "user.name", "Test"]);
        std::fs::write(root.join("first.txt"), "one\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "--quiet", "-m", "first"]);

        // Added but never committed, which is the case that failed.
        std::fs::write(root.join("new.txt"), "a\nb\nc\n").unwrap();
        git(&root, &["add", "new.txt"]);

        let state = AppState::new(root.join("config"));
        state.set_path(root.clone());

        let runs = of(&state, "new.txt", None).unwrap();
        assert_eq!(runs.len(), 1);
        assert!(runs[0].uncommitted);
        assert_eq!(runs[0].summary, "Not committed yet");
        assert_eq!(runs[0].start, 1);
        assert_eq!(runs[0].lines, 3);
        assert_eq!(runs[0].author, "Test");

        // A committed file still gets its real answer.
        let committed = of(&state, "first.txt", None).unwrap();
        assert_eq!(committed.len(), 1);
        assert!(!committed[0].uncommitted);
        assert_eq!(committed[0].summary, "first");

        // A file that is genuinely not there is still an error, and so is one
        // asked about as of a commit that never had it.
        assert!(of(&state, "nothing.txt", None).unwrap().is_empty());
        assert!(of(&state, "new.txt", Some("HEAD")).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_a_commit_a_line() {
        const RAW: &str = concat!(
            "aaa1\u{1f}Ramon Robben\u{1f}ramon@example.com\u{1f}1756000000\u{1f}fix: the thing\n",
            "bbb2\u{1f}A Contributor\u{1f}other@example.com\u{1f}1755000000\u{1f}feat: a thing\n"
        );
        let found = read_history(RAW);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].oid, "aaa1");
        assert_eq!(found[0].short, "aaa1");
        assert_eq!(found[0].author, "Ramon Robben");
        assert_eq!(found[0].time, 1756000000);
        assert_eq!(found[0].summary, "fix: the thing");
        assert_eq!(found[1].email, "other@example.com");
    }

    #[test]
    fn a_summary_holding_the_separator_is_kept_whole() {
        let found = read_history("aaa1\u{1f}R\u{1f}r@x\u{1f}1\u{1f}fix: a\u{1f}b\n");
        assert_eq!(found[0].summary, "fix: a\u{1f}b");
    }

    #[test]
    fn blank_lines_are_not_commits() {
        assert!(read_history("\n\n").is_empty());
    }
}
