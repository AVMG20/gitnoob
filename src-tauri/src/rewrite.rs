//! Rewrites that start from one commit in the graph.
//!
//! - **Fix up**: the staged changes become a `fixup!` commit for an older one,
//!   and (once the user has said yes) a rebase folds it in, by commit id.
//! - **Absorb**: every staged hunk goes to the one commit on this branch that
//!   last touched those lines, the same way, with a plan shown first.
//! - **Split**: the rebase stops at a commit with its changes unstaged, so it
//!   can be committed again in pieces.
//! - **Message**: any commit on the branch gets a new message, not only the
//!   newest.
//!
//! All of them are git's own interactive rebase underneath, so a conflict on
//! the way stops in the same place with the same strip, and `--abort` puts
//! everything back. None of them crosses a merge: the rebase runs without
//! `--rebase-merges` and would flatten it.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git_cmd;
use crate::journal::{self, Mode};
use crate::rebase;
use crate::state::AppState;

/// The file that marks a rebase as a split, holding the commit being split.
pub const SPLIT: &str = "gitnoob-split";

/// The commit a split is stopped at, while one is under way.
pub fn splitting(git_dir: &Path) -> Option<String> {
    std::fs::read_to_string(git_dir.join(SPLIT))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|oid| !oid.is_empty())
}

fn literal(path: &str) -> String {
    format!(":(literal){path}")
}

/// What a rewrite from one commit would involve, asked before anything runs so
/// the window can say no — or ask — rather than fail half-way.
#[derive(Serialize, Debug)]
pub struct RewriteCheck {
    pub oid: String,
    pub short: String,
    pub summary: String,
    /// The whole message, for a dialog that changes it.
    pub message: String,
    /// A remote already has it: rewriting means a force push afterwards.
    pub pushed: bool,
    /// The newest commit, which needs no rebase to change.
    pub is_head: bool,
    /// How many commits sit above it and get replayed.
    pub above: usize,
    /// How many files are staged, for the fixup.
    pub staged: usize,
    /// Why it cannot be done, in words for the dialog. `None` when it can.
    pub refusal: Option<String>,
}

/// The commit a rewrite starts from, worked out once.
struct Target {
    oid: String,
    short: String,
    summary: String,
    message: String,
    parent: Option<String>,
    pushed: bool,
    is_head: bool,
    above: usize,
}

fn quiet(root: &Path, args: &[&str]) -> Option<String> {
    git_cmd::run(root, args)
        .ok()
        .filter(|out| out.ok)
        .map(|out| out.stdout.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Reads the commit and refuses what cannot be rewritten from it.
fn target(state: &AppState, oid: &str) -> Result<Target, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        return Err(
            "A rebase is already part-way through. Continue or abort that one first.".into(),
        );
    }
    let progress = crate::remote::in_progress(state)?;
    if progress.merging || progress.cherry_picking || progress.reverting {
        return Err("Finish or abort what git is part-way through first.".into());
    }
    if journal::current_branch(state).is_none() {
        return Err("HEAD is detached; check out a branch first.".into());
    }
    let full = quiet(
        &root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{oid}^{{commit}}"),
        ],
    )
    .ok_or_else(|| format!("No commit named {oid}"))?;
    let head = quiet(&root, &["rev-parse", "--verify", "--quiet", "HEAD"])
        .ok_or_else(|| "There are no commits yet".to_string())?;
    let on_branch = git_cmd::run(&root, &["merge-base", "--is-ancestor", &full, &head])
        .map(|out| out.ok)
        .unwrap_or(false);
    if !on_branch {
        return Err("That commit is not on the branch you are on.".into());
    }
    let parent = quiet(
        &root,
        &["rev-parse", "--verify", "--quiet", &format!("{full}^")],
    );
    let range = match &parent {
        Some(parent) => format!("{parent}..HEAD"),
        None => "HEAD".to_string(),
    };
    if let Some(why) = rebase::merge_in_range(&root, &range) {
        return Err(why);
    }
    let above = quiet(&root, &["rev-list", "--count", &format!("{full}..HEAD")])
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let message = quiet(
        &root,
        &["log", "-1", "--no-show-signature", "--format=%B", &full],
    )
    .unwrap_or_default();
    Ok(Target {
        short: full.chars().take(7).collect(),
        summary: message.lines().next().unwrap_or("").to_string(),
        message,
        pushed: rebase::is_published(&root, &full),
        is_head: full == head,
        parent,
        above,
        oid: full,
    })
}

/// How many files are staged.
fn staged_count(root: &Path) -> usize {
    git_cmd::run(root, &["diff", "--cached", "--name-only", "-z"])
        .ok()
        .filter(|out| out.ok)
        .map(|out| out.stdout.split('\0').filter(|p| !p.is_empty()).count())
        .unwrap_or(0)
}

/// What fixing up, splitting or rewording this commit would involve.
pub fn check(state: &AppState, oid: &str) -> Result<RewriteCheck, String> {
    let root = state.path()?;
    let staged = staged_count(&root);
    Ok(match target(state, oid) {
        Ok(found) => RewriteCheck {
            oid: found.oid,
            short: found.short,
            summary: found.summary,
            message: found.message,
            pushed: found.pushed,
            is_head: found.is_head,
            above: found.above,
            staged,
            refusal: None,
        },
        Err(why) => RewriteCheck {
            oid: oid.to_string(),
            short: oid.chars().take(7).collect(),
            summary: String::new(),
            message: String::new(),
            pushed: false,
            is_head: false,
            above: 0,
            staged,
            refusal: Some(why),
        },
    })
}

fn refuse_pushed(found: &Target, allow_pushed: bool) -> Result<(), String> {
    if found.pushed && !allow_pushed {
        return Err(format!(
            "{} is already on a remote. Rewriting it means a force push afterwards; confirm \
             that first.",
            found.short
        ));
    }
    Ok(())
}

