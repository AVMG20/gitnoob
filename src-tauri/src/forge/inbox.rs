//! What the forges want from you, across every project at once.
//!
//! The home tab asks each account it has a token for two things: which reviews
//! are waiting on you, and which of your own are stuck — failing checks, or
//! changes asked for. Each row is matched to the project on disk it belongs
//! to, so the home tab can open it there rather than in a browser.

use std::collections::HashMap;

use serde::Serialize;

use crate::config::{self, ForgeKind};
use crate::state::AppState;

use super::*;

/// One thing a forge says needs you.
#[derive(Serialize, Clone)]
pub struct InboxRow {
    /// `requested` (somebody wants your review), `failing` (checks failed on
    /// yours) or `changes` (changes were asked for on yours).
    pub kind: String,
    pub host: String,
    /// `owner/name` of the project the review is in.
    pub repo: String,
    pub profile: String,
    /// The project on disk that is a clone of it, when there is one.
    pub path: Option<String>,
    pub review: Review,
}

#[derive(Serialize, Default)]
pub struct Inbox {
    pub rows: Vec<InboxRow>,
    /// One line per account that could not be asked, so the page can say so
    /// instead of claiming there is nothing waiting.
    pub errors: Vec<String>,
}

/// What a row asks of a review, given which list it came from. `None` for
/// one of your own that is going fine.
pub fn inbox_kind(requested: bool, review: &Review) -> Option<&'static str> {
    if requested {
        return Some("requested");
    }
    if review.checks.as_deref() == Some("failure") {
        return Some("failing");
    }
    if review.review_state.as_deref() == Some("changes_requested") {
        return Some("changes");
    }
    None
}

const GITHUB_FIELDS: &str = "number title state isDraft url updatedAt headRefName baseRefName \
    headRefOid author{login} headRepository{nameWithOwner sshUrl url owner{login}} \
    baseRepository{nameWithOwner}";

const GITLAB_FIELDS: &str = "iid title state draft webUrl updatedAt sourceBranch targetBranch \
    diffHeadSha author{username} project{fullPath} \
    sourceProject{fullPath sshUrlToRepo httpUrlToRepo namespace{path}}";

/// Every project the profile has opened, by the forge projects its remotes
/// point at: `host/owner/name`, lowercased, to the path on disk.
///
/// A remote the app added to check out somebody's review from their fork is
/// not a clone of that fork: it is left out, or every review from that fork
/// opened in whichever project happened to look at one of them once. And a
/// project's own forge remote — the one its reviews are read from — wins over
/// any other project that merely has the same project as a second remote.
pub fn projects_by_slug(state: &AppState) -> HashMap<String, String> {
    let config = state.config();
    let mut out = HashMap::new();
    let Some(profile) = config.active() else {
        return out;
    };
    let mut others: Vec<(String, String)> = Vec::new();
    for project in profile.projects.iter().chain(profile.recents.iter()) {
        let Ok(repo) = git2::Repository::open(&project.path) else {
            continue;
        };
        let Ok(names) = repo.remotes() else {
            continue;
        };
        let settings = repo.config().and_then(|mut c| c.snapshot()).ok();
        let read = |key: &str| settings.as_ref().and_then(|c| c.get_string(key).ok());
        let added_for_review = |name: &str| {
            settings
                .as_ref()
                .and_then(|c| c.get_bool(&format!("remote.{name}.gitnoob-review")).ok())
                .unwrap_or(false)
        };
        let own = read(FORGE_REMOTE_KEY).unwrap_or_else(|| "origin".to_string());
        for name in names.iter().flatten() {
            if added_for_review(name) {
                continue;
            }
            if let Some(slug) = repo
                .find_remote(name)
                .ok()
                .and_then(|remote| remote.url().and_then(parse_remote))
            {
                let key = slug_key(&slug.host, &slug.full());
                if name == own {
                    out.entry(key).or_insert_with(|| project.path.clone());
                } else {
                    others.push((key, project.path.clone()));
                }
            }
        }
    }
    for (key, path) in others {
        out.entry(key).or_insert(path);
    }
    out
}

fn slug_key(host: &str, full: &str) -> String {
    format!("{}/{}", canonical_host(host), full.to_ascii_lowercase())
}

