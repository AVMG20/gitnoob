//! The parts that are GitHub's alone.
//!
//! Its shape of a review, its check runs and statuses, its held-back review
//! comments, and the one GraphQL call REST has never offered an equivalent of.

use super::*;

pub(super) fn github_review(item: &serde_json::Value, current: Option<&str>) -> Review {
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
            && head_from
                .as_ref()
                .map(|from| !from.is_fork)
                .unwrap_or(false),
        source_branch: source,
        target_branch: string(item, &["base", "ref"]),
        url: string(item, &["html_url"]),
        updated_at: string(item, &["updated_at"]),
        head_sha: string(item, &["head", "sha"]),
        source: head_from,
        warning: None,
    }
}

/// Hands a freshly opened pull request to its people.
///
/// GitHub takes neither assignees nor reviewers when the pull request is
/// created: assignees belong to the issue underneath it, reviewers to an
/// endpoint of their own. Both run after the fact, so a failure here leaves a
/// pull request that exists and is only missing its names. That is reported as
/// a warning rather than raised as an error, which would suggest nothing
/// happened and invite a second attempt the forge would refuse.
pub(super) async fn github_people(
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

/// The held-back remarks in the shape GitHub takes them alongside a verdict.
pub(super) fn github_pending_comments(comments: &[PendingComment]) -> serde_json::Value {
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

/// One GitHub review comment — the kind anchored to a line of the diff.
pub(super) fn github_diff_comment(item: &serde_json::Value) -> ReviewComment {
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
pub(super) fn github_issue_comment(item: &serde_json::Value) -> ReviewComment {
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

/// GitHub's own word for a check run, in the five this app draws.
pub(super) fn github_check_state(status: &str, conclusion: &str) -> &'static str {
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
pub(super) fn github_status_state(state: &str) -> &'static str {
    match state {
        "success" => "success",
        "failure" | "error" => "failure",
        "pending" => "pending",
        _ => "skipped",
    }
}

/// The standing verdicts, one per person, out of GitHub's list of reviews.
///
/// They arrive oldest first and one person can leave several. The last
/// position they took is the one that stands; a passing comment afterwards
/// does not unseat the approval they already gave, and a pending review is one
/// they have not sent yet.
pub(super) fn fold_github_verdicts(items: &[serde_json::Value]) -> Vec<Verdict> {
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
pub(super) fn github_graphql_url(host: &str) -> String {
    if host == "github.com" {
        "https://api.github.com/graphql".to_string()
    } else {
        format!("https://{host}/api/graphql")
    }
}

/// Asks GitHub's GraphQL endpoint one question.
pub(super) async fn github_graphql(
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
pub(super) async fn github_threads(
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
            let resolved = node
                .get("isResolved")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let outdated = node
                .get("isOutdated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
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