/// Folds each fixup commit into the commit it was made for, by commit id.
///
/// `fixups` pairs a target with the fixup commit made for it. The todo list is
/// written here — every commit from just above `base` picked, each fixup
/// right after its own target — rather than left to `--autosquash`, which
/// matches a `fixup!` commit to its target by subject: two commits with the
/// same subject, and the fix went into whichever came first.
///
/// Open work rides along on `--autostash` and comes back unstaged once the
/// rebase is done. A conflict stops it in the ordinary way, and the strip
/// under the toolbar continues or aborts it from there; whatever finishes it
/// records the step.
#[allow(clippy::too_many_arguments)]
fn fold_fixups(
    state: &AppState,
    base: Option<&str>,
    fixups: &[(String, String)],
    before: Option<String>,
    branch: Option<String>,
    kind: &str,
    label: String,
    done: String,
) -> Result<String, String> {
    let root = state.path()?;
    let chain = chain_from(&root, base)?;
    let made: HashSet<&str> = fixups.iter().map(|(_, fixup)| fixup.as_str()).collect();
    let mut steps: Vec<rebase::Step> = Vec::new();
    for oid in chain.iter().filter(|oid| !made.contains(oid.as_str())) {
        steps.push(rebase::Step {
            oid: oid.clone(),
            action: rebase::Action::Pick,
        });
        for (_, fixup) in fixups.iter().filter(|(target, _)| target == oid) {
            steps.push(rebase::Step {
                oid: fixup.clone(),
                action: rebase::Action::Fixup,
            });
        }
    }
    let stacked = rebase::carried_by(state, base);
    let todo = rebase::todo_with_refs(&steps, &stacked)?;
    let said = rebase::run_todo(state, base, &todo, &[], "true", &stacked, done)?;
    if said == "Rebase stopped" {
        journal::begin_pending(state, kind, label.clone(), branch, before);
        return Ok(format!(
            "{label} stopped part-way — resolve it, then continue"
        ));
    }
    // Soft: the branch going back is the whole undo. The folded changes are
    // then staged again, which is where they were before any of this.
    journal::record(
        state,
        kind,
        label,
        branch,
        before,
        journal::head_oid(state),
        Mode::Soft,
        false,
    );
    Ok(said)
}

// --- fix up -------------------------------------------------------------------

/// Makes a `fixup!` commit for `oid` out of what is staged, and folds it in
/// when `fold` says so.
///
/// Without `fold` it is one ordinary commit, which a later autosquash (or the
/// next fixup) folds; with it, the history is rewritten here and now, which is
/// why the window asks first.
pub fn fixup(
    state: &AppState,
    oid: &str,
    fold: bool,
    allow_pushed: bool,
) -> Result<String, String> {
    let root = state.path()?;
    let found = target(state, oid)?;
    refuse_pushed(&found, allow_pushed)?;
    if staged_count(&root) == 0 {
        return Err(
            "Nothing is staged. Stage the changes that belong in that commit first.".into(),
        );
    }
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    git_cmd::run_checked(&root, &["commit", &format!("--fixup={}", found.oid)])?;
    let made = journal::head_at(&root).unwrap_or_default();

    let label = format!("Fix up {}", found.short);
    if !fold {
        journal::record(
            state,
            "fixup",
            label,
            branch,
            before,
            journal::head_oid(state),
            Mode::Soft,
            false,
        );
        return Ok(format!(
            "Made a fixup commit for {}; it folds in at the next autosquash",
            found.short
        ));
    }
    fold_fixups(
        state,
        found.parent.as_deref(),
        &[(found.oid.clone(), made)],
        before,
        branch,
        "fixup",
        label,
        format!(
            "Folded the staged changes into {} {}",
            found.short, found.summary
        ),
    )
}

// --- a new message for any commit -----------------------------------------------

/// Gives `oid` a new message, whichever commit on the branch it is.
///
/// The newest one is an amend. Anything older is replayed with `reword`, and
/// the message goes in the way the squash's does: the editor slot is this app,
/// which copies the text the user wrote over the one git offers.
pub fn reword_commit(
    state: &AppState,
    oid: &str,
    message: &str,
    allow_pushed: bool,
) -> Result<String, String> {
    let text = message.trim();
    if text.is_empty() {
        return Err("A commit needs a message".into());
    }
    let found = target(state, oid)?;
    refuse_pushed(&found, allow_pushed)?;
    if found.is_head {
        crate::work::reword(state, &found.oid, text)?;
        return Ok(format!("New message on {}", found.short));
    }

    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let note = git_dir.join("gitnoob-squash-message");
    std::fs::write(&note, format!("{text}\n"))
        .map_err(|e| format!("Could not write the message: {e}"))?;
    let editor = rebase::sequence_editor(&note)?;

    let chain = chain_from(&root, found.parent.as_deref())?;
    let stacked = rebase::carried_by(state, found.parent.as_deref());
    let steps: Vec<rebase::Step> = chain
        .iter()
        .map(|one| rebase::Step {
            oid: one.clone(),
            action: rebase::Action::Pick,
        })
        .collect();
    // `reword` is the one word the plan never writes (it goes in as `edit`
    // there, for the window to ask); here the message is already known.
    let todo = rebase::todo_with_refs(&steps, &stacked)?.replacen(
        &format!("pick {}", found.oid),
        &format!("reword {}", found.oid),
        1,
    );
    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    let said = rebase::run_todo(
        state,
        found.parent.as_deref(),
        &todo,
        &[],
        &editor,
        &stacked,
        format!("New message on {}", found.short),
    )?;
    if said == "Rebase stopped" {
        // Whatever finishes it records the step, as for any rebase that stops.
        journal::begin_pending(
            state,
            "reword",
            format!("Reword {}", found.short),
            branch,
            before,
        );
        return Ok("Changing the message stopped part-way — resolve it, then continue".into());
    }
    journal::record(
        state,
        "reword",
        format!("Reword {}", found.short),
        branch,
        before,
        journal::head_oid(state),
        Mode::Soft,
        false,
    );
    Ok(said)
}

