//! Which forge project a repository's reviews live in, and whose account asks.
//!
//! This used to follow HEAD: the remote the checked-out branch tracked was the
//! project every call went to. Checking out a review from a fork makes the
//! branch track the fork, and from then on approving `#12` approved pull
//! request twelve of the *fork*. The answer now never looks at HEAD. It is, in
//! order: the remote this repository was told to use (`gitnoob.forgeRemote`),
//! the remote that origin was forked from when there is one here (the usual
//! `upstream` of a contributor's clone), and origin.
//!
//! The account is checked against the host as well. A GitHub profile used to
//! take only `owner/name` from a gitlab.com remote and ask api.github.com about
//! it — which is a different project, or somebody else's.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

use crate::config::{self, Config, ForgeKind, Profile};
use crate::state::AppState;

use super::*;

/// The git config key naming the remote a repository's reviews live on.
pub const FORGE_REMOTE_KEY: &str = "gitnoob.forgeRemote";

/// What the repository itself says, read in one go so no git2 handle is held
/// across a request.
#[derive(Debug, Default, Clone)]
pub struct Local {
    /// Every remote with an address that names a forge project, in the order
    /// git lists them.
    pub remotes: Vec<(String, RepoSlug)>,
    /// `gitnoob.forgeRemote`, when set.
    pub explicit: Option<String>,
    /// The branch checked out, `None` when HEAD is detached or unborn.
    pub head: Option<String>,
    /// What that branch is configured to track, as (remote, branch), read from
    /// the config so a tracked branch deleted on the forge still counts.
    pub upstream: Option<(String, String)>,
    /// The same for every local branch that tracks something.
    pub upstreams: HashMap<String, (String, String)>,
}

impl Local {
    pub fn read(state: &AppState) -> Result<Local, String> {
        let repo = state.repo()?;
        let mut remotes = Vec::new();
        if let Ok(names) = repo.remotes() {
            for name in names.iter().flatten() {
                let Ok(remote) = repo.find_remote(name) else {
                    continue;
                };
                if let Some(slug) = remote.url().and_then(parse_remote) {
                    remotes.push((name.to_string(), slug));
                }
            }
        }
        let config = repo.config().ok();
        let read = |key: &str| {
            config
                .as_ref()
                .and_then(|c| c.get_string(key).ok())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let explicit = read(FORGE_REMOTE_KEY);
        let head = repo
            .head()
            .ok()
            .filter(|head| head.is_branch())
            .and_then(|head| head.shorthand().map(str::to_string));
        let mut upstreams = HashMap::new();
        if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
            for (branch, _) in branches.flatten() {
                let Some(name) = branch.name().ok().flatten().map(str::to_string) else {
                    continue;
                };
                let remote = read(&format!("branch.{name}.remote"));
                let merge = read(&format!("branch.{name}.merge"));
                if let (Some(remote), Some(merge)) = (remote, merge) {
                    let short = merge
                        .strip_prefix("refs/heads/")
                        .unwrap_or(&merge)
                        .to_string();
                    upstreams.insert(name, (remote, short));
                }
            }
        }
        let upstream = head
            .as_ref()
            .and_then(|branch| upstreams.get(branch).cloned());
        Ok(Local {
            remotes,
            explicit,
            head,
            upstream,
            upstreams,
        })
    }

    pub fn slug_of(&self, remote: &str) -> Option<&RepoSlug> {
        self.remotes
            .iter()
            .find(|(name, _)| name == remote)
            .map(|(_, slug)| slug)
    }

    /// `origin`, or the first remote when there is no remote of that name.
    pub fn origin(&self) -> Option<&(String, RepoSlug)> {
        self.remotes
            .iter()
            .find(|(name, _)| name == "origin")
            .or_else(|| self.remotes.first())
    }
}

/// Where the forge project was decided to be, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    pub remote: String,
    pub slug: RepoSlug,
}

