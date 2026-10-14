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
        checks: None,
        review_state: None,
    }
}

/// A pull request as GitHub's GraphQL search returns it, in the same shape
/// `github_review` reads from REST.
pub(super) fn github_found_review(node: &serde_json::Value, current: Option<&str>) -> Review {
    let source = string(node, &["headRefName"]);
    let base_repo = string(node, &["baseRepository", "nameWithOwner"]);
    let head_repo = string(node, &["headRepository", "nameWithOwner"]);
    let standing = if node.get("commits").is_some() || node.get("reviewDecision").is_some() {
        github_standing(node)
    } else {
        (None, None)
    };
    let head_from = (!head_repo.is_empty()).then(|| ReviewSource {
        owner: string(node, &["headRepository", "owner", "login"]),
        is_fork: head_repo != base_repo,
        ssh_url: string(node, &["headRepository", "sshUrl"]),
        https_url: format!("{}.git", string(node, &["headRepository", "url"])),
        full_name: head_repo,
    });
    Review {
        number: node.get("number").and_then(|v| v.as_i64()).unwrap_or(0),
        title: string(node, &["title"]),
        author: string(node, &["author", "login"]),
        // REST says `open`, GraphQL `OPEN`.
        state: string(node, &["state"]).to_lowercase(),
        draft: node
            .get("isDraft")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        is_current: current == Some(source.as_str())
            && head_from
                .as_ref()
                .map(|from| !from.is_fork)
                .unwrap_or(false),
        source_branch: source,
        target_branch: string(node, &["baseRefName"]),
        url: string(node, &["url"]),
        updated_at: string(node, &["updatedAt"]),
        head_sha: string(node, &["headRefOid"]),
        source: head_from,
        warning: None,
        // Only a search that asked for the standing carries it.
        checks: standing.0,
        review_state: standing.1,
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
    graphql(http, token, &github_graphql_url(host), query, variables).await
}

/// One GraphQL question to whichever forge answers at `url`.
pub(super) async fn graphql(
    http: &reqwest::Client,
    token: &str,
    url: &str,
    query: &str,
    variables: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let response = http
        .post(url)
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

/// What GraphQL is asked about a pull request to say how it stands: the
/// checks on its last commit and where the reviewing is.
pub(super) const GITHUB_STANDING: &str = "reviewDecision \
    latestOpinionatedReviews(first:20){nodes{state}} \
    commits(last:1){nodes{commit{statusCheckRollup{state}}}}";

/// GitHub's rolled-up check state in this app's four words.
pub(super) fn github_rollup(state: &str) -> &'static str {
    match state {
        "SUCCESS" => "success",
        "FAILURE" | "ERROR" => "failure",
        "PENDING" | "EXPECTED" => "pending",
        _ => "none",
    }
}

/// How a pull request stands, from a node carrying `GITHUB_STANDING`.
///
/// `reviewDecision` is only filled in where the repository requires reviews;
/// everywhere else it stays empty however many people approved, so the latest
/// opinion of each reviewer is read instead: one request for changes is the
/// answer, then any approval.
pub(super) fn github_standing(node: &serde_json::Value) -> (Option<String>, Option<String>) {
    let checks = node
        .pointer("/commits/nodes/0/commit/statusCheckRollup/state")
        .and_then(|v| v.as_str())
        .map(github_rollup)
        .unwrap_or("none")
        .to_string();
    let decision = match string(node, &["reviewDecision"]).as_str() {
        "APPROVED" => Some("approved"),
        "CHANGES_REQUESTED" => Some("changes_requested"),
        other => {
            let states: Vec<String> = node
                .pointer("/latestOpinionatedReviews/nodes")
                .and_then(|v| v.as_array())
                .map(|list| list.iter().map(|one| string(one, &["state"])).collect())
                .unwrap_or_default();
            if states.iter().any(|s| s == "CHANGES_REQUESTED") {
                Some("changes_requested")
            } else if other == "REVIEW_REQUIRED" {
                Some("review_required")
            } else if states.iter().any(|s| s == "APPROVED") {
                Some("approved")
            } else {
                None
            }
        }
    };
    (Some(checks), decision.map(str::to_string))
}

/// How each of a handful of pull requests stands, asked in one request with
/// an alias per number. A forge that will not answer leaves them unknown.
pub(super) async fn github_standings(
    http: &reqwest::Client,
    call: &Call,
    numbers: &[i64],
) -> HashMap<i64, (Option<String>, Option<String>)> {
    let mut out = HashMap::new();
    if numbers.is_empty() {
        return out;
    }
    let asks: Vec<String> = numbers
        .iter()
        .map(|n| format!("p{n}: pullRequest(number:{n}){{number {GITHUB_STANDING}}}"))
        .collect();
    let query = format!(
        "query($owner:String!,$name:String!){{repository(owner:$owner,name:$name){{{}}}}}",
        asks.join(" ")
    );
    let Ok(data) = github_graphql(
        http,
        &call.token,
        &call.host,
        &query,
        serde_json::json!({ "owner": call.slug.owner, "name": call.slug.name }),
    )
    .await
    else {
        return out;
    };
    for number in numbers {
        if let Some(node) = data.pointer(&format!("/repository/p{number}")) {
            if !node.is_null() {
                out.insert(*number, github_standing(node));
            }
        }
    }
    out
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