/// Every commit from just above `base` up to HEAD, oldest first.
fn chain_from(root: &Path, base: Option<&str>) -> Result<Vec<String>, String> {
    let range = match base {
        Some(base) => format!("{base}..HEAD"),
        None => "HEAD".to_string(),
    };
    let listed = git_cmd::run_checked(root, &["rev-list", "--reverse", "--topo-order", &range])?;
    Ok(listed.lines().map(|l| l.trim().to_string()).collect())
}

// --- split --------------------------------------------------------------------

/// Starts splitting `oid`: the rebase stops on it with its changes taken back
/// out of the commit and left unstaged, ready to be committed in pieces.
pub fn split_start(state: &AppState, oid: &str, allow_pushed: bool) -> Result<String, String> {
    let found = target(state, oid)?;
    refuse_pushed(&found, allow_pushed)?;
    let Some(parent) = found.parent.clone() else {
        return Err("The first commit has nothing under it to split onto.".into());
    };
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let before_head = journal::head_at(&root);
    let before_branch = journal::branch_at(&root);

    let chain = chain_from(&root, Some(&parent))?;
    let steps: Vec<rebase::Step> = chain
        .iter()
        .map(|one| rebase::Step {
            oid: one.clone(),
            action: if *one == found.oid {
                rebase::Action::Edit
            } else {
                rebase::Action::Pick
            },
        })
        .collect();
    let stacked = rebase::carried_by(state, Some(&parent));
    let todo = rebase::todo_with_refs(&steps, &stacked)?;

    std::fs::write(git_dir.join(SPLIT), format!("{}\n", found.oid))
        .map_err(|e| format!("Could not mark the split: {e}"))?;
    let said = rebase::run_todo(
        state,
        Some(&parent),
        &todo,
        &[],
        "true",
        &stacked,
        "Split".to_string(),
    );
    let said = match said {
        Ok(said) => said,
        Err(error) => {
            let _ = std::fs::remove_file(git_dir.join(SPLIT));
            return Err(error);
        }
    };
    if said == "Rebase stopped" {
        // The split ends however the rebase does — carried on from the split
        // strip or from the conflict one — and whatever ends it records it.
        journal::begin_pending(
            state,
            "split",
            format!("Split {}", found.short),
            before_branch.clone(),
            before_head.clone(),
        );
    }
    let stopped = std::fs::read_to_string(git_dir.join("rebase-merge").join("stopped-sha"))
        .map(|text| text.trim().to_string())
        .unwrap_or_default();
    if said != "Rebase stopped" || stopped != found.oid || crate::work::has_unmerged(&root) {
        return Ok(said);
    }

    // The commit is there as it was, on top of its parent; taking HEAD back
    // one step with the index reset and the files left alone is what turns it
    // back into changes to commit.
    git_cmd::run_checked(&root, &["reset", "--mixed", "-q", &parent, "--"])?;
    Ok(format!(
        "Splitting {}: its changes are unstaged. Commit them in pieces, then continue.",
        found.summary
    ))
}

/// Clears the way for aborting a split.
///
/// Taking the commit apart left the files it added untracked, and git will
/// not abort a rebase over untracked files it would have to overwrite. Those
/// are removed here — but only where the file is still byte for byte what the
/// commit added, which is exactly what the abort puts back. Anything edited
/// since is left for git to refuse over, rather than thrown away.
pub fn clear_for_abort(state: &AppState) {
    let Ok(root) = state.path() else { return };
    let Ok(git_dir) = crate::remote::git_dir(&root) else {
        return;
    };
    let Some(oid) = splitting(&git_dir) else {
        return;
    };
    let Ok(added) = git_cmd::run_checked(
        &root,
        &[
            "diff",
            "--name-only",
            "-z",
            "--diff-filter=A",
            &format!("{oid}^"),
            &oid,
            "--",
        ],
    ) else {
        return;
    };
    let Ok(repo) = state.repo() else { return };
    let Ok(tree) = repo
        .revparse_single(&oid)
        .and_then(|object| object.peel_to_tree())
    else {
        return;
    };
    let index = repo.index().ok();
    for path in added.split('\0').filter(|p| !p.is_empty()) {
        let tracked = index
            .as_ref()
            .is_some_and(|index| index.get_path(Path::new(path), 0).is_some());
        if tracked {
            continue;
        }
        let Some(blob) = tree
            .get_path(Path::new(path))
            .ok()
            .and_then(|entry| repo.find_blob(entry.id()).ok())
        else {
            continue;
        };
        let full = root.join(path);
        if std::fs::read(&full).is_ok_and(|bytes| bytes == blob.content()) {
            let _ = std::fs::remove_file(&full);
        }
    }
}

