//! What a forge hands back, in this app's own words.
//!
//! Every one of these crosses to the window as JSON, so the field names here
//! are the field names the frontend reads.

use serde::{Deserialize, Serialize};

use crate::config::ForgeKind;

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
