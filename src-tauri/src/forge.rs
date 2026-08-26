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
    /// The tip of the source branch when the forge was last asked. Enough to
    /// point the graph at the review, and enough to check it out when the
    /// branch it came from has since been deleted.
    pub head_sha: String,
    /// Where the branch actually lives. `None` for a review whose fork has
    /// been deleted, which the forges keep listing all the same.
    pub source: Option<ReviewSource>,
    /// Set when the review itself was opened but something after it was not:
    /// GitHub takes the people in separate requests, and one of those failing
    /// is worth saying out loud without pretending the whole thing failed.
    pub warning: Option<String>,
}

/// The repository a review's branch lives in, for the reviews that come from
/// somewhere other than the repository being reviewed.
///
/// A fork's branch is not in any remote this clone has, which is why checking
/// one out used to fail: there was no `origin/their-branch` to track. Carrying
/// the fork's address here is what lets the checkout add the remote it needs.
#[derive(Serialize, Clone)]
pub struct ReviewSource {
    /// `owner/name` on the forge.
    pub full_name: String,
    /// Who owns it, which is the name the added remote takes.
    pub owner: String,
    pub ssh_url: String,
    pub https_url: String,
    /// False when the branch is in the repository being reviewed, which is the
    /// ordinary case and needs no remote adding.
    pub is_fork: bool,
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

    if call.kind == ForgeKind::GitHub {
        return Ok(items
            .iter()
            .map(|item| github_review(item, call.current_branch.as_deref()))
            .collect());
    }

    let mut out = Vec::new();
    // One lookup per fork, not per review: several merge requests from the
    // same fork are the normal shape of a busy project.
    let mut known: HashMap<i64, Option<ReviewSource>> = HashMap::new();
    for item in &items {
        let (mut review, fork) = gitlab_review(item, call.current_branch.as_deref());
        if let Some(id) = fork {
            let source = match known.get(&id) {
                Some(found) => found.clone(),
                None => {
                    let found = gitlab_project(&http, &base, &call.token, id).await;
                    known.insert(id, found.clone());
                    found
                }
            };
            review.source = source;
        }
        out.push(review);
    }
    Ok(out)
}

fn github_review(item: &serde_json::Value, current: Option<&str>) -> Review {
    let source = string(item, &["head", "ref"]);
    let base_repo = string(item, &["base", "repo", "full_name"]);
    let head_repo = string(item, &["head", "repo", "full_name"]);
    // GitHub leaves `head.repo` null once the fork is gone; the review stays in
    // the list with a branch nobody can fetch by name.
    let head_from = (!head_repo.is_empty()).then(|| ReviewSource {
        owner: string(item, &["head", "repo", "owner", "login"]),
        is_fork: head_repo != base_repo,
        ssh_url: string(item, &["head", "repo", "ssh_url"]),
        https_url: string(item, &["head", "repo", "clone_url"]),
        full_name: head_repo,
    });
    Review {
        number: item.get("number").and_then(|v| v.as_i64()).unwrap_or(0),
        title: string(item, &["title"]),
        author: string(item, &["user", "login"]),
        state: string(item, &["state"]),
        draft: item.get("draft").and_then(|v| v.as_bool()).unwrap_or(false),
        // A fork's branch can share a name with one of ours without being it.
        is_current: current == Some(source.as_str())
            && head_from.as_ref().map(|from| !from.is_fork).unwrap_or(false),
        source_branch: source,
        target_branch: string(item, &["base", "ref"]),
        url: string(item, &["html_url"]),
        updated_at: string(item, &["updated_at"]),
        head_sha: string(item, &["head", "sha"]),
        source: head_from,
        warning: None,
    }
}

/// A merge request, and the project id its branch lives in when that is not
/// the project being reviewed. GitLab gives forks as an id rather than as an
/// address, so the caller looks the address up separately.
fn gitlab_review(item: &serde_json::Value, current: Option<&str>) -> (Review, Option<i64>) {
    let source = string(item, &["source_branch"]);
    let number = |key: &str| item.get(key).and_then(|v| v.as_i64());
    let project = number("project_id");
    let source_project = number("source_project_id");
    let from_fork = matches!((project, source_project), (Some(a), Some(b)) if a != b);

    let review = Review {
        number: number("iid").unwrap_or(0),
        title: string(item, &["title"]),
        author: string(item, &["author", "username"]),
        state: string(item, &["state"]),
        draft: item.get("draft").and_then(|v| v.as_bool()).unwrap_or(false),
        is_current: current == Some(source.as_str()) && !from_fork,
        source_branch: source,
        target_branch: string(item, &["target_branch"]),
        url: string(item, &["web_url"]),
        updated_at: string(item, &["updated_at"]),
        head_sha: string(item, &["sha"]),
        // Same project: nothing to add, the branch is already on the remote.
        source: (!from_fork).then(|| ReviewSource {
            full_name: String::new(),
            owner: String::new(),
            ssh_url: String::new(),
            https_url: String::new(),
            is_fork: false,
        }),
        warning: None,
    };
    (review, from_fork.then(|| source_project).flatten())
}

/// Where a forked GitLab project lives, so its branch can be fetched.
async fn gitlab_project(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    id: i64,
) -> Option<ReviewSource> {
    let body: serde_json::Value = http
        .get(format!("{base}/projects/{id}"))
        .bearer_auth(token)
        .send()
        .await
        .ok()
        .filter(|response| response.status().is_success())?
        .json()
        .await
        .ok()?;
    let full_name = string(&body, &["path_with_namespace"]);
    if full_name.is_empty() {
        return None;
    }
    Some(ReviewSource {
        owner: string(&body, &["namespace", "path"]),
        ssh_url: string(&body, &["ssh_url_to_repo"]),
        https_url: string(&body, &["http_url_to_repo"]),
        full_name,
        is_fork: true,
    })
}

/// Somebody a review names: its author, an assignee, a reviewer.
#[derive(Serialize, Clone)]
pub struct Person {
    /// The account name, which is what the forges show next to an action.
    pub login: String,
    /// Their real name where the forge has one; the login otherwise.
    pub name: String,
    /// Their picture as a `data:` URL, when there was one to fetch.
    pub avatar: Option<String>,
}

/// One of a review's labels, with the colour the forge gave it.
#[derive(Serialize, Clone)]
pub struct Label {
    pub name: String,
    /// `#rrggbb`, or empty where the forge does not colour its labels.
    pub color: String,
}

/// Everything about one review that the list of them leaves out.
///
/// The sidebar's list is deliberately thin — it is asked for on every
/// refresh — so the facts that only matter once a particular review is open
/// are fetched separately, when it is.
#[derive(Serialize)]
pub struct ReviewDetail {
    pub number: i64,
    pub title: String,
    /// The review's own description, which is not the head commit's message.
    pub body: String,
    pub state: String,
    pub draft: bool,
    pub author: Person,
    pub assignees: Vec<Person>,
    pub reviewers: Vec<Person>,
    pub labels: Vec<Label>,
    pub milestone: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
    pub url: String,
    pub created_at: String,
    pub updated_at: String,
    /// How many comments have been left on it.
    pub comments: i64,
    /// Whether it can be merged, in the forge's own vocabulary —
    /// the two have no shared one worth inventing.
    pub merge_status: Option<String>,
    /// The review's three tips, which anchoring a comment to a line of a diff
    /// needs naming: GitLab wants all of them, GitHub the head. Empty where
    /// the forge does not say.
    pub base_sha: String,
    pub head_sha: String,
    pub start_sha: String,
}

/// Faces already fetched this run, by URL. Each one is a request and a base64
/// blob, and a project's reviews name the same few people over and over.
static PEOPLE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// A picture for somebody a review names, fetched at most once per URL.
async fn face(url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    if let Some(known) = PEOPLE.lock().unwrap().as_ref().and_then(|map| map.get(url)) {
        return known.clone();
    }
    let found = avatar::from_url(url).await;
    PEOPLE
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(url.to_string(), found.clone());
    found
}

/// Reads one person out of a forge's JSON, whichever forge wrote it, along with
/// where their picture lives.
///
/// GitHub gives an account a `login` and nothing else here; GitLab gives a
/// `username` and a display name. Taking both and falling back keeps one
/// reader for the two shapes. Kept apart from fetching the picture so the
/// reading can be checked without a network.
fn read_person(value: &serde_json::Value) -> (Person, String) {
    let login = {
        let github = string(value, &["login"]);
        if github.is_empty() {
            string(value, &["username"])
        } else {
            github
        }
    };
    let name = {
        let given = string(value, &["name"]);
        if given.is_empty() {
            login.clone()
        } else {
            given
        }
    };
    (
        Person {
            login,
            name,
            avatar: None,
        },
        string(value, &["avatar_url"]),
    )
}

/// One person, with their picture fetched.
async fn person(value: &serde_json::Value) -> Person {
    let (mut read, picture) = read_person(value);
    read.avatar = face(&picture).await;
    read
}

/// The same, for the arrays of people a review carries. Anyone the forge lists
/// without an account name is not somebody to show.
async fn people(value: Option<&serde_json::Value>) -> Vec<Person> {
    let Some(items) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        if read_person(item).0.login.is_empty() {
            continue;
        }
        out.push(person(item).await);
    }
    out
}

/// A review's labels. GitHub writes its colours without the `#`; GitLab writes
/// them with one, and only when asked for the detailed form.
fn labels(value: Option<&serde_json::Value>) -> Vec<Label> {
    let Some(items) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            // GitLab without `with_labels_details` gives bare strings.
            if let Some(name) = item.as_str() {
                return Some(Label {
                    name: name.to_string(),
                    color: String::new(),
                });
            }
            let name = string(item, &["name"]);
            if name.is_empty() {
                return None;
            }
            let color = string(item, &["color"]);
            let color = if color.is_empty() || color.starts_with('#') {
                color
            } else {
                format!("#{color}")
            };
            Some(Label { name, color })
        })
        .collect()
}

