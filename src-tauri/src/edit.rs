//! Reading a file for the editor and saving it back.
//!
//! Saving is plain file I/O, not a git operation: the right panel picks the
//! change up as unstaged the way it would after any editor's save.
//!
//! Three promises, each with a test in `tests/repo.rs`:
//!
//! - **Only inside the work tree.** Every path goes through
//!   [`files::resolve`], so nothing outside the work tree or inside a `.git`
//!   is read or written, symlinks included.
//! - **Nobody's work is lost.** A save names the hash of the file it was
//!   based on, and is refused with a conflict when the file on disk is no
//!   longer that — an agent or another editor wrote it meanwhile. The window
//!   then asks rather than writing over it.
//! - **The bytes stay the file's.** Line endings (CRLF or LF), a UTF-8 byte
//!   order mark and the newline at the end, or its absence, are read off the
//!   file and written back the same way. The editor works in `\n` only, so a
//!   file that mixes endings, or has a lone `\r`, opens read-only: there is
//!   no way to write it back without changing lines nobody touched.
//!
//! A save is atomic: written beside the file and renamed over it, so a crash
//! half-way leaves the old file rather than half of the new one.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::files;
use crate::state::AppState;

/// Past this the file opens read-only: it is generated, or data.
pub const MAX_EDIT: usize = 2 * 1024 * 1024;

/// Past this not even the text is sent: there is nothing to read in it.
const MAX_SHOW: usize = 16 * 1024 * 1024;

const BOM: &[u8] = b"\xEF\xBB\xBF";

const LFS_POINTER: &[u8] = b"version https://git-lfs.github.com/spec/v1";

/// A file as the editor gets it.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct FileRead {
    /// The text with `\n` line endings and no byte order mark.
    pub text: String,
    /// The SHA-256 of the bytes on disk, which a save has to name.
    pub hash: String,
    /// `"crlf"` or `"lf"`: what the file uses, and what a save writes.
    pub eol: String,
    pub bom: bool,
    /// Why it cannot be edited, when it cannot: `binary`, `lfs`, `encoding`,
    /// `large`.
    pub readonly: Option<String>,
    /// Not on disk. Nothing else is filled in.
    pub missing: bool,
}

pub fn hash_of(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What a file's own bytes say about how it is written.
struct Shape {
    crlf: bool,
    bom: bool,
    /// Both endings, or a `\r` on its own: not one way to write it back.
    mixed: bool,
}

fn shape(bytes: &[u8]) -> Shape {
    let bom = bytes.starts_with(BOM);
    let mut crlf = 0usize;
    let mut lf = 0usize;
    let mut lone_cr = 0usize;
    for (at, byte) in bytes.iter().enumerate() {
        match *byte {
            b'\n' if at > 0 && bytes[at - 1] == b'\r' => crlf += 1,
            b'\n' => lf += 1,
            b'\r' if bytes.get(at + 1) != Some(&b'\n') => lone_cr += 1,
            _ => {}
        }
    }
    Shape {
        crlf: crlf > lf,
        bom,
        mixed: (crlf > 0 && lf > 0) || lone_cr > 0,
    }
}

fn binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(8000)].contains(&0)
}

pub fn read_file(state: &AppState, relative: &str) -> Result<FileRead, String> {
    read_file_at(&state.path()?, relative)
}

pub fn read_file_at(root: &Path, relative: &str) -> Result<FileRead, String> {
    let real = files::resolve(root, relative, false)?;
    let meta = match std::fs::metadata(&real) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileRead {
                missing: true,
                eol: "lf".into(),
                ..FileRead::default()
            })
        }
        Err(e) => return Err(format!("Could not read {relative}: {e}")),
    };
    if meta.len() > MAX_SHOW as u64 {
        return too_big(&real, &meta, relative);
    }
    let bytes = match std::fs::read(&real) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileRead {
                missing: true,
                eol: "lf".into(),
                ..FileRead::default()
            })
        }
        Err(e) => return Err(format!("Could not read {relative}: {e}")),
    };
    let hash = hash_of(&bytes);
    let Shape { crlf, bom, mixed } = shape(&bytes);
    let eol = if crlf { "crlf" } else { "lf" }.to_string();

    if binary(&bytes) {
        return Ok(FileRead {
            hash,
            eol,
            bom,
            readonly: Some("binary".into()),
            ..FileRead::default()
        });
    }
    if bytes.starts_with(LFS_POINTER) {
        return Ok(FileRead {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            hash,
            eol,
            bom,
            readonly: Some("lfs".into()),
            missing: false,
        });
    }
    let body = if bom { &bytes[BOM.len()..] } else { &bytes[..] };
    let (text, mut readonly) = match std::str::from_utf8(body) {
        Ok(text) => (text.to_string(), None),
        Err(_) => (
            String::from_utf8_lossy(body).into_owned(),
            Some("encoding".to_string()),
        ),
    };
    if bytes.len() > MAX_EDIT && readonly.is_none() {
        readonly = Some("large".into());
    }
    if mixed && readonly.is_none() {
        readonly = Some("endings".into());
    }
    // A dependency is read, never written: the next `composer install` or
    // `npm install` would put it back anyway.
    if readonly.is_none() && crate::deps::is_dependency(root, relative) {
        readonly = Some("dependency".into());
    }
    // Always `\n` for the editor, whatever is kept on disk.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok(FileRead {
        text,
        hash,
        eol,
        bom,
        readonly,
        missing: false,
    })
}