/// The decision, given what is known about origin's parent.
///
/// Pure, so the order of preference can be tested without a forge: `parent`
/// is the `owner/name` origin was forked from, when the forge has said.
pub fn choose(local: &Local, parent: Option<&str>) -> Result<Option<Chosen>, String> {
    if let Some(name) = &local.explicit {
        return match local.slug_of(name) {
            Some(slug) => Ok(Some(Chosen {
                remote: name.clone(),
                slug: slug.clone(),
            })),
            None => Err(format!(
                "{FORGE_REMOTE_KEY} names the remote {name}, which is not a remote of this repository with a forge address"
            )),
        };
    }
    let Some((origin_name, origin)) = local.origin() else {
        return Ok(None);
    };
    if let Some(parent) = parent {
        let matches = |slug: &RepoSlug| {
            same_host(&slug.host, &origin.host) && slug.full().eq_ignore_ascii_case(parent)
        };
        // `upstream` first, because that is the name everybody gives it; any
        // other remote pointing at the parent after that.
        let found = local
            .remotes
            .iter()
            .filter(|(name, slug)| name != origin_name && matches(slug))
            .min_by_key(|(name, _)| name != "upstream");
        if let Some((name, slug)) = found {
            return Ok(Some(Chosen {
                remote: name.clone(),
                slug: slug.clone(),
            }));
        }
    }
    Ok(Some(Chosen {
        remote: origin_name.clone(),
        slug: origin.clone(),
    }))
}

/// Whether origin has company on its own host worth asking the forge about.
/// A clone with one remote never needs its parent looked up.
pub fn wants_parent(local: &Local) -> bool {
    if local.explicit.is_some() {
        return false;
    }
    let Some((origin_name, origin)) = local.origin() else {
        return false;
    };
    local.remotes.iter().any(|(name, slug)| {
        name != origin_name && same_host(&slug.host, &origin.host) && slug.full() != origin.full()
    })
}

/// Where a branch lives on the forge, for opening a review from it: the
/// project it was pushed to and its name there. A branch that tracks nothing
/// is taken to be in the project itself under its own name.
pub fn pushed_to(local: &Local, forge: &RepoSlug, branch: &str) -> (RepoSlug, String) {
    match local.upstreams.get(branch) {
        Some((remote, name)) => match local.slug_of(remote) {
            Some(slug) => (slug.clone(), name.clone()),
            None => (forge.clone(), name.clone()),
        },
        None => (forge.clone(), branch.to_string()),
    }
}

/* ---------- hosts ---------------------------------------------------------- */

/// A host the way two spellings of it can be compared: lowercased, without a
/// port, and without the `ssh.` and `altssh.` both forges serve ssh from on
/// port 443.
///
/// The port is how the forge is reached, not which forge it is: a remote's
/// address loses it when it is read (`parse_remote`), while a profile for a
/// self-hosted forge on `:8443` keeps it, and the two never matched — every
/// call on such a forge was refused as the wrong account.
pub fn canonical_host(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    let host = match host.rsplit_once(':') {
        Some((name, port))
            if !name.is_empty()
                && !name.contains(':')
                && !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit()) =>
        {
            name.to_string()
        }
        _ => host,
    };
    let host = host.trim_end_matches('.').to_string();
    for prefix in ["ssh.", "altssh.", "www."] {
        if let Some(rest) = host.strip_prefix(prefix) {
            if rest.contains('.') {
                return rest.to_string();
            }
        }
    }
    host
}

/// Hosts `ssh -G` resolved, so an alias costs one process per session.
static SSH_NAMES: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// What `~/.ssh/config` says an alias really connects to.
///
/// Two accounts on one forge are usually told apart with a `Host github-work`
/// block, so `git@github-work:acme/app.git` is github.com as far as the API is
/// concerned. `ssh -G` prints the configuration without connecting anywhere.
fn ssh_hostname(alias: &str) -> Option<String> {
    if alias.is_empty() || alias.starts_with('-') {
        return None;
    }
    if let Some(known) = SSH_NAMES
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|seen| seen.get(alias).cloned())
    {
        return known;
    }
    let mut command = std::process::Command::new("ssh");
    crate::git_cmd::quiet(&mut command);
    let found = command
        .arg("-G")
        .arg(alias)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .find_map(|line| line.strip_prefix("hostname ").map(|h| h.trim().to_string()))
        })
        .filter(|name| !name.is_empty() && name != alias);
    SSH_NAMES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(alias.to_string(), found.clone());
    found
}

/// Whether a remote's host is the host an account talks to.
pub fn same_host(remote: &str, account: &str) -> bool {
    let (remote, account) = (canonical_host(remote), canonical_host(account));
    if remote.is_empty() || account.is_empty() {
        return false;
    }
    if remote == account {
        return true;
    }
    ssh_hostname(&remote)
        .map(|real| canonical_host(&real) == account)
        .unwrap_or(false)
}

