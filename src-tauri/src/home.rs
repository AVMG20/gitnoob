//! What the home tab shows: every project, and a year of your own commits.
//!
//! Read straight from the repositories the profile has opened before, with git
//! rather than with a database: the answer has to be true of the folders as
//! they are now, and anything cached would be wrong the moment a commit was
//! made in a terminal. Everything here is a plain `git log` or `git status`,
//! and the whole lot runs off the window's thread.

use std::collections::HashMap;

use serde::Serialize;

use crate::git_cmd;
use crate::state::AppState;

/// How far back the year grid goes. 53 weeks, so it always starts on a Monday
/// column and the shape does not shift as the days go by.
const DAYS: usize = 371;
/// Enough projects for anybody's list without walking a disk full of them.
const MAX_REPOS: usize = 40;

/// One project, as the list draws it.
#[derive(Serialize, Debug, Default)]
pub struct RepoCard {
    pub path: String,
    pub name: String,
    /// The branch it is on, or the short id when HEAD is detached.
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    /// Paths git reports as changed, staged and unstaged and untracked alike.
    pub dirty: usize,
    /// When it was last committed to, as a unix time; zero when unknown.
    pub last_commit: i64,
    /// False when the folder is not where it was, so the row can say so
    /// instead of the list quietly getting shorter.
    pub exists: bool,
}

/// The numbers along the top, and the grid under them.
#[derive(Serialize, Debug, Default)]
pub struct HomeStats {
    /// Commits a day, oldest first, `DAYS` long. The last entry is today.
    pub days: Vec<u32>,
    pub week: u32,
    /// Last week's, so this week's number has something to be bigger than.
    pub previous_week: u32,
    /// Days in a row up to today with at least one commit.
    pub streak: u32,
    pub best_streak: u32,
    /// How many of the commits read went into the answer above.
    pub read: u32,
    pub added: u64,
    pub removed: u64,
    pub repos_this_week: u32,
    /// The word most of your subjects start with, and how often. Empty when
    /// there is nothing to count.
    pub favourite_word: String,
    pub favourite_count: u32,
}

#[derive(Serialize, Debug, Default)]
pub struct HomeSummary {
    pub repos: Vec<RepoCard>,
    pub stats: HomeStats,
    /// The address the commits were counted for, so the page can say whose
    /// year it is drawing.
    pub author: Option<String>,
}

/// Everything the home tab draws, read in one go.
pub fn summary(state: &AppState) -> Result<HomeSummary, String> {
    let config = state.config();
    let profile = config.active();
    let author = profile
        .and_then(|one| one.git_email.clone())
        .filter(|email| !email.trim().is_empty());

    // Tabs first, then whatever else was opened before them; each path once.
    let mut paths: Vec<(String, String)> = Vec::new();
    if let Some(profile) = profile {
        for project in profile.projects.iter().chain(profile.recents.iter()) {
            if !paths.iter().any(|(path, _)| path == &project.path) {
                paths.push((project.path.clone(), project.name.clone()));
            }
        }
    }
    paths.truncate(MAX_REPOS);

    let mut repos = Vec::with_capacity(paths.len());
    let mut days = vec![0u32; DAYS];
    let mut words: HashMap<String, u32> = HashMap::new();
    let mut read = 0u32;
    let mut added = 0u64;
    let mut removed = 0u64;
    let mut repos_this_week = 0u32;
    let today = days_since_epoch(now());

    for (path, name) in paths {
        let root = std::path::PathBuf::from(&path);
        if !root.join(".git").exists() {
            repos.push(RepoCard {
                path,
                name,
                exists: false,
                ..RepoCard::default()
            });
            continue;
        }

        let mut card = card_for(&root, &path, &name);

        let mut this_week = 0u32;
        for commit in read_year(&root, author.as_deref()) {
            read += 1;
            let at = DAYS as i64 - 1 - (today - commit.day);
            if (0..DAYS as i64).contains(&at) {
                days[at as usize] += 1;
            }
            if today - commit.day < 7 {
                this_week += 1;
            }
            if let Some(word) = first_word(&commit.subject) {
                *words.entry(word).or_default() += 1;
            }
            card.last_commit = card.last_commit.max(commit.at);
        }
        if this_week > 0 {
            repos_this_week += 1;
        }
        // A repository you have never committed to yourself still has a date
        // worth showing: whoever did commit to it last.
        if card.last_commit == 0 {
            card.last_commit = git_cmd::run_checked(&root, &["log", "-1", "--format=%ct"])
                .ok()
                .and_then(|out| out.trim().parse().ok())
                .unwrap_or(0);
        }
        let (plus, minus) = read_week_lines(&root, author.as_deref());
        added += plus;
        removed += minus;

        repos.push(card);
    }

    let week: u32 = days.iter().rev().take(7).sum();
    let previous_week: u32 = days.iter().rev().skip(7).take(7).sum();
    let (favourite_word, favourite_count) = words
        .into_iter()
        .max_by_key(|(word, count)| (*count, word.clone()))
        .unwrap_or_default();

    Ok(HomeSummary {
        repos,
        stats: HomeStats {
            streak: streak(&days),
            best_streak: best_streak(&days),
            days,
            week,
            previous_week,
            read,
            added,
            removed,
            repos_this_week,
            favourite_word,
            favourite_count,
        },
        author,
    })
}

