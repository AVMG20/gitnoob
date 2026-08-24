use serde::Serialize;

use crate::git_cmd;
use crate::journal::{self, Mode};
use crate::state::AppState;

#[derive(Serialize)]
pub struct AmendDraft {
    pub summary: String,
    pub body: String,
    /// True when the commit being amended has already been pushed, which makes
    /// the amend a history rewrite that needs a force push to publish.
    pub is_pushed: bool,
    pub short: String,
}

pub fn stage(state: &AppState, paths: &[String]) -> Result<String, String> {
    let root = state.path()?;
    let mut args = vec!["add", "--"];
    let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    args.extend(refs);
    git_cmd::run_checked(&root, &args)
}

pub fn stage_all(state: &AppState) -> Result<String, String> {
    let root = state.path()?;
    git_cmd::run_checked(&root, &["add", "--all"])
}

pub fn unstage(state: &AppState, paths: &[String]) -> Result<String, String> {
    let root = state.path()?;
    let mut args = vec!["restore", "--staged", "--"];
    let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    args.extend(refs);
    git_cmd::run_checked(&root, &args)
}

/// Throws away working-tree changes to the given paths. Untracked files are left
/// alone: deleting a file the user never committed is not something to do as a
/// side effect of "discard changes".
pub fn discard(state: &AppState, paths: &[String]) -> Result<String, String> {
    let root = state.path()?;
    let mut args = vec!["restore", "--worktree", "--"];
    let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    args.extend(refs);
    git_cmd::run_checked(&root, &args)
}

pub fn commit(state: &AppState, message: &str, amend: bool) -> Result<String, String> {
    let root = state.path()?;
    if message.trim().is_empty() {
        return Err("A commit needs a message".to_string());
    }
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    let mut args = vec!["commit", "-m", message];
    if amend {
        args.push("--amend");
    }
    let output = git_cmd::run_checked(&root, &args)?;

    // Undo is a soft reset back to `before`: for an amend that is the commit
    // that was replaced, which still exists as an object.
    let summary = message.lines().next().unwrap_or("").trim().to_string();
    journal::record(
        state,
        if amend { "amend" } else { "commit" },
        format!("{}: {summary}", if amend { "Amend" } else { "Commit" }),
        branch,
        before,
        journal::head_oid(state),
        Mode::Soft,
        false,
    );
    Ok(output)
}

/// Loads HEAD's message so the amend dialog can start from it, and reports
/// whether amending would rewrite published history.
pub fn amend_draft(state: &AppState) -> Result<AmendDraft, String> {
    let repo = state.repo()?;
    let head = repo
        .head()
        .map_err(|_| "There is no commit to amend".to_string())?;
    let commit = head.peel_to_commit().map_err(|e| e.message().to_string())?;
    let oid = commit.id();

    let message = commit.message().unwrap_or("");
    let summary = commit.summary().unwrap_or("").to_string();
    let body = message
        .strip_prefix(&summary)
        .unwrap_or("")
        .trim_start_matches('\n')
        .trim_end()
        .to_string();

    // If any remote-tracking branch contains this commit, it is already
    // published as far as this clone can tell.
    let mut is_pushed = false;
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Remote)) {
        for branch in branches.flatten() {
            if let Some(target) = branch.0.get().target() {
                if target == oid || repo.graph_descendant_of(target, oid).unwrap_or(false) {
                    is_pushed = true;
                    break;
                }
            }
        }
    }

    Ok(AmendDraft {
        summary,
        body,
        is_pushed,
        short: oid.to_string()[..7].to_string(),
    })
}

pub fn stash_push(
    state: &AppState,
    message: Option<&str>,
    include_untracked: bool,
) -> Result<String, String> {
    let root = state.path()?;
    let mut args = vec!["stash", "push"];
    if include_untracked {
        args.push("--include-untracked");
    }
    if let Some(message) = message {
        args.push("-m");
        args.push(message);
    }
    let output = git_cmd::run_checked(&root, &args)?;
    journal::record(
        state,
        "stash",
        format!("Stash: {}", message.unwrap_or("uncommitted changes")),
        journal::current_branch(state),
        None,
        None,
        Mode::Stash,
        true,
    );
    Ok(output)
}