/// The host a profile's forge lives on.
pub fn profile_host(profile: &Profile) -> String {
    if profile.host.trim().is_empty() {
        profile.forge.default_host().to_string()
    } else {
        profile.host.trim().to_string()
    }
}

/// How well a profile fits a remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    /// Another host, or no forge at all.
    No,
    /// The right host, and the profile is not limited to particular owners.
    Host,
    /// The right host and one of the owners the profile is for.
    Owner,
}

/// Whether a profile is the account for a project on the forge.
pub fn fit(profile: &Profile, slug: &RepoSlug) -> Fit {
    if profile.forge == ForgeKind::None || !same_host(&slug.host, &profile_host(profile)) {
        return Fit::No;
    }
    let owners: Vec<&str> = profile
        .owners
        .iter()
        .map(|o| o.trim().trim_matches('/'))
        .filter(|o| !o.is_empty())
        .collect();
    if owners.is_empty() {
        return Fit::Host;
    }
    // A GitLab group is a path; a profile for `acme` is for `acme/web/app` too.
    let owner = slug.owner.to_ascii_lowercase();
    let mine = owners.iter().any(|want| {
        let want = want.to_ascii_lowercase();
        owner == want || owner.starts_with(&format!("{want}/"))
    });
    if mine {
        Fit::Owner
    } else {
        Fit::No
    }
}

/// The profiles that fit a project best: the ones scoped to its owner when
/// there are any, every profile for its host otherwise.
pub fn best_profiles<'a>(config: &'a Config, slug: &RepoSlug) -> Vec<&'a Profile> {
    let fits: Vec<(&Profile, Fit)> = config
        .profiles
        .iter()
        .map(|profile| (profile, fit(profile, slug)))
        .filter(|(_, fit)| *fit != Fit::No)
        .collect();
    let best = fits.iter().map(|(_, fit)| *fit).max();
    fits.into_iter()
        .filter(|(_, fit)| Some(*fit) == best)
        .map(|(profile, _)| profile)
        .collect()
}

/// What a refusal says when the account and the project are on different
/// hosts: where the project is, which profile is asking, and which would fit.
pub fn mismatch_message(config: &Config, asking: &Profile, slug: &RepoSlug) -> String {
    let fitting: Vec<String> = best_profiles(config, slug)
        .iter()
        .map(|profile| profile.name.clone())
        .collect();
    let advice = match fitting.len() {
        0 => format!(
            "No profile is set up for {}; add one in Settings › Profiles.",
            slug.host
        ),
        1 => format!("The {} profile fits it.", fitting[0]),
        _ => format!("These profiles fit it: {}.", fitting.join(", ")),
    };
    format!(
        "This repository is on {}, and the {} profile talks to {}, so its token is not sent there. {advice}",
        slug.host,
        asking.name,
        profile_host(asking)
    )
}

/* ---------- the account ---------------------------------------------------- */

/// The profile the open repository answers to, its host and its token.
pub fn repo_account(state: &AppState) -> Result<(Profile, String, String), String> {
    let config = state.config();
    let path = state
        .path()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let profile = config
        .profile_for(&path)
        .cloned()
        .ok_or_else(|| "No profile is active".to_string())?;
    if profile.forge == ForgeKind::None {
        return Err(format!(
            "The {} profile has no forge configured",
            profile.name
        ));
    }
    let token = config::secret_get(&config::forge_key(&profile.id))
        .ok_or_else(|| format!("No access token stored for the {} profile", profile.name))?;
    let host = profile_host(&profile);
    Ok((profile, host, token))
}

/* ---------- what a forge project is ---------------------------------------- */

/// The facts about a project that do not change while it is open: its default
/// branch, how it lets reviews merge, and what it was forked from.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct RepoFacts {
    pub full_name: String,
    pub default_branch: String,
    /// `merge`, `squash`, `rebase`: the ones the project allows, in that order.
    /// All three when the forge did not say.
    pub merge_methods: Vec<String>,
    /// `owner/name` of the project this one is a fork of.
    pub parent: Option<String>,
    /// GitLab's numeric project id, which a merge request from a fork needs.
    pub id: i64,
}

static FACTS: Mutex<Option<HashMap<String, RepoFacts>>> = Mutex::new(None);

