use std::cell::{Cell, RefCell};

use git2::{Commit, Delta, Diff, DiffOptions, Oid, Repository, Tree};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
}

#[derive(Serialize)]
pub struct CommitDetail {
    pub oid: String,
    pub short: String,
    pub summary: String,
    pub body: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub committer: String,
    pub commit_time: i64,
    pub parents: Vec<String>,
    pub files: Vec<FileChange>,
}

#[derive(Serialize)]
pub struct DiffLine {
    /// ' ' context, '+' addition, '-' deletion.
    pub origin: char,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
    pub content: String,
}

#[derive(Serialize)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Serialize)]
pub struct FileDiff {
    pub path: String,
    pub binary: bool,
    pub hunks: Vec<DiffHunk>,
    /// Lines beyond [`MAX_DIFF_LINES`] that were not collected. Zero for the
    /// diffs anyone reads; large for a generated file nobody does.
    pub truncated: usize,
}

/// How many diff lines are worth sending to the window.
///
/// A regenerated lockfile is a hundred thousand lines that no one is going to
/// read, and collecting them costs twice: once building the JSON here, and
/// again laying out a DOM node per line there. Ten thousand is past any diff a
/// person reviews by eye and still renders without a pause.
const MAX_DIFF_LINES: usize = 10_000;

/// Which side of the index a working-tree diff should describe.
#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Staged,
    Unstaged,
}

pub fn commit_detail(state: &AppState, oid: &str) -> Result<CommitDetail, String> {
    let repo = state.repo()?;
    let oid = parse_oid(oid)?;
    let commit = repo.find_commit(oid).map_err(err)?;
    let diff = commit_diff(&repo, &commit, None)?;

    let author = commit.author();
    let committer = commit.committer();
    let message = commit.message().unwrap_or("");
    let summary = commit.summary().unwrap_or("").to_string();
    // Everything after the summary line, minus the blank line that separates it.
    let body = message
        .strip_prefix(&summary)
        .unwrap_or("")
        .trim_start_matches('\n')
        .trim_end()
        .to_string();

    Ok(CommitDetail {
        oid: oid.to_string(),
        short: oid.to_string()[..7].to_string(),
        summary,
        body,
        author: author.name().unwrap_or("").to_string(),
        email: author.email().unwrap_or("").to_string(),
        time: author.when().seconds(),
        committer: committer.name().unwrap_or("").to_string(),
        commit_time: committer.when().seconds(),
        parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        files: file_changes(&diff)?,
    })
}

pub fn commit_file_diff(state: &AppState, oid: &str, path: &str) -> Result<FileDiff, String> {
    let repo = state.repo()?;
    let oid = parse_oid(oid)?;
    let commit = repo.find_commit(oid).map_err(err)?;
    let diff = commit_diff(&repo, &commit, Some(path))?;
    collect_hunks(&diff, path)
}

pub fn working_file_diff(state: &AppState, path: &str, side: Side) -> Result<FileDiff, String> {
    let repo = state.repo()?;
    let mut opts = base_options();
    opts.pathspec(path);
    // Untracked files have nothing to diff against, so show them whole. Listing
    // them is not enough on its own: without the content flag the file appears
    // in the diff with no hunks, which reads as "no changes" for a file that is
    // entirely new.
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);

    let diff = match side {
        Side::Unstaged => repo.diff_index_to_workdir(None, Some(&mut opts)),
        Side::Staged => {
            let head_tree = head_tree(&repo)?;
            repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts))
        }
    }
    .map_err(err)?;

    collect_hunks(&diff, path)
}

fn commit_diff<'r>(
    repo: &'r Repository,
    commit: &Commit<'r>,
    path: Option<&str>,
) -> Result<Diff<'r>, String> {
    let mut opts = base_options();
    if let Some(path) = path {
        opts.pathspec(path);
    }
    let new_tree = commit.tree().map_err(err)?;
    // A root commit is diffed against nothing, which shows it as all additions.
    let old_tree = match commit.parent(0) {
        Ok(parent) => Some(parent.tree().map_err(err)?),
        Err(_) => None,
    };
    let mut diff = repo
        .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut opts))
        .map_err(err)?;
    let _ = diff.find_similar(None);
    Ok(diff)
}

