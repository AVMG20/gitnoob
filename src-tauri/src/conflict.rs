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

#[derive(Serialize)]
pub struct ConflictFile {
    pub path: String,
    pub blocks: Vec<Block>,
    pub conflict_count: usize,
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

    let mut text = out.join("\n");
    // Keep the file ending in a newline; git and every other tool expect it.
    if !text.is_empty() {
        text.push('\n');
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

/// Resolves a whole file from one side, the `--ours` / `--theirs` shortcut.
pub fn resolve_whole(state: &AppState, path: &str, side: &str) -> Result<String, String> {
    let root = state.path()?;
    let flag = match side {
        "ours" => "--ours",
        "theirs" => "--theirs",
        other => return Err(format!("Unknown side: {other}")),
    };
    git_cmd::run_checked(&root, &["checkout", flag, "--", path])?;
    git_cmd::run_checked(&root, &["add", "--", path])?;
    Ok(format!("Resolved {path} using {side}"))
}

/// Extracts the branch name a conflict marker carries, e.g. `<<<<<<< HEAD`.
fn label(line: &str, marker: &str) -> String {
    line.trim_start_matches(marker).trim().to_string()
}
