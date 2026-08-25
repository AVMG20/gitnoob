//! Pictures for the people in the history.
//!
//! A column of names all in the same colour is read one line at a time; a face
//! is recognised without reading. The picture is looked up from the commit's
//! author email, which is the only thing a git commit records about who wrote
//! it, and the lookup is done once per email per machine.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use crate::config::ForgeKind;
use crate::state::AppState;

/// Wide enough for the row's slot on a 2x display, and no wider.
const SIZE: u32 = 48;

/// The profile menu draws the signed-in face larger than a row's, and on a
/// screen that may be sharper still.
const FACE: u32 = 64;

/// How long to believe that an email has no picture before asking again. A
/// person who signs up for a gravatar today should not have to reinstall to be
/// seen, and a miss costs one request a week either way.
const MISS_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Emails looked up this session: the picture, or `None` for "asked, nothing
/// there". A thousand commits are written by a handful of people, so this
/// answers nearly every call without touching the disk.
static SEEN: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

fn remembered(email: &str) -> Option<Option<String>> {
    SEEN.lock()
        .unwrap()
        .as_ref()
        .and_then(|seen| seen.get(email).cloned())
}

fn remember(email: &str, found: Option<String>) {
    SEEN.lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(email.to_string(), found);
}

/// Finds the picture for one author, as a `data:` URL the window can draw
/// directly.
///
/// `Ok(None)` means the lookup ran and found nothing, which is an answer: the
/// window draws its own initials rather than leaving a hole in the column.
pub async fn find(state: &AppState, email: &str) -> Result<Option<String>, String> {
    let email = email.trim().to_lowercase();
    if email.is_empty() {
        return Ok(None);
    }
    if let Some(known) = remembered(&email) {
        return Ok(known);
    }
    let config = state.config();
    if !config.global.show_avatars {
        return Ok(None);
    }

    let digest = hex(&Sha256::digest(email.as_bytes()));
    let cache = state.config_dir().join("avatars");
    let hit = cache.join(format!("{digest}.img"));
    let miss = cache.join(format!("{digest}.none"));

    if let Ok(bytes) = fs::read(&hit) {
        let url = data_url(&bytes);
        remember(&email, Some(url.clone()));
        return Ok(Some(url));
    }
    if fresh_miss(&miss) {
        remember(&email, None);
        return Ok(None);
    }

    // The profile decides only whether GitLab is worth asking. Everything else
    // is the same wherever the code is hosted.
    let gitlab = config.active().and_then(|profile| match profile.forge {
        ForgeKind::GitLab if !profile.host.is_empty() => Some(profile.host.clone()),
        ForgeKind::GitLab => Some("gitlab.com".to_string()),
        _ => None,
    });

    let found = fetch(&email, &digest, gitlab).await;
    let _ = fs::create_dir_all(&cache);
    match &found {
        Some(bytes) => {
            let _ = fs::write(&hit, bytes);
        }
        None => {
            let _ = fs::write(&miss, b"");
        }
    }

    let url = found.as_deref().map(data_url);
    remember(&email, url.clone());
    Ok(url)
}

/// Fetches one picture from a URL that is already known.
///
/// The author lookup guesses where a face might live from an email address;
/// this is for the other case, where a forge has handed over the link to the
/// signed-in user's own avatar and there is nothing to work out.
pub async fn from_url(url: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent("gitnoob/0.1")
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    image(&client, &sized(url)).await.as_deref().map(data_url)
}

/// Asks for a small copy where the host is known to offer one: the picture is
/// drawn at a couple of dozen pixels and travels to the window as base64.
fn sized(url: &str) -> String {
    if url.contains("avatars.githubusercontent.com") || url.contains("gravatar.com") {
        let joiner = if url.contains('?') { '&' } else { '?' };
        format!("{url}{joiner}s={FACE}")
    } else {
        url.to_string()
    }
}

/// Whether a "nothing there" answer is recent enough to trust.
fn fresh_miss(marker: &Path) -> bool {
    fs::metadata(marker)
        .and_then(|meta| meta.modified())
        .map(|when| SystemTime::now().duration_since(when).unwrap_or_default() < MISS_FOR)
        .unwrap_or(false)
}