pub fn stash_pop(state: &AppState, index: usize) -> Result<String, String> {
    let root = state.path()?;
    let name = format!("stash@{{{index}}}");
    git_cmd::run_checked(&root, &["stash", "pop", &name])
}

// --- stash -----------------------------------------------------------------

/// One stash entry, with enough detail to tell two of them apart.
#[derive(Serialize)]
pub struct StashEntry {
    pub index: usize,
    pub oid: String,
    pub message: String,
    /// The branch that was checked out when it was made.
    pub branch: Option<String>,
    pub time: i64,
    pub files: usize,
}

pub fn stash_list(state: &AppState) -> Result<Vec<StashEntry>, String> {
    let root = state.path()?;
    // A record per line: index, hash, subject, branch, unix time.
    let raw = git_cmd::run_checked(
        &root,
        &[
            "stash",
            "list",
            "--format=%gd%x00%H%x00%gs%x00%at",
        ],
    )?;

    let mut out = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let mut parts = line.split('\0');
        let _selector = parts.next().unwrap_or("");
        let oid = parts.next().unwrap_or("").to_string();
        let subject = parts.next().unwrap_or("").to_string();
        let time = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);

        // `git stash list` subjects read "WIP on main: 1234abc Message" or
        // "On main: my message"; pull the branch out and keep the rest.
        let (branch, message) = split_subject(&subject);
        let files = git_cmd::run_checked(&root, &["stash", "show", "--name-only", &format!("stash@{{{index}}}")])
            .map(|text| text.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);

        out.push(StashEntry {
            index,
            oid,
            message,
            branch,
            time,
            files,
        });
    }
    Ok(out)
}

fn split_subject(subject: &str) -> (Option<String>, String) {
    let trimmed = subject
        .strip_prefix("WIP on ")
        .or_else(|| subject.strip_prefix("On "))
        .unwrap_or(subject);
    match trimmed.split_once(": ") {
        Some((branch, rest)) => (Some(branch.to_string()), rest.to_string()),
        None => (None, trimmed.to_string()),
    }
}

/// The commit a stash entry points at, so its diff can be shown like any other.
pub fn stash_oid(state: &AppState, index: usize) -> Result<String, String> {
    let root = state.path()?;
    let name = format!("stash@{{{index}}}");
    Ok(git_cmd::run_checked(&root, &["rev-parse", &name])?
        .trim()
        .to_string())
}

/// Applies a stash and keeps it in the list.
pub fn stash_apply(state: &AppState, index: usize) -> Result<String, String> {
    let root = state.path()?;
    let name = format!("stash@{{{index}}}");
    git_cmd::run_checked(&root, &["stash", "apply", &name])?;
    Ok(format!("Applied {name}"))
}

pub fn stash_drop(state: &AppState, index: usize) -> Result<String, String> {
    let root = state.path()?;
    let name = format!("stash@{{{index}}}");
    git_cmd::run_checked(&root, &["stash", "drop", &name])?;
    Ok(format!("Dropped {name}"))
}

/// Turns a stash into a branch, which is the safe way out of a stash that no
/// longer applies cleanly to the current branch.
pub fn stash_branch(state: &AppState, index: usize, name: &str) -> Result<String, String> {
    let root = state.path()?;
    let selector = format!("stash@{{{index}}}");
    git_cmd::run_checked(&root, &["stash", "branch", name, &selector])?;
    Ok(format!("Created {name} from {selector}"))
}

// --- auto-stash ------------------------------------------------------------

/// Marker returned by [`stash_before`], to be handed back to [`restore_after`].
pub struct Held {
    pub stashed: bool,
}

fn is_dirty(state: &AppState) -> bool {
    state
        .path()
        .ok()
        .and_then(|root| git_cmd::run_checked(&root, &["status", "--porcelain"]).ok())
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false)
}

/// Stashes uncommitted work so an operation that needs a clean tree can run.
///
/// This is the trick that makes switching branches with local edits feel
/// ordinary: stash, switch, put the edits back.
pub fn stash_before(state: &AppState, reason: &str) -> Result<Held, String> {
    if !state.config().global.auto_stash || !is_dirty(state) {
        return Ok(Held { stashed: false });
    }
    let root = state.path()?;
    let message = format!("gitui auto-stash: {reason}");
    git_cmd::run_checked(
        &root,
        &["stash", "push", "--include-untracked", "-m", &message],
    )?;
    Ok(Held { stashed: true })
}