/// Everything one review says about itself.
pub async fn review_detail(state: &AppState, number: i64) -> Result<ReviewDetail, String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;

    let response = match call.kind {
        ForgeKind::GitHub => {
            let url = format!("{base}/repos/{}/pulls/{number}", call.slug.full());
            http.get(url)
                .bearer_auth(&call.token)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
        }
        ForgeKind::GitLab => {
            let project = urlencode(&call.slug.full());
            let url = format!(
                "{base}/projects/{project}/merge_requests/{number}?with_labels_details=true"
            );
            http.get(url).bearer_auth(&call.token).send().await
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    let item: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let text = |key: &str| item.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let flag = |key: &str| item.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    let count = |key: &str| item.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
    let milestone = {
        let title = string(&item, &["milestone", "title"]);
        (!title.is_empty()).then_some(title)
    };

    Ok(match call.kind {
        ForgeKind::GitHub => ReviewDetail {
            number: item.get("number").and_then(|v| v.as_i64()).unwrap_or(number),
            title: text("title"),
            body: text("body"),
            // GitHub leaves a merged review's state as `closed`, and merged
            // against closed-unmerged is the distinction a reader wants.
            state: if flag("merged") {
                "merged".to_string()
            } else {
                text("state")
            },
            draft: flag("draft"),
            author: person(item.get("user").unwrap_or(&serde_json::Value::Null)).await,
            assignees: people(item.get("assignees")).await,
            reviewers: people(item.get("requested_reviewers")).await,
            labels: labels(item.get("labels")),
            milestone,
            source_branch: string(&item, &["head", "ref"]),
            target_branch: string(&item, &["base", "ref"]),
            url: string(&item, &["html_url"]),
            created_at: text("created_at"),
            updated_at: text("updated_at"),
            // Comments on the conversation and comments on the diff are two
            // counts on GitHub and one on GitLab; added, they mean the same.
            comments: count("comments") + count("review_comments"),
            merge_status: {
                let status = text("mergeable_state");
                (!status.is_empty() && status != "unknown").then_some(status)
            },
            base_sha: string(&item, &["base", "sha"]),
            head_sha: string(&item, &["head", "sha"]),
            start_sha: String::new(),
        },
        _ => ReviewDetail {
            number: item.get("iid").and_then(|v| v.as_i64()).unwrap_or(number),
            title: text("title"),
            body: text("description"),
            state: text("state"),
            draft: flag("draft"),
            author: person(item.get("author").unwrap_or(&serde_json::Value::Null)).await,
            assignees: people(item.get("assignees")).await,
            reviewers: people(item.get("reviewers")).await,
            labels: labels(item.get("labels")),
            milestone,
            source_branch: text("source_branch"),
            target_branch: text("target_branch"),
            url: text("web_url"),
            created_at: text("created_at"),
            updated_at: text("updated_at"),
            comments: count("user_notes_count"),
            merge_status: {
                // Newer GitLab spells it out in `detailed_merge_status`; older
                // installs only have the coarse `merge_status`.
                let detailed = text("detailed_merge_status");
                let status = if detailed.is_empty() {
                    text("merge_status")
                } else {
                    detailed
                };
                (!status.is_empty()).then_some(status)
            },
            base_sha: string(&item, &["diff_refs", "base_sha"]),
            head_sha: string(&item, &["diff_refs", "head_sha"]),
            start_sha: string(&item, &["diff_refs", "start_sha"]),
        },
    })
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
    // A review opened from here always comes from this repository, so the
    // fork lookup gitlab_review may ask for has nothing to find.
    let mut review = match call.kind {
        ForgeKind::GitHub => github_review(&item, Some(&source)),
        _ => gitlab_review(&item, Some(&source)).0,
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

// --- review conversation -----------------------------------------------------
//
// Everything a review page is made of beyond what the list already carries:
// the discussion under it, every file it changes across all of its commits,
// and the actions taken on it — comment, reply, approve, request changes,
// merge, close.

/// One entry of a review's conversation, flattened to what both forges share.
///
/// The two forges name their conversations differently — GitHub splits them
/// into issue comments (the conversation tab) and review comments (hanging off
/// lines of the diff); GitLab keeps one notes list where some carry a diff
/// position — so the reading side is given one shape and both feeds are
/// mapped into it here.
#[derive(Serialize, Clone)]
pub struct ReviewComment {
    pub id: i64,
    pub author: Person,
    pub body: String,
    pub created_at: String,
    /// Same as `created_at` unless the forge edited one; kept for the tooltip.
    pub updated_at: String,
    /// `issue` for a conversation comment, `diff` for one anchored to a line.
    pub kind: String,
    /// Diff comments only: the file, in the review's own terms.
    pub path: Option<String>,
    pub line: Option<i64>,
    /// Which half of the diff the line sits on: `new`, or `old`.
    pub side: Option<String>,
    /// The comment this answers, already normalised: GitHub says
    /// `in_reply_to_id`, GitLab buries the answer inside a discussion id, and
    /// both end up pointing at the root of their thread.
    pub reply_to: Option<i64>,
    /// The thread this remark belongs to, in whatever the forge needs back to
    /// resolve it: GitLab's discussion id, GitHub's review-thread node id.
    /// Empty where there is no thread to resolve — a plain conversation
    /// comment, or a forge that did not answer the question.
    pub thread: String,
    /// Whether the forge lets this thread be marked settled at all.
    pub resolvable: bool,
    /// Whether it already is.
    pub resolved: bool,
    /// Whether the lines it was written against have since moved on.
    pub outdated: bool,
}

/// A remark written on a line and held back until the verdict goes with it.
///
/// This is what a review is on GitHub — remarks pending under one review,
/// sent when it is submitted — and what a reader expects everywhere else: a
/// pass through the diff should not fire a notification per line.
#[derive(Serialize, Deserialize, Clone)]
pub struct PendingComment {
    pub path: String,
    pub line: i64,
    /// `new` or `old`, as the rest of the app names the halves of a diff.
    pub side: String,
    pub body: String,
}

/// The held-back remarks in the shape GitHub takes them alongside a verdict.
fn github_pending_comments(comments: &[PendingComment]) -> serde_json::Value {
    serde_json::Value::Array(
        comments
            .iter()
            .map(|comment| {
                serde_json::json!({
                    "path": comment.path,
                    "body": comment.body,
                    "line": comment.line,
                    "side": if comment.side == "old" { "LEFT" } else { "RIGHT" }
                })
            })
            .collect(),
    )
}

/// One check a forge ran against the review's head: a CI job, a status.
#[derive(Serialize, Clone)]
pub struct Check {
    pub name: String,
    /// success | failure | pending | cancelled | skipped.
    pub state: String,
    /// What the forge says about it, where it says anything.
    pub description: String,
    /// Where the run itself can be read.
    pub url: String,
}

/// What one person has said about the review as a whole.
#[derive(Serialize, Clone)]
pub struct Verdict {
    pub author: Person,
    /// approved | changes_requested | commented | dismissed.
    pub state: String,
    pub submitted_at: String,
    pub body: String,
}

/// Whether the review can land, and what stands in the way of it.
///
/// Kept apart from the detail because it is the half that changes while the
/// page is open — a pipeline finishes, somebody approves — and because it
/// costs several requests that reading the description should not wait for.
#[derive(Serialize, Clone, Default)]
pub struct ReviewStatus {
    pub checks: Vec<Check>,
    /// The checks rolled into one word: failure, pending, success, skipped,
    /// or none where nothing ran at all.
    pub checks_state: String,
    pub verdicts: Vec<Verdict>,
    pub approvals: i64,
    /// How many the forge insists on; zero where it does not count them.
    pub approvals_required: i64,
    /// The forge's own yes or no, where it has made its mind up.
    pub mergeable: Option<bool>,
    /// Its own word for the state: `clean`, `blocked`, `not_approved`, …
    pub merge_status: Option<String>,
    pub conflicts: bool,
}

/// A file a review touches, counted across every commit of it rather than just
/// the last one.
#[derive(Serialize, Clone)]
pub struct ReviewFileChange {
    pub path: String,
    /// Where the file used to live, when it moved.
    pub old_path: Option<String>,
    /// added | deleted | modified | renamed.
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    /// Nothing to colour: either a real binary, or a patch too large to send.
    pub binary: bool,
    /// The unified patch text, which the window reads straight into the same
    /// hunk shape a local diff takes. Empty when there is nothing to read.
    pub patch: String,
}

/// One commit of a review's source branch, as the Commits pane lists them.
#[derive(Serialize, Clone)]
pub struct ReviewCommit {
    pub sha: String,
    /// Full message; the pane shows the first line and keeps the rest.
    pub message: String,
    pub author: String,
    pub created_at: String,
}

/* ---------- pure mappers, checked without a network --------------------------- */

/// Reads an author off a comment without fetching their picture.
fn comment_author(item: &serde_json::Value, person_key: &str) -> Person {
    let (mut person, _) = read_person(item.get(person_key).unwrap_or(&serde_json::Value::Null));
    person.avatar = None;
    person
}

/// One GitHub review comment — the kind anchored to a line of the diff.
fn github_diff_comment(item: &serde_json::Value) -> ReviewComment {
    // Newer answers name the side outright; older ones leave the line only on
    // the half it belongs to, which says the same thing.
    let named = string(item, &["side"]);
    let line = item.get("line").and_then(|v| v.as_i64());
    let side = if named == "LEFT" || (line.is_none() && named.is_empty()) {
        "old"
    } else {
        "new"
    };
    let resolved = line.or_else(|| item.get("original_line").and_then(|v| v.as_i64()));
    let path = string(item, &["path"]);
    ReviewComment {
        id: item.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
        author: comment_author(item, "user"),
        body: string(item, &["body"]),
        created_at: string(item, &["created_at"]),
        updated_at: string(item, &["updated_at"]),
        kind: "diff".to_string(),
        path: (!path.is_empty()).then_some(path),
        line: resolved,
        side: Some(side.to_string()),
        reply_to: item.get("in_reply_to_id").and_then(|v| v.as_i64()),
        // GitHub keeps resolution out of REST entirely; the GraphQL pass
        // below fills these in, and leaves them alone when it cannot answer.
        thread: String::new(),
        resolvable: false,
        resolved: false,
        // No live line but an original one: the code it was written against
        // has moved on since.
        outdated: line.is_none() && item.get("original_line").is_some(),
    }
}

/// One GitHub conversation comment.
fn github_issue_comment(item: &serde_json::Value) -> ReviewComment {
    ReviewComment {
        id: item.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
        author: comment_author(item, "user"),
        body: string(item, &["body"]),
        created_at: string(item, &["created_at"]),
        updated_at: string(item, &["updated_at"]),
        kind: "issue".to_string(),
        path: None,
        line: None,
        side: None,
        reply_to: None,
        // A conversation comment is not a thread anybody settles.
        thread: String::new(),
        resolvable: false,
        resolved: false,
        outdated: false,
    }
}

/// One GitLab note, when it is worth keeping.
///
/// Notes are the one list everything lives in over there: conversation
/// comments, system notices and diff-anchored ones alike, told apart by having
/// a `position` and by being marked `system`.
fn gitlab_note(item: &serde_json::Value) -> Option<ReviewComment> {
    if item.get("system").and_then(|v| v.as_bool()).unwrap_or(false) {
        return None;
    }
    let position = item.get("position");
    let new_line = position.and_then(|p| p.get("new_line")).and_then(|v| v.as_i64());
    let old_line = position.and_then(|p| p.get("old_line")).and_then(|v| v.as_i64());
    let kind = if position.is_some() { "diff" } else { "issue" };
    let new_path = position.map(|p| string(p, &["new_path"]));
    let old_path = position.map(|p| string(p, &["old_path"]));
    let path = match (&new_path, &old_path) {
        (Some(path), _) if !path.is_empty() => Some(path.clone()),
        (_, Some(path)) if !path.is_empty() => Some(path.clone()),
        _ => None,
    };
    let line = new_line.or(old_line);
    Some(ReviewComment {
        id: item.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
        author: comment_author(item, "author"),
        body: string(item, &["body"]),
        created_at: string(item, &["created_at"]),
        updated_at: string(item, &["updated_at"]),
        kind: kind.to_string(),
        path,
        line,
        side: if position.is_none() {
            None
        } else {
            Some(if new_line.is_some() { "new" } else { "old" }.to_string())
        },
        // Filled in once every note of the merge request is on hand: GitLab
        // points threads with a shared discussion id instead of a parent.
        reply_to: None,
        thread: string(item, &["discussion_id"]),
        resolvable: item
            .get("resolvable")
            .and_then(|v| v.as_bool())
            .unwrap_or(position.is_some()),
        resolved: item.get("resolved").and_then(|v| v.as_bool()).unwrap_or(false),
        // GitLab marks the position itself when the line has moved out from
        // under the remark.
        outdated: position
            .and_then(|p| p.get("outdated"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

fn is_system_note(item: &serde_json::Value) -> bool {
    item.get("system").and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Answers GitLab notes to the root of their thread.
///
/// Every note of a thread shares a discussion id; walking the notes in order,
/// the first note seen under each id is its opening remark and every later one
/// answers it. The kept comments ride along in the same order as their raw
/// notes, so pairing the two walks stays aligned.
fn gitlab_thread_replies(comments: &mut [ReviewComment], notes_in_order: &[serde_json::Value]) {
    // Discussion id -> the note id that opened the thread.
    let mut roots: HashMap<String, i64> = HashMap::new();
    for (item, comment) in notes_in_order.iter().zip(comments.iter_mut()) {
        let Some(discussion) = item.get("discussion_id").and_then(|v| v.as_str()) else {
            continue;
        };
        match roots.get(discussion) {
            Some(root) => comment.reply_to = Some(*root),
            None => {
                roots.insert(discussion.to_string(), comment.id);
            }
        }
    }
}

/// Splits GitLab's three booleans into the one status word both sides read.
fn gitlab_file_status(new_file: bool, deleted: bool, renamed: bool) -> &'static str {
    if new_file {
        "added"
    } else if deleted {
        "deleted"
    } else if renamed {
        "renamed"
    } else {
        "modified"
    }
}

/// Counts the changed lines a unified patch holds.
///
/// The file headers carry a sign like the body does — `+++ b/path` reads as an
/// addition to a glance at the first character — so they are named apart
/// before anything is counted; everything else unsigned counts for neither.
fn count_patch_lines(patch: &str) -> (i64, i64) {
    let mut additions = 0i64;
    let mut deletions = 0i64;
    for line in patch.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => additions += 1,
            Some(b'-') => deletions += 1,
            _ => {}
        }
    }
    (additions, deletions)
}

/* ---------- endpoints --------------------------------------------------------- */

/// Fetches a paginated collection of JSON items until a short page arrives.
///
/// Ten pages of a hundred is the same ceiling `repos` walks to. The first page
/// failing means the answer cannot be given at all; a later one failing leaves
/// what already arrived rather than nothing.
async fn paged(
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
        let mut items: Vec<serde_json::Value> =
            response.json().await.map_err(|e| e.to_string())?;
        let full = items.len() >= PER_PAGE;
        out.append(&mut items);
        if !full {
            break;
        }
    }
    Ok(out)
}

/// Everything said under one review: the conversation and every diff thread.
pub async fn review_comments(state: &AppState, number: i64) -> Result<Vec<ReviewComment>, String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    let (mut comments, notes_in_order) = match call.kind {        ForgeKind::GitHub => {
            let pulls = format!("{base}/repos/{slug}/pulls/{number}");
            let issues = format!("{base}/repos/{slug}/issues/{number}");
            let diff = paged(&http, &call.token, move |page| {
                format!("{pulls}/comments?per_page=100&page={page}")
            })
            .await?;
            let talk = paged(&http, &call.token, move |page| {
                format!("{issues}/comments?per_page=100&page={page}")
            })
            .await?;
            let mut out: Vec<ReviewComment> = diff.iter().map(github_diff_comment).collect();
            // Which thread each diff remark belongs to, and whether it has
            // been settled: REST knows neither, so GraphQL is asked once and
            // its silence costs nothing but the threads staying open.
            if !out.is_empty() {
                let threads = github_threads(
                    &http,
                    &call.token,
                    &call.host,
                    &call.slug.owner,
                    &call.slug.name,
                    number,
                )
                .await;
                for comment in &mut out {
                    if let Some((thread, resolved, outdated)) = threads.get(&comment.id) {
                        comment.thread = thread.clone();
                        comment.resolvable = true;
                        comment.resolved = *resolved;
                        comment.outdated = *outdated;
                    }
                }
            }
            out.extend(talk.iter().map(github_issue_comment));
            (out, Vec::new())
        }
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            let notes_url =
                format!("{base}/projects/{project}/merge_requests/{number}/notes?sort=asc&order_by=created_at");
            let notes = paged(&http, &call.token, move |page| {
                format!("{notes_url}&per_page=100&page={page}")
            })
            .await?;
            let kept: Vec<ReviewComment> = notes.iter().filter_map(gitlab_note).collect();
            // The kept notes again, as raw JSON in the same order, for the
            // thread walk: system notices were dropped from both sides alike,
            // so the two lists stay paired.
            let cloned: Vec<serde_json::Value> =
                notes.iter().filter(|item| !is_system_note(item)).cloned().collect();
            (kept, cloned)
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    };

    if call.kind == ForgeKind::GitLab {
        gitlab_thread_replies(&mut comments, &notes_in_order);
    }

    // Oldest first, which is how a conversation reads. ISO timestamps sort
    // correctly as text; one tiebreak keeps them stable on a shared second.
    comments.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    Ok(comments)
}

/// Every file a review changes, counted across all of its commits.
pub async fn review_files(state: &AppState, number: i64) -> Result<Vec<ReviewFileChange>, String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    let files = match call.kind {
        ForgeKind::GitHub => {
            let pulls = format!("{base}/repos/{slug}/pulls/{number}");
            let items = paged(&http, &call.token, move |page| {
                format!("{pulls}/files?per_page=100&page={page}")
            })
            .await?;
            items
                .iter()
                .map(|item| {
                    let status = match string(item, &["status"]).as_str() {
                        "added" => "added",
                        "removed" => "deleted",
                        "renamed" => "renamed",
                        _ => "modified",
                    };
                    let previous = string(item, &["previous_filename"]);
                    let patch = string(item, &["patch"]);
                    ReviewFileChange {
                        path: string(item, &["filename"]),
                        old_path: (status == "renamed" && !previous.is_empty())
                            .then_some(previous),
                        status: status.to_string(),
                        additions: item.get("additions").and_then(|v| v.as_i64()).unwrap_or(0),
                        deletions: item.get("deletions").and_then(|v| v.as_i64()).unwrap_or(0),
                        binary: patch.is_empty()
                            && item.get("changes").and_then(|v| v.as_i64()).unwrap_or(0) == 0,
                        patch,
                    }
                })
                .collect()
        }
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            let diffs_url = format!("{base}/projects/{project}/merge_requests/{number}/diffs");
            let items = paged(&http, &call.token, move |page| {
                format!("{diffs_url}?per_page=100&page={page}")
            })
            .await?;
            items
                .iter()
                .map(|item| {
                    let flag = |key: &str| item.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
                    let old_path = string(item, &["old_path"]);
                    let new_path = string(item, &["new_path"]);
                    let renamed = flag("renamed_file");
                    let patch = string(item, &["diff"]);
                    let (additions, deletions) = count_patch_lines(&patch);
                    // Older installs never say `binary`; an answer of all
                    // headers and no lines is the only hint there is.
                    let binary = item
                        .get("binary")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(additions == 0 && deletions == 0 && !flag("deleted_file"));
                    let is_new = new_path.is_empty();
                    ReviewFileChange {
                        path: if is_new { old_path.clone() } else { new_path.clone() },
                        old_path: (renamed && old_path != new_path).then_some(old_path),
                        status: gitlab_file_status(flag("new_file"), flag("deleted_file"), renamed)
                            .to_string(),
                        additions,
                        deletions,
                        binary,
                        patch,
                    }
                })
                .collect()
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    };
    Ok(files)
}

/// The commits a review's source branch puts ahead of its target.
pub async fn review_commits(state: &AppState, number: i64) -> Result<Vec<ReviewCommit>, String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    let items = match call.kind {
        ForgeKind::GitHub => {
            let pulls = format!("{base}/repos/{slug}/pulls/{number}");
            paged(&http, &call.token, move |page| {
                format!("{pulls}/commits?per_page=100&page={page}")
            })
            .await?
        }
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            let commits_url = format!("{base}/projects/{project}/merge_requests/{number}/commits");
            paged(&http, &call.token, move |page| {
                format!("{commits_url}?per_page=100&page={page}")
            })
            .await?
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    };

    Ok(items
        .iter()
        .map(|item| {
            // GitHub wraps the facts one level deeper than GitLab does, and
            // then wraps them one deeper still: its name and date live under
            // `commit.author` (with the committer beside it), not flat on the
            // commit itself. The reader tries the flat key first, then the
            // git-commit object, then that object's author and committer.
            let inner = item.get("commit");
            let person = || inner.and_then(|c| c.get("author")).or(inner.and_then(|c| c.get("committer")));
            let read = |key: &str| -> Option<String> {
                item.get(key)
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| inner.and_then(|c| c.get(key)).and_then(|v| v.as_str()).map(String::from))
                    .or_else(|| person().and_then(|p| p.get(key)).and_then(|v| v.as_str()).map(String::from))
            };
            let author = read("author_name")
                .or_else(|| read("name"))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "unknown".to_string());
            ReviewCommit {
                sha: read("id").or_else(|| read("sha")).unwrap_or_default(),
                message: read("message").unwrap_or_default(),
                author,
                created_at: read("created_at").or_else(|| read("date")).unwrap_or_default(),
            }
        })
        .collect())
}