/// A file past [`MAX_SHOW`]: only its first bytes are looked at, and the
/// "hash" is its size and time, which is all a read-only view needs to see
/// that it changed.
fn too_big(real: &Path, meta: &std::fs::Metadata, relative: &str) -> Result<FileRead, String> {
    use std::io::Read;
    let mut head = vec![0u8; 8000];
    let mut file =
        std::fs::File::open(real).map_err(|e| format!("Could not read {relative}: {e}"))?;
    let got = file
        .read(&mut head)
        .map_err(|e| format!("Could not read {relative}: {e}"))?;
    head.truncate(got);
    let when = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    let Shape { crlf, bom, .. } = shape(&head);
    Ok(FileRead {
        text: String::new(),
        hash: format!("size:{}:{when}", meta.len()),
        eol: if crlf { "crlf" } else { "lf" }.into(),
        bom,
        readonly: Some(if binary(&head) { "binary" } else { "large" }.into()),
        missing: false,
    })
}

/// What a refused save says. The window reads the prefix.
pub const CONFLICT: &str = "Conflict";

/// Writes `text` over the file, if the file is still the one it was read as.
///
/// `expected` is the hash [`read_file`] gave, or the last save returned;
/// `None` for a file that does not exist yet. Returns the hash of what was
/// written, which the next save names.
///
/// `new_shape` is how to write a file that is not there: `(crlf, bom)`, the
/// way it was when it was read, for one deleted under the editor and saved
/// again.
pub fn write_file(
    state: &AppState,
    relative: &str,
    text: &str,
    expected: Option<&str>,
    new_shape: Option<(bool, bool)>,
) -> Result<String, String> {
    write_file_at(&state.path()?, relative, text, expected, new_shape)
}

pub fn write_file_at(
    root: &Path,
    relative: &str,
    text: &str,
    expected: Option<&str>,
    new_shape: Option<(bool, bool)>,
) -> Result<String, String> {
    let real = files::resolve(root, relative, false)?;
    if crate::deps::is_dependency(root, relative) {
        return Err(format!("{relative} is a dependency, and read-only"));
    }
    let current = match std::fs::read(&real) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Could not read {relative}: {e}")),
    };

    let shape = match (&current, expected) {
        (Some(bytes), Some(hash)) if hash_of(bytes) == hash => {
            if binary(bytes) || bytes.starts_with(LFS_POINTER) {
                return Err(format!("{relative} is not a text file"));
            }
            let found = shape(bytes);
            if found.mixed {
                return Err(format!(
                    "{relative} mixes line endings; saving it would change lines nobody edited"
                ));
            }
            found
        }
        (Some(_), _) => {
            return Err(format!(
                "{CONFLICT}: {relative} changed on disk since it was opened"
            ))
        }
        (None, None) => {
            let (crlf, bom) = new_shape.unwrap_or((false, false));
            Shape {
                crlf,
                bom,
                mixed: false,
            }
        }
        (None, Some(_)) => {
            return Err(format!(
                "{CONFLICT}: {relative} was deleted since it was opened"
            ))
        }
    };

    let mut out = Vec::with_capacity(text.len() + 8);
    if shape.bom {
        out.extend_from_slice(BOM);
    }
    if shape.crlf {
        // The editor sends `\n`; anything already `\r\n` stays as it is
        // rather than becoming `\r\r\n`.
        out.extend_from_slice(text.replace("\r\n", "\n").replace('\n', "\r\n").as_bytes());
    } else {
        out.extend_from_slice(text.as_bytes());
    }

    write_atomic(&real, &out).map_err(|e| format!("Could not save {relative}: {e}"))?;
    Ok(hash_of(&out))
}

/// Writes beside the file and renames over it, keeping its permissions.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("no folder"))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = dir.join(format!(
        ".{name}.gitnoob-{}-{}.tmp",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::SeqCst)
    ));
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Ok(meta) = std::fs::metadata(path) {
            std::fs::set_permissions(&temp, meta.permissions())?;
        }
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_crlf_from_lf_by_the_majority() {
        assert!(shape(b"a\r\nb\r\nc\n").crlf);
        assert!(!shape(b"a\nb\nc\r\n").crlf);
        assert!(!shape(b"no newline").crlf);
        assert!(shape(b"\xEF\xBB\xBFx").bom);
        assert!(shape(b"a\r\nb\n").mixed);
        assert!(shape(b"a\rb").mixed);
        assert!(!shape(b"a\r\nb\r\n").mixed);
        assert!(!shape(b"a\nb\n").mixed);
    }
}
