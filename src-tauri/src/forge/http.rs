//! Reaching a forge at all: whose account, which host, and the request.
//!
//! Nothing here knows what a review is. It knows how to be pointed at an API
//! and how to bring a page of JSON back.

use crate::config::{self, ForgeKind};
use crate::state::AppState;

use super::*;

/// Everything a request needs, gathered before any `await` so no `!Send` git2
/// handle is held across it.
pub(super) struct Call {
    pub(super) kind: ForgeKind,
    pub(super) host: String,
    pub(super) token: String,
    pub(super) slug: RepoSlug,
    pub(super) current_branch: Option<String>,
}

/// The forge, host and token of the active profile: everything a request needs
/// that has nothing to do with which repository happens to be open.
pub fn account(state: &AppState) -> Result<(ForgeKind, String, String), String> {
    let config = state.config();
    let profile = config
        .active()
        .ok_or_else(|| "No profile is active".to_string())?;
    if profile.forge == ForgeKind::None {
        return Err("This profile has no forge configured".to_string());
    }
    let token = config::secret_get(&config::forge_key(&profile.id))
        .ok_or_else(|| format!("No access token stored for the {} profile", profile.name))?;
    let host = if profile.host.is_empty() {
        profile.forge.default_host().to_string()
    } else {
        profile.host.clone()
    };
    Ok((profile.forge, host, token))
}

pub(super) fn prepare(state: &AppState) -> Result<Call, String> {
    let (kind, host, token) = account(state)?;
    let slug = remote_slug(state)
        .ok_or_else(|| "Could not work out the project from the git remote".to_string())?;
    let current_branch = state.repo().ok().and_then(|repo| {
        repo.head()
            .ok()
            .and_then(|head| head.shorthand().map(|s| s.to_string()))
    });

    Ok(Call {
        kind,
        host,
        token,
        slug,
        current_branch,
    })
}

pub(super) fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("gitnoob/0.1")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// GitHub's API lives on a separate host; GitLab's sits under the web host.
pub(super) fn api_base(kind: ForgeKind, host: &str) -> String {
    match kind {
        ForgeKind::GitHub if host == "github.com" => "https://api.github.com".to_string(),
        // GitHub Enterprise.
        ForgeKind::GitHub => format!("https://{host}/api/v3"),
        ForgeKind::GitLab => format!("https://{host}/api/v4"),
        ForgeKind::None => String::new(),
    }
}

/// The forge's own page for creating an access token, with the scopes this
/// app needs already ticked and a name filled in.
///
/// A real OAuth sign-in would need an application registered with each forge;
/// until there is one, a link is the same number of clicks without that
/// dependency. It only opens a page — the token still has to be pasted back.
pub fn token_url(kind: ForgeKind, host: &str) -> Option<String> {
    let host = if host.trim().is_empty() {
        kind.default_host()
    } else {
        host.trim()
    };
    match kind {
        ForgeKind::GitHub => Some(format!(
            "https://{host}/settings/tokens/new?description=gitnoob&scopes=repo,read:org,read:user"
        )),
        ForgeKind::GitLab => Some(format!(
            "https://{host}/-/user_settings/personal_access_tokens?name=gitnoob&scopes=api,read_user"
        )),
        ForgeKind::None => None,
    }
}

/// Fetches a paginated collection of JSON items until a short page arrives.
///
/// Ten pages of a hundred is the same ceiling `repos` walks to. The first page
/// failing means the answer cannot be given at all; a later one failing leaves
/// what already arrived rather than nothing.
pub(super) async fn paged(
    http: &reqwest::Client,
    token: &str,
    url_for_page: impl Fn(usize) -> String,
) -> Result<Vec<serde_json::Value>, String> {
    const PER_PAGE: usize = 100;
    const PAGE_LIMIT: usize = 10;
    let mut out = Vec::new();
    for page in 1..=PAGE_LIMIT {
        let response = http
            .get(url_for_page(page))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            if page == 1 {
                return Err(describe(response).await);
            }
            break;
        }
        let mut items: Vec<serde_json::Value> = response.json().await.map_err(|e| e.to_string())?;
        let full = items.len() >= PER_PAGE;
        out.append(&mut items);
        if !full {
            break;
        }
    }
    Ok(out)
}

/// One GET that answers with JSON, or with whatever the forge complained.
pub(super) async fn fetch(
    http: &reqwest::Client,
    token: &str,
    url: &str,
) -> Result<serde_json::Value, String> {
    let response = http
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    response.json().await.map_err(|e| e.to_string())
}

/// Percent-encodes a path for a URL segment while keeping its own slashes:
/// `src/review/pane.ts` stays readable as the path it names.
pub(super) fn urlencode_path(path: &str) -> String {
    path.split('/').map(urlencode).collect::<Vec<_>>().join("/")
}

/// Turns a failed response into a message worth showing, using the forge's own
/// wording where there is one.
pub(super) async fn describe(response: reqwest::Response) -> String {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|body| {
            body.get("message")
                .or_else(|| body.get("error"))
                .or_else(|| body.get("error_description"))
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        })
        .unwrap_or_else(|| text.chars().take(200).collect());

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        format!("{status}: {detail} — check the profile's access token and its scopes")
    } else {
        format!("{status}: {detail}")
    }
}

pub(super) fn string(value: &serde_json::Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        match current.get(key) {
            Some(next) => current = next,
            None => return String::new(),
        }
    }
    current.as_str().unwrap_or("").to_string()
}

/// Percent-encodes the characters that matter for a path segment.
pub fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
