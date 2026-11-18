//! Searching inside the files of the work tree, for the search window.
//!
//! Built on ripgrep's own pieces: `grep-regex` for the pattern and
//! `grep-searcher` for reading files a line at a time, over the same list of
//! paths the file finder uses (git's ignore rules respected). Results go out a
//! file at a time as they are found, so the first ones are on screen long
//! before a big repository has been read to the end. A newer search makes an
//! older one stop.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::sinks::Lossy;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use serde::{Deserialize, Serialize};

use crate::files;
use crate::state::AppState;

/// How many matching lines are sent before the rest are only counted.
pub const MAX_HITS: usize = 2000;

/// Files larger than this are not searched: they are generated, or data.
const MAX_FILE: u64 = 8 * 1024 * 1024;

/// A line longer than this is cut down around its first match.
const MAX_LINE: usize = 400;

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct SearchOptions {
    /// Match case exactly. Off, `Tax` finds `tax` and `TAX`.
    pub case: bool,
    /// Whole words only.
    pub word: bool,
    /// The query is a regular expression rather than plain text.
    pub regex: bool,
    /// Only the files the working tree panel lists as changed.
    pub changed_only: bool,
    /// File masks, `*.php` or `*.php, *.vue`. A mask with a `/` in it is
    /// matched against the whole path, one without against the file name.
    pub glob: String,
    /// Only the files in this folder, relative to the work tree. A folder git
    /// ignores (`vendor/magento/module-catalog`, `generated`) is searched
    /// whole, whatever `ignored` and `dependencies` say.
    pub scope: String,
    /// The files git ignores too, outside the dependencies, after the
    /// project's.
    pub ignored: bool,
    /// The files in `vendor/` and `node_modules/` too, last.
    pub dependencies: bool,
}

/// One matching line.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LineHit {
    /// Counted from 1, the way an editor counts.
    pub line: u64,
    pub text: String,
    /// Where the matches are in `text`, in UTF-16 units, which is what
    /// JavaScript counts a string in.
    pub ranges: Vec<(u32, u32)>,
}

/// Every matching line in one file.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FileHits {
    pub request: u64,
    pub path: String,
    pub hits: Vec<LineHit>,
}

/// How a search ended.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct SearchDone {
    pub request: u64,
    /// Matching lines sent.
    pub matches: usize,
    /// Files they were in.
    pub files: usize,
    /// Matching lines found past the cap and not sent.
    pub more: usize,
    /// The cap was reached with files still to read, and they were left
    /// unread: `more` counts only what the last file read had past it.
    pub capped: bool,
    /// How many files were looked at.
    pub searched: usize,
    /// A newer search took over before this one finished.
    pub cancelled: bool,
    /// The pattern did not compile, said in words.
    pub error: Option<String>,
}

pub enum Event {
    Hits(FileHits),
    Done(SearchDone),
}

/// The event names the window listens on.
pub const HIT_EVENT: &str = "search:hit";
pub const DONE_EVENT: &str = "search:done";

/// The newest search per work tree.
fn latest(root: &Path) -> Arc<AtomicU64> {
    static LATEST: OnceLock<Mutex<HashMap<PathBuf, Arc<AtomicU64>>>> = OnceLock::new();
    let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut all = LATEST
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    all.entry(key).or_default().clone()
}

/// Marks `request` as the newest search, so any older one still running stops.
pub fn claim(root: &Path, request: u64) {
    latest(root).fetch_max(request, Ordering::SeqCst);
}

/// The paths the working tree panel lists: staged, unstaged and conflicted,
/// less the deletions, which have nothing on disk to search.
pub fn changed_paths(state: &AppState) -> Result<HashSet<String>, String> {
    let status = crate::refs::status(state)?;
    let mut out = HashSet::new();
    for entry in status.staged.iter().chain(&status.unstaged) {
        if entry.kind != "deleted" {
            out.insert(entry.path.clone());
        }
    }
    out.extend(status.conflicted);
    Ok(out)
}

fn matcher(query: &str, options: &SearchOptions) -> Result<RegexMatcher, String> {
    RegexMatcherBuilder::new()
        .case_insensitive(!options.case)
        .word(options.word)
        .fixed_strings(!options.regex)
        // `^` and `$` at each line, and before a `\r\n` too, in a CRLF file.
        .multi_line(true)
        .crlf(true)
        .line_terminator(Some(b'\n'))
        .build(query)
        .map_err(|e| {
            let text = e.to_string();
            // The regex crate's message is several lines with a caret drawn
            // under the problem; the last line says what it is.
            let last = text.lines().rev().find(|line| !line.trim().is_empty());
            last.unwrap_or(&text).trim().to_string()
        })
}

