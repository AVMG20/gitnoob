//! An image's bytes, for the viewer: from the work tree, the index, HEAD, a
//! commit or that commit's parent.
//!
//! Reads only: the work tree through `files::resolve`, everything else from
//! libgit2. The extension is only why the window asked; the type is sniffed
//! from the bytes, and anything that does not look like an image comes back
//! with no type. An LFS pointer is said to be one rather than handed over as
//! a broken image, and anything over the cap is only measured.

use std::path::Path;

use base64::Engine;
use git2::{Oid, Repository};
use serde::{Deserialize, Serialize};

use crate::files;
use crate::state::AppState;

/// The most the viewer is handed. Bigger images are measured, not sent.
pub const MAX_IMAGE: u64 = 30 * 1024 * 1024;

const LFS_POINTER: &[u8] = b"version https://git-lfs.github.com/spec/v1";

/// Where to read the image from.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The file on disk.
    Work,
    /// What is staged.
    Index,
    /// The last commit.
    Head,
    /// The commit named with it.
    Commit,
    /// The first parent of the commit named with it.
    Parent,
}

#[derive(Serialize, Default, Debug, PartialEq)]
pub struct ImageRead {
    /// Sniffed from the bytes: `image/png`, `image/svg+xml`… None when the
    /// bytes are not an image the viewer can show.
    pub mime: Option<String>,
    /// In bytes, whether or not they were sent.
    pub size: u64,
    /// Over [`MAX_IMAGE`]: nothing sent.
    pub too_large: bool,
    /// The pointer text when what is there is a Git LFS pointer.
    pub lfs: Option<String>,
    /// Not there on that side: a new or a deleted image.
    pub missing: bool,
    /// The bytes, base64.
    pub data: Option<String>,
}

pub fn image_read(
    state: &AppState,
    path: &str,
    source: Source,
    commit: Option<&str>,
) -> Result<ImageRead, String> {
    image_read_at(&state.path()?, path, source, commit)
}

pub fn image_read_at(
    root: &Path,
    path: &str,
    source: Source,
    commit: Option<&str>,
) -> Result<ImageRead, String> {
    // The same guard as every path from the window, for blobs too: a path
    // that would leave the work tree is no path in it either.
    let real = files::resolve(root, path, false)?;
    let bytes = match source {
        Source::Work => {
            let meta = match std::fs::metadata(&real) {
                Ok(meta) if meta.is_file() => meta,
                Ok(_) => return Err(format!("{path} is not a file")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(missing()),
                Err(e) => return Err(format!("Could not read {path}: {e}")),
            };
            if meta.len() > MAX_IMAGE {
                return Ok(too_large(meta.len()));
            }
            std::fs::read(&real).map_err(|e| format!("Could not read {path}: {e}"))?
        }
        _ => {
            let repo = Repository::open(root).map_err(|e| e.message().to_string())?;
            match blob(&repo, path, source, commit)? {
                Some(oid) => {
                    let blob = repo.find_blob(oid).map_err(|e| e.message().to_string())?;
                    if blob.size() as u64 > MAX_IMAGE {
                        return Ok(too_large(blob.size() as u64));
                    }
                    blob.content().to_vec()
                }
                None => return Ok(missing()),
            }
        }
    };
    Ok(answer(bytes))
}

fn missing() -> ImageRead {
    ImageRead {
        missing: true,
        ..ImageRead::default()
    }
}

fn too_large(size: u64) -> ImageRead {
    ImageRead {
        size,
        too_large: true,
        ..ImageRead::default()
    }
}

fn answer(bytes: Vec<u8>) -> ImageRead {
    let size = bytes.len() as u64;
    if bytes.starts_with(LFS_POINTER) {
        return ImageRead {
            size,
            lfs: Some(String::from_utf8_lossy(&bytes).into_owned()),
            ..ImageRead::default()
        };
    }
    let mime = sniff(&bytes);
    ImageRead {
        data: mime
            .is_some()
            .then(|| base64::engine::general_purpose::STANDARD.encode(&bytes)),
        mime: mime.map(str::to_string),
        size,
        ..ImageRead::default()
    }
}

/// The blob a path has on one side, or None when it has none there.
fn blob(
    repo: &Repository,
    path: &str,
    source: Source,
    commit: Option<&str>,
) -> Result<Option<Oid>, String> {
    let at = Path::new(path);
    let tree = match source {
        Source::Work => return Ok(None),
        Source::Index => {
            let index = repo.index().map_err(|e| e.message().to_string())?;
            return Ok(index.get_path(at, 0).map(|entry| entry.id));
        }
        Source::Head => match repo.head().and_then(|head| head.peel_to_commit()) {
            Ok(head) => head.tree().map_err(|e| e.message().to_string())?,
            // No commits yet: nothing was there before.
            Err(_) => return Ok(None),
        },
        Source::Commit | Source::Parent => {
            let oid = commit.ok_or("Which commit?")?;
            let oid = Oid::from_str(oid).map_err(|_| format!("Not a valid object id: {oid}"))?;
            let found = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
            let found = if source == Source::Parent {
                match found.parent(0) {
                    Ok(parent) => parent,
                    Err(_) => return Ok(None),
                }
            } else {
                found
            };
            found.tree().map_err(|e| e.message().to_string())?
        }
    };
    Ok(tree.get_path(at).ok().map(|entry| entry.id()))
}

/// The image type the bytes say they are, or None.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && matches!(&bytes[8..12], b"avif" | b"avis") {
        return Some("image/avif");
    }
    if bytes.starts_with(b"BM") && bytes.len() > 14 {
        return Some("image/bmp");
    }
    if bytes.starts_with(&[0, 0, 1, 0]) && bytes.len() > 6 {
        return Some("image/x-icon");
    }
    // SVG is text: an <svg element near the start, after any prolog.
    let head = &bytes[..bytes.len().min(4096)];
    let text = String::from_utf8_lossy(head);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if (trimmed.starts_with('<') && text.contains("<svg")) && !text.contains("<html") {
        return Some("image/svg+xml");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_the_type_from_the_bytes_not_the_name() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff(b"GIF89a...."), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"\0\0\0\x1cftypavif...."), Some("image/avif"));
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?>\n<svg xmlns=\"x\"/>"),
            Some("image/svg+xml")
        );
        assert_eq!(sniff(b"hello, not an image"), None);
        assert_eq!(sniff(b"<html><svg/></html>"), None);
    }
}