fn facts_key(kind: ForgeKind, host: &str, slug: &RepoSlug) -> String {
    format!(
        "{kind:?}@{}/{}",
        canonical_host(host),
        slug.full().to_ascii_lowercase()
    )
}

/// Reads GitHub's answer for a repository.
pub fn github_facts(item: &serde_json::Value) -> RepoFacts {
    let flag = |key: &str| item.get(key).and_then(|v| v.as_bool());
    let mut methods = Vec::new();
    let said = [
        ("merge", flag("allow_merge_commit")),
        ("squash", flag("allow_squash_merge")),
        ("rebase", flag("allow_rebase_merge")),
    ];
    // The flags are only sent to somebody who may change them; a reader is
    // told nothing, and then all three are worth offering.
    if said.iter().all(|(_, value)| value.is_none()) {
        methods = vec!["merge".into(), "squash".into(), "rebase".into()];
    } else {
        for (name, value) in said {
            if value.unwrap_or(false) {
                methods.push(name.to_string());
            }
        }
    }
    let parent = string(item, &["parent", "full_name"]);
    RepoFacts {
        full_name: string(item, &["full_name"]),
        default_branch: string(item, &["default_branch"]),
        merge_methods: methods,
        parent: (!parent.is_empty()).then_some(parent),
        id: item.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
    }
}

/// Reads GitLab's answer for a project.
///
/// GitLab merges the way the project is set up to, so the choice here is only
/// whether to squash: `never` and `always` take the choice away.
pub fn gitlab_facts(item: &serde_json::Value) -> RepoFacts {
    let methods: Vec<String> = match string(item, &["squash_option"]).as_str() {
        "never" => vec!["merge".into()],
        "always" => vec!["squash".into()],
        _ => vec!["merge".into(), "squash".into()],
    };
    let parent = string(item, &["forked_from_project", "path_with_namespace"]);
    RepoFacts {
        full_name: string(item, &["path_with_namespace"]),
        default_branch: string(item, &["default_branch"]),
        merge_methods: methods,
        parent: (!parent.is_empty()).then_some(parent),
        id: item.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
    }
}

/// The facts about one project, asked once a session.
pub async fn facts(
    http: &reqwest::Client,
    kind: ForgeKind,
    host: &str,
    token: &str,
    slug: &RepoSlug,
) -> Result<RepoFacts, String> {
    let key = facts_key(kind, host, slug);
    if let Some(known) = FACTS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|all| all.get(&key).cloned())
    {
        return Ok(known);
    }
    let base = api_base(kind, host);
    let found = match kind {
        ForgeKind::GitHub => {
            let item = fetch(http, token, &format!("{base}/repos/{}", slug.full())).await?;
            github_facts(&item)
        }
        ForgeKind::GitLab => {
            let item = fetch(
                http,
                token,
                &format!("{base}/projects/{}", urlencode(&slug.full())),
            )
            .await?;
            gitlab_facts(&item)
        }
        ForgeKind::None => return Err("No forge configured".to_string()),
    };
    FACTS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(key, found.clone());
    Ok(found)
}

/* ---------- the decision, made ------------------------------------------- */

/// The branch checked out, for telling which review is the one you are on.
#[derive(Debug, Clone, Default)]
pub struct Current {
    pub branch: Option<String>,
    /// The branch name on the remote it tracks.
    pub upstream_branch: Option<String>,
    /// `owner/name` of the project that remote points at.
    pub upstream_repo: Option<String>,
}

impl Current {
    pub fn from_local(local: &Local) -> Current {
        let (upstream_branch, upstream_repo) = match &local.upstream {
            Some((remote, branch)) => (
                Some(branch.clone()),
                local.slug_of(remote).map(|slug| slug.full()),
            ),
            None => (None, None),
        };
        Current {
            branch: local.head.clone(),
            upstream_branch,
            upstream_repo,
        }
    }
}

/// Whether a review is the one for the branch checked out.
///
/// By what the branch tracks rather than what it is called here: `fix` tracking
/// `origin/arno/fix` is the review from `arno/fix`. A branch tracking nothing
/// is matched by its own name, in the project itself.
pub fn is_current(review: &Review, current: &Current, forge_repo: &str) -> bool {
    // GitLab leaves a review from the project itself without a name for where
    // it lives; that is the project being reviewed.
    let head_repo = review.source.as_ref().map(|source| {
        if source.full_name.is_empty() && !source.is_fork {
            forge_repo.to_string()
        } else {
            source.full_name.clone()
        }
    });
    let Some(head_repo) = head_repo else {
        return false;
    };
    match (&current.upstream_branch, &current.upstream_repo) {
        (Some(branch), Some(repo)) => {
            &review.source_branch == branch && head_repo.eq_ignore_ascii_case(repo)
        }
        _ => {
            current.branch.as_deref() == Some(review.source_branch.as_str())
                && head_repo.eq_ignore_ascii_case(forge_repo)
        }
    }
}

