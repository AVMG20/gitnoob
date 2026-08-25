use std::fs;

use serde::{Deserialize, Serialize};

use crate::git_cmd;
use crate::state::AppState;

/// One region of a conflicted file.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Block {
    /// Lines both sides agree on.
    Context { lines: Vec<String> },
    Conflict {
        /// Ordinal among the conflicts only, so the UI can index resolutions.
        index: usize,
        ours: Vec<String>,
        base: Vec<String>,
        theirs: Vec<String>,
        /// False when the file was written without `merge.conflictStyle=diff3`,
        /// in which case there is no merge base to show.
        has_base: bool,
        ours_label: String,
        theirs_label: String,
    },
}

/// Which of the three index stages a conflicted path actually has.
///
/// A file both sides edited has all three. One side deleting it leaves a hole,
/// and that hole is the whole conflict: there is nothing to merge line by line,
/// only a question of whether the file should exist.
#[derive(Serialize, Clone, Copy, Default, PartialEq, Debug)]
pub struct Stages {
    pub base: bool,
    pub ours: bool,
    pub theirs: bool,
}

#[derive(Serialize)]
pub struct ConflictFile {
    pub path: String,
    pub blocks: Vec<Block>,
    pub conflict_count: usize,
    /// What the index holds for this path, so a file with no conflict markers
    /// in it can still be explained rather than shown as two identical panes.
    pub stages: Stages,
    /// How the file on disk ends its lines. Parsing throws the endings away, so
    /// writing the resolution back has to put the right ones on again.
    #[serde(skip)]
    pub eol: Eol,
    /// Whether the file on disk ended with a line ending. Adding one to a file
    /// that never had one is a change the user did not ask for.
    #[serde(skip)]
    pub final_newline: bool,
}

/// The line ending a file uses.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub enum Eol {
    #[default]
    Lf,
    Crlf,
}

impl Eol {
    fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::Crlf => "\r\n",
        }
    }
}

/// The ending the file already uses, by majority.
///
/// A merge can leave both in one file — one side's lines came from a checkout
/// that converted them and the other's did not — and rewriting the file in
/// whichever is rarer would show every line as changed. The majority is the one
/// the file is meant to have.
fn detect_eol(text: &str) -> Eol {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    if crlf > lf {
        Eol::Crlf
    } else {
        Eol::Lf
    }
}

/// What the user chose for one conflict region.
#[derive(Deserialize)]
pub struct Resolution {
    #[serde(default)]
    pub take_ours: bool,
    #[serde(default)]
    pub take_theirs: bool,
    /// When both sides are taken, controls which is written first.
    #[serde(default = "yes")]
    pub ours_first: bool,
    /// Hand-edited replacement; wins over the checkboxes when present.
    #[serde(default)]
    pub custom: Option<Vec<String>>,
}

fn yes() -> bool {
    true
}

const OURS: &str = "<<<<<<<";
const BASE: &str = "|||||||";
const SPLIT: &str = "=======";
const THEIRS: &str = ">>>>>>>";

