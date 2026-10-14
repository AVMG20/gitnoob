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
    /// Whether it has a remote at all; without one, nothing is "not pushed".
    pub has_remote: bool,
    /// Local branches that track nothing and hold commits no remote has —
    /// work that exists on this disk and nowhere else. HEAD included.
    pub unpushed: Vec<BranchCount>,
    /// The trunk, when it is behind what it tracks.
    pub trunk: Option<String>,
    pub trunk_behind: usize,
    /// Stashes older than `OLD_STASH_DAYS`, and the age of the oldest in days.
    pub old_stashes: usize,
    pub oldest_stash_days: u32,
    /// Branches untouched for `STALE_DAYS` whose upstream has been deleted:
    /// usually merged and forgotten.
    pub gone: Vec<String>,
}

/// A branch and how many commits it has that matter for what is asked.
#[derive(Serialize, Debug, Default, Clone, PartialEq)]
pub struct BranchCount {
    pub name: String,
    pub count: usize,
}

/// A stash this old has usually been forgotten rather than kept.
const OLD_STASH_DAYS: i64 = 14;
/// A branch untouched this long with its upstream gone is most likely done.
const STALE_DAYS: i64 = 30;
/// Enough branches to ask about without a repository of hundreds stalling the
/// page; the rest are still on the project's own sidebar.
const MAX_BRANCHES: usize = 30;

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
    /// year it is drawing. The profile's, or the one git commits with when the
    /// profile names none.
    pub author: Option<String>,
    /// Projects whose commits could not be counted because there is no
    /// address to tell yours from everybody else's.
    pub uncounted: Vec<String>,
}

/// Whose commits to count in one repository: the profile's address, else the
/// one git would commit with there. `None` means nobody can be told apart,
/// and then nothing is counted rather than everybody.
pub fn author_for(profile_email: Option<&str>, configured: Option<&str>) -> Option<String> {
    profile_email
        .or(configured)
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .map(str::to_string)
}

/// Everything the home tab draws, read in one go.
pub fn summary(state: &AppState) -> Result<HomeSummary, String> {
    let config = state.config();
    let profile = config.active();
    let mut uncounted: Vec<String> = Vec::new();
    let mut shown_author: Option<String> = None;

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

        // Whose commits these are: the address of the profile this project
        // answers to, else what git commits with here. With neither, the year
        // would be everybody's work presented as yours.
        let profile_email = config
            .profile_for(&path)
            .and_then(|one| one.git_email.clone());
        let configured = git_cmd::run_checked(&root, &["config", "--get", "user.email"]).ok();
        let Some(author) = author_for(profile_email.as_deref(), configured.as_deref()) else {
            uncounted.push(name.clone());
            card.last_commit = git_cmd::run_checked(&root, &["log", "-1", "--format=%ct"])
                .ok()
                .and_then(|out| out.trim().parse().ok())
                .unwrap_or(0);
            repos.push(card);
            continue;
        };
        if shown_author.is_none() {
            shown_author = Some(author.clone());
        }
        let author = Some(author);

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
        author: shown_author,
        uncounted,
    })
}

/// The state of one repository: the branch, the counts, and what has been
/// left lying about in it.
pub fn card_for(root: &std::path::Path, path: &str, name: &str) -> RepoCard {
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
    // error, which is not one: it has nothing to be ahead of, and what it has
    // that no remote does is counted below instead.
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

    let has_remote = git_cmd::run_checked(root, &["remote"])
        .map(|out| out.lines().any(|line| !line.trim().is_empty()))
        .unwrap_or(false);

    let mut card = RepoCard {
        path: path.to_string(),
        name: name.to_string(),
        branch,
        ahead,
        behind,
        dirty,
        last_commit: 0,
        exists: true,
        has_remote,
        ..RepoCard::default()
    };
    survey_branches(root, &mut card);
    survey_stashes(root, &mut card);
    card
}

/// One local branch as `for-each-ref` describes it.
#[derive(Debug, PartialEq)]
pub struct BranchLine {
    pub name: String,
    /// The full ref it is configured to track; empty when nothing.
    pub upstream: String,
    /// `[gone]`, `[ahead 1, behind 2]`, or empty.
    pub track: String,
    /// When its tip was committed, unix seconds.
    pub at: i64,
}