/// Everything a request about the open repository needs.
pub(super) async fn prepare(state: &AppState) -> Result<Call, String> {
    let local = Local::read(state)?;
    let (profile, host, token) = repo_account(state)?;
    let kind = profile.forge;

    let parent = if wants_parent(&local) {
        match local.origin() {
            Some((_, origin)) if same_host(&origin.host, &host) => {
                parent_of(kind, &host, &token, origin).await?
            }
            _ => None,
        }
    } else {
        None
    };
    let chosen = choose(&local, parent.as_deref())?
        .ok_or_else(|| "Could not work out the project from the git remote".to_string())?;
    if !same_host(&chosen.slug.host, &host) {
        return Err(mismatch_message(&state.config(), &profile, &chosen.slug));
    }

    Ok(Call {
        kind,
        host,
        token,
        current_branch: local.head.clone(),
        current: Current::from_local(&local),
        slug: chosen.slug,
        remote: chosen.remote,
        local,
    })
}

/// Lookups of a fork's parent that failed lately, by [`facts_key`], with when
/// and why.
static PARENT_FAILED: Mutex<Option<HashMap<String, (std::time::Instant, String)>>> =
    Mutex::new(None);

/// How long a failed parent lookup is believed before it is tried again.
const PARENT_RETRY: std::time::Duration = std::time::Duration::from_secs(300);

/// The project `origin` was forked from, when another remote on the same host
/// says it may have been.
///
/// A lookup that fails is remembered for a few minutes and refused with words
/// rather than taken as "not a fork": falling back to origin on one request
/// and finding the parent on the next made the project the reviews come from
/// flip between the fork and the parent from one refresh to another.
async fn parent_of(
    kind: ForgeKind,
    host: &str,
    token: &str,
    origin: &RepoSlug,
) -> Result<Option<String>, String> {
    let key = facts_key(kind, host, origin);
    let refused = |why: &str| {
        format!(
            "Could not ask {host} which project {} was forked from, so it is not clear which \
             one the reviews are in: {why}. Try again in a few minutes, or name the remote \
             with `git config gitnoob.forgeRemote <remote>`.",
            origin.full()
        )
    };
    if let Some((at, why)) = PARENT_FAILED
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|all| all.get(&key).cloned())
    {
        if at.elapsed() < PARENT_RETRY {
            return Err(refused(&why));
        }
    }
    let http = client()?;
    match facts(&http, kind, host, token, origin).await {
        Ok(known) => {
            if let Some(all) = PARENT_FAILED.lock().unwrap().as_mut() {
                all.remove(&key);
            }
            Ok(known.parent)
        }
        Err(why) => {
            PARENT_FAILED
                .lock()
                .unwrap()
                .get_or_insert_with(HashMap::new)
                .insert(key, (std::time::Instant::now(), why.clone()));
            Err(refused(&why))
        }
    }
}

/// The remote the reviews of the open repository live on, without asking the
/// forge anything: the explicit choice, then a parent already learned this
/// session, then origin. For the callers that cannot wait on a request.
pub fn known_target(state: &AppState) -> Option<Chosen> {
    let local = Local::read(state).ok()?;
    let parent = local.origin().and_then(|(_, origin)| {
        let all = FACTS.lock().unwrap();
        let all = all.as_ref()?;
        all.iter()
            .find(|(key, _)| key.ends_with(&format!("/{}", origin.full().to_ascii_lowercase())))
            .and_then(|(_, facts)| facts.parent.clone())
    });
    choose(&local, parent.as_deref()).ok().flatten()
}

/* ---------- which profile a repository belongs to -------------------------- */

/// A profile, as a choice offered to the window.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ProfileChoice {
    pub id: String,
    pub name: String,
}