/// Asks every account with a token what is waiting.
pub async fn inbox(state: &AppState) -> Inbox {
    let accounts: Vec<(String, ForgeKind, String, String)> = {
        let config = state.config();
        config
            .profiles
            .iter()
            .filter(|profile| profile.forge != ForgeKind::None)
            .filter_map(|profile| {
                let token = config::secret_get(&config::forge_key(&profile.id))?;
                Some((
                    profile.name.clone(),
                    profile.forge,
                    profile_host(profile),
                    token,
                ))
            })
            .collect()
    };
    let projects = projects_by_slug(state);
    let mut out = Inbox::default();
    let Ok(http) = client() else {
        return out;
    };

    // Two profiles on one account would list everything twice.
    let mut seen: Vec<String> = Vec::new();
    for (name, kind, host, token) in accounts {
        if seen.contains(&token) {
            continue;
        }
        seen.push(token.clone());
        let found = match kind {
            ForgeKind::GitHub => github_inbox(&http, &host, &token).await,
            ForgeKind::GitLab => gitlab_inbox(&http, &host, &token).await,
            ForgeKind::None => continue,
        };
        match found {
            Ok(pairs) => {
                for (requested, repo, review) in pairs {
                    let Some(kind) = inbox_kind(requested, &review) else {
                        continue;
                    };
                    // The same review can be yours and waiting on you.
                    if out.rows.iter().any(|row| {
                        row.repo == repo && row.review.number == review.number && row.host == host
                    }) {
                        continue;
                    }
                    out.rows.push(InboxRow {
                        kind: kind.to_string(),
                        path: projects.get(&slug_key(&host, &repo)).cloned(),
                        host: host.clone(),
                        repo,
                        profile: name.clone(),
                        review,
                    });
                }
            }
            Err(error) => out.errors.push(format!("{name} ({host}): {error}")),
        }
    }
    out
}

/// GitHub's two searches, in one request: waiting on me, and mine.
async fn github_inbox(
    http: &reqwest::Client,
    host: &str,
    token: &str,
) -> Result<Vec<(bool, String, Review)>, String> {
    let query = format!(
        "query($asked:String!,$mine:String!){{\
         asked: search(query:$asked,type:ISSUE,first:20){{nodes{{... on PullRequest{{{GITHUB_FIELDS} {GITHUB_STANDING}}}}}}} \
         mine: search(query:$mine,type:ISSUE,first:30){{nodes{{... on PullRequest{{{GITHUB_FIELDS} {GITHUB_STANDING}}}}}}}}}"
    );
    let data = github_graphql(
        http,
        token,
        host,
        &query,
        serde_json::json!({
            "asked": "is:open is:pr archived:false review-requested:@me",
            "mine": "is:open is:pr archived:false author:@me"
        }),
    )
    .await?;
    let mut out = Vec::new();
    for (list, requested) in [("asked", true), ("mine", false)] {
        for node in data
            .pointer(&format!("/{list}/nodes"))
            .and_then(|v| v.as_array())
            .unwrap_or(&Vec::new())
        {
            if node.get("number").is_none() {
                continue;
            }
            let repo = string(node, &["baseRepository", "nameWithOwner"]);
            out.push((requested, repo, github_found_review(node, None)));
        }
    }
    Ok(out)
}