/// The masks as one set: `(names, paths)`. Empty when there are none.
fn masks(glob: &str) -> Result<Option<(GlobSet, GlobSet)>, String> {
    let parts: Vec<&str> = glob
        .split([',', ' ', ';'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return Ok(None);
    }
    let mut names = GlobSetBuilder::new();
    let mut paths = GlobSetBuilder::new();
    for part in parts {
        let glob = GlobBuilder::new(part)
            .literal_separator(true)
            .build()
            .map_err(|e| format!("{part} is not a file mask: {e}"))?;
        if part.contains('/') {
            paths.add(glob);
        } else {
            names.add(glob);
        }
    }
    Ok(Some((
        names.build().map_err(|e| e.to_string())?,
        paths.build().map_err(|e| e.to_string())?,
    )))
}

/// Where each byte offset of `text` falls in UTF-16 units.
fn utf16_at(text: &str, byte: usize) -> u32 {
    text[..byte.min(text.len())]
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum()
}

/// The line as it is sent: no line ending, and cut to a readable length
/// around the first match. Returns the text and the byte offset it starts at.
fn trimmed(line: &str, first: usize) -> (&str, usize) {
    let line = line.trim_end_matches(['\n', '\r']);
    if line.len() <= MAX_LINE {
        return (line, 0);
    }
    let mut start = first.saturating_sub(MAX_LINE / 4);
    while !line.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (start + MAX_LINE).min(line.len());
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    (&line[start..end], start)
}

/// Searches the work tree at `root`, sending what it finds to `emit`.
///
/// `changed` limits the search to those paths when it is given. The final
/// [`SearchDone`] is both sent and returned.
pub fn search(
    root: &Path,
    query: &str,
    options: &SearchOptions,
    request: u64,
    changed: Option<HashSet<String>>,
    mut emit: impl FnMut(Event),
) -> SearchDone {
    let newest = latest(root);
    newest.fetch_max(request, Ordering::SeqCst);
    let superseded = || request < newest.load(Ordering::SeqCst);
    let mut done = SearchDone {
        request,
        ..SearchDone::default()
    };
    let finish = |done: SearchDone, emit: &mut dyn FnMut(Event)| {
        emit(Event::Done(done.clone()));
        done
    };

    if query.is_empty() {
        return finish(done, &mut emit);
    }
    let matcher = match matcher(query, options) {
        Ok(matcher) => matcher,
        Err(error) => {
            done.error = Some(error);
            return finish(done, &mut emit);
        }
    };
    let masks = match masks(&options.glob) {
        Ok(masks) => masks,
        Err(error) => {
            done.error = Some(error);
            return finish(done, &mut emit);
        }
    };
    let paths = match paths_to_search(root, options, &superseded) {
        Ok(Some(paths)) => paths,
        Ok(None) => {
            done.cancelled = true;
            return finish(done, &mut emit);
        }
        Err(error) => {
            done.error = Some(error);
            return finish(done, &mut emit);
        }
    };
    let Ok(base) = root.canonicalize() else {
        done.error = Some("Could not read the work tree".into());
        return finish(done, &mut emit);
    };

    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .line_number(true)
        .build();

    for path in paths.iter() {
        if superseded() {
            done.cancelled = true;
            return finish(done, &mut emit);
        }
        // Past the cap nothing more is sent, and reading the rest of the
        // tree only to put a number on it kept the disk busy for minutes on
        // a common word.
        if done.matches >= MAX_HITS {
            done.capped = true;
            break;
        }
        if let Some(changed) = &changed {
            if !changed.contains(path) {
                continue;
            }
        }
        if let Some((names, full)) = &masks {
            let name = path.rsplit('/').next().unwrap_or(path);
            if !names.is_match(name) && !full.is_match(path) {
                continue;
            }
        }
        let Some(file) = searchable(&base, path) else {
            continue;
        };
        done.searched += 1;

        let room = MAX_HITS.saturating_sub(done.matches);
        let (hits, counted) = hits_in(&mut searcher, &matcher, &file, room);
        done.more += counted;
        if !hits.is_empty() {
            done.matches += hits.len();
            done.files += 1;
            emit(Event::Hits(FileHits {
                request,
                path: path.clone(),
                hits,
            }));
        }
    }
    finish(done, &mut emit)
}

/// The files a search reads, in the order it reads them: the project's,
/// then (when asked) what git ignores, then the dependencies, and in each the
/// tests after the rest, so they only take what room the cap leaves.
///
/// With a `scope`, only what is in that folder. A folder git ignores, or
/// whose contents it ignores (`vendor/acme/kit`, Magento's `generated` under
/// `/generated/*`), is searched whole with no toggle: from the kept list of
/// dependencies or ignored files when there is a complete one, else by
/// walking just that folder (and the kept list is built meanwhile for next
/// time). `None` when `stop` said to give up during that walk.
pub fn paths_to_search(
    root: &Path,
    options: &SearchOptions,
    stop: &dyn Fn() -> bool,
) -> Result<Option<Vec<String>>, String> {
    let asked = options.scope.trim_matches('/');
    // The folder as it really is: `vendor/..` is the root, not a dependency.
    let scope = if asked.is_empty() {
        String::new()
    } else {
        let base = root
            .canonicalize()
            .map_err(|e| format!("Could not read the work tree: {e}"))?;
        let dir = files::resolve(&base, asked, true)?;
        if !dir.is_dir() {
            return Err(format!("{asked} is not a folder"));
        }
        dir.strip_prefix(&base)
            .map(files::slashed)
            .map_err(|_| format!("{asked} is outside the work tree"))?
    };
    let prefix = if scope.is_empty() {
        String::new()
    } else {
        format!("{scope}/")
    };
    let mut ranked: Vec<(u8, String)> = Vec::new();
    let take = |ranked: &mut Vec<(u8, String)>, source: u8, list: &[String]| {
        for path in list {
            if prefix.is_empty() || path.starts_with(&prefix) {
                ranked.push((
                    source * 2 + u8::from(files::is_test_path(path)),
                    path.clone(),
                ));
            }
        }
    };
    take(&mut ranked, 0, &files::all_files(root)?);
    let dependency = !scope.is_empty() && files::in_dependency(&scope);
    if dependency || (!scope.is_empty() && files::folder_ignored(root, &scope)?) {
        let kept = if dependency {
            files::dependency_walk_kept(root)
        } else if files::left_out_of_ignored(&scope) {
            None
        } else {
            files::ignored_walk_kept(root)
        };
        match kept {
            Some(walked) if walked.complete => take(&mut ranked, 1, &walked.paths),
            _ => {
                if !files::left_out_of_ignored(&scope) || dependency {
                    let root = root.to_path_buf();
                    std::thread::spawn(move || {
                        let _ = if dependency {
                            files::dependency_files(&root)
                        } else {
                            files::ignored_files(&root)
                        };
                    });
                }
                let Some(found) = files::files_under(root, &scope, stop)? else {
                    return Ok(None);
                };
                let seen: HashSet<String> = ranked.iter().map(|(_, path)| path.clone()).collect();
                let fresh: Vec<String> = found
                    .into_iter()
                    .filter(|path| !seen.contains(path))
                    .collect();
                take(&mut ranked, 1, &fresh);
            }
        }
    } else {
        if options.ignored {
            take(&mut ranked, 1, &files::ignored_files(root)?);
        }
        if options.dependencies {
            take(&mut ranked, 2, &files::dependency_files(root)?);
        }
    }
    // Stable: each part keeps its own (sorted) order.
    ranked.sort_by_key(|(rank, _)| *rank);
    Ok(Some(ranked.into_iter().map(|(_, path)| path).collect()))
}

/// The file to read for a listed path, when it is one worth reading: a
/// regular file no bigger than [`MAX_FILE`]. A named pipe would block the
/// read for good, and a link is followed only when it stays in the work tree
/// (a committed `notes.txt -> ~/.ssh/config` is not searched).
fn searchable(base: &Path, path: &str) -> Option<std::path::PathBuf> {
    let file = base.join(path);
    let own = std::fs::symlink_metadata(&file).ok()?;
    let file = if own.file_type().is_symlink() {
        files::resolve(base, path, false).ok()?
    } else {
        file
    };
    let meta = std::fs::metadata(&file).ok()?;
    (meta.is_file() && meta.len() <= MAX_FILE).then_some(file)
}

/// Every matching line in one file, up to `room` of them; the rest are only
/// counted.
fn hits_in(
    searcher: &mut grep_searcher::Searcher,
    matcher: &RegexMatcher,
    file: &Path,
    room: usize,
) -> (Vec<LineHit>, usize) {
    let mut hits: Vec<LineHit> = Vec::new();
    let mut counted = 0usize;
    let _ = searcher.search_path(
        matcher,
        file,
        Lossy(|number, line| {
            if hits.len() >= room {
                counted += 1;
                return Ok(true);
            }
            let mut found: Vec<(usize, usize)> = Vec::new();
            let _ = matcher.find_iter(line.as_bytes(), |m| {
                found.push((m.start(), m.end()));
                true
            });
            let first = found.first().map(|m| m.0).unwrap_or(0);
            let (text, start) = trimmed(line, first);
            let ranges = found
                .iter()
                .filter(|(from, to)| *from >= start && *to <= start + text.len())
                .map(|(from, to)| (utf16_at(text, from - start), utf16_at(text, to - start)))
                .collect();
            hits.push(LineHit {
                line: number,
                text: text.to_string(),
                ranges,
            });
            Ok(true)
        }),
    );
    (hits, counted)
}

/// The same search over one file, for the search window to bring that file's
/// hits up to date after it was edited in the preview, without running the
/// whole search again. The file's masks and "changed only" are not asked
/// again: the file was in the results already.
pub fn search_file(
    root: &Path,
    relative: &str,
    query: &str,
    options: &SearchOptions,
) -> Result<Vec<LineHit>, String> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let matcher = matcher(query, options)?;
    let file = files::resolve(root, relative, false)?;
    if !std::fs::metadata(&file)
        .map(|meta| meta.is_file() && meta.len() <= MAX_FILE)
        .unwrap_or(false)
    {
        return Ok(Vec::new());
    }
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .line_number(true)
        .build();
    Ok(hits_in(&mut searcher, &matcher, &file, MAX_HITS).0)
}

