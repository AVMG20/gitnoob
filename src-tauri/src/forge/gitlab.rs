//! The parts that are GitLab's alone.
//!
//! Its shape of a merge request, its pipeline jobs, its notes and threads, and
//! the `Draft:` prefix it uses where GitHub has a flag.

use super::*;

/// A merge request, and the project id its branch lives in when that is not
/// the project being reviewed. GitLab gives forks as an id rather than as an
/// address, so the caller looks the address up separately.
pub(super) fn gitlab_review(
    item: &serde_json::Value,
    current: Option<&str>,
) -> (Review, Option<i64>) {
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
        checks: None,
        review_state: None,
    };
    (review, from_fork.then_some(source_project).flatten())
}

/// Where a forked GitLab project lives, so its branch can be fetched.
pub(super) async fn gitlab_project(
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

/// One GitLab note, when it is worth keeping.
///
/// Notes are the one list everything lives in over there: conversation
/// comments, system notices and diff-anchored ones alike, told apart by having
/// a `position` and by being marked `system`.
pub(super) fn gitlab_note(item: &serde_json::Value) -> Option<ReviewComment> {
    if item
        .get("system")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let position = item.get("position");
    let new_line = position
        .and_then(|p| p.get("new_line"))
        .and_then(|v| v.as_i64());
    let old_line = position
        .and_then(|p| p.get("old_line"))
        .and_then(|v| v.as_i64());
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
        resolved: item
            .get("resolved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        // GitLab marks the position itself when the line has moved out from
        // under the remark.
        outdated: position
            .and_then(|p| p.get("outdated"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

pub(super) fn is_system_note(item: &serde_json::Value) -> bool {
    item.get("system")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Answers GitLab notes to the root of their thread.
///
/// Every note of a thread shares a discussion id; walking the notes in order,
/// the first note seen under each id is its opening remark and every later one
/// answers it. The kept comments ride along in the same order as their raw
/// notes, so pairing the two walks stays aligned.
pub(super) fn gitlab_thread_replies(
    comments: &mut [ReviewComment],
    notes_in_order: &[serde_json::Value],
) {
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
pub(super) fn gitlab_file_status(new_file: bool, deleted: bool, renamed: bool) -> &'static str {
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

/// The same again, for a GitLab job.
pub(super) fn gitlab_job_state(status: &str) -> &'static str {
    match status {
        "success" => "success",
        "failed" => "failure",
        "canceled" | "canceling" => "cancelled",
        "skipped" | "manual" => "skipped",
        // created, pending, running, preparing, scheduled, waiting_for_resource
        _ => "pending",
    }
}

/// The title a GitLab merge request wears for the draft it is or is not.
///
/// GitLab has no draft flag to set: a merge request is a draft because its
/// title begins with the word, so marking one ready is a rename.
pub(super) fn gitlab_draft_title(title: &str, draft: bool) -> String {
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

/// Where GitLab answers GraphQL.
pub(super) fn gitlab_graphql_url(host: &str) -> String {
    format!("https://{host}/api/graphql")
}

/// What GraphQL is asked about a merge request to say how it stands.
pub(super) const GITLAB_STANDING: &str =
    "iid headPipeline{status} approvedBy{nodes{username}} detailedMergeStatus";

/// GitLab's pipeline status in this app's four words. GraphQL shouts it and
/// REST does not, so either case is read.
pub(super) fn gitlab_pipeline(status: &str) -> &'static str {
    match status.to_ascii_lowercase().as_str() {
        "" => "none",
        "success" => "success",
        "failed" => "failure",
        "canceled" | "canceling" | "skipped" | "manual" => "none",
        // created, pending, running, preparing, scheduled, waiting_for_resource
        _ => "pending",
    }
}

/// How a merge request stands, from a node carrying `GITLAB_STANDING`.
///
/// Changes requested shows up in the detailed merge status where the install
/// has the verdict at all. Approved means somebody approved, not that the
/// rules are met: a project that needs no approvals would otherwise read as
/// approved by nobody, which is not news worth a badge.
pub(super) fn gitlab_standing(node: &serde_json::Value) -> (Option<String>, Option<String>) {
    let checks = gitlab_pipeline(&string(node, &["headPipeline", "status"])).to_string();
    let approvals = node
        .pointer("/approvedBy/nodes")
        .and_then(|v| v.as_array())
        .map(|list| list.len())
        .unwrap_or(0);
    let decision = match string(node, &["detailedMergeStatus"]).as_str() {
        "REQUESTED_CHANGES" => Some("changes_requested"),
        _ if approvals > 0 => Some("approved"),
        "NOT_APPROVED" => Some("review_required"),
        _ => None,
    };
    (Some(checks), decision.map(str::to_string))
}

/// How each of a handful of merge requests stands, in one request.
pub(super) async fn gitlab_standings(
    http: &reqwest::Client,
    call: &Call,
    numbers: &[i64],
) -> HashMap<i64, (Option<String>, Option<String>)> {
    let mut out = HashMap::new();
    if numbers.is_empty() {
        return out;
    }
    let query = format!(
        "query($path:ID!,$iids:[String!]){{project(fullPath:$path){{mergeRequests(iids:$iids){{nodes{{{GITLAB_STANDING}}}}}}}}}"
    );
    let iids: Vec<String> = numbers.iter().map(|n| n.to_string()).collect();
    let Ok(data) = graphql(
        http,
        &call.token,
        &gitlab_graphql_url(&call.host),
        &query,
        serde_json::json!({ "path": call.slug.full(), "iids": iids }),
    )
    .await
    else {
        return out;
    };
    for node in data
        .pointer("/project/mergeRequests/nodes")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
    {
        let iid = string(node, &["iid"]).parse::<i64>().unwrap_or(0);
        if iid > 0 {
            out.insert(iid, gitlab_standing(node));
        }
    }
    out
}

/// A merge request as GitLab's GraphQL describes it, in the shape `Review`
/// takes everywhere else.
pub(super) fn gitlab_found_review(node: &serde_json::Value) -> Review {
    let project = string(node, &["project", "fullPath"]);
    let source_project = string(node, &["sourceProject", "fullPath"]);
    let fork = !source_project.is_empty() && !source_project.eq_ignore_ascii_case(&project);
    let (checks, review_state) = gitlab_standing(node);
    let from_fork = |path: &[&str]| {
        if fork {
            string(node, path)
        } else {
            String::new()
        }
    };
    Review {
        number: string(node, &["iid"]).parse().unwrap_or(0),
        title: string(node, &["title"]),
        author: string(node, &["author", "username"]),
        state: string(node, &["state"]),
        draft: node.get("draft").and_then(|v| v.as_bool()).unwrap_or(false),
        is_current: false,
        source_branch: string(node, &["sourceBranch"]),
        target_branch: string(node, &["targetBranch"]),
        url: string(node, &["webUrl"]),
        updated_at: string(node, &["updatedAt"]),
        head_sha: string(node, &["diffHeadSha"]),
        source: Some(ReviewSource {
            full_name: from_fork(&["sourceProject", "fullPath"]),
            owner: from_fork(&["sourceProject", "namespace", "path"]),
            ssh_url: from_fork(&["sourceProject", "sshUrlToRepo"]),
            https_url: from_fork(&["sourceProject", "httpUrlToRepo"]),
            is_fork: fork,
        }),
        warning: None,
        checks,
        review_state,
    }
}