/// What a split's own work comes to as it is abandoned, set somewhere safe.
///
/// `rebase --abort` resets everything to where the branch started, and the
/// files of a split are where the work of one is done: an edit made while
/// committing the pieces was gone with them. So before the abort, anything
/// that differs from HEAD — tracked or not — goes into a stash, and if what
/// is there, pieces and all, is more than the commit being split held, the
/// note says where it went. A split abandoned without touching anything
/// leaves nothing behind.
///
/// `Err` when the work could not be set down; the abort must not go ahead.
pub fn keep_for_abort(state: &AppState) -> Result<Option<String>, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let Some(oid) = splitting(&git_dir) else {
        return Ok(None);
    };
    let head = journal::head_at(&root).unwrap_or_default();
    let short: String = head.chars().take(7).collect();
    if crate::work::has_unmerged(&root) {
        // Stopped on a conflict further up: what is open is git's half-made
        // pick, not work, and a stash refuses an unmerged index anyway. The
        // pieces are commits, and the abort leaves them in the reflog.
        return Ok(Some(format!(
            "Anything committed during the split is at {short}, in the reflog."
        )));
    }
    let dirty = !git_cmd::run_checked(&root, &["status", "--porcelain"])?
        .trim()
        .is_empty();
    let mut kept = None;
    if dirty {
        let before = journal::stash_ref(&root);
        git_cmd::run_checked(
            &root,
            &[
                "stash",
                "push",
                "--include-untracked",
                "-m",
                "gitnoob: work from an abandoned split",
            ],
        )?;
        kept = journal::stash_ref(&root).filter(|made| Some(made) != before.as_ref());
        if kept.is_none() {
            return Err(
                "The open files could not be stashed, so the split was not abandoned.".into(),
            );
        }
    }
    // What the work comes to: the stash when there is one (HEAD and the
    // changes on it, its untracked files beside), otherwise HEAD — against
    // the commit being split.
    let now = kept.clone().unwrap_or_else(|| head.clone());
    let untracked = kept
        .as_ref()
        .map(|stash| format!("{stash}^3"))
        .filter(|third| quiet(&root, &["rev-parse", "--verify", "--quiet", third]).is_some());
    if !more_than(&root, &oid, &now, untracked.as_deref()) {
        // Nothing here the abort does not put back as it was.
        if let Some(stash) = &kept {
            if let Some(at) = journal::stash_index_at(&root, stash) {
                let _ = git_cmd::run(&root, &["stash", "drop", "-q", &format!("stash@{{{at}}}")]);
            }
        }
        return Ok(None);
    }
    Ok(Some(match &kept {
        Some(_) => format!(
            "Your work from the split is in the stash, as “gitnoob: work from an abandoned \
             split”. Pieces you had committed are at {short}."
        ),
        None => format!(
            "The pieces you had committed are at {short}; `git branch <name> {short}` keeps them."
        ),
    }))
}

/// Whether `now` — a commit, with `untracked` the tree of files beside it —
/// holds anything the commit `oid` does not. A file the commit added shows up
/// untracked during a split, and is the same file as long as its bytes are.
fn more_than(root: &Path, oid: &str, now: &str, untracked: Option<&str>) -> bool {
    let blob = |rev: &str, path: &str| {
        quiet(
            root,
            &["rev-parse", "--verify", "--quiet", &format!("{rev}:{path}")],
        )
    };
    let Ok(differ) = git_cmd::run_checked(
        root,
        &["diff", "--name-only", "-z", "--no-renames", oid, now, "--"],
    ) else {
        return true;
    };
    let tracked_extra = differ
        .split('\0')
        .filter(|path| !path.is_empty())
        .any(|path| {
            let carried = untracked.and_then(|third| blob(third, path));
            carried.is_none() || carried != blob(oid, path)
        });
    if tracked_extra {
        return true;
    }
    let Some(third) = untracked else {
        return false;
    };
    let Ok(listed) = git_cmd::run_checked(root, &["ls-tree", "-r", "-z", "--name-only", third])
    else {
        return true;
    };
    listed
        .split('\0')
        .filter(|path| !path.is_empty())
        .any(|path| blob(third, path) != blob(oid, path))
}

/// What carrying on from a split found.
#[derive(Serialize, Debug)]
pub struct SplitOutcome {
    /// The split is over and the rest of the branch was replayed.
    pub finished: bool,
    /// Files of the original commit that are not committed yet, when that is
    /// why it did not carry on.
    pub left: Vec<String>,
    pub message: String,
}

/// What to do with what is left of the commit when the user carries on.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Rest {
    /// Carry on only when nothing is left; otherwise say what is.
    Ask,
    /// Commit what is left as one more piece, with the original message.
    Commit,
    /// Carry on without it — for changes the user discarded on purpose.
    Leave,
}

/// Carries on from a split once the pieces are committed.
pub fn split_continue(state: &AppState, rest: Rest) -> Result<SplitOutcome, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let oid = splitting(&git_dir).ok_or_else(|| "No split is under way".to_string())?;
    if crate::work::has_unmerged(&root) {
        return Err("Some files are still conflicted. Resolve them first.".into());
    }

    let left: Vec<String> =
        git_cmd::run_checked(&root, &["diff", "--name-only", "-z", "HEAD", &oid, "--"])?
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .collect();
    if !left.is_empty() {
        match rest {
            Rest::Ask => {
                return Ok(SplitOutcome {
                    finished: false,
                    message: format!(
                        "{} {} of the original commit {} not committed yet",
                        left.len(),
                        if left.len() == 1 { "file" } else { "files" },
                        if left.len() == 1 { "is" } else { "are" }
                    ),
                    left,
                });
            }
            Rest::Commit => {
                let specs: Vec<String> = left.iter().map(|path| literal(path)).collect();
                let mut args = vec!["add", "-A", "--"];
                args.extend(specs.iter().map(String::as_str));
                git_cmd::run_checked(&root, &args)?;
                let nothing = git_cmd::run(&root, &["diff", "--cached", "--quiet"])
                    .map(|out| out.ok)
                    .unwrap_or(false);
                if !nothing {
                    git_cmd::run_checked(&root, &["commit", "-q", "--no-edit", "-C", &oid])?;
                }
            }
            Rest::Leave => {}
        }
    }

    // `rebase --continue` refuses over uncommitted changes, and anything still
    // here is not part of the split: better said in these words than git's.
    let dirty = git_cmd::run_checked(&root, &["status", "--porcelain", "--untracked-files=no"])?;
    if !dirty.trim().is_empty() {
        return Err(
            "There are uncommitted changes left. Commit them as one more piece, or discard them, \
             before carrying on."
                .into(),
        );
    }

    let started = std::fs::read_to_string(git_dir.join("rebase-merge").join("orig-head"))
        .ok()
        .map(|text| text.trim().to_string());
    let branch = std::fs::read_to_string(git_dir.join("rebase-merge").join("head-name"))
        .ok()
        .and_then(|text| text.trim().strip_prefix("refs/heads/").map(str::to_string));
    // A split that began waiting on the history is recorded by the continue
    // itself, as any rebase that stopped is; only one from before that note
    // existed is recorded here.
    let waiting = journal::pending(state).is_some();
    let said = rebase::resume(state)?;
    if said == "Rebase stopped" {
        return Ok(SplitOutcome {
            finished: false,
            left: Vec::new(),
            message: "The split is done; the rebase stopped again further up".into(),
        });
    }
    if !waiting {
        journal::record(
            state,
            "split",
            format!("Split {}", &oid[..oid.len().min(7)]),
            branch,
            started,
            journal::head_oid(state),
            Mode::Soft,
            false,
        );
    }
    Ok(SplitOutcome {
        finished: true,
        left: Vec::new(),
        message: said.replacen("Rebase finished", "Split finished", 1),
    })
}