/// Reads `for-each-ref` lines written with `BRANCH_FORMAT`.
pub fn parse_branches(raw: &str) -> Vec<BranchLine> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let name = parts.next()?.to_string();
            if name.is_empty() {
                return None;
            }
            Some(BranchLine {
                name,
                upstream: parts.next().unwrap_or("").to_string(),
                track: parts.next().unwrap_or("").to_string(),
                at: parts
                    .next()
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0),
            })
        })
        .collect()
}

/// How far behind a `%(upstream:track)` says a branch is.
pub fn behind_in(track: &str) -> usize {
    track
        .trim_matches(['[', ']'])
        .split(", ")
        .find_map(|part| part.strip_prefix("behind "))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0)
}

const BRANCH_FORMAT: &str =
    "--format=%(refname:lstrip=2)%1f%(upstream)%1f%(upstream:track)%1f%(committerdate:unix)";

/// Never-pushed work, a trunk behind, and branches whose upstream is gone.
fn survey_branches(root: &std::path::Path, card: &mut RepoCard) {
    let Ok(raw) = git_cmd::run_checked(root, &["for-each-ref", BRANCH_FORMAT, "refs/heads"]) else {
        return;
    };
    let branches = parse_branches(&raw);
    let now = now();

    let trunk = git_cmd::run_checked(root, &["config", "--get", "gitnoob.trunk"])
        .ok()
        .map(|out| out.trim().to_string())
        .filter(|name| branches.iter().any(|b| &b.name == name))
        .or_else(|| {
            ["main", "master"]
                .iter()
                .find(|name| branches.iter().any(|b| b.name == **name))
                .map(|name| name.to_string())
        });
    if let Some(trunk) = &trunk {
        if let Some(line) = branches.iter().find(|b| &b.name == trunk) {
            card.trunk_behind = behind_in(&line.track);
            card.trunk = Some(trunk.clone());
        }
    }

    for line in branches.iter().take(MAX_BRANCHES) {
        if line.upstream.is_empty() {
            // Only where there is somewhere to push to: a repository with no
            // remote keeps everything on this disk by choice.
            if !card.has_remote {
                continue;
            }
            let tip = format!("refs/heads/{}", line.name);
            let count = git_cmd::run_checked(
                root,
                &[
                    "log",
                    "--format=%h",
                    "--max-count=1000",
                    &tip,
                    "--not",
                    "--remotes",
                ],
            )
            .map(|out| out.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
            if count > 0 {
                card.unpushed.push(BranchCount {
                    name: line.name.clone(),
                    count,
                });
            }
        } else if line.track.contains("gone")
            && line.at > 0
            && now - line.at > STALE_DAYS * 86_400
            && Some(&line.name) != trunk.as_ref()
        {
            card.gone.push(line.name.clone());
        }
    }
}

/// Stashes old enough to have been forgotten.
fn survey_stashes(root: &std::path::Path, card: &mut RepoCard) {
    let Ok(raw) = git_cmd::run_checked(root, &["stash", "list", "--format=%ct"]) else {
        return;
    };
    let now = now();
    for stamp in raw
        .lines()
        .filter_map(|line| line.trim().parse::<i64>().ok())
    {
        let days = (now - stamp) / 86_400;
        if days >= OLD_STASH_DAYS {
            card.old_stashes += 1;
            card.oldest_stash_days = card.oldest_stash_days.max(days as u32);
        }
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
        filter = format!("--author=<{email}>");
        args.push(&filter);
    }
    // Every local branch, each commit once: work on a branch not checked out
    // right now is still work done this year. The author filter is what keeps
    // somebody else's branches out, and `-F` with the angle brackets makes it
    // the whole address rather than anything containing it.
    args.push("--fixed-strings");
    args.push("--branches");
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
        filter = format!("--author=<{email}>");
        args.push(&filter);
    }
    args.push("--fixed-strings");
    args.push("--branches");
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
