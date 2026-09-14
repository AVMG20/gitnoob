//! Putting a screenshot in a comment without leaving the window.
//!
//! Dragging an image onto a comment box is how a review gets a screenshot on
//! every forge's own site, and doing it here meant going to the browser, doing
//! it there, and copying the markdown back. Both forges will take the file
//! over their API with the access token the profile already holds, so it can
//! be done from here instead.
//!
//! The two are not the same request and are not the same age. GitLab has had a
//! documented project uploads endpoint for years. GitHub had nothing until it
//! shipped one for its own CLI, which is first-party and token-authenticated
//! but is not in the REST reference and carries no version contract — so it is
//! called exactly the way `gh` calls it, and a failure says plainly what went
//! wrong rather than pretending the feature is not there.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::ForgeKind;
use crate::state::AppState;

use super::http::{api_base, client, client_for, describe, fetch, prepare, urlencode};

/// The largest file that is worth pushing through, in bytes.
///
/// GitHub's own limit for an image is ten megabytes and GitLab's is set per
/// instance, so ten is the number both agree on. It is also about as much as
/// should cross to the backend in one go: the bytes travel as text, which
/// costs a third again on top.
const SIZE_LIMIT: usize = 10 * 1024 * 1024;

/// What can be attached, and what to call it when the forge asks.
///
/// An allowlist rather than whatever the window said the file was: the type is
/// read from a dropped file by the browser, and a forge that is handed
/// something else answers with its own wording, which is worse than ours.
const KINDS: [(&str, &str); 5] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
];

/// The content type for a name and whatever the window claimed, or nothing.
fn kind_of(name: &str, claimed: &str) -> Option<&'static str> {
    let claimed = claimed.split(';').next().unwrap_or("").trim().to_lowercase();
    if let Some((_, mime)) = KINDS.iter().find(|(_, mime)| *mime == claimed) {
        return Some(mime);
    }
    // A file dragged out of some applications arrives with no type at all, so
    // the name is the fallback rather than the first answer.
    let extension = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let extension = if extension == "jpeg" { "jpg" } else { extension.as_str() };
    KINDS
        .iter()
        .find(|(suffix, _)| *suffix == extension)
        .map(|(_, mime)| *mime)
}

/// Sends one file to the forge and answers with the address to point at it.
///
/// The markdown is written by the window, because only the window knows what
/// the author typed for alt text.
pub async fn upload_attachment(
    state: &AppState,
    name: &str,
    mime: &str,
    data: &str,
) -> Result<String, String> {
    let mime = kind_of(name, mime)
        .ok_or_else(|| "Only PNG, JPEG, GIF, WebP and SVG images can be attached".to_string())?;
    let bytes = from_base64(data)?;
    if bytes.is_empty() {
        return Err("That file is empty".to_string());
    }
    if bytes.len() > SIZE_LIMIT {
        return Err(format!(
            "That file is {}, and the limit is {}",
            megabytes(bytes.len()),
            megabytes(SIZE_LIMIT)
        ));
    }
    let name = tidy_name(name);

    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    // Two clients: the ordinary one for the small lookup, and one whose
    // deadline is set by how much there is to send for the upload itself.
    let asking = client()?;
    let sending = client_for(bytes.len())?;
    let slug = call.slug.full();

    match call.kind {
        ForgeKind::GitHub => {
            let host = uploads_host(&call.host).ok_or_else(|| {
                "Attaching files is not supported on GitHub Enterprise Server".to_string()
            })?;
            // The endpoint wants the repository's number rather than its name,
            // and nothing else here has ever needed it.
            let repo = fetch(&asking, &call.token, &format!("{base}/repos/{slug}")).await?;
            let id = repo
                .get("id")
                .and_then(Value::as_i64)
                .ok_or_else(|| "Could not work out which repository to attach to".to_string())?;

            let response = sending
                .post(github_url(&host, &name, mime, id))
                .bearer_auth(&call.token)
                .header("Accept", "application/vnd.github+json")
                .header("Content-Type", "application/octet-stream")
                .body(bytes)
                .send()
                .await
                .map_err(failure)?;
            if !response.status().is_success() {
                let refused = response.status() == reqwest::StatusCode::NOT_FOUND;
                let said = describe(response).await;
                // The endpoint answers 404 rather than 403 when the token
                // cannot write, so the status alone names the wrong problem —
                // but its own words are kept, because the day the route moves
                // is the day everyone is told to check their token instead.
                return Err(if refused {
                    format!("{said} — attaching files needs push access to this repository")
                } else {
                    said
                });
            }
            let asset: Value = response.json().await.map_err(|e| e.to_string())?;
            // `url` is what the endpoint answers with and what GitHub's own
            // client reads. The others are there because this route is not in
            // the REST reference and promises nothing about its shape.
            read_url(&asset, &["url", "href", "asset_url", "browser_download_url"])
                .ok_or_else(|| "The upload finished without an address".to_string())
        }
        ForgeKind::GitLab => {
            let (boundary, body) = multipart(&name, mime, &bytes)?;
            let project = urlencode(&slug);
            let response = sending
                .post(format!("{base}/projects/{project}/uploads"))
                .bearer_auth(&call.token)
                .header(
                    "Content-Type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(body)
                .send()
                .await
                .map_err(failure)?;
            if !response.status().is_success() {
                return Err(describe(response).await);
            }
            let asset: Value = response.json().await.map_err(|e| e.to_string())?;
            gitlab_url(&call.host, &slug, &asset)
                .ok_or_else(|| "The upload finished without an address".to_string())
        }
        ForgeKind::None => Err("No forge configured".to_string()),
    }
}

/// The first of these the answer actually carries.
fn read_url(asset: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| asset.get(key).and_then(Value::as_str))
        .map(str::trim)
        .find(|url| url.starts_with("https://"))
        .map(str::to_string)
}