// --- absorb -------------------------------------------------------------------

/// One staged hunk and where it would go.
#[derive(Serialize, Debug, Clone)]
pub struct AbsorbHunk {
    pub path: String,
    /// `@@ -a,b +c,d @@`, as git numbers it against HEAD.
    pub header: String,
    /// A few of the changed lines, `+` and `-` included, to recognise it by.
    pub preview: Vec<String>,
    /// The commit it goes into, or `None` when it stays staged.
    pub target: Option<AbsorbTarget>,
    /// Why it stays, when it does.
    pub why: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct AbsorbTarget {
    pub oid: String,
    pub short: String,
    pub summary: String,
}

/// The whole plan, shown before anything happens.
#[derive(Serialize, Debug)]
pub struct AbsorbPlan {
    pub hunks: Vec<AbsorbHunk>,
    /// Staged files that are not hunks of an existing file — new, deleted,
    /// binary — and so stay staged whatever happens.
    pub kept_files: Vec<String>,
    /// The staged state and HEAD the plan was made against; carrying it out
    /// is refused if either moved since it was shown.
    pub fingerprint: String,
    /// Why nothing can be absorbed at all.
    pub refusal: Option<String>,
}

/// One hunk of a staged diff, by line numbers alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
}

/// `@@ -a[,b] +c[,d] @@` → the four numbers; a missing count means one.
fn parse_header(line: &str) -> Option<Span> {
    let inner = line.strip_prefix("@@ ")?.split(" @@").next()?;
    let mut parts = inner.split_whitespace();
    let read = |part: &str, sign: char| -> Option<(usize, usize)> {
        let rest = part.strip_prefix(sign)?;
        let mut numbers = rest.split(',');
        let start = numbers.next()?.parse().ok()?;
        let count = match numbers.next() {
            Some(count) => count.parse().ok()?,
            None => 1,
        };
        Some((start, count))
    };
    let (old_start, old_count) = read(parts.next()?, '-')?;
    let (new_start, new_count) = read(parts.next()?, '+')?;
    Some(Span {
        old_start,
        old_count,
        new_start,
        new_count,
    })
}

/// A file's bytes split into lines, each keeping its own ending — `\r\n`
/// stays `\r\n`, and a last line without one stays without one.
fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (at, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            out.push(&bytes[start..=at]);
            start = at + 1;
        }
    }
    if start < bytes.len() {
        out.push(&bytes[start..]);
    }
    out
}

/// The file as HEAD has it with only some of the staged hunks applied.
///
/// Every hunk is numbered against HEAD's copy, and hunks never overlap, so any
/// subset can be laid over it in one pass: HEAD's lines where a hunk is not
/// taken, the staged copy's where it is. The lines are byte slices of the two
/// real blobs, which is what keeps line endings and a missing final newline
/// exactly as they were.
fn with_hunks(old: &[u8], new: &[u8], spans: &[Span], take: &[bool]) -> Vec<u8> {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let mut out: Vec<u8> = Vec::with_capacity(old.len().max(new.len()));
    // How many of HEAD's lines have been dealt with.
    let mut done = 0usize;
    for (span, taken) in spans.iter().zip(take) {
        // A pure addition (`-a,0`) goes in after line `a`; anything else
        // starts at line `a` itself.
        let from = if span.old_count == 0 {
            span.old_start
        } else {
            span.old_start - 1
        };
        for line in old_lines.iter().take(from).skip(done) {
            out.extend_from_slice(line);
        }
        if *taken {
            let first = if span.new_count == 0 {
                0
            } else {
                span.new_start - 1
            };
            for line in new_lines.iter().skip(first).take(span.new_count) {
                out.extend_from_slice(line);
            }
        } else {
            for line in old_lines.iter().skip(from).take(span.old_count) {
                out.extend_from_slice(line);
            }
        }
        done = from + span.old_count;
    }
    for line in old_lines.iter().skip(done) {
        out.extend_from_slice(line);
    }
    out
}

/// One staged file, read for absorbing.
struct StagedFile {
    path: String,
    old: Vec<u8>,
    new: Vec<u8>,
    mode: String,
    spans: Vec<Span>,
}

