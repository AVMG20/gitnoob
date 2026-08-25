use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::avatar;
use crate::config::{self, ForgeKind};
use crate::state::AppState;

/// Faces already fetched this run, by profile id. `None` records a profile
/// whose forge had nothing to show, so it is not asked again on every opening.
static FACES: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// The `owner/name` pair a forge API needs, parsed out of a git remote URL.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct RepoSlug {
    pub host: String,
    /// Everything between the host and the repository name. GitLab allows
    /// nested groups, so this can contain slashes.
    pub owner: String,
    pub name: String,
}

impl RepoSlug {
    pub fn full(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

#[derive(Serialize)]
pub struct ForgeStatus {
    pub kind: ForgeKind,
    pub host: String,
    pub has_token: bool,
    /// The account the token belongs to, once it has been checked.
    pub user: Option<String>,
    pub slug: Option<RepoSlug>,
    pub error: Option<String>,
}

/// The account an access token belongs to.
#[derive(Serialize, Clone)]
pub struct ForgeUser {
    pub login: String,
    /// GitLab addresses people by number rather than by name, so the id is kept
    /// alongside the login and the picker hands back whichever the forge wants.
    pub id: i64,
    /// Their picture as a `data:` URL, when the forge has one for them.
    pub avatar: Option<String>,
}

/// Someone a review can be handed to, either to own it or to look at it.
#[derive(Serialize, Deserialize, Clone)]
pub struct Member {
    /// What GitLab wants: the numeric user id.
    pub id: i64,
    /// What GitHub wants: the account name.
    pub login: String,
    /// Their real name, when the forge admits to one; the login otherwise.
    pub name: String,
}

/// A pull request or merge request, flattened to the fields both forges share.
#[derive(Serialize)]
pub struct Review {
    pub number: i64,
    pub title: String,
    pub author: String,
    pub state: String,
    pub draft: bool,
    pub source_branch: String,
    pub target_branch: String,
    pub url: String,
    pub updated_at: String,
    /// True when the source branch is checked out right now.
    pub is_current: bool,
    /// Set when the review itself was opened but something after it was not:
    /// GitHub takes the people in separate requests, and one of those failing
    /// is worth saying out loud without pretending the whole thing failed.
    pub warning: Option<String>,
}

/// A repository the token can see, flattened to what picking one to clone
/// needs. `full_name` keeps the forge's own nesting (`group/sub/app`).
#[derive(Serialize, Clone, Debug)]
pub struct ForgeRepo {
    pub name: String,
    pub full_name: String,
    /// Who owns it: an account, an organisation, a group.
    pub owner: String,
    /// The address to clone over ssh, using the profile's key.
    pub ssh_url: String,
    /// The same repository over https, for a machine with no key.
    pub https_url: String,
    pub updated_at: String,
}

/// The repositories the active profile's token can see.
///
/// Not tied to whichever folder happens to be open — the point is choosing one
/// before any of them is — so this reads the account rather than `prepare`.
/// Pagination is followed until a page comes back short, so an account with
/// three hundred repositories gets all of them; a page that fails ends the walk
/// with what already arrived rather than nothing.
pub async fn repos(state: &AppState) -> Result<Vec<ForgeRepo>, String> {
    let (kind, host, token) = account(state)?;
    let base = api_base(kind, &host);
    let http = client()?;

    let mut out: Vec<ForgeRepo> = Vec::new();
    for page in 1..=10 {
        let per_page = 100usize;
        let url = match kind {
            ForgeKind::GitHub => format!(
                "{base}/user/repos?per_page={per_page}&page={page}\
                 &sort=updated&affiliation=owner,collaborator,organization_member"
            ),
            ForgeKind::GitLab => format!(
                "{base}/projects?membership=true&simple=true&per_page={per_page}&page={page}\
                 &order_by=last_activity_at"
            ),
            ForgeKind::None => return Err("No forge configured".to_string()),
        };
        let response = http
            .get(url)
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            // The first page failing means the answer cannot be given at all;
            // a later one failing means the walk got most of the way there.
            if page == 1 {
                return Err(describe(response).await);
            }
            break;
        }
        let items: Vec<serde_json::Value> = response.json().await.map_err(|e| e.to_string())?;
        let short = items.len() < per_page;
        for item in &items {
            let repo = match kind {
                ForgeKind::GitHub => ForgeRepo {
                    name: string(item, &["name"]),
                    full_name: string(item, &["full_name"]),
                    owner: string(item, &["owner", "login"]),
                    ssh_url: string(item, &["ssh_url"]),
                    https_url: string(item, &["clone_url"]),
                    updated_at: string(item, &["updated_at"]),
                },
                _ => ForgeRepo {
                    name: string(item, &["path"]).rsplit('/').next().unwrap_or_default().to_string(),
                    full_name: string(item, &["path_with_namespace"]),
                    owner: string(item, &["namespace", "path"]),
                    ssh_url: string(item, &["ssh_url_to_repo"]),
                    https_url: string(item, &["http_url_to_repo"]),
                    updated_at: string(item, &["last_activity_at"]),
                },
            };
            // `simple=true` keeps GitLab's answer small but a fork can still
            // arrive without an address worth cloning.
            if !repo.ssh_url.is_empty() || !repo.https_url.is_empty() {
                out.push(repo);
            }
        }
        if short {
            break;
        }
    }

    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out.dedup_by(|a, b| a.full_name == b.full_name);
    Ok(out)
}

/// Turns a git remote URL into a slug.
///
/// Handles the three shapes in the wild: `git@host:path.git`,
/// `ssh://git@host/path.git` and `https://host/path.git`.
pub fn parse_remote(url: &str) -> Option<RepoSlug> {
    let trimmed = url.trim().trim_end_matches('/');

    let (host, path) = if let Some(rest) = trimmed.strip_prefix("git@") {
        // scp-like syntax: git@host:owner/repo.git
        let (host, path) = rest.split_once(':')?;
        (host.to_string(), path.to_string())
    } else if let Some(rest) = trimmed
        .strip_prefix("ssh://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .or_else(|| trimmed.strip_prefix("http://"))
        .or_else(|| trimmed.strip_prefix("git://"))
    {
        // Drop any userinfo, then split host from path.
        let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
        let (host, path) = rest.split_once('/')?;
        // A port in the host is not part of the API host name.
        let host = host.split_once(':').map(|(h, _)| h).unwrap_or(host);
        (host.to_string(), path.to_string())
    } else {
        return None;
    };

    let path = path.trim_start_matches('/').trim_end_matches(".git");
    let (owner, name) = path.rsplit_once('/')?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }

    Some(RepoSlug {
        host,
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

/// Reads the push URL of the remote a branch tracks, falling back to `origin`.
pub fn remote_slug(state: &AppState) -> Option<RepoSlug> {
    let repo = state.repo().ok()?;
    let preferred = repo
        .head()
        .ok()
        .and_then(|head| head.shorthand().map(|s| s.to_string()))
        .and_then(|branch| repo.branch_upstream_remote(&format!("refs/heads/{branch}")).ok())
        .and_then(|buf| buf.as_str().map(|s| s.to_string()));

    let name = preferred.unwrap_or_else(|| "origin".to_string());
    let remote = repo.find_remote(&name).or_else(|_| repo.find_remote("origin")).ok()?;
    remote.url().and_then(parse_remote)
}

pub fn status(state: &AppState) -> ForgeStatus {
    let config = state.config();
    let profile = config.active();
    let kind = profile.map(|p| p.forge).unwrap_or_default();
    let host = profile
        .map(|p| {
            if p.host.is_empty() {
                p.forge.default_host().to_string()
            } else {
                p.host.clone()
            }
        })
        .unwrap_or_default();
    let has_token = profile
        .map(|p| config::secret_get(&config::forge_key(&p.id)).is_some())
        .unwrap_or(false);

    ForgeStatus {
        kind,
        host,
        has_token,
        user: None,
        slug: remote_slug(state),
        error: None,
    }
}

/// Everything a request needs, gathered before any `await` so no `!Send` git2
/// handle is held across it.
struct Call {
    kind: ForgeKind,
    host: String,
    token: String,
    slug: RepoSlug,
    current_branch: Option<String>,
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

fn prepare(state: &AppState) -> Result<Call, String> {
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

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("gitnoob/0.1")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// GitHub's API lives on a separate host; GitLab's sits under the web host.
fn api_base(kind: ForgeKind, host: &str) -> String {
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

/// Checks a token and returns the account it belongs to.
pub async fn check(state: &AppState) -> Result<String, String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let url = match call.kind {
        ForgeKind::GitHub => format!("{base}/user"),
        ForgeKind::GitLab => format!("{base}/user"),
        ForgeKind::None => return Err("No forge configured".to_string()),
    };

    let response = client()?
        .get(url)
        .bearer_auth(&call.token)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let user = body
        .get("login")
        .or_else(|| body.get("username"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    Ok(user)
}

/// Who the stored token belongs to, with their picture.
///
/// Unlike `check`, this asks nothing about the repository: a profile has an
/// account whether or not the folder open right now is hosted on that forge,
/// and the profile menu wants the face either way.
pub async fn me(state: &AppState) -> Result<ForgeUser, String> {
    let (kind, host, token) = account(state)?;
    let base = api_base(kind, &host);

    let response = client()?
        .get(format!("{base}/user"))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let login = body
        .get("login")
        .or_else(|| body.get("username"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    // Fetched here rather than handed over as a link: the window draws from a
    // `data:` URL, so the page itself never reaches out to the forge.
    let avatar = match body.get("avatar_url").and_then(|v| v.as_str()) {
        Some(url) if !url.is_empty() => avatar::from_url(url).await,
        _ => None,
    };

    // The history is drawn from commit emails, and the address someone commits
    // with is usually not one the forge will admit to over the API. This is the
    // one case where the two are known to be the same person, so say so and the
    // user's own face appears in the graph without a lookup that would miss.
    if let Some(picture) = &avatar {
        let config = state.config();
        // Every address the account admits to. GitLab keeps three — the one it
        // logs in with, the one it shows publicly, and the one it writes into
        // commits — and it is the last of those that the history is drawn from.
        let mut addresses: Vec<String> = ["email", "commit_email", "public_email"]
            .iter()
            .filter_map(|field| body.get(*field).and_then(|v| v.as_str()))
            .map(String::from)
            .collect();
        addresses.extend(config.active().and_then(|profile| profile.git_email.clone()));
        for address in addresses {
            avatar::note(&address, picture);
        }
    }

    let id = body.get("id").and_then(|v| v.as_i64()).unwrap_or(0);

    Ok(ForgeUser { login, id, avatar })
}

/// The faces of every profile, so the switcher shows accounts rather than a
/// list of names.
///
/// One request per profile, once per run: the menu is opened often and the
/// answer does not change between openings.
pub async fn faces(state: &AppState) -> HashMap<String, String> {
    let profiles: Vec<(String, ForgeKind, String, String)> = {
        let config = state.config();
        config
            .profiles
            .iter()
            .filter(|profile| profile.forge != ForgeKind::None)
            .filter_map(|profile| {
                let token = config::secret_get(&config::forge_key(&profile.id))?;
                let host = if profile.host.is_empty() {
                    profile.forge.default_host().to_string()
                } else {
                    profile.host.clone()
                };
                Some((profile.id.clone(), profile.forge, host, token))
            })
            .collect()
    };

    let mut found = HashMap::new();
    for (id, kind, host, token) in profiles {
        let known = FACES
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|seen| seen.get(&id).cloned());
        if let Some(known) = known {
            if let Some(picture) = known {
                found.insert(id, picture);
            }
            continue;
        }
        let picture = one_face(kind, &host, &token).await;
        FACES
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(id.clone(), picture.clone());
        if let Some(picture) = picture {
            found.insert(id, picture);
        }
    }
    found
}

async fn one_face(kind: ForgeKind, host: &str, token: &str) -> Option<String> {
    let base = api_base(kind, host);
    let body: serde_json::Value = client()
        .ok()?
        .get(format!("{base}/user"))
        .bearer_auth(token)
        .send()
        .await
        .ok()
        .filter(|response| response.status().is_success())?
        .json()
        .await
        .ok()?;
    let url = body.get("avatar_url")?.as_str().filter(|url| !url.is_empty())?;
    avatar::from_url(url).await
}

pub async fn reviews(state: &AppState) -> Result<Vec<Review>, String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;

    let response = match call.kind {
        ForgeKind::GitHub => {
            let url = format!("{base}/repos/{}/pulls?state=open&per_page=50", call.slug.full());
            http.get(url)
                .bearer_auth(&call.token)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
        }
        ForgeKind::GitLab => {
            // GitLab wants the whole namespace path URL-encoded as one segment.
            let project = urlencode(&call.slug.full());
            let url = format!("{base}/projects/{project}/merge_requests?state=opened&per_page=50");
            http.get(url).bearer_auth(&call.token).send().await
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    let items: Vec<serde_json::Value> = response.json().await.map_err(|e| e.to_string())?;

    Ok(items
        .iter()
        .map(|item| match call.kind {
            ForgeKind::GitHub => github_review(item, call.current_branch.as_deref()),
            _ => gitlab_review(item, call.current_branch.as_deref()),
        })
        .collect())
}

fn github_review(item: &serde_json::Value, current: Option<&str>) -> Review {
    let source = string(item, &["head", "ref"]);
    Review {
        number: item.get("number").and_then(|v| v.as_i64()).unwrap_or(0),
        title: string(item, &["title"]),
        author: string(item, &["user", "login"]),
        state: string(item, &["state"]),
        draft: item.get("draft").and_then(|v| v.as_bool()).unwrap_or(false),
        is_current: current == Some(source.as_str()),
        source_branch: source,
        target_branch: string(item, &["base", "ref"]),
        url: string(item, &["html_url"]),
        updated_at: string(item, &["updated_at"]),
        warning: None,
    }
}

fn gitlab_review(item: &serde_json::Value, current: Option<&str>) -> Review {
    let source = string(item, &["source_branch"]);
    Review {
        number: item.get("iid").and_then(|v| v.as_i64()).unwrap_or(0),
        title: string(item, &["title"]),
        author: string(item, &["author", "username"]),
        state: string(item, &["state"]),
        draft: item.get("draft").and_then(|v| v.as_bool()).unwrap_or(false),
        is_current: current == Some(source.as_str()),
        source_branch: source,
        target_branch: string(item, &["target_branch"]),
        url: string(item, &["web_url"]),
        updated_at: string(item, &["updated_at"]),
        warning: None,
    }
}

/// Everyone this project's review can be handed to.
///
/// Assignees and reviewers come from the same list on both forges, so one
/// lookup feeds both pickers.
pub async fn members(state: &AppState) -> Result<Vec<Member>, String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;

    let response = match call.kind {
        ForgeKind::GitHub => {
            let url = format!(
                "{base}/repos/{}/collaborators?per_page=100&affiliation=all",
                call.slug.full()
            );
            http.get(url)
                .bearer_auth(&call.token)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
        }
        ForgeKind::GitLab => {
            let project = urlencode(&call.slug.full());
            // `members/all` rather than `members`: on GitLab the people who
            // would review a project are usually members of the group above it
            // and never appear in the project's own list.
            let url = format!("{base}/projects/{project}/members/all?per_page=100");
            http.get(url).bearer_auth(&call.token).send().await
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    let items: Vec<serde_json::Value> = response.json().await.map_err(|e| e.to_string())?;

    let mut people: Vec<Member> = items
        .iter()
        .filter(|item| match call.kind {
            // A guest cannot be given a GitLab merge request and a blocked
            // account cannot be given anything, so offering either would only
            // produce a request that fails once the review already exists.
            ForgeKind::GitLab => {
                let level = item.get("access_level").and_then(|v| v.as_i64()).unwrap_or(0);
                let state = item.get("state").and_then(|v| v.as_str()).unwrap_or("active");
                level >= 20 && state == "active"
            }
            _ => true,
        })
        .map(|item| {
            let login = item
                .get("login")
                .or_else(|| item.get("username"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&login)
                .to_string();
            Member {
                id: item.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
                login,
                name,
            }
        })
        .filter(|person| !person.login.is_empty())
        .collect();

    // `members/all` lists someone once per level they hold: the project, the
    // group it is in, and every group above that.
    people.sort_by(|a, b| a.login.to_lowercase().cmp(&b.login.to_lowercase()));
    people.dedup_by(|a, b| a.login == b.login);
    Ok(people)
}

/// Opens a pull request or merge request.
pub async fn create_review(
    state: &AppState,
    source: Option<String>,
    target: String,
    title: String,
    body: String,
    draft: bool,
    assignees: Vec<Member>,
    reviewers: Vec<Member>,
) -> Result<Review, String> {
    let call = prepare(state)?;
    // The dialog names the branch to merge; HEAD is only the fallback for a
    // caller that has nothing to say about it.
    let source = source
        .filter(|name| !name.trim().is_empty())
        .or_else(|| call.current_branch.clone())
        .ok_or_else(|| "HEAD is detached; choose a branch to merge from".to_string())?;
    if source == target {
        return Err("A branch cannot be merged into itself".to_string());
    }
    let base = api_base(call.kind, &call.host);
    let http = client()?;

    let response = match call.kind {
        ForgeKind::GitHub => {
            let url = format!("{base}/repos/{}/pulls", call.slug.full());
            http.post(url)
                .bearer_auth(&call.token)
                .header("Accept", "application/vnd.github+json")
                .json(&serde_json::json!({
                    "title": title,
                    "body": body,
                    "head": source,
                    "base": target,
                    "draft": draft,
                }))
                .send()
                .await
        }
        ForgeKind::GitLab => {
            let project = urlencode(&call.slug.full());
            let url = format!("{base}/projects/{project}/merge_requests");
            // GitLab has no draft flag; the convention is a title prefix.
            let title = if draft && !title.starts_with("Draft:") {
                format!("Draft: {title}")
            } else {
                title
            };
            http.post(url)
                .bearer_auth(&call.token)
                .json(&serde_json::json!({
                    "title": title,
                    "description": body,
                    "source_branch": source,
                    "target_branch": target,
                    // GitLab takes the people with the merge request itself.
                    "assignee_ids": assignees.iter().map(|m| m.id).collect::<Vec<_>>(),
                    "reviewer_ids": reviewers.iter().map(|m| m.id).collect::<Vec<_>>(),
                }))
                .send()
                .await
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    let item: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let mut review = match call.kind {
        ForgeKind::GitHub => github_review(&item, Some(&source)),
        _ => gitlab_review(&item, Some(&source)),
    };

    if call.kind == ForgeKind::GitHub {
        review.warning = github_people(&http, &call, review.number, &assignees, &reviewers).await;
    }
    Ok(review)
}

/// Hands a freshly opened pull request to its people.
///
/// GitHub takes neither assignees nor reviewers when the pull request is
/// created: assignees belong to the issue underneath it, reviewers to an
/// endpoint of their own. Both run after the fact, so a failure here leaves a
/// pull request that exists and is only missing its names. That is reported as
/// a warning rather than raised as an error, which would suggest nothing
/// happened and invite a second attempt the forge would refuse.
async fn github_people(
    http: &reqwest::Client,
    call: &Call,
    number: i64,
    assignees: &[Member],
    reviewers: &[Member],
) -> Option<String> {
    let base = api_base(call.kind, &call.host);
    let slug = call.slug.full();
    let mut trouble = Vec::new();

    if !assignees.is_empty() {
        let logins: Vec<&str> = assignees.iter().map(|m| m.login.as_str()).collect();
        let url = format!("{base}/repos/{slug}/issues/{number}/assignees");
        match http
            .post(url)
            .bearer_auth(&call.token)
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({ "assignees": logins }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => trouble.push(format!("assignees: {}", describe(response).await)),
            Err(error) => trouble.push(format!("assignees: {error}")),
        }
    }

    if !reviewers.is_empty() {
        let logins: Vec<&str> = reviewers.iter().map(|m| m.login.as_str()).collect();
        let url = format!("{base}/repos/{slug}/pulls/{number}/requested_reviewers");
        match http
            .post(url)
            .bearer_auth(&call.token)
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({ "reviewers": logins }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => trouble.push(format!("reviewers: {}", describe(response).await)),
            Err(error) => trouble.push(format!("reviewers: {error}")),
        }
    }

    if trouble.is_empty() {
        None
    } else {
        Some(format!("Opened, but {}", trouble.join("; ")))
    }
}

/// The forge's own "new review" page, with what has been typed already in it.
///
/// For everything the API does not carry — labels, milestones, the template a
/// project wants filled in — the honest answer is to hand the work over rather
/// than grow a form that will always be a subset of the forge's own.
pub fn compare_url(
    state: &AppState,
    source: &str,
    target: &str,
    title: &str,
    body: &str,
) -> Result<String, String> {
    let (kind, host, _) = account(state)?;
    let slug = remote_slug(state)
        .ok_or_else(|| "Could not work out the project from the git remote".to_string())?;
    let path = slug.full();

    Ok(match kind {
        // GitHub reads the branches out of the path, so those two keep their
        // slashes; everything else is a query value.
        ForgeKind::GitHub => format!(
            "https://{host}/{path}/compare/{target}...{source}?expand=1&title={}&body={}",
            urlencode(title),
            urlencode(body)
        ),
        ForgeKind::GitLab => format!(
            "https://{host}/{path}/-/merge_requests/new\
             ?merge_request%5Bsource_branch%5D={}\
             &merge_request%5Btarget_branch%5D={}\
             &merge_request%5Btitle%5D={}\
             &merge_request%5Bdescription%5D={}",
            urlencode(source),
            urlencode(target),
            urlencode(title),
            urlencode(body)
        ),
        ForgeKind::None => return Err("No forge configured".to_string()),
    })
}

/// Turns a failed response into a message worth showing, using the forge's own
/// wording where there is one.
async fn describe(response: reqwest::Response) -> String {
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

fn string(value: &serde_json::Value, path: &[&str]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_remote_url_shapes() {
        let expect = |url: &str, host: &str, owner: &str, name: &str| {
            let slug = parse_remote(url).unwrap_or_else(|| panic!("failed to parse {url}"));
            assert_eq!(slug.host, host, "host for {url}");
            assert_eq!(slug.owner, owner, "owner for {url}");
            assert_eq!(slug.name, name, "name for {url}");
        };

        expect("git@github.com:arno/gitnoob.git", "github.com", "arno", "gitnoob");
        expect("https://github.com/arno/gitnoob.git", "github.com", "arno", "gitnoob");
        expect("https://github.com/arno/gitnoob", "github.com", "arno", "gitnoob");
        expect("ssh://git@gitlab.com/group/sub/app.git", "gitlab.com", "group/sub", "app");
        expect("git@gitlab.bigbridge.nl:team/deep/nest/app.git", "gitlab.bigbridge.nl", "team/deep/nest", "app");
        expect("https://gitlab.example.com:8443/team/app.git", "gitlab.example.com", "team", "app");
        expect("https://user:token@github.com/arno/gitnoob.git", "github.com", "arno", "gitnoob");

        assert!(parse_remote("/local/path/repo").is_none());
        assert!(parse_remote("git@github.com:noslash").is_none());
    }

    #[test]
    fn builds_api_bases() {
        assert_eq!(api_base(ForgeKind::GitHub, "github.com"), "https://api.github.com");
        assert_eq!(
            api_base(ForgeKind::GitHub, "github.acme.dev"),
            "https://github.acme.dev/api/v3"
        );
        assert_eq!(
            api_base(ForgeKind::GitLab, "gitlab.bigbridge.nl"),
            "https://gitlab.bigbridge.nl/api/v4"
        );
    }

    #[test]
    fn encodes_nested_group_paths() {
        assert_eq!(urlencode("group/sub/app"), "group%2Fsub%2Fapp");
    }
}