/// GitLab's two lists, in one request: review requested from me, and mine.
async fn gitlab_inbox(
    http: &reqwest::Client,
    host: &str,
    token: &str,
) -> Result<Vec<(bool, String, Review)>, String> {
    let query = format!(
        "query{{currentUser{{\
         asked: reviewRequestedMergeRequests(state:opened,first:20){{nodes{{{GITLAB_FIELDS} {GITLAB_STANDING}}}}} \
         mine: authoredMergeRequests(state:opened,first:30){{nodes{{{GITLAB_FIELDS} {GITLAB_STANDING}}}}}}}}}"
    );
    let data = graphql(
        http,
        token,
        &gitlab_graphql_url(host),
        &query,
        serde_json::json!({}),
    )
    .await?;
    let mut out = Vec::new();
    for (list, requested) in [("asked", true), ("mine", false)] {
        for node in data
            .pointer(&format!("/currentUser/{list}/nodes"))
            .and_then(|v| v.as_array())
            .unwrap_or(&Vec::new())
        {
            let repo = string(node, &["project", "fullPath"]);
            out.push((requested, repo, gitlab_found_review(node)));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(checks: Option<&str>, state: Option<&str>) -> Review {
        Review {
            number: 1,
            title: String::new(),
            author: String::new(),
            state: "open".into(),
            draft: false,
            source_branch: "x".into(),
            target_branch: "main".into(),
            url: String::new(),
            updated_at: String::new(),
            is_current: false,
            head_sha: String::new(),
            source: None,
            warning: None,
            checks: checks.map(str::to_string),
            review_state: state.map(str::to_string),
        }
    }

    #[test]
    fn only_the_stuck_ones_of_your_own_need_you() {
        assert_eq!(inbox_kind(true, &review(None, None)), Some("requested"));
        assert_eq!(
            inbox_kind(false, &review(Some("failure"), Some("approved"))),
            Some("failing")
        );
        assert_eq!(
            inbox_kind(false, &review(Some("success"), Some("changes_requested"))),
            Some("changes")
        );
        assert_eq!(inbox_kind(false, &review(Some("pending"), None)), None);
        assert_eq!(
            inbox_kind(false, &review(Some("success"), Some("approved"))),
            None
        );
    }

    #[test]
    fn reads_both_forges_standing() {
        let (checks, decision) = github_standing(&serde_json::json!({
            "reviewDecision": null,
            "latestOpinionatedReviews": { "nodes": [{ "state": "APPROVED" }] },
            "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "state": "FAILURE" } } }] }
        }));
        assert_eq!(checks.as_deref(), Some("failure"));
        assert_eq!(
            decision.as_deref(),
            Some("approved"),
            "an approval counts where the repository does not require one"
        );

        let (checks, decision) = github_standing(&serde_json::json!({
            "reviewDecision": "REVIEW_REQUIRED",
            "latestOpinionatedReviews": { "nodes": [{ "state": "CHANGES_REQUESTED" }] },
            "commits": { "nodes": [{ "commit": { "statusCheckRollup": null } }] }
        }));
        assert_eq!(checks.as_deref(), Some("none"));
        assert_eq!(decision.as_deref(), Some("changes_requested"));

        let (_, decision) =
            github_standing(&serde_json::json!({ "reviewDecision": "REVIEW_REQUIRED" }));
        assert_eq!(decision.as_deref(), Some("review_required"));
        assert_eq!(github_rollup("EXPECTED"), "pending");
        assert_eq!(github_rollup("ERROR"), "failure");

        let (checks, decision) = gitlab_standing(&serde_json::json!({
            "iid": "4", "headPipeline": { "status": "RUNNING" },
            "approvedBy": { "nodes": [] }, "detailedMergeStatus": "NOT_APPROVED"
        }));
        assert_eq!(checks.as_deref(), Some("pending"));
        assert_eq!(decision.as_deref(), Some("review_required"));
        let (checks, decision) = gitlab_standing(&serde_json::json!({
            "headPipeline": null, "approvedBy": { "nodes": [{ "username": "kai" }] },
            "detailedMergeStatus": "MERGEABLE"
        }));
        assert_eq!(checks.as_deref(), Some("none"));
        assert_eq!(decision.as_deref(), Some("approved"));
        assert_eq!(gitlab_pipeline("failed"), "failure");
        assert_eq!(gitlab_pipeline("SUCCESS"), "success");
        assert_eq!(gitlab_pipeline("canceled"), "none");
    }

    #[test]
    fn reads_a_merge_request_from_gitlab_graphql() {
        let node = serde_json::json!({
            "iid": "38", "title": "Fix it", "state": "opened", "draft": false,
            "webUrl": "https://gitlab.com/acme/app/-/merge_requests/38",
            "sourceBranch": "fix", "targetBranch": "main", "diffHeadSha": "abc",
            "author": { "username": "arno" },
            "project": { "fullPath": "acme/app" },
            "sourceProject": {
                "fullPath": "arno/app", "sshUrlToRepo": "git@gitlab.com:arno/app.git",
                "httpUrlToRepo": "https://gitlab.com/arno/app.git", "namespace": { "path": "arno" }
            },
            "headPipeline": { "status": "FAILED" }, "approvedBy": { "nodes": [] },
            "detailedMergeStatus": "REQUESTED_CHANGES"
        });
        let review = gitlab_found_review(&node);
        assert_eq!(review.number, 38);
        assert_eq!(review.head_sha, "abc");
        assert_eq!(review.checks.as_deref(), Some("failure"));
        assert_eq!(review.review_state.as_deref(), Some("changes_requested"));
        let source = review.source.expect("a source");
        assert!(source.is_fork);
        assert_eq!(source.owner, "arno");

        // From the project itself: nothing to add to reach it.
        let mut same = node.clone();
        same["sourceProject"]["fullPath"] = serde_json::json!("acme/app");
        let source = gitlab_found_review(&same).source.expect("a source");
        assert!(!source.is_fork);
        assert!(source.full_name.is_empty());
    }
}