/// Paths git currently reports as conflicted.
pub fn list(state: &AppState) -> Result<Vec<String>, String> {
    let repo = state.repo()?;
    let index = repo.index().map_err(|e| e.message().to_string())?;
    if !index.has_conflicts() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in index.conflicts().map_err(|e| e.message().to_string())? {
        let entry = entry.map_err(|e| e.message().to_string())?;
        // Prefer "ours" for the path; on a delete/modify conflict only one side
        // has an entry.
        let bytes = entry
            .our
            .as_ref()
            .or(entry.their.as_ref())
            .or(entry.ancestor.as_ref())
            .map(|e| e.path.clone());
        if let Some(bytes) = bytes {
            let path = String::from_utf8_lossy(&bytes).into_owned();
            if !out.contains(&path) {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Which stages the index holds for a conflicted path.
pub fn stages(state: &AppState, path: &str) -> Result<Stages, String> {
    let root = state.path()?;
    let listed = git_cmd::run_checked(&root, &["ls-files", "--unmerged", "--", path])?;
    Ok(parse_stages(&listed))
}

/// `<mode> <sha> <stage>\t<path>`, one line per stage that exists.
fn parse_stages(listed: &str) -> Stages {
    let mut found = Stages::default();
    for line in listed.lines() {
        let Some((meta, _)) = line.split_once('\t') else {
            continue;
        };
        match meta.split_whitespace().nth(2) {
            Some("1") => found.base = true,
            Some("2") => found.ours = true,
            Some("3") => found.theirs = true,
            _ => {}
        }
    }
    found
}

/// Reads a conflicted file and splits it into agreed context and conflict
/// regions by parsing the markers git wrote into the working tree.
pub fn read(state: &AppState, path: &str) -> Result<ConflictFile, String> {
    let full = state.path()?.join(path);
    let text = fs::read_to_string(&full)
        .map_err(|e| format!("Could not read {}: {}", full.display(), e))?;

    let mut blocks = Vec::new();
    let mut context: Vec<String> = Vec::new();
    let mut conflict_count = 0usize;
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        if !line.starts_with(OURS) {
            context.push(line.to_string());
            continue;
        }

        if !context.is_empty() {
            blocks.push(Block::Context {
                lines: std::mem::take(&mut context),
            });
        }

        let ours_label = label(line, OURS);
        let mut ours = Vec::new();
        let mut base = Vec::new();
        let mut theirs = Vec::new();
        let mut has_base = false;
        let mut theirs_label = String::new();
        // Which side of the markers we are currently reading into.
        let mut section = 0u8; // 0 = ours, 1 = base, 2 = theirs

        while let Some(line) = lines.next() {
            if line.starts_with(BASE) {
                has_base = true;
                section = 1;
            } else if line.starts_with(SPLIT) {
                section = 2;
            } else if line.starts_with(THEIRS) {
                theirs_label = label(line, THEIRS);
                break;
            } else {
                match section {
                    0 => ours.push(line.to_string()),
                    1 => base.push(line.to_string()),
                    _ => theirs.push(line.to_string()),
                }
            }
        }

        blocks.push(Block::Conflict {
            index: conflict_count,
            ours,
            base,
            theirs,
            has_base,
            ours_label,
            theirs_label,
        });
        conflict_count += 1;
    }

    if !context.is_empty() {
        blocks.push(Block::Context { lines: context });
    }

    Ok(ConflictFile {
        path: path.to_string(),
        blocks,
        conflict_count,
        stages: stages(state, path)?,
        eol: detect_eol(&text),
        final_newline: text.ends_with('\n'),
    })
}

/// Renders the file the given resolutions produce, without writing it.
///
/// The UI calls this for the result pane so the preview and the eventual write
/// come from the same code.
pub fn preview(state: &AppState, path: &str, choices: &[Resolution]) -> Result<String, String> {
    let file = read(state, path)?;
    let mut out: Vec<String> = Vec::new();

    for block in &file.blocks {
        match block {
            Block::Context { lines } => out.extend(lines.iter().cloned()),
            Block::Conflict {
                index,
                ours,
                theirs,
                ..
            } => {
                let choice = choices.get(*index);
                match choice.and_then(|c| c.custom.as_ref()) {
                    Some(custom) => out.extend(custom.iter().cloned()),
                    None => {
                        // No answer yet for this region counts as "keep ours",
                        // so the preview is never mysteriously empty.
                        let (take_ours, take_theirs, ours_first) = match choice {
                            Some(c) => (c.take_ours, c.take_theirs, c.ours_first),
                            None => (true, false, true),
                        };
                        if ours_first {
                            if take_ours {
                                out.extend(ours.iter().cloned());
                            }
                            if take_theirs {
                                out.extend(theirs.iter().cloned());
                            }
                        } else {
                            if take_theirs {
                                out.extend(theirs.iter().cloned());
                            }
                            if take_ours {
                                out.extend(ours.iter().cloned());
                            }
                        }
                    }
                }
            }
        }
    }

    // Rejoin with the endings the file came in with. `str::lines` dropped the
    // carriage returns on the way in, and writing plain LF back would rewrite
    // every line of a CRLF file to resolve one conflict in it.
    let mut text = out.join(file.eol.as_str());
    if !text.is_empty() && file.final_newline {
        text.push_str(file.eol.as_str());
    }
    Ok(text)
}

/// Writes the resolved file and stages it, which is what clears the conflict.
pub fn resolve(state: &AppState, path: &str, choices: &[Resolution]) -> Result<String, String> {
    let text = preview(state, path, choices)?;
    let root = state.path()?;
    let full = root.join(path);
    fs::write(&full, text).map_err(|e| format!("Could not write {}: {}", full.display(), e))?;
    git_cmd::run_checked(&root, &["add", "--", path])?;
    Ok(format!("Resolved {path}"))
}

/// Resolves a whole file from one side.
///
/// `git checkout --ours` is the obvious way and it fails on the case that needs
/// help most: when a side deleted the file it has no stage to check out, and
/// git says "does not have our version" rather than doing the one thing that
/// side could have meant. Taking a side that deleted the file means deleting
/// it, so that is what happens.
pub fn resolve_whole(state: &AppState, path: &str, side: &str) -> Result<String, String> {
    let root = state.path()?;
    let found = stages(state, path)?;
    let (kept, flag) = match side {
        "ours" => (found.ours, "--ours"),
        "theirs" => (found.theirs, "--theirs"),
        other => return Err(format!("Unknown side: {other}")),
    };

    if !kept {
        // `-f` because the working tree holds the other side's content, which
        // git would otherwise refuse to throw away. That content is still in
        // the index stage it came from until the conflict is finished.
        git_cmd::run_checked(&root, &["rm", "-f", "--", path])?;
        return Ok(format!("Deleted {path}, which is what {side} did to it"));
    }

    git_cmd::run_checked(&root, &["checkout", flag, "--", path])?;
    git_cmd::run_checked(&root, &["add", "--", path])?;
    Ok(format!("Resolved {path} using {side}"))
}

/// Ends a conflict by keeping exactly what is on disk right now.
///
/// The way out when neither side is the answer: a file with no conflict markers
/// left in it, hand-edited or written by a merge driver, is finished — staging
/// it is all git is waiting for.
pub fn resolve_as_is(state: &AppState, path: &str) -> Result<String, String> {
    let root = state.path()?;
    let full = root.join(path);
    if !full.exists() {
        git_cmd::run_checked(&root, &["rm", "-f", "--", path])?;
        return Ok(format!("Deleted {path}"));
    }
    git_cmd::run_checked(&root, &["add", "--", path])?;
    Ok(format!("Resolved {path} as it stands"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_which_stages_a_path_has() {
        // Both sides edited it: base, ours, theirs.
        let all = "100644 aaa 1\tsrc/a.rs\n100644 bbb 2\tsrc/a.rs\n100644 ccc 3\tsrc/a.rs\n";
        assert_eq!(
            parse_stages(all),
            Stages { base: true, ours: true, theirs: true }
        );

        // Deleted by us, modified by them: no stage 2, and so no markers in the
        // working tree and nothing for "all ours" to act on.
        let deleted_by_us = "100644 aaa 1\ttests/T.php\n100644 ccc 3\ttests/T.php\n";
        assert_eq!(
            parse_stages(deleted_by_us),
            Stages { base: true, ours: false, theirs: true }
        );

        // Added on both sides, with no common ancestor.
        let add_add = "100644 bbb 2\tnew.txt\n100644 ccc 3\tnew.txt\n";
        assert_eq!(
            parse_stages(add_add),
            Stages { base: false, ours: true, theirs: true }
        );

        assert_eq!(parse_stages(""), Stages::default());
    }
}

/// Extracts the branch name a conflict marker carries, e.g. `<<<<<<< HEAD`.
fn label(line: &str, marker: &str) -> String {
    line.trim_start_matches(marker).trim().to_string()
}