/// The state of one repository, in the three cheapest reads that say it.
fn card_for(root: &std::path::Path, path: &str, name: &str) -> RepoCard {
    let branch = git_cmd::run_checked(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map(|out| out.trim().to_string())
        .or_else(|_| {
            git_cmd::run_checked(root, &["rev-parse", "--short", "HEAD"])
                .map(|out| format!("detached at {}", out.trim()))
        })
        .unwrap_or_default();

    let dirty = git_cmd::run_checked(root, &["status", "--porcelain"])
        .map(|out| out.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0);

    // One command for both counts. A branch with no upstream answers with an
    // error, which is not one: it simply has nothing to be ahead of.
    let (ahead, behind) = git_cmd::run_checked(
        root,
        &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
    )
    .ok()
    .and_then(|out| {
        let mut parts = out.split_whitespace();
        let behind = parts.next()?.parse().ok()?;
        let ahead = parts.next()?.parse().ok()?;
        Some((ahead, behind))
    })
    .unwrap_or((0, 0));

    RepoCard {
        path: path.to_string(),
        name: name.to_string(),
        branch,
        ahead,
        behind,
        dirty,
        last_commit: 0,
        exists: true,
    }
}

/// One commit, in the three things the grid and the tiles need from it.
struct Commit {
    /// Days since the epoch, in the machine's own timezone.
    day: i64,
    /// The time itself, for the "last touched" a list of dates wants.
    at: i64,
    subject: String,
}

/// A year of commits from one repository, yours where the profile has an
/// address to filter by.
fn read_year(root: &std::path::Path, author: Option<&str>) -> Vec<Commit> {
    let since = format!("--since={} days ago", DAYS);
    let mut args = vec![
        "log",
        "--no-merges",
        "--no-show-signature",
        &since,
        // Local time, so a commit made at eleven at night counts for the day
        // it felt like rather than for the one UTC says it was.
        "--date=format-local:%s",
        "--format=%ad%x1f%s",
    ];
    let filter;
    if let Some(email) = author {
        filter = format!("--author={email}");
        args.push(&filter);
    }
    // `HEAD` only: branches nobody has checked out are somebody else's work
    // just as often as they are yours, and counting them twice over is worse
    // than missing them.
    let Ok(raw) = git_cmd::run_checked(root, &args) else {
        return Vec::new();
    };

    raw.lines()
        .filter_map(|line| {
            let (stamp, subject) = line.split_once('\u{1f}')?;
            let seconds: i64 = stamp.trim().parse().ok()?;
            Some(Commit {
                day: days_since_epoch(seconds),
                at: seconds,
                subject: subject.to_string(),
            })
        })
        .collect()
}

/// Lines added and removed in the last week, from git's own summary of them.
fn read_week_lines(root: &std::path::Path, author: Option<&str>) -> (u64, u64) {
    let mut args = vec![
        "log",
        "--no-merges",
        "--since=7 days ago",
        "--shortstat",
        "--format=",
    ];
    let filter;
    if let Some(email) = author {
        filter = format!("--author={email}");
        args.push(&filter);
    }
    let Ok(raw) = git_cmd::run_checked(root, &args) else {
        return (0, 0);
    };

    let mut added = 0u64;
    let mut removed = 0u64;
    for line in raw.lines() {
        // " 3 files changed, 12 insertions(+), 4 deletions(-)"
        let mut count = 0u64;
        for word in line.split_whitespace() {
            if let Ok(number) = word.parse::<u64>() {
                count = number;
            } else if word.starts_with("insertion") {
                added += count;
            } else if word.starts_with("deletion") {
                removed += count;
            }
        }
    }
    (added, removed)
}

/// The first word of a subject, lowercased, skipping the ones that say nothing
/// about what the commit did.
fn first_word(subject: &str) -> Option<String> {
    let word: String = subject
        .split(|c: char| c.is_whitespace() || c == ':' || c == '(')
        .find(|part| !part.is_empty())?
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect();
    (word.len() > 2).then_some(word)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

fn days_since_epoch(seconds: i64) -> i64 {
    seconds.div_euclid(86_400)
}

/// Days in a row with a commit, counting back from today. Today itself not
/// having one yet does not break it: the streak is measured from yesterday.
fn streak(days: &[u32]) -> u32 {
    let mut count = 0;
    for (at, day) in days.iter().enumerate().rev() {
        if *day > 0 {
            count += 1;
        } else if at != days.len() - 1 {
            break;
        }
    }
    count
}

fn best_streak(days: &[u32]) -> u32 {
    let mut best = 0;
    let mut running = 0;
    for day in days {
        if *day > 0 {
            running += 1;
            best = best.max(running);
        } else {
            running = 0;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_streak_runs_back_from_today() {
        // …and an empty today does not end it: the day is not over.
        assert_eq!(streak(&[1, 1, 0, 1, 1, 1, 0]), 3);
        assert_eq!(streak(&[1, 1, 0, 1, 1, 1, 1]), 4);
        assert_eq!(streak(&[0, 0, 0]), 0);
    }

    #[test]
    fn the_best_streak_is_the_longest_run_anywhere() {
        assert_eq!(best_streak(&[1, 1, 1, 0, 1, 1, 0, 1]), 3);
        assert_eq!(best_streak(&[0, 0]), 0);
    }

    #[test]
    fn a_subject_gives_up_its_first_real_word() {
        assert_eq!(
            first_word("fix(diff): a pure move"),
            Some("fix".to_string())
        );
        assert_eq!(first_word("Add the parser"), Some("add".to_string()));
        // Too short to say anything, and nothing at all.
        assert_eq!(first_word("wip"), Some("wip".to_string()));
        assert_eq!(first_word("a thing"), None);
        assert_eq!(first_word(""), None);
    }

    #[test]
    fn a_shortstat_line_is_read_as_lines_moved() {
        let (added, removed) = ("", "");
        let _ = (added, removed);
        assert_eq!(
            read_shortstat(" 3 files changed, 12 insertions(+), 4 deletions(-)\n"),
            (12, 4)
        );
        assert_eq!(read_shortstat(" 1 file changed, 2 insertions(+)\n"), (2, 0));
        assert_eq!(read_shortstat(""), (0, 0));
    }

    /// The parsing half of [`read_week_lines`], where a test can reach it.
    fn read_shortstat(raw: &str) -> (u64, u64) {
        let mut added = 0u64;
        let mut removed = 0u64;
        for line in raw.lines() {
            let mut count = 0u64;
            for word in line.split_whitespace() {
                if let Ok(number) = word.parse::<u64>() {
                    count = number;
                } else if word.starts_with("insertion") {
                    added += count;
                } else if word.starts_with("deletion") {
                    removed += count;
                }
            }
        }
        (added, removed)
    }
}