/// What sending the file failed with, said in words rather than in reqwest's.
///
/// The two that matter are the two a big file on a small connection actually
/// hits; anything else is rare enough to be worth reading verbatim.
fn failure(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "The upload timed out — the file may be too large for this connection".to_string()
    } else if error.is_connect() {
        "Could not reach the forge".to_string()
    } else {
        error.to_string()
    }
}

/// Where GitHub takes the bytes, which is not where it takes anything else.
///
/// Enterprise Server has no such host, which is why this can answer nothing;
/// a data residency tenant on `ghe.com` has one.
fn uploads_host(host: &str) -> Option<String> {
    if host == "github.com" || host.ends_with(".ghe.com") {
        Some(format!("uploads.{host}"))
    } else {
        None
    }
}

fn github_url(host: &str, name: &str, mime: &str, repository: i64) -> String {
    format!(
        "https://{host}/user-attachments/assets?name={}&content_type={}&repository_id={repository}",
        urlencode(name),
        urlencode(mime)
    )
}

/// The address GitLab's answer points at, made absolute.
///
/// Both fields it can answer with are relative — one to the site, one to the
/// project — and a relative address in a body only resolves where the body is
/// being read. The comment is read here as well as there.
fn gitlab_url(host: &str, slug: &str, asset: &Value) -> Option<String> {
    let read = |key: &str| {
        asset
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| path.starts_with('/'))
    };
    // `full_path` is relative to the site and arrived in GitLab 17.1; `url` is
    // relative to the project and has always been there.
    if let Some(path) = read("full_path") {
        return Some(format!("https://{host}{path}"));
    }
    read("url").map(|path| format!("https://{host}/{slug}{path}"))
}

/// A file name that cannot break out of the header it is written into.
fn tidy_name(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim()
        .trim_matches('.');
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
        .take(100)
        .collect();
    if cleaned.is_empty() {
        "image.png".to_string()
    } else {
        cleaned
    }
}

/// The one-part form GitLab wants, written out rather than pulled in.
///
/// Adding reqwest's multipart feature to send four headers and a file would
/// cost two crates in the build for something with no decisions in it. The
/// boundary comes from the file's own digest, so a body cannot contain it
/// without containing its own hash; the loop after it is there because "cannot
/// without" is not "cannot".
fn multipart(name: &str, mime: &str, bytes: &[u8]) -> Result<(String, Vec<u8>), String> {
    // Half the digest: a boundary may be seventy characters and no more, and
    // sixteen bytes of hash is far past the point where guessing one matters.
    // The salt is only there so that "no body contains this" can be settled by
    // looking rather than by arguing about how unlikely it would be.
    let mut boundary = String::new();
    for salt in 0..8u8 {
        let mut hash = Sha256::new();
        hash.update([salt]);
        hash.update(bytes);
        let candidate = format!(
            "gitnoob{}",
            hash.finalize()
                .iter()
                .take(16)
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        if !holds(bytes, candidate.as_bytes()) {
            boundary = candidate;
            break;
        }
    }
    if boundary.is_empty() {
        return Err("That file could not be sent as it is".to_string());
    }

    let head = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n\
         Content-Type: {mime}\r\n\r\n"
    );
    let tail = format!("\r\n--{boundary}--\r\n");

    let mut body = Vec::with_capacity(head.len() + bytes.len() + tail.len());
    body.extend_from_slice(head.as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(tail.as_bytes());
    Ok((boundary, body))
}

fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|run| run == needle)
}