/// Puts auto-stashed work back. A conflict here is reported, not swallowed: the
/// changes are still in the stash and the user needs to know that.
pub fn restore_after(state: &AppState, held: Held) -> Result<Option<String>, String> {
    if !held.stashed {
        return Ok(None);
    }
    let root = state.path()?;
    match git_cmd::run_checked(&root, &["stash", "pop"]) {
        Ok(_) => Ok(Some("Local changes were stashed and put back".to_string())),
        Err(error) => Err(format!(
            "Your changes are safe in the stash, but putting them back hit a problem: {error}"
        )),
    }
}

// --- moving a branch and replaying commits ---------------------------------

/// How much a reset touches.
#[derive(serde::Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResetMode {
    /// Move the branch; leave the index and working tree alone. Changes end up
    /// staged.
    Soft,
    /// Move the branch and the index. Changes end up unstaged.
    Mixed,
    /// Move everything. Uncommitted work is destroyed.
    Hard,
}

impl ResetMode {
    fn flag(&self) -> &'static str {
        match self {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        }
    }
}

/// What a reset would do, so the UI can say it before it happens.
#[derive(Serialize)]
pub struct ResetPreview {
    pub target: String,
    pub short: String,
    pub summary: String,
    pub branch: Option<String>,
    /// Commits that would no longer be on the branch.
    pub dropped: Vec<crate::remote::CommitSummary>,
    /// True when the branch is not an ancestor, i.e. this moves sideways.
    pub diverges: bool,
    pub staged_files: usize,
    pub unstaged_files: usize,
}

pub fn reset_preview(state: &AppState, oid: &str) -> Result<ResetPreview, String> {
    let root = state.path()?;
    let branch = journal::current_branch(state);
    let head = journal::head_oid(state);

    let summary = git_cmd::run_checked(&root, &["log", "-1", "--format=%s", oid])?
        .trim()
        .to_string();

    // Commits on the branch now that would not be on it afterwards.
    let raw = git_cmd::run_checked(
        &root,
        &["log", "--format=%H%x00%h%x00%s%x00%an%x00%at", &format!("{oid}..HEAD")],
    )
    .unwrap_or_default();
    let dropped: Vec<crate::remote::CommitSummary> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split('\0');
            crate::remote::CommitSummary {
                oid: parts.next().unwrap_or("").to_string(),
                short: parts.next().unwrap_or("").to_string(),
                summary: parts.next().unwrap_or("").to_string(),
                author: parts.next().unwrap_or("").to_string(),
                time: parts.next().unwrap_or("0").parse().unwrap_or(0),
            }
        })
        .collect();

    // If the target is not an ancestor of HEAD, this is not a plain rewind.
    let diverges = head
        .as_deref()
        .map(|head| {
            git_cmd::run_checked(&root, &["merge-base", "--is-ancestor", oid, head]).is_err()
        })
        .unwrap_or(false);

    let status = crate::refs::status(state)?;

    Ok(ResetPreview {
        target: oid.to_string(),
        short: oid.chars().take(7).collect(),
        summary,
        branch,
        dropped,
        diverges,
        staged_files: status.staged.len(),
        unstaged_files: status.unstaged.len(),
    })
}

/// Moves the current branch to a commit.
pub fn reset(state: &AppState, oid: &str, mode: ResetMode) -> Result<String, String> {
    let root = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    git_cmd::run_checked(&root, &["reset", mode.flag(), oid])?;

    let short: String = oid.chars().take(7).collect();
    journal::record(
        state,
        "reset",
        format!("Reset to {short}"),
        branch,
        before,
        journal::head_oid(state),
        // Undoing a hard reset has to put the working tree back too.
        if mode == ResetMode::Hard {
            Mode::Hard
        } else {
            Mode::Soft
        },
        mode == ResetMode::Hard,
    );
    Ok(format!("Branch moved to {short}"))
}