/// Which of `words` appear as whole words anywhere in the work tree's files,
/// in one pass over them. `skip` leaves one file out: the caller looks at the
/// copy in the editor itself. Every file git does not ignore is read, whatever
/// its language, so a name in a route, a template or a config file counts.
/// Without `case`, the names come back lowercased.
///
/// `None` when `stop` said to give up part-way.
pub fn words_found(
    root: &Path,
    words: &[String],
    case: bool,
    skip: Option<&str>,
    stop: &dyn Fn() -> bool,
) -> Result<Option<HashSet<String>>, String> {
    let mut seen: HashSet<String> = HashSet::new();
    let wanted: HashSet<String> = words
        .iter()
        .map(|word| {
            if case {
                word.clone()
            } else {
                word.to_lowercase()
            }
        })
        .collect();
    if wanted.is_empty() {
        return Ok(Some(seen));
    }
    let pattern = wanted
        .iter()
        .map(|word| regex_syntax_escape(word))
        .collect::<Vec<_>>()
        .join("|");
    let options = SearchOptions {
        case,
        word: true,
        regex: true,
        ..SearchOptions::default()
    };
    let matcher = matcher(&format!("(?:{pattern})"), &options)?;
    let base = root
        .canonicalize()
        .map_err(|e| format!("Could not read the work tree: {e}"))?;
    let paths = files::all_files(&base)?;
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .build();
    for path in paths.iter() {
        if stop() {
            return Ok(None);
        }
        if skip == Some(path.as_str()) {
            continue;
        }
        let Some(file) = searchable(&base, path) else {
            continue;
        };
        let _ = searcher.search_path(
            &matcher,
            &file,
            Lossy(|_, line| {
                let _ = matcher.find_iter(line.as_bytes(), |m| {
                    let found = &line[m.start()..m.end()];
                    // Word mode may take the edges along; the name is inside.
                    let found = found.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_'));
                    seen.insert(if case {
                        found.to_string()
                    } else {
                        found.to_lowercase()
                    });
                    true
                });
                Ok(seen.len() < wanted.len())
            }),
        );
        if wanted.iter().all(|word| seen.contains(word)) {
            break;
        }
    }
    seen.retain(|word| wanted.contains(word));
    Ok(Some(seen))
}

/// A name as a regex that matches only itself.
fn regex_syntax_escape(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_offsets_the_way_javascript_does() {
        // `é` is two bytes and one unit; the emoji is four bytes and two units.
        let text = "é😀x";
        assert_eq!(utf16_at(text, 2), 1);
        assert_eq!(utf16_at(text, 6), 3);
    }

    #[test]
    fn reads_file_masks_by_name_or_by_path() {
        let (names, paths) = masks("*.php, app/**/*.vue").unwrap().unwrap();
        assert!(names.is_match("Tax.php"));
        assert!(!names.is_match("Tax.vue"));
        assert!(paths.is_match("app/pages/x.vue"));
        assert!(masks("  ").unwrap().is_none());
    }

    #[test]
    fn cuts_a_long_line_round_its_match() {
        let line = format!("{}needle{}", "a".repeat(1000), "b".repeat(1000));
        let (text, start) = trimmed(&line, 1000);
        assert!(text.contains("needle"));
        assert!(text.len() <= MAX_LINE);
        assert!(start > 0);
    }
}