fn megabytes(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// Base64 back to bytes: the window sends the file as text, because the call
/// it travels on carries named arguments as JSON and an array of ten million
/// numbers is not a reasonable way to write a file down.
fn from_base64(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut packed: u32 = 0;
    let mut held: u32 = 0;
    for byte in text.bytes() {
        // Padding says how the last group ends, which the bit count already
        // knows; whitespace is what a line-wrapped encoder leaves behind.
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        let value = sextet(byte).ok_or_else(|| "That file could not be read".to_string())?;
        packed = (packed << 6) | u32::from(value);
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push(((packed >> held) & 0xFF) as u8);
        }
    }
    Ok(out)
}

fn sextet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_file_back_out_of_the_text_it_travelled_as() {
        // "hello" and the three lengths that end a group differently.
        assert_eq!(from_base64("aGVsbG8=").expect("read"), b"hello");
        assert_eq!(from_base64("YQ==").expect("read"), b"a");
        assert_eq!(from_base64("YWI=").expect("read"), b"ab");
        assert_eq!(from_base64("YWJj").expect("read"), b"abc");
        // An encoder that wrapped its lines is still an encoder.
        assert_eq!(from_base64("aGVs\r\nbG8=").expect("read"), b"hello");
        assert_eq!(from_base64("").expect("read"), Vec::<u8>::new());
        // Every byte value, round tripped through the pair.
        let all: Vec<u8> = (0..=255u8).collect();
        let text = base64_for_test(&all);
        assert_eq!(from_base64(&text).expect("read"), all);
        assert!(from_base64("not valid!").is_err());
    }

    /// The encoder from `avatar`, copied rather than shared: it exists there to
    /// build a data url and this exists here to check the decoder against
    /// something that is not itself.
    fn base64_for_test(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
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

    #[test]
    fn names_the_type_from_what_was_claimed_or_from_the_name() {
        assert_eq!(kind_of("a.png", "image/png"), Some("image/png"));
        // Some windows hand over a charset with it.
        assert_eq!(kind_of("a.png", "image/png; charset=binary"), Some("image/png"));
        // Dragged from an application that said nothing about the type.
        assert_eq!(kind_of("shot.JPEG", ""), Some("image/jpeg"));
        assert_eq!(kind_of("shot.jpg", ""), Some("image/jpeg"));
        assert_eq!(kind_of("d.WEBP", ""), Some("image/webp"));
        assert_eq!(kind_of("logo.svg", ""), Some("image/svg+xml"));
        // Not an image, whichever way round it is asked.
        assert_eq!(kind_of("notes.pdf", "application/pdf"), None);
        assert_eq!(kind_of("clip.mp4", "video/mp4"), None);
        assert_eq!(kind_of("script", ""), None);
    }

    #[test]
    fn points_github_at_the_upload_host_and_says_no_to_enterprise_server() {
        assert_eq!(
            uploads_host("github.com").as_deref(),
            Some("uploads.github.com")
        );
        assert_eq!(
            uploads_host("acme.ghe.com").as_deref(),
            Some("uploads.acme.ghe.com")
        );
        assert_eq!(uploads_host("github.acme.dev"), None);

        assert_eq!(
            github_url("uploads.github.com", "a shot.png", "image/png", 42),
            "https://uploads.github.com/user-attachments/assets\
             ?name=a%20shot.png&content_type=image%2Fpng&repository_id=42"
        );
    }

    #[test]
    fn reads_githubs_address_whichever_of_its_names_it_arrives_under() {
        let keys = ["url", "href", "asset_url", "browser_download_url"];
        let asset = serde_json::json!({ "url": "https://github.com/user-attachments/assets/a" });
        assert_eq!(
            read_url(&asset, &keys).as_deref(),
            Some("https://github.com/user-attachments/assets/a")
        );
        // The route is not in the reference and promises nothing about its
        // shape, so a different name is read rather than reported as failure.
        let renamed = serde_json::json!({ "href": "https://github.com/user-attachments/assets/b" });
        assert_eq!(
            read_url(&renamed, &keys).as_deref(),
            Some("https://github.com/user-attachments/assets/b")
        );
        // Order is the order asked for, not the order the answer happens in.
        let both = serde_json::json!({ "href": "https://b.example", "url": "https://a.example" });
        assert_eq!(read_url(&both, &keys).as_deref(), Some("https://a.example"));
        assert_eq!(read_url(&serde_json::json!({}), &keys), None);
        assert_eq!(read_url(&serde_json::json!({ "url": "" }), &keys), None);
        // Nothing that is not an address this window would fetch.
        let odd = serde_json::json!({ "url": "javascript:alert(1)" });
        assert_eq!(read_url(&odd, &keys), None);
    }

    #[test]
    fn makes_gitlabs_relative_answer_an_address_that_resolves_anywhere() {
        let modern = serde_json::json!({
            "url": "/uploads/66db/dk.png",
            "full_path": "/-/project/1234/uploads/66db/dk.png",
            "markdown": "![dk](/uploads/66db/dk.png)"
        });
        assert_eq!(
            gitlab_url("gitlab.com", "group/app", &modern).as_deref(),
            Some("https://gitlab.com/-/project/1234/uploads/66db/dk.png")
        );

        // Before 17.1 there was only the project relative one.
        let older = serde_json::json!({ "url": "/uploads/66db/dk.png" });
        assert_eq!(
            gitlab_url("gitlab.example.com", "group/sub/app", &older).as_deref(),
            Some("https://gitlab.example.com/group/sub/app/uploads/66db/dk.png")
        );

        assert_eq!(gitlab_url("gitlab.com", "a/b", &serde_json::json!({})), None);
        // Anything that is not a path is not an answer.
        let odd = serde_json::json!({ "url": "https://evil.example/x.png" });
        assert_eq!(gitlab_url("gitlab.com", "a/b", &odd), None);
    }

    #[test]
    fn writes_a_form_the_boundary_cannot_appear_inside() {
        let (boundary, body) = multipart("a shot.png", "image/png", b"\x89PNG\r\n").expect("form");
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with(&format!("--{boundary}\r\n")));
        assert!(
            text.contains("Content-Disposition: form-data; name=\"file\"; filename=\"a shot.png\"\r\n")
        );
        assert!(text.contains("Content-Type: image/png\r\n\r\n"));
        assert!(text.ends_with(&format!("\r\n--{boundary}--\r\n")));
        assert!(body.windows(6).any(|run| run == b"\x89PNG\r\n"));
        // A boundary is seventy characters at the outside, and the two dashes
        // in front of it on the wire count towards nothing but the line.
        assert!(boundary.len() <= 70, "{} characters", boundary.len());
        assert!(boundary.bytes().all(|byte| byte.is_ascii_alphanumeric()));

        // Whatever the file holds, the boundary is not in it.
        for file in [b"".as_slice(), b"x", b"--gitnoob", &vec![0xffu8; 4096]] {
            let (boundary, _) = multipart("a.png", "image/png", file).expect("form");
            assert!(!holds(file, boundary.as_bytes()));
        }

        // Which rests on the search, so the search is checked on its own.
        assert!(holds(b"ab--gitnoobcd", b"--gitnoob"));
        assert!(!holds(b"ab--gitnoocd", b"--gitnoob"));
        assert!(!holds(b"short", b"a longer needle"));
        assert!(holds(b"exact", b"exact"));
    }

    #[test]
    fn keeps_a_file_name_inside_the_header_it_is_written_into() {
        assert_eq!(tidy_name("C:\\shots\\a.png"), "a.png");
        assert_eq!(tidy_name("/home/robin/a.png"), "a.png");
        // A quote or a newline would end the header early, so neither is a
        // character a name is allowed to be made of.
        assert_eq!(tidy_name("a\";\r\nContent-Type: x;y=\".png"), "a;Content-Type: x;y=.png");
        assert_eq!(tidy_name("sh\"ot.png"), "shot.png");
        // A separator anywhere means everything before it was a directory.
        assert_eq!(tidy_name("dir/a\".png"), "a.png");
        assert_eq!(tidy_name("   "), "image.png");
        assert_eq!(tidy_name(""), "image.png");
        assert_eq!(tidy_name("....").len(), 9, "nothing but dots is not a name");
    }
}