/// Copies a commit onto the current branch.
pub fn cherry_pick(state: &AppState, oid: &str) -> Result<String, String> {
    let root = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    let out = git_cmd::run(&root, &["cherry-pick", oid])?;
    if !out.ok {
        let conflicts = crate::conflict::list(state).unwrap_or_default();
        if conflicts.is_empty() {
            return Err(if out.stderr.trim().is_empty() {
                out.stdout
            } else {
                out.stderr
            });
        }
        return Ok(format!(
            "Cherry-pick stopped with conflicts in {}",
            conflicts.join(", ")
        ));
    }

    let short: String = oid.chars().take(7).collect();
    journal::record(
        state,
        "cherry-pick",
        format!("Cherry-pick {short}"),
        branch,
        before,
        journal::head_oid(state),
        Mode::Hard,
        true,
    );
    Ok(format!("Cherry-picked {short}"))
}

/// Adds a commit that undoes an earlier one, leaving history intact.
pub fn revert(state: &AppState, oid: &str) -> Result<String, String> {
    let root = state.path()?;
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);

    let out = git_cmd::run(&root, &["revert", "--no-edit", oid])?;
    if !out.ok {
        let conflicts = crate::conflict::list(state).unwrap_or_default();
        if conflicts.is_empty() {
            return Err(if out.stderr.trim().is_empty() {
                out.stdout
            } else {
                out.stderr
            });
        }
        return Ok(format!(
            "Revert stopped with conflicts in {}",
            conflicts.join(", ")
        ));
    }

    let short: String = oid.chars().take(7).collect();
    journal::record(
        state,
        "revert",
        format!("Revert {short}"),
        branch,
        before,
        journal::head_oid(state),
        Mode::Hard,
        true,
    );
    Ok(format!("Reverted {short}"))
}

/// Creates a tag, annotated when a message is given.
pub fn create_tag(
    state: &AppState,
    name: &str,
    oid: &str,
    message: Option<&str>,
) -> Result<String, String> {
    let root = state.path()?;
    let mut args = vec!["tag"];
    if let Some(message) = message.filter(|m| !m.trim().is_empty()) {
        args.extend(["-a", name, "-m", message]);
    } else {
        args.push(name);
    }
    args.push(oid);
    git_cmd::run_checked(&root, &args)?;
    Ok(format!("Tagged {} as {name}", &oid[..oid.len().min(7)]))
}

pub fn delete_tag(state: &AppState, name: &str) -> Result<String, String> {
    let root = state.path()?;
    git_cmd::run_checked(&root, &["tag", "-d", name])?;
    Ok(format!("Deleted tag {name}"))
}

/// Full message of one commit, for the copy-to-clipboard actions.
pub fn commit_message_text(state: &AppState, oid: &str) -> Result<String, String> {
    let root = state.path()?;
    Ok(git_cmd::run_checked(&root, &["log", "-1", "--format=%B", oid])?
        .trim_end()
        .to_string())
}

/// The full patch for a commit, for the "copy patch" action.
pub fn commit_patch(state: &AppState, oid: &str) -> Result<String, String> {
    let root = state.path()?;
    git_cmd::run_checked(&root, &["show", "--no-color", "--patch", oid])
}

/// Hands a path to the desktop's file manager.
pub fn reveal(state: &AppState, relative: &str) -> Result<(), String> {
    let root = state.path()?;
    let target = root.join(relative);
    if !target.exists() {
        return Err(format!("{} is not there any more", target.display()));
    }
    let (program, args): (&str, Vec<String>) = if cfg!(target_os = "macos") {
        ("open", vec!["-R".into(), target.to_string_lossy().into_owned()])
    } else if cfg!(target_os = "windows") {
        ("explorer", vec![format!("/select,{}", target.display())])
    } else {
        // No portable "reveal" on Linux; open the containing directory.
        let dir = target.parent().unwrap_or(&root);
        ("xdg-open", vec![dir.to_string_lossy().into_owned()])
    };
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not reveal {}: {e}", target.display()))
}

// --- hunk-level staging ----------------------------------------------------

/// What to do with a single hunk.
#[derive(serde::Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HunkAction {
    /// Move it into the index.
    Stage,
    /// Take it back out of the index.
    Unstage,
    /// Throw it away in the working tree.
    Discard,
}