/// Whether the open repository's remote fits the profile it answers to, and
/// which profile would if it does not.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct ProfileMatch {
    /// Where the project is, when the remote names a forge at all.
    pub host: Option<String>,
    pub repo: Option<String>,
    /// The profile in force for this repository.
    pub using: Option<ProfileChoice>,
    /// True when that profile was given to this repository rather than being
    /// the active one.
    pub bound: bool,
    /// True when the profile in force fits, or when nothing could: a remote
    /// that names no forge, or one no profile is set up for.
    pub fits: bool,
    /// The profiles that fit best.
    pub best: Vec<ProfileChoice>,
}

/// The answer for a project, given the config: pure, for the tests.
pub fn match_profile(config: &Config, path: &str, slug: Option<&RepoSlug>) -> ProfileMatch {
    let using = config.profile_for(path);
    let bound = config
        .repo_profiles
        .get(path)
        .is_some_and(|id| config.profiles.iter().any(|p| &p.id == id));
    let Some(slug) = slug else {
        return ProfileMatch {
            using: using.map(|p| ProfileChoice {
                id: p.id.clone(),
                name: p.name.clone(),
            }),
            bound,
            fits: true,
            ..ProfileMatch::default()
        };
    };
    let best: Vec<ProfileChoice> = best_profiles(config, slug)
        .into_iter()
        .map(|p| ProfileChoice {
            id: p.id.clone(),
            name: p.name.clone(),
        })
        .collect();
    let fits = match using {
        Some(profile) => best.is_empty() || best.iter().any(|b| b.id == profile.id),
        None => best.is_empty(),
    };
    ProfileMatch {
        host: Some(slug.host.clone()),
        repo: Some(slug.full()),
        using: using.map(|p| ProfileChoice {
            id: p.id.clone(),
            name: p.name.clone(),
        }),
        bound,
        fits,
        best,
    }
}

/// The same for the repository that is open.
pub fn profile_match(state: &AppState) -> ProfileMatch {
    let path = state
        .path()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let slug = known_target(state).map(|chosen| chosen.slug);
    match_profile(&state.config(), &path, slug.as_ref())
}