/// The commits on this branch a hunk may go into, newest first: not on any
/// remote, above where the branch left the trunk, and above the newest merge.
fn absorbable(state: &AppState) -> Result<Vec<String>, String> {
    let root = state.path()?;
    let fork = {
        let repo = state.repo()?;
        let head = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .ok_or_else(|| "There are no commits yet".to_string())?;
        crate::refs::trunk_of(&repo)
            .and_then(|name| repo.revparse_single(&name).ok())
            .and_then(|object| object.peel_to_commit().ok())
            .and_then(|trunk| repo.merge_base(head, trunk.id()).ok())
            // Standing on the trunk itself, the fork is HEAD, which would
            // leave nothing; there only the remotes draw the line.
            .filter(|fork| *fork != head)
            .map(|oid| oid.to_string())
    };
    let mut args: Vec<String> = vec![
        "rev-list".into(),
        "--parents".into(),
        "--max-count=1000".into(),
        "HEAD".into(),
    ];
    if let Some(fork) = &fork {
        args.push(format!("^{fork}"));
    }
    args.extend(["--not".into(), "--remotes".into()]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let listed = git_cmd::run_checked(&root, &refs)?;
    let mut out = Vec::new();
    for line in listed.lines() {
        let mut parts = line.split_whitespace();
        let Some(oid) = parts.next() else { continue };
        if parts.count() > 1 {
            break;
        }
        out.push(oid.to_string());
    }
    Ok(out)
}

/// Reads what is staged, file by file, with HEAD's copy and the staged copy.
fn staged_files(state: &AppState) -> Result<(Vec<StagedFile>, Vec<String>), String> {
    let root = state.path()?;
    if crate::work::has_unmerged(&root) {
        return Err("Some files are conflicted. Resolve them first.".into());
    }
    let listed = git_cmd::run_checked(
        &root,
        &["diff", "--cached", "--name-status", "-z", "--no-renames"],
    )?;
    let mut fields = listed.split('\0').filter(|f| !f.is_empty());
    let repo = state.repo()?;
    let head_tree = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_tree().ok())
        .ok_or_else(|| "There are no commits yet".to_string())?;
    let index = repo.index().map_err(|e| e.message().to_string())?;

    let mut files = Vec::new();
    let mut kept = Vec::new();
    while let (Some(status), Some(path)) = (fields.next(), fields.next()) {
        if status != "M" {
            kept.push(path.to_string());
            continue;
        }
        let diff = git_cmd::run_checked(
            &root,
            &[
                "diff",
                "--cached",
                "-U0",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--",
                &literal(path),
            ],
        )?;
        let spans: Vec<Span> = diff.lines().filter_map(parse_header).collect();
        if spans.is_empty() || diff.contains("\nBinary files ") || diff.starts_with("Binary files ")
        {
            kept.push(path.to_string());
            continue;
        }
        let old = head_tree
            .get_path(Path::new(path))
            .ok()
            .and_then(|entry| repo.find_blob(entry.id()).ok())
            .map(|blob| blob.content().to_vec());
        let staged = index.get_path(Path::new(path), 0);
        let (Some(old), Some(staged)) = (old, staged) else {
            kept.push(path.to_string());
            continue;
        };
        let Ok(blob) = repo.find_blob(staged.id) else {
            kept.push(path.to_string());
            continue;
        };
        let new = blob.content().to_vec();
        if old.contains(&0) || new.contains(&0) {
            kept.push(path.to_string());
            continue;
        }
        files.push(StagedFile {
            path: path.to_string(),
            old,
            new,
            mode: format!("{:o}", staged.mode),
            spans,
        });
    }
    Ok((files, kept))
}