/// Leaves one comment on the conversation itself.
///
/// A single-shot note rather than the start of a formal review with pending
/// remarks; those are what the diff-comment calls are for, and each arrives as
/// it is written instead of waiting to be submitted.
pub async fn post_comment(state: &AppState, number: i64, body: &str) -> Result<(), String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    let response = match call.kind {
        ForgeKind::GitHub => http
            .post(format!("{base}/repos/{slug}/issues/{number}/comments"))
            .bearer_auth(&call.token)
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await,
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            http.post(format!(
                "{base}/projects/{project}/merge_requests/{number}/notes"
            ))
            .bearer_auth(&call.token)
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    Ok(())
}

/// Answers one comment, keeping whatever thread shape each forge has.
///
/// A reply to a diff comment stays attached to its line this way: GitHub takes
/// `in_reply_to`, GitLab wants the discussion id behind the parent note, which
/// costs one lookup that is worth every request it spends.
pub async fn reply_comment(
    state: &AppState,
    number: i64,
    parent_id: i64,
    body: &str,
) -> Result<(), String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    let response = match call.kind {
        ForgeKind::GitHub => http
            .post(format!("{base}/repos/{slug}/pulls/{number}/comments"))
            .bearer_auth(&call.token)
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({ "body": body, "in_reply_to": parent_id }))
            .send()
            .await,
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            // One page usually holds every discussion; asking for ten keeps
            // the same walk everything else here takes.
            let mut discussion_id: Option<String> = None;
            'walk: for page in 1..=10usize {
                let url = format!(
                    "{base}/projects/{project}/merge_requests/{number}/discussions?per_page=100&page={page}"
                );
                let response = http
                    .get(url)
                    .bearer_auth(&call.token)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if !response.status().is_success() {
                    break;
                }
                let items: Vec<serde_json::Value> =
                    response.json().await.map_err(|e| e.to_string())?;
                let last = items.len() < 100;
                for item in items {
                    let Some(notes) = item.get("notes").and_then(|v| v.as_array()) else {
                        continue;
                    };
                    if notes
                        .iter()
                        .any(|note| note.get("id").and_then(|v| v.as_i64()) == Some(parent_id))
                    {
                        discussion_id =
                            item.get("id").and_then(|v| v.as_str()).map(String::from);
                        break 'walk;
                    }
                }
                if last {
                    break;
                }
            }
            let Some(discussion_id) = discussion_id else {
                return Err(format!("Could not find comment {parent_id} on the forge"));
            };
            http.post(format!(
                "{base}/projects/{project}/merge_requests/{number}/discussions/{discussion_id}/notes"
            ))
            .bearer_auth(&call.token)
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    Ok(())
}