/// Asks each source in turn and stops at the first picture.
///
/// GitHub is asked first and costs nothing to guess: an address at
/// `users.noreply.github.com` carries the account it belongs to, so the picture
/// can be addressed without searching for the person. Gravatar comes next
/// because it is what the forges themselves fall back to. GitLab is asked last,
/// and only for a GitLab project, because unlike the others it is told the
/// address in the clear rather than as a hash.
async fn fetch(email: &str, digest: &str, gitlab: Option<String>) -> Option<Vec<u8>> {
    let client = reqwest::Client::builder()
        .user_agent("gitnoob/0.1")
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;

    if let Some(url) = github_url(email) {
        if let Some(bytes) = image(&client, &url).await {
            return Some(bytes);
        }
    }

    // `d=404` asks gravatar for an answer rather than a drawing: for someone it
    // has never heard of, the window makes a better one itself.
    let url = format!("https://www.gravatar.com/avatar/{digest}?s={SIZE}&d=404");
    if let Some(bytes) = image(&client, &url).await {
        return Some(bytes);
    }

    if let Some(host) = gitlab {
        let url = format!("https://{host}/api/v4/avatar?email={email}&size={SIZE}");
        let answer = client
            .get(&url)
            .send()
            .await
            .ok()?
            .json::<serde_json::Value>()
            .await
            .ok()?;
        // GitLab hands back a gravatar URL when it knows nobody by that
        // address, and gravatar has already been asked and missed.
        if let Some(found) = answer.get("avatar_url").and_then(|value| value.as_str()) {
            if !found.contains("gravatar.com") {
                return image(&client, found).await;
            }
        }
    }

    None
}

/// The picture for a GitHub address that names its own account.
///
/// Commits made through GitHub's own interface are authored as
/// `12345+octocat@users.noreply.github.com`, and older ones as
/// `octocat@users.noreply.github.com`. Both forms address a picture directly.
fn github_url(email: &str) -> Option<String> {
    let local = email.strip_suffix("@users.noreply.github.com")?;
    match local.split_once('+') {
        Some((id, _)) if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => {
            Some(format!("https://avatars.githubusercontent.com/u/{id}?s={SIZE}"))
        }
        _ => Some(format!("https://github.com/{local}.png?size={SIZE}")),
    }
}

/// Fetches a URL and keeps the body only if it is an image of a sane size.
async fn image(client: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().await.ok()?.to_vec();
    if bytes.len() > 512 * 1024 || kind(&bytes).is_none() {
        return None;
    }
    Some(bytes)
}

/// The image type, read from the first bytes rather than taken from the header
/// the server claimed: a picture labelled as the wrong type would not be drawn.
fn kind(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

fn data_url(bytes: &[u8]) -> String {
    let mime = kind(bytes).unwrap_or("image/png");
    format!("data:{mime};base64,{}", base64(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Base64, written out rather than pulled in: one table and three lines of
/// arithmetic.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let padded = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let packed =
            u32::from(padded[0]) << 16 | u32::from(padded[1]) << 8 | u32::from(padded[2]);
        for slot in 0..4 {
            if slot <= chunk.len() {
                out.push(TABLE[(packed >> (18 - 6 * slot)) as usize & 0x3F] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_github_noreply_address_names_its_own_picture() {
        assert_eq!(
            github_url("12345+octocat@users.noreply.github.com"),
            Some(format!("https://avatars.githubusercontent.com/u/12345?s={SIZE}"))
        );
        assert_eq!(
            github_url("octocat@users.noreply.github.com"),
            Some(format!("https://github.com/octocat.png?size={SIZE}"))
        );
        assert_eq!(github_url("someone@example.com"), None);
    }

    #[test]
    fn an_image_is_recognised_by_its_first_bytes() {
        assert_eq!(kind(&[0x89, b'P', b'N', b'G', 0x0D]), Some("image/png"));
        assert_eq!(kind(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(kind(b"<html><body>not found"), None);
    }

    #[test]
    fn base64_matches_the_examples_from_the_rfc() {
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