/// Which commit last touched each of the given lines of HEAD's copy, by line.
fn blame_lines(
    root: &Path,
    path: &str,
    lines: &[usize],
    boundary: Option<&str>,
) -> HashMap<usize, String> {
    let mut found = HashMap::new();
    if lines.is_empty() {
        return found;
    }
    let mut args: Vec<String> = vec!["blame".into(), "--porcelain".into()];
    let mut sorted: Vec<usize> = lines.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    // Neighbouring lines go in as one range, so a long hunk is one `-L`.
    let mut at = 0;
    while at < sorted.len() {
        let start = sorted[at];
        let mut end = start;
        while at + 1 < sorted.len() && sorted[at + 1] == end + 1 {
            at += 1;
            end = sorted[at];
        }
        args.push("-L".into());
        args.push(format!("{start},{end}"));
        at += 1;
    }
    if let Some(boundary) = boundary {
        args.push(format!("^{boundary}"));
    }
    args.push("HEAD".into());
    args.push("--".into());
    args.push(path.to_string());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let Ok(out) = git_cmd::run(root, &refs) else {
        return found;
    };
    if !out.ok {
        return found;
    }
    for line in out.stdout.lines() {
        let mut parts = line.split(' ');
        let (Some(oid), Some(_), Some(final_line)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if oid.len() < 40 || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        if let Ok(number) = final_line.parse::<usize>() {
            found.insert(number, oid.to_string());
        }
    }
    found
}

/// The lines of HEAD's copy that decide where a hunk goes: the ones it
/// changes or takes out, or — for a pure addition — the lines either side of
/// where it goes in.
fn deciding_lines(span: &Span, old_len: usize) -> Vec<usize> {
    if span.old_count > 0 {
        return (span.old_start..span.old_start + span.old_count).collect();
    }
    [span.old_start, span.old_start + 1]
        .into_iter()
        .filter(|line| *line >= 1 && *line <= old_len)
        .collect()
}

fn preview_of(file: &StagedFile, span: &Span) -> Vec<String> {
    let old = split_lines(&file.old);
    let new = split_lines(&file.new);
    let show = |line: &[u8]| {
        String::from_utf8_lossy(line)
            .trim_end_matches(['\n', '\r'])
            .to_string()
    };
    let mut out = Vec::new();
    if span.old_count > 0 {
        for line in old
            .iter()
            .skip(span.old_start - 1)
            .take(span.old_count.min(3))
        {
            out.push(format!("-{}", show(line)));
        }
    }
    if span.new_count > 0 {
        for line in new
            .iter()
            .skip(span.new_start - 1)
            .take(span.new_count.min(3))
        {
            out.push(format!("+{}", show(line)));
        }
    }
    out
}

/// The staged state and HEAD, as one string that changes when either does.
///
/// The raw staged diff names every staged blob by its id, so any change to
/// what is staged changes it, and reading it writes nothing.
fn fingerprint(root: &Path) -> Result<String, String> {
    use std::hash::{Hash, Hasher};
    let head = git_cmd::run_checked(root, &["rev-parse", "HEAD"])?;
    let staged = git_cmd::run_checked(
        root,
        &[
            "diff",
            "--cached",
            "--raw",
            "-z",
            "--no-renames",
            "--no-abbrev",
        ],
    )?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    staged.hash(&mut hasher);
    Ok(format!("{} {:016x}", head.trim(), hasher.finish()))
}

/// Works out where each staged hunk would go, without changing anything.
pub fn absorb_plan(state: &AppState) -> Result<AbsorbPlan, String> {
    let root = state.path()?;
    let refuse = |why: String| AbsorbPlan {
        hunks: Vec::new(),
        kept_files: Vec::new(),
        fingerprint: String::new(),
        refusal: Some(why),
    };
    let git_dir = crate::remote::git_dir(&root)?;
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        return Ok(refuse(
            "A rebase is already part-way through. Continue or abort that one first.".into(),
        ));
    }
    if journal::current_branch(state).is_none() {
        return Ok(refuse("HEAD is detached; check out a branch first.".into()));
    }
    let (files, kept_files) = match staged_files(state) {
        Ok(found) => found,
        Err(why) => return Ok(refuse(why)),
    };
    if files.is_empty() && kept_files.is_empty() {
        return Ok(refuse("Nothing is staged.".into()));
    }
    let candidates = absorbable(state)?;
    let summaries: HashMap<String, String> = candidates
        .iter()
        .map(|oid| {
            let summary = quiet(
                &root,
                &["log", "-1", "--no-show-signature", "--format=%s", oid],
            )
            .unwrap_or_default();
            (oid.clone(), summary)
        })
        .collect();
    let pool: HashSet<&str> = candidates.iter().map(String::as_str).collect();
    // Blame need not look further back than the oldest commit that could be
    // an answer: everything older reads as the boundary, which is no answer.
    let boundary = candidates.last().and_then(|oldest| {
        quiet(
            &root,
            &["rev-parse", "--verify", "--quiet", &format!("{oldest}^")],
        )
    });

    let mut hunks = Vec::new();
    for file in &files {
        let old_len = split_lines(&file.old).len();
        let wanted: Vec<usize> = file
            .spans
            .iter()
            .flat_map(|span| deciding_lines(span, old_len))
            .collect();
        let blamed = if candidates.is_empty() {
            HashMap::new()
        } else {
            blame_lines(&root, &file.path, &wanted, boundary.as_deref())
        };
        for span in &file.spans {
            let mut found: Vec<&str> = Vec::new();
            for line in deciding_lines(span, old_len) {
                if let Some(oid) = blamed.get(&line) {
                    if pool.contains(oid.as_str()) && !found.contains(&oid.as_str()) {
                        found.push(oid.as_str());
                    }
                }
            }
            let (target, why) = match found.as_slice() {
                [one] => (
                    Some(AbsorbTarget {
                        oid: one.to_string(),
                        short: one.chars().take(7).collect(),
                        summary: summaries.get(*one).cloned().unwrap_or_default(),
                    }),
                    None,
                ),
                [] => (
                    None,
                    Some(
                        "Those lines come from commits already pushed or older than this branch"
                            .to_string(),
                    ),
                ),
                many => (
                    None,
                    Some(format!(
                        "Those lines come from {} of your commits: {}",
                        many.len(),
                        many.iter()
                            .map(|oid| oid.chars().take(7).collect::<String>())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                ),
            };
            hunks.push(AbsorbHunk {
                path: file.path.clone(),
                header: format!(
                    "@@ -{},{} +{},{} @@",
                    span.old_start, span.old_count, span.new_start, span.new_count
                ),
                preview: preview_of(file, span),
                target,
                why,
            });
        }
    }

    let refusal = (!hunks.iter().any(|hunk| hunk.target.is_some())).then(|| {
        "None of the staged changes belongs to exactly one commit of yours that is not pushed yet."
            .to_string()
    });
    Ok(AbsorbPlan {
        hunks,
        kept_files,
        fingerprint: fingerprint(&root)?,
        refusal,
    })
}

/// Carries the plan out: one `fixup!` commit per target, made from just its
/// hunks, then (with `fold`) the autosquash that folds them in.
///
/// Each fixup commit is built in a separate index, so the user's own index —
/// the staged state they looked at — is never touched until the rebase at the
/// end, and a failure part-way leaves it exactly as it was. Whatever does not
/// go anywhere stays staged against the new HEAD, and comes back unstaged once
/// an autosquash has run.
pub fn absorb(state: &AppState, fingerprint_seen: &str, fold: bool) -> Result<String, String> {
    let root = state.path()?;
    let git_dir = crate::remote::git_dir(&root)?;
    let plan = absorb_plan(state)?;
    if let Some(why) = plan.refusal {
        return Err(why);
    }
    if plan.fingerprint != fingerprint_seen {
        return Err("What is staged changed since the plan was shown. Look at it again.".into());
    }
    let (files, _) = staged_files(state)?;
    let candidates = absorbable(state)?;

    // Targets oldest first, so the fixups read in history order.
    let mut targets: Vec<AbsorbTarget> = Vec::new();
    for hunk in &plan.hunks {
        if let Some(target) = &hunk.target {
            if !targets.contains(target) {
                targets.push(target.clone());
            }
        }
    }
    targets.sort_by_key(|target| {
        std::cmp::Reverse(
            candidates
                .iter()
                .position(|oid| *oid == target.oid)
                .unwrap_or(0),
        )
    });

    let before = journal::head_oid(state);
    let branch = journal::current_branch(state);
    let scratch = git_dir.join("gitnoob-absorb");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)
        .map_err(|e| format!("Could not make a scratch space: {e}"))?;
    let index_file = scratch.join("index");
    let index_env = index_file.to_string_lossy().to_string();

    let mut fixups: Vec<(String, String)> = Vec::new();
    let made = (|| -> Result<(), String> {
        for (step, target) in targets.iter().enumerate() {
            let wanted: Vec<&AbsorbTarget> = targets.iter().take(step + 1).collect();
            git_cmd::run_with_env(
                &root,
                &["read-tree", "HEAD"],
                &[("GIT_INDEX_FILE", &index_env)],
            )
            .and_then(|out| {
                if out.ok {
                    Ok(())
                } else {
                    Err(rebase::one_message(&out))
                }
            })?;
            for file in &files {
                let take: Vec<bool> = file
                    .spans
                    .iter()
                    .map(|span| {
                        plan.hunks.iter().any(|hunk| {
                            hunk.path == file.path
                                && hunk.header
                                    == format!(
                                        "@@ -{},{} +{},{} @@",
                                        span.old_start,
                                        span.old_count,
                                        span.new_start,
                                        span.new_count
                                    )
                                && hunk
                                    .target
                                    .as_ref()
                                    .is_some_and(|t| wanted.iter().any(|w| w.oid == t.oid))
                        })
                    })
                    .collect();
                if !take.iter().any(|t| *t) {
                    continue;
                }
                let content = with_hunks(&file.old, &file.new, &file.spans, &take);
                let blob_file = scratch.join("blob");
                std::fs::write(&blob_file, &content)
                    .map_err(|e| format!("Could not write {}: {e}", file.path))?;
                let blob_path = blob_file.to_string_lossy().to_string();
                let blob = git_cmd::run_checked(
                    &root,
                    &["hash-object", "-w", "--no-filters", "--", &blob_path],
                )?;
                let info = format!("{},{},{}", file.mode, blob.trim(), file.path);
                let out = git_cmd::run_with_env(
                    &root,
                    &["update-index", "--cacheinfo", &info],
                    &[("GIT_INDEX_FILE", &index_env)],
                )?;
                if !out.ok {
                    return Err(rebase::one_message(&out));
                }
            }
            let out = git_cmd::run_with_env(
                &root,
                &["commit", "-q", &format!("--fixup={}", target.oid)],
                &[("GIT_INDEX_FILE", &index_env)],
            )?;
            if !out.ok {
                return Err(rebase::one_message(&out));
            }
            let fixup = journal::head_at(&root).ok_or("The fixup commit went missing")?;
            fixups.push((target.oid.clone(), fixup));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    if let Err(error) = made {
        // Put the branch back where it was. The user's index was never
        // touched, so a soft move is the whole of it.
        if let Some(before) = &before {
            let _ = git_cmd::run(&root, &["reset", "--soft", "-q", before]);
        }
        return Err(format!(
            "Absorbing stopped and nothing was changed: {error}"
        ));
    }

    let moved = plan.hunks.iter().filter(|h| h.target.is_some()).count();
    let stayed = plan.hunks.len() - moved;
    let label = format!(
        "Absorb {moved} {} into {} {}",
        if moved == 1 { "hunk" } else { "hunks" },
        targets.len(),
        if targets.len() == 1 {
            "commit"
        } else {
            "commits"
        }
    );
    let left = if stayed > 0 || !plan.kept_files.is_empty() {
        " What did not belong anywhere is still there."
    } else {
        ""
    };
    if !fold {
        journal::record(
            state,
            "absorb",
            label.clone(),
            branch,
            before,
            journal::head_oid(state),
            Mode::Soft,
            false,
        );
        return Ok(format!(
            "Made {} fixup {}; they fold in at the next autosquash.{left}",
            targets.len(),
            if targets.len() == 1 {
                "commit"
            } else {
                "commits"
            }
        ));
    }
    let oldest = targets.first().map(|t| t.oid.clone()).unwrap_or_default();
    let base = quiet(
        &root,
        &["rev-parse", "--verify", "--quiet", &format!("{oldest}^")],
    );
    fold_fixups(
        state,
        base.as_deref(),
        &fixups,
        before,
        branch,
        "absorb",
        label.clone(),
        format!("{label}.{left}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hunk_headers_with_and_without_counts() {
        assert_eq!(
            parse_header("@@ -3,2 +3,4 @@ fn x()"),
            Some(Span {
                old_start: 3,
                old_count: 2,
                new_start: 3,
                new_count: 4
            })
        );
        assert_eq!(
            parse_header("@@ -7 +7 @@"),
            Some(Span {
                old_start: 7,
                old_count: 1,
                new_start: 7,
                new_count: 1
            })
        );
        assert_eq!(
            parse_header("@@ -5,0 +6,2 @@"),
            Some(Span {
                old_start: 5,
                old_count: 0,
                new_start: 6,
                new_count: 2
            })
        );
        assert_eq!(parse_header("diff --git a/x b/x"), None);
    }

    #[test]
    fn any_subset_of_hunks_lays_over_the_old_copy() {
        let old = b"one\r\ntwo\r\nthree\r\nfour\r\nfive";
        let new = b"one\r\nTWO\r\nthree\r\nfour\r\nadded\r\nfive!";
        let spans = [
            Span {
                old_start: 2,
                old_count: 1,
                new_start: 2,
                new_count: 1,
            },
            Span {
                old_start: 4,
                old_count: 0,
                new_start: 5,
                new_count: 1,
            },
            Span {
                old_start: 5,
                old_count: 1,
                new_start: 6,
                new_count: 1,
            },
        ];
        assert_eq!(
            with_hunks(old, new, &spans, &[true, true, true]),
            new.to_vec()
        );
        assert_eq!(
            with_hunks(old, new, &spans, &[false, false, false]),
            old.to_vec()
        );
        assert_eq!(
            with_hunks(old, new, &spans, &[true, false, false]),
            b"one\r\nTWO\r\nthree\r\nfour\r\nfive".to_vec()
        );
        assert_eq!(
            with_hunks(old, new, &spans, &[false, true, true]),
            b"one\r\ntwo\r\nthree\r\nfour\r\nadded\r\nfive!".to_vec()
        );
    }

    #[test]
    fn an_addition_at_the_very_top_goes_in_before_the_first_line() {
        let old = b"a\nb\n";
        let new = b"new\na\nb\n";
        let spans = [Span {
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: 1,
        }];
        assert_eq!(with_hunks(old, new, &spans, &[true]), new.to_vec());
        assert_eq!(deciding_lines(&spans[0], 2), vec![1]);
    }

    #[test]
    fn a_pure_addition_is_decided_by_the_lines_either_side() {
        let span = Span {
            old_start: 4,
            old_count: 0,
            new_start: 5,
            new_count: 2,
        };
        assert_eq!(deciding_lines(&span, 10), vec![4, 5]);
        // At the end of the file there is only the line before.
        assert_eq!(deciding_lines(&span, 4), vec![4]);
    }
}