/// Applies one hunk of a file's diff.
///
/// Rather than reimplementing patch arithmetic, this asks git for the same diff
/// the UI is showing, keeps the file header and the one hunk the user picked,
/// and feeds that back to `git apply`. The index and the working tree are then
/// changed by git itself, which is the only way to be sure the result matches
/// what a command-line `git add -p` would have produced.
pub fn apply_hunk(
    state: &AppState,
    path: &str,
    hunk_index: usize,
    action: HunkAction,
) -> Result<String, String> {
    let root = state.path()?;

    // Unstaging reads the index-versus-HEAD diff; the other two read the
    // working tree against the index.
    let diff = if action == HunkAction::Unstage {
        git_cmd::run_checked(
            &root,
            &["diff", "--cached", "--no-color", "--unified=3", "--", path],
        )?
    } else {
        git_cmd::run_checked(&root, &["diff", "--no-color", "--unified=3", "--", path])?
    };

    if diff.trim().is_empty() {
        return Err(format!(
            "No {} changes left in {path}",
            if action == HunkAction::Unstage {
                "staged"
            } else {
                "unstaged"
            }
        ));
    }

    let patch = single_hunk_patch(&diff, hunk_index)?;

    let args: &[&str] = match action {
        HunkAction::Stage => &["apply", "--cached", "--whitespace=nowarn", "-"],
        HunkAction::Unstage => &["apply", "--cached", "--reverse", "--whitespace=nowarn", "-"],
        HunkAction::Discard => &["apply", "--reverse", "--whitespace=nowarn", "-"],
    };

    let out = git_cmd::run_with_input(&root, args, &patch)?;
    if !out.ok {
        let detail = if out.stderr.trim().is_empty() {
            out.stdout
        } else {
            out.stderr
        };
        return Err(format!("git could not apply that hunk: {}", detail.trim()));
    }

    Ok(match action {
        HunkAction::Stage => format!("Staged one hunk of {path}"),
        HunkAction::Unstage => format!("Unstaged one hunk of {path}"),
        HunkAction::Discard => format!("Discarded one hunk of {path}"),
    })
}

/// Rebuilds a patch containing the file header and exactly one of its hunks.
fn single_hunk_patch(diff: &str, hunk_index: usize) -> Result<String, String> {
    let mut header: Vec<&str> = Vec::new();
    let mut hunks: Vec<Vec<&str>> = Vec::new();

    for line in diff.lines() {
        if line.starts_with("@@") {
            hunks.push(vec![line]);
        } else if let Some(current) = hunks.last_mut() {
            current.push(line);
        } else {
            header.push(line);
        }
    }

    let hunk = hunks.get(hunk_index).ok_or_else(|| {
        format!(
            "That hunk is no longer there — the file has {} {}",
            hunks.len(),
            if hunks.len() == 1 { "hunk" } else { "hunks" }
        )
    })?;

    let mut patch = header.join("\n");
    patch.push('\n');
    patch.push_str(&hunk.join("\n"));
    // `git apply` insists on a trailing newline.
    patch.push('\n');
    Ok(patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/a.txt b/a.txt
index 1234567..89abcde 100644
--- a/a.txt
+++ b/a.txt
@@ -1,3 +1,4 @@
 one
+first addition
 two
 three
@@ -10,3 +11,4 @@
 ten
+second addition
 eleven
 twelve
";

    #[test]
    fn keeps_the_header_and_only_the_chosen_hunk() {
        let first = single_hunk_patch(DIFF, 0).unwrap();
        assert!(first.contains("--- a/a.txt"));
        assert!(first.contains("+++ b/a.txt"));
        assert!(first.contains("@@ -1,3 +1,4 @@"));
        assert!(first.contains("+first addition"));
        assert!(!first.contains("+second addition"));
        assert!(!first.contains("@@ -10,3 +11,4 @@"));
        assert!(first.ends_with('\n'));

        let second = single_hunk_patch(DIFF, 1).unwrap();
        assert!(second.contains("@@ -10,3 +11,4 @@"));
        assert!(second.contains("+second addition"));
        assert!(!second.contains("+first addition"));
    }

    #[test]
    fn reports_a_hunk_that_is_no_longer_there() {
        let error = single_hunk_patch(DIFF, 7).unwrap_err();
        assert!(error.contains("2 hunks"), "unexpected message: {error}");
    }
}