/// Starts a fresh thread on one line of one file's diff.
///
/// The three tips name which versions were compared, because GitLab positions
/// are meaningless without them: a comment anchored without those shas can be
/// shown against nothing when the branch moves on. GitHub needs only the head.
#[allow(clippy::too_many_arguments)]
pub async fn add_diff_comment(
    state: &AppState,
    number: i64,
    head_sha: &str,
    base_sha: &str,
    start_sha: &str,
    path: &str,
    line: i64,
    side: &str,
    body: &str,
) -> Result<(), String> {
    let call = prepare(state)?;
    let base_api = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    let response = match call.kind {
        ForgeKind::GitHub => {
            let payload = if side == "old" {
                serde_json::json!({
                    "body": body, "commit_id": head_sha, "path": path,
                    "side": "LEFT", "line": line
                })
            } else {
                serde_json::json!({
                    "body": body, "commit_id": head_sha, "path": path,
                    "side": "RIGHT", "line": line
                })
            };
            http.post(format!("{base_api}/repos/{slug}/pulls/{number}/comments"))
                .bearer_auth(&call.token)
                .header("Accept", "application/vnd.github+json")
                .json(&payload)
                .send()
                .await
        }
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            let mut position = serde_json::json!({
                "position_type": "text",
                "base_sha": base_sha,
                "start_sha": start_sha,
                "head_sha": head_sha,
                "new_path": path,
                "old_path": path,
            });
            if side == "old" {
                position["old_line"] = serde_json::json!(line);
            } else {
                position["new_line"] = serde_json::json!(line);
            }
            http.post(format!(
                "{base_api}/projects/{project}/merge_requests/{number}/discussions"
            ))
            .bearer_auth(&call.token)
            .json(&serde_json::json!({ "body": body, "position": position }))
            .send()
            .await
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    Ok(())
}

/// Hands down a verdict.
///
/// On GitLab, approval has an endpoint but requesting changes does not —
/// approvals there say yes, and saying no is done in words. So a GitLab
/// request-for-changes posts the text plainly rather than pretending there is
/// a state behind the button.
pub async fn submit_review(
    state: &AppState,
    number: i64,
    event: &str,
    body: &str,
    comments: Vec<PendingComment>,
) -> Result<(), String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    match call.kind {
        ForgeKind::GitHub => {
            let event = match event {
                "approve" => "APPROVE",
                "request_changes" => "REQUEST_CHANGES",
                _ => "COMMENT",
            };
            // One request carries the verdict and every remark held back for
            // it, which is what makes them arrive together rather than as a
            // trickle of notifications while the reading is still going on.
            let mut payload = serde_json::json!({ "event": event, "body": body });
            if !comments.is_empty() {
                payload["comments"] = github_pending_comments(&comments);
            }
            let response = http
                .post(format!("{base}/repos/{slug}/pulls/{number}/reviews"))
                .bearer_auth(&call.token)
                .header("Accept", "application/vnd.github+json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !response.status().is_success() {
                return Err(describe(response).await);
            }
        }
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            let notes = format!("{base}/projects/{project}/merge_requests/{number}/notes");

            // GitLab has no pending review to submit, so the held-back remarks
            // are posted as their threads first and the verdict follows: the
            // same order a reader would have written them in, in one gesture.
            if !comments.is_empty() {
                let detail = fetch(
                    &http,
                    &call.token,
                    &format!("{base}/projects/{project}/merge_requests/{number}"),
                )
                .await?;
                let base_sha = string(&detail, &["diff_refs", "base_sha"]);
                let head_sha = string(&detail, &["diff_refs", "head_sha"]);
                let start_sha = string(&detail, &["diff_refs", "start_sha"]);
                for comment in &comments {
                    add_diff_comment(
                        state,
                        number,
                        &head_sha,
                        &base_sha,
                        &start_sha,
                        &comment.path,
                        comment.line,
                        &comment.side,
                        &comment.body,
                    )
                    .await?;
                }
            }

            if event == "approve" {
                // With something to say alongside, so it survives a wall of
                // anonymous green ticks elsewhere.
                if !body.trim().is_empty() {
                    let said = http
                        .post(&notes)
                        .bearer_auth(&call.token)
                        .json(&serde_json::json!({ "body": body }))
                        .send()
                        .await
                        .map_err(|e| e.to_string())?;
                    if !said.status().is_success() {
                        return Err(describe(said).await);
                    }
                }
                let response = http
                    .post(format!("{base}/projects/{project}/merge_requests/{number}/approve"))
                    .bearer_auth(&call.token)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if !response.status().is_success() {
                    return Err(describe(response).await);
                }
            } else {
                let worded = if event == "request_changes" && !body.trim().is_empty() {
                    format!("Requested changes\n\n{body}")
                } else if !body.trim().is_empty() {
                    body.to_string()
                } else if event == "request_changes" {
                    "Requested changes".to_string()
                } else {
                    return Ok(());
                };
                let response = http
                    .post(&notes)
                    .bearer_auth(&call.token)
                    .json(&serde_json::json!({ "body": worded }))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if !response.status().is_success() {
                    return Err(describe(response).await);
                }
            }
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    Ok(())
}

/// Merges the review, squashed or not, or says why the forge refused.
///
/// Deleting the branch afterwards is one flag on GitLab and a request of its
/// own on GitHub — and one this only makes for a branch in the repository
/// being reviewed, since somebody else's fork is not ours to tidy.
pub async fn merge_review(
    state: &AppState,
    number: i64,
    squash: bool,
    delete_branch: bool,
) -> Result<String, String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    // GitHub needs the branch's name before the merge, since the answer to a
    // merge does not carry it and the pull request is closed by then.
    let branch = if delete_branch && call.kind == ForgeKind::GitHub {
        let pull = fetch(&http, &call.token, &format!("{base}/repos/{slug}/pulls/{number}")).await?;
        let same_repo = string(&pull, &["head", "repo", "full_name"]) == slug;
        same_repo.then(|| string(&pull, &["head", "ref"]))
    } else {
        None
    };

    let response = match call.kind {
        ForgeKind::GitHub => http
            .put(format!("{base}/repos/{slug}/pulls/{number}/merge"))
            .bearer_auth(&call.token)
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({
                "merge_method": if squash { "squash" } else { "merge" }
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?,
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            http.put(format!(
                "{base}/projects/{project}/merge_requests/{number}/merge"
            ))
            .bearer_auth(&call.token)
            .json(&serde_json::json!({
                "squash": squash,
                "should_remove_source_branch": delete_branch
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    };
    if !response.status().is_success() {
        return Err(describe(response).await);
    }

    // The merge landed either way: a branch that will not delete is worth
    // saying, not worth undoing the merge over.
    if let Some(branch) = branch.filter(|name| !name.is_empty()) {
        let gone = http
            .delete(format!("{base}/repos/{slug}/git/refs/heads/{branch}"))
            .bearer_auth(&call.token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await;
        match gone {
            Ok(response) if response.status().is_success() => {
                return Ok(format!("Merged, and deleted {branch}"))
            }
            _ => return Ok("Merged; the branch is still there".to_string()),
        }
    }
    Ok("Merged".to_string())
}

/// Closes or reopens a review.
pub async fn set_review_state(state: &AppState, number: i64, action: &str) -> Result<(), String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    let response = match call.kind {
        ForgeKind::GitHub => {
            let state_word = if action == "close" { "closed" } else { "open" };
            http.patch(format!("{base}/repos/{slug}/issues/{number}"))
                .bearer_auth(&call.token)
                .header("Accept", "application/vnd.github+json")
                .json(&serde_json::json!({ "state": state_word }))
                .send()
                .await
        }
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            http.put(format!(
                "{base}/projects/{project}/merge_requests/{number}"
            ))
            .bearer_auth(&call.token)
            .json(&serde_json::json!({
                "state_event": if action == "close" { "close" } else { "reopen" }
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
    Ok(())
}

/* ---------- how the review stands ---------------------------------------- */

/// One GET that answers with JSON, or with whatever the forge complained.
async fn fetch(
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

/// GitHub's own word for a check run, in the five this app draws.
fn github_check_state(status: &str, conclusion: &str) -> &'static str {
    if status != "completed" {
        return "pending";
    }
    match conclusion {
        "success" => "success",
        "failure" | "timed_out" | "action_required" | "startup_failure" => "failure",
        "cancelled" => "cancelled",
        // neutral, skipped, stale: it ran, and it has nothing to say.
        _ => "skipped",
    }
}

/// The same, for the older commit-status API that some projects still use.
fn github_status_state(state: &str) -> &'static str {
    match state {
        "success" => "success",
        "failure" | "error" => "failure",
        "pending" => "pending",
        _ => "skipped",
    }
}

/// The same again, for a GitLab job.
fn gitlab_job_state(status: &str) -> &'static str {
    match status {
        "success" => "success",
        "failed" => "failure",
        "canceled" | "canceling" => "cancelled",
        "skipped" | "manual" => "skipped",
        // created, pending, running, preparing, scheduled, waiting_for_resource
        _ => "pending",
    }
}

/// The one word a wall of checks adds up to.
///
/// A single failure is the answer whatever else passed; anything still running
/// makes the answer "wait"; a wall of skips is not a pass anybody earned.
fn roll_up(checks: &[Check]) -> String {
    let any = |what: &str| checks.iter().any(|check| check.state == what);
    if checks.is_empty() {
        "none"
    } else if any("failure") {
        "failure"
    } else if any("pending") {
        "pending"
    } else if any("success") {
        "success"
    } else {
        "skipped"
    }
    .to_string()
}

/// The standing verdicts, one per person, out of GitHub's list of reviews.
///
/// They arrive oldest first and one person can leave several. The last
/// position they took is the one that stands; a passing comment afterwards
/// does not unseat the approval they already gave, and a pending review is one
/// they have not sent yet.
fn fold_github_verdicts(items: &[serde_json::Value]) -> Vec<Verdict> {
    let mut out: Vec<Verdict> = Vec::new();
    for item in items {
        let state = string(item, &["state"]).to_lowercase();
        if state.is_empty() || state == "pending" {
            continue;
        }
        let (author, _) = read_person(item.get("user").unwrap_or(&serde_json::Value::Null));
        if author.login.is_empty() {
            continue;
        }
        let verdict = Verdict {
            author,
            state: state.clone(),
            submitted_at: string(item, &["submitted_at"]),
            body: string(item, &["body"]),
        };
        match out
            .iter()
            .position(|seen| seen.author.login == verdict.author.login)
        {
            Some(at) => {
                if state == "commented" && out[at].state != "commented" {
                    continue;
                }
                out[at] = verdict;
            }
            None => out.push(verdict),
        }
    }
    out
}

/// Where GitHub answers GraphQL, which is not under the REST base on either
/// github.com or an Enterprise install.
fn github_graphql_url(host: &str) -> String {
    if host == "github.com" {
        "https://api.github.com/graphql".to_string()
    } else {
        format!("https://{host}/api/graphql")
    }
}

/// Asks GitHub's GraphQL endpoint one question.
async fn github_graphql(
    http: &reqwest::Client,
    token: &str,
    host: &str,
    query: &str,
    variables: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let response = http
        .post(github_graphql_url(host))
        .bearer_auth(token)
        .json(&serde_json::json!({ "query": query, "variables": variables }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    // GraphQL answers 200 with the failure inside the body.
    if let Some(problem) = body
        .get("errors")
        .and_then(|v| v.as_array())
        .and_then(|list| list.first())
    {
        return Err(string(problem, &["message"]));
    }
    Ok(body.get("data").cloned().unwrap_or(serde_json::Value::Null))
}

/// What each of a GitHub pull request's review threads knows about itself,
/// keyed by every comment id in it.
///
/// REST neither says whether a thread is settled nor offers a way to settle
/// one, so this is the single place GraphQL earns its extra request. A forge
/// that refuses the question — an old Enterprise, a token without the
/// scope — simply leaves the threads unresolvable rather than failing the read.
async fn github_threads(
    http: &reqwest::Client,
    token: &str,
    host: &str,
    owner: &str,
    name: &str,
    number: i64,
) -> HashMap<i64, (String, bool, bool)> {
    const QUERY: &str = "query($owner:String!,$name:String!,$number:Int!,$after:String){\
        repository(owner:$owner,name:$name){pullRequest(number:$number){\
        reviewThreads(first:100,after:$after){pageInfo{hasNextPage endCursor}\
        nodes{id isResolved isOutdated comments(first:100){nodes{databaseId}}}}}}}";

    let mut out: HashMap<i64, (String, bool, bool)> = HashMap::new();
    let mut after = serde_json::Value::Null;
    for _ in 0..10 {
        let variables = serde_json::json!({
            "owner": owner, "name": name, "number": number, "after": after
        });
        let Ok(data) = github_graphql(http, token, host, QUERY, variables).await else {
            return out;
        };
        let threads = data
            .pointer("/repository/pullRequest/reviewThreads")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        for node in threads
            .get("nodes")
            .and_then(|v| v.as_array())
            .unwrap_or(&Vec::new())
        {
            let id = string(node, &["id"]);
            let resolved = node.get("isResolved").and_then(|v| v.as_bool()).unwrap_or(false);
            let outdated = node.get("isOutdated").and_then(|v| v.as_bool()).unwrap_or(false);
            for comment in node
                .pointer("/comments/nodes")
                .and_then(|v| v.as_array())
                .unwrap_or(&Vec::new())
            {
                if let Some(comment_id) = comment.get("databaseId").and_then(|v| v.as_i64()) {
                    out.insert(comment_id, (id.clone(), resolved, outdated));
                }
            }
        }
        let more = threads
            .pointer("/pageInfo/hasNextPage")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !more {
            break;
        }
        after = threads
            .pointer("/pageInfo/endCursor")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
    }
    out
}

/// Whether the review can land, what ran against it, and who has said what.
pub async fn review_status(state: &AppState, number: i64) -> Result<ReviewStatus, String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();
    let mut out = ReviewStatus::default();

    match call.kind {
        ForgeKind::GitHub => {
            let item = fetch(&http, &call.token, &format!("{base}/repos/{slug}/pulls/{number}")).await?;
            out.mergeable = item.get("mergeable").and_then(|v| v.as_bool());
            let word = string(&item, &["mergeable_state"]);
            out.conflicts = word == "dirty";
            out.merge_status = (!word.is_empty() && word != "unknown").then_some(word);

            let head = string(&item, &["head", "sha"]);
            if !head.is_empty() {
                // The modern checks, and the older statuses beside them: a
                // project can be using either, and plenty use both.
                let runs = format!("{base}/repos/{slug}/commits/{head}/check-runs?per_page=100");
                if let Ok(answer) = fetch(&http, &call.token, &runs).await {
                    for run in answer
                        .get("check_runs")
                        .and_then(|v| v.as_array())
                        .unwrap_or(&Vec::new())
                    {
                        out.checks.push(Check {
                            name: string(run, &["name"]),
                            state: github_check_state(
                                &string(run, &["status"]),
                                &string(run, &["conclusion"]),
                            )
                            .to_string(),
                            description: string(run, &["output", "title"]),
                            url: string(run, &["html_url"]),
                        });
                    }
                }
                let statuses = format!("{base}/repos/{slug}/commits/{head}/status");
                if let Ok(answer) = fetch(&http, &call.token, &statuses).await {
                    for one in answer
                        .get("statuses")
                        .and_then(|v| v.as_array())
                        .unwrap_or(&Vec::new())
                    {
                        out.checks.push(Check {
                            name: string(one, &["context"]),
                            state: github_status_state(&string(one, &["state"])).to_string(),
                            description: string(one, &["description"]),
                            url: string(one, &["target_url"]),
                        });
                    }
                }
            }

            let reviews = format!("{base}/repos/{slug}/pulls/{number}/reviews");
            if let Ok(items) = paged(&http, &call.token, move |page| {
                format!("{reviews}?per_page=100&page={page}")
            })
            .await
            {
                out.verdicts = fold_github_verdicts(&items);
            }
        }
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            let mr = format!("{base}/projects/{project}/merge_requests/{number}");
            let item = fetch(&http, &call.token, &mr).await?;
            out.conflicts = item
                .get("has_conflicts")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let detailed = string(&item, &["detailed_merge_status"]);
            let word = if detailed.is_empty() {
                string(&item, &["merge_status"])
            } else {
                detailed
            };
            out.mergeable = match word.as_str() {
                "" | "checking" | "unchecked" => None,
                "mergeable" | "can_be_merged" => Some(true),
                _ => Some(false),
            };
            out.merge_status = (!word.is_empty()).then_some(word);

            // The pipeline that ran against the head, and the jobs inside it.
            let pipeline = item
                .get("head_pipeline")
                .filter(|v| !v.is_null())
                .or_else(|| item.get("pipeline").filter(|v| !v.is_null()))
                .cloned();
            if let Some(pipeline) = pipeline {
                let id = pipeline.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                let jobs = format!("{base}/projects/{project}/pipelines/{id}/jobs?per_page=100");
                let listed = if id > 0 {
                    fetch(&http, &call.token, &jobs).await.ok()
                } else {
                    None
                };
                match listed.as_ref().and_then(|v| v.as_array()) {
                    Some(items) if !items.is_empty() => {
                        for job in items {
                            out.checks.push(Check {
                                name: {
                                    let stage = string(job, &["stage"]);
                                    let name = string(job, &["name"]);
                                    if stage.is_empty() {
                                        name
                                    } else {
                                        format!("{stage} · {name}")
                                    }
                                },
                                state: gitlab_job_state(&string(job, &["status"])).to_string(),
                                description: if job
                                    .get("allow_failure")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false)
                                {
                                    "allowed to fail".to_string()
                                } else {
                                    String::new()
                                },
                                url: string(job, &["web_url"]),
                            });
                        }
                    }
                    // A pipeline whose jobs cannot be listed still says how it
                    // went, which is better than saying nothing ran.
                    _ => out.checks.push(Check {
                        name: "Pipeline".to_string(),
                        state: gitlab_job_state(&string(&pipeline, &["status"])).to_string(),
                        description: string(&pipeline, &["status"]),
                        url: string(&pipeline, &["web_url"]),
                    }),
                }
            }

            // Approvals, where the install has them: who has approved, and how
            // many the project wants before it will merge.
            let approvals = format!("{mr}/approvals");
            if let Ok(answer) = fetch(&http, &call.token, &approvals).await {
                out.approvals_required = answer
                    .get("approvals_required")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                for one in answer
                    .get("approved_by")
                    .and_then(|v| v.as_array())
                    .unwrap_or(&Vec::new())
                {
                    let who = one.get("user").unwrap_or(one);
                    let (author, _) = read_person(who);
                    if author.login.is_empty() {
                        continue;
                    }
                    out.verdicts.push(Verdict {
                        author,
                        state: "approved".to_string(),
                        submitted_at: String::new(),
                        body: String::new(),
                    });
                }
            }
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }

    out.checks_state = roll_up(&out.checks);
    out.approvals = out
        .verdicts
        .iter()
        .filter(|verdict| verdict.state == "approved")
        .count() as i64;

    // The faces, once the list is settled: a verdict without one is a row of
    // grey circles where the reader is looking for a person.
    for verdict in &mut out.verdicts {
        // github.com serves an account's picture from its login; an Enterprise
        // install and GitLab do not, and the initials stand in there.
        if call.kind == ForgeKind::GitHub && call.host == "github.com" {
            let url = format!("https://github.com/{}.png?size=64", verdict.author.login);
            verdict.author.avatar = face(&url).await;
        }
    }
    Ok(out)
}

/// Marks one thread settled, or unsettles it again.
pub async fn resolve_thread(
    state: &AppState,
    number: i64,
    thread: &str,
    resolved: bool,
) -> Result<(), String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    if thread.is_empty() {
        return Err("That thread cannot be resolved from here".to_string());
    }

    match call.kind {
        ForgeKind::GitHub => {
            let query = if resolved {
                "mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{id isResolved}}}"
            } else {
                "mutation($id:ID!){unresolveReviewThread(input:{threadId:$id}){thread{id isResolved}}}"
            };
            github_graphql(
                &http,
                &call.token,
                &call.host,
                query,
                serde_json::json!({ "id": thread }),
            )
            .await?;
        }
        ForgeKind::GitLab => {
            let project = urlencode(&call.slug.full());
            let url = format!(
                "{base}/projects/{project}/merge_requests/{number}/discussions/{thread}?resolved={resolved}"
            );
            let response = http
                .put(url)
                .bearer_auth(&call.token)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !response.status().is_success() {
                return Err(describe(response).await);
            }
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    Ok(())
}

/* ---------- managing the review ------------------------------------------ */

/// Sets who owns the review and who is being asked to look at it.
///
/// GitLab takes both in one write; GitHub keeps assignees on the issue and
/// reviewers on the pull request, and only ever adds or removes reviewers —
/// so the ones already asked are read first and the difference sent.
pub async fn set_review_people(
    state: &AppState,
    number: i64,
    assignees: Vec<Member>,
    reviewers: Vec<Member>,
) -> Result<(), String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    match call.kind {
        ForgeKind::GitHub => {
            let logins: Vec<String> = assignees.iter().map(|one| one.login.clone()).collect();
            let response = http
                .patch(format!("{base}/repos/{slug}/issues/{number}"))
                .bearer_auth(&call.token)
                .header("Accept", "application/vnd.github+json")
                .json(&serde_json::json!({ "assignees": logins }))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !response.status().is_success() {
                return Err(describe(response).await);
            }

            let wanted: Vec<String> = reviewers.iter().map(|one| one.login.clone()).collect();
            let pull = fetch(&http, &call.token, &format!("{base}/repos/{slug}/pulls/{number}")).await?;
            let asked: Vec<String> = pull
                .get("requested_reviewers")
                .and_then(|v| v.as_array())
                .map(|list| list.iter().map(|one| string(one, &["login"])).collect())
                .unwrap_or_default();

            let added: Vec<&String> = wanted.iter().filter(|one| !asked.contains(one)).collect();
            let dropped: Vec<&String> = asked.iter().filter(|one| !wanted.contains(one)).collect();
            let url = format!("{base}/repos/{slug}/pulls/{number}/requested_reviewers");
            if !added.is_empty() {
                let response = http
                    .post(&url)
                    .bearer_auth(&call.token)
                    .header("Accept", "application/vnd.github+json")
                    .json(&serde_json::json!({ "reviewers": added }))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if !response.status().is_success() {
                    return Err(describe(response).await);
                }
            }
            if !dropped.is_empty() {
                let response = http
                    .delete(&url)
                    .bearer_auth(&call.token)
                    .header("Accept", "application/vnd.github+json")
                    .json(&serde_json::json!({ "reviewers": dropped }))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if !response.status().is_success() {
                    return Err(describe(response).await);
                }
            }
        }
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            // GitLab addresses people by number. Somebody the project's member
            // list did not know arrives without one, and sending a zero would
            // read as "nobody" — which would quietly clear the row instead of
            // keeping them on it.
            let nameless: Vec<&str> = assignees
                .iter()
                .chain(reviewers.iter())
                .filter(|one| one.id <= 0)
                .map(|one| one.login.as_str())
                .collect();
            if !nameless.is_empty() {
                return Err(format!(
                    "GitLab needs an account id for {}, and this project's member list does not have one. Open the merge request on the forge to change that row.",
                    nameless.join(", ")
                ));
            }
            let assignee_ids: Vec<i64> = assignees.iter().map(|one| one.id).collect();
            let reviewer_ids: Vec<i64> = reviewers.iter().map(|one| one.id).collect();
            let response = http
                .put(format!("{base}/projects/{project}/merge_requests/{number}"))
                .bearer_auth(&call.token)
                .json(&serde_json::json!({
                    "assignee_ids": assignee_ids,
                    "reviewer_ids": reviewer_ids
                }))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !response.status().is_success() {
                return Err(describe(response).await);
            }
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    Ok(())
}

/// Every label this project has, for the picker to offer.
pub async fn project_labels(state: &AppState) -> Result<Vec<Label>, String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    let items = match call.kind {
        ForgeKind::GitHub => {
            let repo = format!("{base}/repos/{slug}/labels");
            paged(&http, &call.token, move |page| {
                format!("{repo}?per_page=100&page={page}")
            })
            .await?
        }
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            let repo = format!("{base}/projects/{project}/labels");
            paged(&http, &call.token, move |page| {
                format!("{repo}?per_page=100&page={page}")
            })
            .await?
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    };
    Ok(labels(Some(&serde_json::Value::Array(items))))
}

/// Sets the review's labels to exactly these.
pub async fn set_labels(state: &AppState, number: i64, names: Vec<String>) -> Result<(), String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    let response = match call.kind {
        ForgeKind::GitHub => http
            .put(format!("{base}/repos/{slug}/issues/{number}/labels"))
            .bearer_auth(&call.token)
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({ "labels": names }))
            .send()
            .await,
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            http.put(format!("{base}/projects/{project}/merge_requests/{number}"))
                .bearer_auth(&call.token)
                .json(&serde_json::json!({ "labels": names.join(",") }))
                .send()
                .await
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    Ok(())
}

/// Rewrites the review's title and description.
pub async fn update_review(
    state: &AppState,
    number: i64,
    title: &str,
    body: &str,
) -> Result<(), String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    let response = match call.kind {
        ForgeKind::GitHub => http
            .patch(format!("{base}/repos/{slug}/pulls/{number}"))
            .bearer_auth(&call.token)
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({ "title": title, "body": body }))
            .send()
            .await,
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            http.put(format!("{base}/projects/{project}/merge_requests/{number}"))
                .bearer_auth(&call.token)
                .json(&serde_json::json!({ "title": title, "description": body }))
                .send()
                .await
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    Ok(())
}

/// The title a GitLab merge request wears for the draft it is or is not.
///
/// GitLab has no draft flag to set: a merge request is a draft because its
/// title begins with the word, so marking one ready is a rename.
fn gitlab_draft_title(title: &str, draft: bool) -> String {
    let bare = title
        .trim_start_matches("Draft:")
        .trim_start_matches("draft:")
        .trim_start_matches("WIP:")
        .trim_start_matches("wip:")
        .trim()
        .to_string();
    if draft {
        format!("Draft: {bare}")
    } else {
        bare
    }
}

/// Marks the review ready to be read, or puts it back to a draft.
pub async fn set_draft(state: &AppState, number: i64, draft: bool) -> Result<(), String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    match call.kind {
        ForgeKind::GitHub => {
            // REST cannot do this one either way round, so GraphQL again: the
            // pull request's node id, and then the mutation that fits.
            const ID: &str = "query($owner:String!,$name:String!,$number:Int!){\
                repository(owner:$owner,name:$name){pullRequest(number:$number){id}}}";
            let data = github_graphql(
                &http,
                &call.token,
                &call.host,
                ID,
                serde_json::json!({
                    "owner": call.slug.owner, "name": call.slug.name, "number": number
                }),
            )
            .await?;
            let id = data
                .pointer("/repository/pullRequest/id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                return Err("The forge did not say which pull request that is".to_string());
            }
            let query = if draft {
                "mutation($id:ID!){convertPullRequestToDraft(input:{pullRequestId:$id}){pullRequest{id isDraft}}}"
            } else {
                "mutation($id:ID!){markPullRequestReadyForReview(input:{pullRequestId:$id}){pullRequest{id isDraft}}}"
            };
            github_graphql(
                &http,
                &call.token,
                &call.host,
                query,
                serde_json::json!({ "id": id }),
            )
            .await?;
        }
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            let url = format!("{base}/projects/{project}/merge_requests/{number}");
            let item = fetch(&http, &call.token, &url).await?;
            let title = gitlab_draft_title(&string(&item, &["title"]), draft);
            let response = http
                .put(url)
                .bearer_auth(&call.token)
                .json(&serde_json::json!({ "title": title }))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !response.status().is_success() {
                return Err(describe(response).await);
            }
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    Ok(())
}

/// One file as it stands at the review's head, for reading the change in
/// place rather than as a patch.
///
/// The patch only carries the lines around what changed; the file view wants
/// the whole thing, which neither forge offers as part of the review itself —
/// so the head sha is read off the review and the file fetched at it.
pub async fn review_file_text(
    state: &AppState,
    number: i64,
    path: &str,
) -> Result<String, String> {
    let call = prepare(state)?;
    let base = api_base(call.kind, &call.host);
    let http = client()?;
    let slug = call.slug.full();

    // The review's head, which is the only ref that names what is being read.
    let head = {
        let response = match call.kind {
            ForgeKind::GitHub => http
                .get(format!("{base}/repos/{slug}/pulls/{number}"))
                .bearer_auth(&call.token)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await,
            ForgeKind::GitLab => {
                let project = urlencode(&slug);
                http.get(format!("{base}/projects/{project}/merge_requests/{number}"))
                    .bearer_auth(&call.token)
                    .send()
                    .await
            }
            ForgeKind::None => return Err("No forge configured".to_string()),
        }
        .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(describe(response).await);
        }
        let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        match call.kind {
            ForgeKind::GitHub => string(&body, &["head", "sha"]),
            _ => {
                let from_refs = string(&body, &["diff_refs", "head_sha"]);
                if from_refs.is_empty() {
                    string(&body, &["sha"])
                } else {
                    from_refs
                }
            }
        }
    };
    if head.is_empty() {
        return Err("The review does not say which commit it is on".to_string());
    }

    let response = match call.kind {
        // The raw media type turns the contents endpoint into the file itself.
        ForgeKind::GitHub => http
            .get(format!("{base}/repos/{slug}/contents/{}", urlencode_path(path)))
            .bearer_auth(&call.token)
            .header("Accept", "application/vnd.github.raw")
            .query(&[("ref", head.as_str())])
            .send()
            .await,
        ForgeKind::GitLab => {
            let project = urlencode(&slug);
            http.get(format!(
                "{base}/projects/{project}/repository/files/{}/raw",
                urlencode_path(path)
            ))
            .query(&[("ref", head.as_str())])
            .bearer_auth(&call.token)
            .send()
            .await
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    }
    .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(describe(response).await);
    }
    Ok(response.text().await.map_err(|e| e.to_string())?)
}

/// Percent-encodes a path for a URL segment while keeping its own slashes:
/// `src/review/pane.ts` stays readable as the path it names.
fn urlencode_path(path: &str) -> String {
    path.split('/')
        .map(urlencode)
        .collect::<Vec<_>>()
        .join("/")
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
    fn reads_the_label_shapes_the_two_forges_send() {
        // GitLab without the detailed form: bare strings, no colour.
        let bare = serde_json::json!(["backend", "urgent"]);
        let read = labels(Some(&bare));
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].name, "backend");
        assert!(read[0].color.is_empty());

        // GitHub writes its colours without the `#`; GitLab writes them with.
        let rich = serde_json::json!([
            { "name": "bug", "color": "d73a4a" },
            { "name": "chore", "color": "#428bca" },
            { "name": "" }
        ]);
        let read = labels(Some(&rich));
        assert_eq!(read.len(), 2, "a nameless label is not a label");
        assert_eq!(read[0].color, "#d73a4a");
        assert_eq!(read[1].color, "#428bca");

        assert!(labels(None).is_empty());
    }

    #[test]
    fn reads_a_person_from_either_forge() {
        // GitHub: a login and nothing else.
        let (read, picture) = read_person(&serde_json::json!({
            "login": "arno",
            "avatar_url": "https://example.test/a.png"
        }));
        assert_eq!(read.login, "arno");
        assert_eq!(read.name, "arno", "with no real name the login stands in");
        assert_eq!(picture, "https://example.test/a.png");

        // GitLab: a username and a display name.
        let (read, _) = read_person(&serde_json::json!({
            "username": "arno",
            "name": "Arno Visker"
        }));
        assert_eq!(read.login, "arno");
        assert_eq!(read.name, "Arno Visker");

        // Anybody the forge lists without an account name is not somebody to
        // show, and there is no picture to go looking for either.
        let (read, picture) = read_person(&serde_json::json!({}));
        assert!(read.login.is_empty());
        assert!(picture.is_empty());
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

    #[test]
    fn reads_github_diff_comments_with_their_anchor() {
        // The modern shape: a named side and a line on it.
        let modern = github_diff_comment(&serde_json::json!({
            "id": 7, "user": { "login": "arno" },
            "body": "looks off", "created_at": "2026-01-02T03:04:05Z",
            "path": "app/main.rs", "side": "RIGHT", "line": 42
        }));
        assert_eq!(modern.kind, "diff");
        assert_eq!(modern.path.as_deref(), Some("app/main.rs"));
        assert_eq!(modern.line, Some(42));
        assert_eq!(modern.side.as_deref(), Some("new"));

        // LEFT is the old half of the diff, whatever its line.
        let left = github_diff_comment(&serde_json::json!({
            "id": 8, "user": { "login": "arno" }, "body": "",
            "path": "a", "side": "LEFT", "line": 3
        }));
        assert_eq!(left.side.as_deref(), Some("old"));

        // An older answer names no side; a comment with only an original line
        // was written against one that has since gone.
        let legacy = github_diff_comment(&serde_json::json!({
            "id": 9, "user": { "login": "arno" }, "body": "",
            "path": "a", "original_line": 5
        }));
        assert_eq!(legacy.side.as_deref(), Some("old"));
        assert_eq!(legacy.line, Some(5));

        // A reply points at the root of its thread.
        let reply = github_diff_comment(&serde_json::json!({
            "id": 10, "in_reply_to_id": 7, "user": { "login": "kai" },
            "body": "fair", "path": "app/main.rs", "side": "RIGHT", "line": 42
        }));
        assert_eq!(reply.reply_to, Some(7));
    }

    #[test]
    fn reads_gitlab_notes_and_leaves_system_ones() {
        // A conversation note: no position, no path.
        let talk = gitlab_note(&serde_json::json!({
            "id": 1, "author": { "username": "arno", "name": "Arno" },
            "body": "hello", "system": false,
            "discussion_id": "abc"
        }))
        .expect("kept");
        assert_eq!(talk.kind, "issue");
        assert_eq!(talk.path, None);
        assert_eq!(talk.author.name, "Arno");

        // A diff-anchored one lands on the side its line says.
        let diff = gitlab_note(&serde_json::json!({
            "id": 2, "author": { "username": "arno" },
            "body": "here?", "position": {
                "new_path": "src/a.ts", "old_path": "src/a.ts",
                "new_line": 12
            }
        }))
        .expect("kept");
        assert_eq!(diff.kind, "diff");
        assert_eq!(diff.path.as_deref(), Some("src/a.ts"));
        assert_eq!(diff.line, Some(12));
        assert_eq!(diff.side.as_deref(), Some("new"));

        // A deletion answered from the old side alone.
        let old_side = gitlab_note(&serde_json::json!({
            "id": 3, "author": { "username": "arno" },
            "body": "gone", "position": {
                "new_path": "f", "old_path": "f", "old_line": 4
            }
        }))
        .expect("kept");
        assert_eq!(old_side.side.as_deref(), Some("old"));
        assert_eq!(old_side.line, Some(4));

        // GitLab's own bookkeeping is not somebody talking.
        assert!(gitlab_note(&serde_json::json!({
            "id": 4, "author": { "username": "gitlab" },
            "body": "changed target branch", "system": true
        }))
        .is_none());
    }

    #[test]
    fn answers_gitlab_replies_to_the_root_of_their_discussion() {
        let raw = vec![
            serde_json::json!({ "id": 100, "discussion_id": "d1", "body": "root" }),
            serde_json::json!({ "id": 101, "discussion_id": "d2", "body": "other root" }),
            serde_json::json!({ "id": 102, "discussion_id": "d1", "body": "reply" }),
            serde_json::json!({ "id": 103, "discussion_id": "d1", "body": "reply to the reply" }),
        ];
        let mut comments: Vec<ReviewComment> = raw
            .iter()
            .filter_map(|item| {
                gitlab_note(&clone_json(item))
            })
            .collect();
        let in_order: Vec<serde_json::Value> =
            raw.iter().filter(|item| !is_system_note(item)).cloned().collect();
        gitlab_thread_replies(&mut comments, &in_order);

        assert_eq!(comments[0].reply_to, None, "the first of a thread opens it");
        assert_eq!(comments[2].reply_to, Some(100));
        assert_eq!(comments[3].reply_to, Some(100), "replies chain to the root, not each other");
        assert_eq!(comments[1].reply_to, None, "another thread stays another thread");
    }

    fn clone_json(value: &serde_json::Value) -> serde_json::Value {
        value.clone()
    }

    #[test]
    fn names_a_gitlab_file_from_its_three_booleans() {
        assert_eq!(gitlab_file_status(true, false, false), "added");
        assert_eq!(gitlab_file_status(false, true, false), "deleted");
        assert_eq!(gitlab_file_status(false, false, true), "renamed");
        assert_eq!(gitlab_file_status(false, false, false), "modified");
    }

    #[test]
    fn names_every_check_state_both_forges_report() {
        // Anything not finished is still running, whatever it will conclude.
        assert_eq!(github_check_state("in_progress", ""), "pending");
        assert_eq!(github_check_state("queued", "success"), "pending");
        assert_eq!(github_check_state("completed", "success"), "success");
        assert_eq!(github_check_state("completed", "failure"), "failure");
        assert_eq!(github_check_state("completed", "timed_out"), "failure");
        assert_eq!(github_check_state("completed", "action_required"), "failure");
        assert_eq!(github_check_state("completed", "cancelled"), "cancelled");
        assert_eq!(github_check_state("completed", "neutral"), "skipped");

        assert_eq!(github_status_state("success"), "success");
        assert_eq!(github_status_state("error"), "failure");
        assert_eq!(github_status_state("failure"), "failure");
        assert_eq!(github_status_state("pending"), "pending");

        assert_eq!(gitlab_job_state("success"), "success");
        assert_eq!(gitlab_job_state("failed"), "failure");
        assert_eq!(gitlab_job_state("canceled"), "cancelled");
        assert_eq!(gitlab_job_state("manual"), "skipped");
        assert_eq!(gitlab_job_state("running"), "pending");
        assert_eq!(gitlab_job_state("waiting_for_resource"), "pending");
    }

    #[test]
    fn rolls_a_wall_of_checks_into_one_word() {
        let check = |state: &str| Check {
            name: state.to_string(),
            state: state.to_string(),
            description: String::new(),
            url: String::new(),
        };
        assert_eq!(roll_up(&[]), "none");
        assert_eq!(roll_up(&[check("success"), check("success")]), "success");
        assert_eq!(
            roll_up(&[check("success"), check("pending")]),
            "pending",
            "anything still running means wait"
        );
        assert_eq!(
            roll_up(&[check("pending"), check("failure"), check("success")]),
            "failure",
            "one failure is the answer whatever else passed"
        );
        assert_eq!(
            roll_up(&[check("skipped"), check("cancelled")]),
            "skipped",
            "a wall of skips is not a pass anybody earned"
        );
    }

    #[test]
    fn keeps_one_standing_verdict_per_person() {
        let review = |login: &str, state: &str, at: &str| {
            serde_json::json!({ "user": { "login": login }, "state": state, "submitted_at": at, "body": "" })
        };
        let folded = fold_github_verdicts(&[
            review("kai", "COMMENTED", "1"),
            review("kai", "APPROVED", "2"),
            review("nadia", "CHANGES_REQUESTED", "3"),
            // A passing remark afterwards does not unseat a position taken.
            review("kai", "COMMENTED", "4"),
            review("nadia", "APPROVED", "5"),
            // Not sent yet: it belongs to nobody but its writer.
            review("sam", "PENDING", "6"),
            review("", "APPROVED", "7"),
        ]);

        assert_eq!(folded.len(), 2, "one row per person who took a position");
        let kai = folded.iter().find(|v| v.author.login == "kai").expect("kai");
        assert_eq!(kai.state, "approved");
        assert_eq!(kai.submitted_at, "2");
        let nadia = folded.iter().find(|v| v.author.login == "nadia").expect("nadia");
        assert_eq!(nadia.state, "approved", "the later position stands");

        // Somebody who has only ever commented still reads as having spoken.
        let only_talk = fold_github_verdicts(&[review("ada", "COMMENTED", "1")]);
        assert_eq!(only_talk.len(), 1);
        assert_eq!(only_talk[0].state, "commented");
    }

    #[test]
    fn hands_github_the_held_back_remarks_with_the_verdict() {
        let payload = github_pending_comments(&[
            PendingComment {
                path: "app/main.rs".to_string(),
                line: 12,
                side: "new".to_string(),
                body: "rename this".to_string(),
            },
            PendingComment {
                path: "app/old.rs".to_string(),
                line: 4,
                side: "old".to_string(),
                body: "why was this dropped?".to_string(),
            },
        ]);
        let list = payload.as_array().expect("an array of remarks");
        assert_eq!(list.len(), 2);
        assert_eq!(string(&list[0], &["path"]), "app/main.rs");
        assert_eq!(list[0].get("line").and_then(|v| v.as_i64()), Some(12));
        // GitHub names the halves of a diff by hand, not by ours.
        assert_eq!(string(&list[0], &["side"]), "RIGHT");
        assert_eq!(string(&list[1], &["side"]), "LEFT");
        assert_eq!(string(&list[1], &["body"]), "why was this dropped?");

        assert!(github_pending_comments(&[]).as_array().unwrap().is_empty());
    }

    #[test]
    fn renames_a_gitlab_merge_request_into_and_out_of_draft() {
        assert_eq!(gitlab_draft_title("Add the pane", true), "Draft: Add the pane");
        assert_eq!(gitlab_draft_title("Draft: Add the pane", false), "Add the pane");
        assert_eq!(gitlab_draft_title("WIP: Add the pane", false), "Add the pane");
        // Marking a draft a draft again is not two prefixes.
        assert_eq!(
            gitlab_draft_title("Draft: Add the pane", true),
            "Draft: Add the pane"
        );
    }

    #[test]
    fn points_graphql_at_the_right_host() {
        assert_eq!(
            github_graphql_url("github.com"),
            "https://api.github.com/graphql"
        );
        assert_eq!(
            github_graphql_url("github.acme.dev"),
            "https://github.acme.dev/api/graphql",
            "Enterprise answers GraphQL beside the REST base, not under it"
        );
    }

    #[test]
    fn reads_whether_a_thread_is_settled() {
        // GitLab says so on the note itself, along with the discussion it is in.
        let note = gitlab_note(&serde_json::json!({
            "id": 5, "author": { "username": "arno" }, "body": "still?",
            "discussion_id": "d9", "resolvable": true, "resolved": true,
            "position": { "new_path": "a.ts", "old_path": "a.ts", "new_line": 2, "outdated": true }
        }))
        .expect("kept");
        assert_eq!(note.thread, "d9");
        assert!(note.resolvable);
        assert!(note.resolved);
        assert!(note.outdated);

        // A conversation note is not a thread anybody settles.
        let talk = gitlab_note(&serde_json::json!({
            "id": 6, "author": { "username": "arno" }, "body": "hi", "discussion_id": "d1"
        }))
        .expect("kept");
        assert!(!talk.resolvable);

        // GitHub leaves both to the GraphQL pass, and a live line is not old.
        let fresh = github_diff_comment(&serde_json::json!({
            "id": 7, "user": { "login": "arno" }, "body": "",
            "path": "a", "side": "RIGHT", "line": 4
        }));
        assert!(!fresh.resolvable);
        assert!(!fresh.outdated);
        let gone = github_diff_comment(&serde_json::json!({
            "id": 8, "user": { "login": "arno" }, "body": "",
            "path": "a", "original_line": 4
        }));
        assert!(gone.outdated, "no line left to stand on");
    }

    #[test]
    fn counts_only_signed_lines_in_a_patch() {
        let patch = "\
@@ -1,3 +1,4 @@\
\n unchanged\
\n-removed line\
\n+added line\
\n\\ No newline at end of file\
\n+++ b/path\
\n--- a/path";
        let (additions, deletions) = count_patch_lines(patch);
        assert_eq!(additions, 1);
        assert_eq!(deletions, 1);
        assert_eq!(count_patch_lines(""), (0, 0));
    }
}