fn base_options() -> DiffOptions {
    let mut opts = DiffOptions::new();
    opts.context_lines(3).indent_heuristic(true);
    opts
}

fn head_tree(repo: &Repository) -> Result<Option<Tree<'_>>, String> {
    match repo.head() {
        Ok(head) => {
            let commit = head.peel_to_commit().map_err(err)?;
            Ok(Some(commit.tree().map_err(err)?))
        }
        // No commits yet: the index is compared against an empty tree.
        Err(_) => Ok(None),
    }
}

fn file_changes(diff: &Diff) -> Result<Vec<FileChange>, String> {
    // `foreach` hands out one closure per callback and each wants the same list,
    // so ownership lives in a RefCell rather than in the closures.
    let files: RefCell<Vec<FileChange>> = RefCell::new(Vec::new());

    diff.foreach(
        &mut |delta, _| {
            let new_path = delta
                .new_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned());
            let old_path = delta
                .old_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned());
            files.borrow_mut().push(FileChange {
                path: new_path.clone().or_else(|| old_path.clone()).unwrap_or_default(),
                old_path: old_path.filter(|p| Some(p) != new_path.as_ref()),
                status: status_name(delta.status()).to_string(),
                additions: 0,
                deletions: 0,
                binary: delta.new_file().is_binary() || delta.old_file().is_binary(),
            });
            true
        },
        None,
        None,
        Some(&mut |_, _, line| {
            // Deltas arrive in order, so the last pushed entry owns these lines.
            if let Some(file) = files.borrow_mut().last_mut() {
                match line.origin() {
                    '+' => file.additions += 1,
                    '-' => file.deletions += 1,
                    _ => {}
                }
            }
            true
        }),
    )
    .map_err(err)?;

    Ok(files.into_inner())
}

fn collect_hunks(diff: &Diff, path: &str) -> Result<FileDiff, String> {
    let binary = Cell::new(false);
    let hunks: RefCell<Vec<DiffHunk>> = RefCell::new(Vec::new());
    let taken = Cell::new(0usize);
    let dropped = Cell::new(0usize);

    diff.foreach(
        &mut |delta, _| {
            if delta.new_file().is_binary() || delta.old_file().is_binary() {
                binary.set(true);
            }
            true
        },
        None,
        Some(&mut |_, hunk| {
            // Past the cap, stop opening hunks too, or the view ends on a run of
            // empty headers.
            if taken.get() >= MAX_DIFF_LINES {
                return true;
            }
            hunks.borrow_mut().push(DiffHunk {
                header: String::from_utf8_lossy(hunk.header()).trim_end().to_string(),
                lines: Vec::new(),
            });
            true
        }),
        Some(&mut |_, _, line| {
            if taken.get() >= MAX_DIFF_LINES {
                dropped.set(dropped.get() + 1);
                return true;
            }
            if let Some(current) = hunks.borrow_mut().last_mut() {
                current.lines.push(DiffLine {
                    origin: line.origin(),
                    old_lineno: line.old_lineno(),
                    new_lineno: line.new_lineno(),
                    content: String::from_utf8_lossy(line.content())
                        .trim_end_matches('\n')
                        .to_string(),
                });
                taken.set(taken.get() + 1);
            }
            true
        }),
    )
    .map_err(err)?;

    Ok(FileDiff {
        path: path.to_string(),
        binary: binary.get(),
        hunks: hunks.into_inner(),
        truncated: dropped.get(),
    })
}

fn status_name(status: Delta) -> &'static str {
    match status {
        Delta::Added => "added",
        Delta::Deleted => "deleted",
        Delta::Modified => "modified",
        Delta::Renamed => "renamed",
        Delta::Copied => "copied",
        Delta::Typechange => "typechange",
        Delta::Untracked => "untracked",
        _ => "modified",
    }
}

fn parse_oid(s: &str) -> Result<Oid, String> {
    Oid::from_str(s).map_err(|_| format!("Not a valid object id: {s}"))
}

fn err(e: git2::Error) -> String {
    e.message().to_string()
}