/// Whether a profile's identity belongs in this repository: it does unless
/// the profile is for a forge and the repository's remote is on another one.
/// A profile with no forge is a name and an address, and fits anywhere.
pub fn identity_fits(profile: &Profile, slug: Option<&RepoSlug>) -> bool {
    match slug {
        Some(slug) if profile.forge != ForgeKind::None => fit(profile, slug) != Fit::No,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slug(host: &str, owner: &str, name: &str) -> RepoSlug {
        RepoSlug {
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    fn profile(name: &str, forge: ForgeKind, host: &str, owners: &[&str]) -> Profile {
        Profile {
            host: host.into(),
            owners: owners.iter().map(|o| o.to_string()).collect(),
            ..Profile::new(name, forge)
        }
    }

    #[test]
    fn a_parent_lookup_that_failed_is_refused_in_words_until_it_is_tried_again() {
        let origin = slug("github.example.com", "robin", "app");
        let key = facts_key(ForgeKind::GitHub, "github.example.com", &origin);
        PARENT_FAILED
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(key, (std::time::Instant::now(), "timed out".to_string()));
        // Remembered: no request goes out, and the answer is not "not a fork".
        let said = tauri::async_runtime::block_on(parent_of(
            ForgeKind::GitHub,
            "github.example.com",
            "token",
            &origin,
        ))
        .unwrap_err();
        assert!(
            said.contains("forked from") && said.contains("timed out"),
            "{said}"
        );
    }

    #[test]
    fn hosts_compare_without_their_port() {
        assert_eq!(
            canonical_host("GitLab.Example.com:8443"),
            "gitlab.example.com"
        );
        assert_eq!(canonical_host("ssh.github.com:443"), "github.com");
    }

    #[test]
    fn hosts_compare_without_case_or_the_ssh_prefixes() {
        assert_eq!(canonical_host("SSH.GitHub.com"), "github.com");
        assert_eq!(canonical_host("altssh.gitlab.com"), "gitlab.com");
        assert!(same_host("ssh.github.com", "github.com"));
        assert!(same_host("GitHub.com", "github.com"));
        assert!(!same_host("gitlab.com", "github.com"));
        assert!(!same_host("", "github.com"));
        // A host that merely ends in the same words is a different host.
        assert!(!same_host("notgithub.com", "github.com"));
    }

    #[test]
    fn origin_is_the_project_when_nothing_else_is_said() {
        let local = Local {
            remotes: vec![("origin".into(), slug("github.com", "acme", "widget"))],
            ..Local::default()
        };
        let chosen = choose(&local, None).unwrap().unwrap();
        assert_eq!(chosen.remote, "origin");
        assert!(!wants_parent(&local), "one remote needs no lookup");
    }

    #[test]
    fn upstream_wins_when_origin_is_a_fork_of_it() {
        let local = Local {
            remotes: vec![
                ("origin".into(), slug("github.com", "arno", "widget")),
                ("upstream".into(), slug("github.com", "acme", "widget")),
                ("kai".into(), slug("github.com", "kai", "widget")),
            ],
            // HEAD tracking somebody's fork changes nothing about the answer.
            upstream: Some(("kai".into(), "fix-typo".into())),
            head: Some("fix-typo".into()),
            ..Local::default()
        };
        assert!(wants_parent(&local));
        let chosen = choose(&local, Some("acme/widget")).unwrap().unwrap();
        assert_eq!(chosen.remote, "upstream");
        assert_eq!(chosen.slug.full(), "acme/widget");

        // Not a fork, or a fork of something else: origin.
        assert_eq!(choose(&local, None).unwrap().unwrap().remote, "origin");
        assert_eq!(
            choose(&local, Some("other/widget"))
                .unwrap()
                .unwrap()
                .remote,
            "origin"
        );
    }

    #[test]
    fn the_parent_is_found_under_any_name() {
        let local = Local {
            remotes: vec![
                ("origin".into(), slug("github.com", "arno", "widget")),
                ("main-repo".into(), slug("github.com", "Acme", "Widget")),
            ],
            ..Local::default()
        };
        assert_eq!(
            choose(&local, Some("acme/widget")).unwrap().unwrap().remote,
            "main-repo"
        );
    }

    #[test]
    fn an_explicit_choice_beats_everything_and_must_exist() {
        let mut local = Local {
            remotes: vec![
                ("origin".into(), slug("github.com", "arno", "widget")),
                ("upstream".into(), slug("github.com", "acme", "widget")),
            ],
            explicit: Some("origin".into()),
            ..Local::default()
        };
        assert!(!wants_parent(&local));
        assert_eq!(
            choose(&local, Some("acme/widget")).unwrap().unwrap().remote,
            "origin"
        );
        local.explicit = Some("nope".into());
        assert!(choose(&local, None).is_err());
    }

    #[test]
    fn no_remote_is_no_project() {
        assert_eq!(choose(&Local::default(), None).unwrap(), None);
    }

    #[test]
    fn profiles_fit_by_host_and_then_by_owner() {
        let gh = profile("Personal", ForgeKind::GitHub, "github.com", &[]);
        let work = profile("Work", ForgeKind::GitHub, "github.com", &["acme"]);
        let lab = profile("Lab", ForgeKind::GitLab, "gitlab.com", &[]);
        let none = profile("Plain", ForgeKind::None, "", &[]);

        let acme = slug("github.com", "acme", "widget");
        assert_eq!(fit(&gh, &acme), Fit::Host);
        assert_eq!(fit(&work, &acme), Fit::Owner);
        assert_eq!(fit(&lab, &acme), Fit::No);
        assert_eq!(fit(&none, &acme), Fit::No);
        assert_eq!(fit(&work, &slug("github.com", "arno", "x")), Fit::No);
        // A group's subgroups belong to it.
        let group = profile("Group", ForgeKind::GitLab, "gitlab.com", &["acme"]);
        assert_eq!(
            fit(&group, &slug("gitlab.com", "acme/web", "app")),
            Fit::Owner
        );

        let config = Config {
            profiles: vec![gh.clone(), work.clone(), lab.clone()],
            ..Config::default()
        };
        let best: Vec<String> = best_profiles(&config, &acme)
            .iter()
            .map(|p| p.name.clone())
            .collect();
        assert_eq!(
            best,
            vec!["Work"],
            "the scoped profile beats the general one"
        );
        let best: Vec<String> = best_profiles(&config, &slug("github.com", "arno", "x"))
            .iter()
            .map(|p| p.name.clone())
            .collect();
        assert_eq!(best, vec!["Personal"]);
    }

    #[test]
    fn a_refusal_names_the_profile_that_would_fit() {
        let gh = profile("Personal", ForgeKind::GitHub, "github.com", &[]);
        let lab = profile("Lab", ForgeKind::GitLab, "gitlab.com", &[]);
        let config = Config {
            profiles: vec![gh.clone(), lab],
            ..Config::default()
        };
        let said = mismatch_message(&config, &gh, &slug("gitlab.com", "acme", "widget"));
        assert!(said.contains("gitlab.com"), "{said}");
        assert!(said.contains("Personal"), "{said}");
        assert!(said.contains("The Lab profile fits it"), "{said}");

        let said = mismatch_message(&config, &gh, &slug("git.acme.dev", "acme", "widget"));
        assert!(
            said.contains("No profile is set up for git.acme.dev"),
            "{said}"
        );
    }

    #[test]
    fn the_current_review_is_matched_by_what_the_branch_tracks() {
        let review = |branch: &str, repo: &str, fork: bool| Review {
            number: 1,
            title: String::new(),
            author: String::new(),
            state: "open".into(),
            draft: false,
            source_branch: branch.into(),
            target_branch: "main".into(),
            url: String::new(),
            updated_at: String::new(),
            is_current: false,
            head_sha: String::new(),
            source: Some(ReviewSource {
                full_name: repo.into(),
                owner: String::new(),
                ssh_url: String::new(),
                https_url: String::new(),
                is_fork: fork,
            }),
            warning: None,
            checks: None,
            review_state: None,
        };
        // `fix` here tracks `origin/arno/fix`.
        let tracking = Current {
            branch: Some("fix".into()),
            upstream_branch: Some("arno/fix".into()),
            upstream_repo: Some("acme/widget".into()),
        };
        assert!(is_current(
            &review("arno/fix", "acme/widget", false),
            &tracking,
            "acme/widget"
        ));
        assert!(!is_current(
            &review("fix", "acme/widget", false),
            &tracking,
            "acme/widget"
        ));
        // Same branch name, somebody else's fork.
        assert!(!is_current(
            &review("arno/fix", "kai/widget", true),
            &tracking,
            "acme/widget"
        ));

        // A contributor's branch pushed to their fork, reviewed upstream.
        let forked = Current {
            branch: Some("feature".into()),
            upstream_branch: Some("feature".into()),
            upstream_repo: Some("arno/widget".into()),
        };
        assert!(is_current(
            &review("feature", "arno/widget", true),
            &forked,
            "acme/widget"
        ));

        // Nothing tracked: the name, in the project itself.
        let bare = Current {
            branch: Some("feature".into()),
            ..Current::default()
        };
        assert!(is_current(
            &review("feature", "acme/widget", false),
            &bare,
            "acme/widget"
        ));
        assert!(!is_current(
            &review("feature", "kai/widget", true),
            &bare,
            "acme/widget"
        ));
        // GitLab's own-project review carries no name for where it lives.
        assert!(is_current(
            &review("feature", "", false),
            &bare,
            "acme/widget"
        ));
    }

    #[test]
    fn reads_what_each_forge_allows_a_merge_to_be() {
        let open = github_facts(&serde_json::json!({
            "full_name": "acme/widget", "default_branch": "trunk", "id": 9
        }));
        assert_eq!(open.default_branch, "trunk");
        assert_eq!(open.merge_methods, vec!["merge", "squash", "rebase"]);
        assert_eq!(open.parent, None);

        let squash_only = github_facts(&serde_json::json!({
            "full_name": "arno/widget", "default_branch": "main",
            "allow_merge_commit": false, "allow_squash_merge": true, "allow_rebase_merge": false,
            "parent": { "full_name": "acme/widget" }
        }));
        assert_eq!(squash_only.merge_methods, vec!["squash"]);
        assert_eq!(squash_only.parent.as_deref(), Some("acme/widget"));

        let lab = gitlab_facts(&serde_json::json!({
            "path_with_namespace": "group/app", "default_branch": "develop", "id": 42,
            "squash_option": "always",
            "forked_from_project": { "path_with_namespace": "upstream/app" }
        }));
        assert_eq!(lab.merge_methods, vec!["squash"]);
        assert_eq!(lab.id, 42);
        assert_eq!(lab.parent.as_deref(), Some("upstream/app"));
        assert_eq!(
            gitlab_facts(&serde_json::json!({ "squash_option": "default_off" })).merge_methods,
            vec!["merge", "squash"]
        );
    }
}
